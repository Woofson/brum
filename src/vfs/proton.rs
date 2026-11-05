//! Proton Drive through Proton's official command-line program (`proton-drive`).
//!
//! * The program is found in `[proton] cli_path`, in Brum's data folder (where the Install button
//!   puts it), or on the system (`PATH`, `~/.local/bin`, `/usr/local/bin`, `/opt`).
//! * Install and Update read Proton's own release list (`version.json`), pick the build for this
//!   machine, download it from proton.me and check its SHA-512 fingerprint before keeping it.
//! * Each Brum user has a private Proton login in `<data>/proton/users/<name>/`, kept by the
//!   program itself in its file store (`PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file`), so it works
//!   for a background service and in Docker, where there is no desktop keyring.
//! * Signing in runs `proton-drive auth login`, which prints a link to open on any device and then
//!   waits until the sign-in is finished.

use crate::vfs::{FileEntry, VfsResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Where Proton publishes its releases (also used by the program for its own update check)
const RELEASES_URL: &str = "https://proton.me/download/drive/cli/version.json";
const DOWNLOAD_PREFIX: &str = "https://proton.me/download/drive/cli/";
pub const DOWNLOAD_PAGE: &str = "https://proton.me/download/drive/cli/index.html";
#[cfg(windows)]
const BIN_NAME: &str = "proton-drive.exe";
#[cfg(not(windows))]
const BIN_NAME: &str = "proton-drive";

/// `[proton]` in config.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtonConfig {
    /// A program to use instead of searching for one
    #[serde(default)]
    pub cli_path: String,
    /// Whether admins may download the program from proton.me with the Install button
    #[serde(default = "default_true")]
    pub allow_download: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ProtonConfig {
    fn default() -> Self {
        Self { cli_path: String::new(), allow_download: true }
    }
}

struct Runtime {
    data_dir: PathBuf,
    config: ProtonConfig,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Called once at start-up with the database path, so Proton files live next to it
pub fn init(database_path: &str, config: &ProtonConfig) {
    let parent = Path::new(database_path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let data_dir = if parent.as_os_str().is_empty() { std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")) } else { parent };
    let _ = RUNTIME.set(Runtime { data_dir, config: config.clone() });
}

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime { data_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")), config: ProtonConfig::default() })
}

fn proton_dir() -> PathBuf {
    runtime().data_dir.join("proton")
}

/// Where the Install button puts the program
pub fn installed_bin_path() -> PathBuf {
    proton_dir().join("bin").join(BIN_NAME)
}

/// A folder name that cannot step outside `users/`
fn safe_user_dir_name(user: &str) -> String {
    let ok = !user.is_empty() && user.len() <= 64 && user.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') && !user.starts_with('.');
    if ok { user.to_string() } else { format!("u-{}", hex::encode(user.as_bytes())) }
}

/// The private folder holding this Brum user's Proton login and caches
pub fn user_home(user: &str) -> PathBuf {
    proton_dir().join("users").join(safe_user_dir_name(user))
}

/// Creates the user's folder, keeping `proton/` and `proton/users/` private to Brum as well
fn ensure_user_home(user: &str) -> std::io::Result<PathBuf> {
    ensure_private_dir(&proton_dir())?;
    ensure_private_dir(&proton_dir().join("users"))?;
    let home = user_home(user);
    ensure_private_dir(&home)?;
    Ok(home)
}

fn ensure_private_dir(p: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(p)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn session_file(user: &str) -> PathBuf {
    user_home(user).join(".local/share/proton-drive-cli/auth-session.json")
}

fn account_file(user: &str) -> PathBuf {
    user_home(user).join("brum-account.json")
}

// ---------------------------------------------------------------- finding the program

#[derive(Debug, Clone, Serialize)]
pub struct CliInfo {
    pub path: String,
    /// "config", "brum" (installed with the Install button) or "system"
    pub source: String,
    pub version: Option<String>,
}

fn is_executable(p: &Path) -> bool {
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        _ => false,
    }
}

/// Every place the program may be, best first
fn candidates() -> Vec<(PathBuf, &'static str)> {
    let mut out = Vec::new();
    let cfg = runtime().config.cli_path.trim();
    if !cfg.is_empty() {
        out.push((PathBuf::from(cfg), "config"));
    }
    out.push((installed_bin_path(), "brum"));
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            out.push((dir.join(BIN_NAME), "system"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        out.push((home.join(".local/bin").join(BIN_NAME), "system"));
    }
    for d in ["/usr/local/bin", "/usr/bin", "/opt/proton-drive", "/opt/proton-drive-cli"] {
        out.push((Path::new(d).join(BIN_NAME), "system"));
    }
    out
}

pub fn find_cli() -> Option<(PathBuf, String)> {
    candidates().into_iter().find(|(p, _)| is_executable(p)).map(|(p, s)| (p, s.to_string()))
}

/// "Proton Drive CLI cli-drive@0.9.0+75612299" → "0.9.0"
fn parse_version(text: &str) -> Option<String> {
    let idx = text.find("cli-drive@")?;
    let rest = &text[idx + "cli-drive@".len()..];
    let v: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    if v.is_empty() { None } else { Some(v) }
}

/// Version of a program, remembered by path and file time so the slow start-up runs once
fn cli_version(path: &Path) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (std::time::SystemTime, Option<String>)>>> = OnceLock::new();
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((t, v)) = cache.lock().ok()?.get(path) {
        if *t == mtime {
            return v.clone();
        }
    }
    let scratch = proton_dir().join("version-check");
    let _ = ensure_private_dir(&proton_dir());
    let _ = ensure_private_dir(&scratch);
    let out = std::process::Command::new(path)
        .arg("--version")
        .envs(cli_env(&scratch))
        .current_dir(&scratch)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let v = parse_version(&String::from_utf8_lossy(&out.stdout)).or_else(|| parse_version(&String::from_utf8_lossy(&out.stderr)));
    if let Ok(mut c) = cache.lock() {
        c.insert(path.to_path_buf(), (mtime, v.clone()));
    }
    v
}

pub fn cli_info() -> Option<CliInfo> {
    let (path, source) = find_cli()?;
    let version = cli_version(&path);
    Some(CliInfo { path: path.to_string_lossy().to_string(), source, version })
}

/// The program keeps everything under HOME / XDG folders; point them at the user's private folder
fn cli_env(home: &Path) -> Vec<(String, String)> {
    let h = home.to_string_lossy().to_string();
    let mut v = vec![
        ("HOME".to_string(), h.clone()),
        ("XDG_CONFIG_HOME".to_string(), format!("{h}/.config")),
        ("XDG_DATA_HOME".to_string(), format!("{h}/.local/share")),
        ("XDG_STATE_HOME".to_string(), format!("{h}/.local/state")),
        ("XDG_CACHE_HOME".to_string(), format!("{h}/.cache")),
        ("PROTON_DRIVE_CREDENTIALS_STORE".to_string(), "unsafe_file".to_string()),
        ("NO_COLOR".to_string(), "1".to_string()),
    ];
    if cfg!(windows) {
        v.push(("USERPROFILE".to_string(), h.clone()));
        v.push(("APPDATA".to_string(), format!("{h}\\AppData\\Roaming")));
        v.push(("LOCALAPPDATA".to_string(), format!("{h}\\AppData\\Local")));
    }
    v
}

/// Runs the program for a Brum user and returns what it printed
pub async fn run_cli(user: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let (bin, _) = find_cli().ok_or_else(|| "The Proton Drive program is not installed. Set it up in Settings → Proton Drive.".to_string())?;
    let home = ensure_user_home(user).map_err(|e| format!("Cannot create the Proton folder for this user: {e}"))?;
    let mut cmd = Command::new(&bin);
    cmd.args(args).envs(cli_env(&home)).current_dir(&home).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| "Proton Drive did not answer in time".to_string())?
        .map_err(|e| format!("Could not start the Proton Drive program: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr.lines().chain(stdout.lines()).map(str::trim).find(|l| !l.is_empty() && !l.starts_with('{') && !l.starts_with('[')).unwrap_or("Proton Drive reported an error").to_string();
        if msg.contains("You need to login first") {
            Err("Not signed in to Proton Drive. Sign in in Settings → Proton Drive.".to_string())
        } else {
            Err(msg)
        }
    }
}

// ---------------------------------------------------------------- status

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AccountInfo {
    email: Option<String>,
    signed_in_at: Option<u64>,
}

fn read_account(user: &str) -> AccountInfo {
    std::fs::read_to_string(account_file(user)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write_account(user: &str, a: &AccountInfo) {
    if let Ok(s) = serde_json::to_string(a) {
        let _ = std::fs::write(account_file(user), s);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtonStatus {
    pub installed: bool,
    pub cli: Option<CliInfo>,
    pub signed_in: bool,
    pub account: Option<String>,
    pub allow_download: bool,
    pub install_path: String,
    pub download_page: String,
    /// The release name for this machine in Proton's list, if there is one
    pub platform: Option<String>,
    pub login: Option<LoginView>,
    pub install: InstallView,
}

pub fn status(user: &str) -> ProtonStatus {
    let cli = cli_info();
    let signed_in = session_file(user).is_file();
    let account = if signed_in { read_account(user).email } else { None };
    ProtonStatus {
        installed: cli.is_some(),
        cli,
        signed_in,
        account,
        allow_download: runtime().config.allow_download,
        install_path: installed_bin_path().to_string_lossy().to_string(),
        download_page: DOWNLOAD_PAGE.to_string(),
        platform: platform_name(),
        login: login_view(user),
        install: install_view(),
    }
}

/// Asks Proton who is signed in (slow, about two seconds) and remembers the answer
pub async fn refresh_account(user: &str) -> Result<String, String> {
    let out = run_cli(user, &["filesystem", "info", "-j", "/my-files"], Duration::from_secs(60)).await?;
    let v: serde_json::Value = serde_json::from_str(out.trim()).map_err(|_| "Unexpected answer from Proton Drive".to_string())?;
    let email = v["ownedBy"]["email"].as_str().or_else(|| v["keyAuthor"]["value"].as_str()).unwrap_or("").to_string();
    let mut a = read_account(user);
    a.email = if email.is_empty() { None } else { Some(email.clone()) };
    write_account(user, &a);
    Ok(email)
}

// ---------------------------------------------------------------- install and update

/// The name of this machine's build in Proton's list, e.g. "linux/x64" or "linux/arm64-musl"
pub fn platform_name() -> Option<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        "freebsd" => "freebsd",
        _ => return None,
    };
    let musl = os == "linux" && is_musl();
    Some(format!("{os}/{arch}{}", if musl { "-musl" } else { "" }))
}

/// Alpine and other musl systems (Brum's Docker image) need the musl build
fn is_musl() -> bool {
    if cfg!(target_env = "musl") {
        return true;
    }
    let glibc = ["/lib64/ld-linux-x86-64.so.2", "/lib/ld-linux-aarch64.so.1", "/lib/x86_64-linux-gnu/libc.so.6", "/lib/aarch64-linux-gnu/libc.so.6", "/usr/lib/libc.so.6"].iter().any(|p| Path::new(p).exists());
    let musl = ["/lib/ld-musl-x86_64.so.1", "/lib/ld-musl-aarch64.so.1"].iter().any(|p| Path::new(p).exists());
    musl && !glibc
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseList {
    #[serde(rename = "Releases")]
    releases: Vec<Release>,
}

#[derive(Debug, Clone, Deserialize)]
struct Release {
    #[serde(rename = "CategoryName")]
    category: String,
    #[serde(rename = "Version")]
    version: String,
    #[serde(rename = "ReleaseDate", default)]
    date: String,
    #[serde(rename = "Files")]
    files: Vec<ReleaseFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseFile {
    #[serde(rename = "Url")]
    url: String,
    #[serde(rename = "Sha512CheckSum")]
    sha512: String,
    #[serde(rename = "Platform")]
    platform: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatestRelease {
    pub version: String,
    pub date: String,
    pub platform: String,
    pub url: String,
    pub sha512: String,
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("Brum/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// The newest stable release for this machine
pub async fn latest_release() -> Result<LatestRelease, String> {
    let platform = platform_name().ok_or("Proton does not offer the program for this kind of computer")?;
    let list: ReleaseList = http_client()?
        .get(RELEASES_URL)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Could not reach proton.me: {e}"))?
        .error_for_status()
        .map_err(|e| format!("proton.me answered with an error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Could not read Proton's release list: {e}"))?;
    let rel = list.releases.iter().find(|r| r.category.eq_ignore_ascii_case("stable")).ok_or("Proton's release list has no stable release")?;
    let file = rel.files.iter().find(|f| f.platform == platform).ok_or_else(|| format!("Proton has no {platform} build of version {}", rel.version))?;
    if !file.url.starts_with(DOWNLOAD_PREFIX) {
        return Err("Proton's release list points outside proton.me; not downloading".to_string());
    }
    if file.sha512.len() != 128 || !file.sha512.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Proton's release list has no valid fingerprint for this build".to_string());
    }
    Ok(LatestRelease { version: rel.version.clone(), date: rel.date.clone(), platform, url: file.url.clone(), sha512: file.sha512.to_lowercase() })
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct InstallView {
    /// "idle", "downloading", "verifying", "done" or "failed"
    pub phase: String,
    pub version: Option<String>,
    pub received: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
}

fn install_state() -> &'static Mutex<InstallView> {
    static S: OnceLock<Mutex<InstallView>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(InstallView { phase: "idle".into(), ..Default::default() }))
}

pub fn install_view() -> InstallView {
    install_state().lock().map(|s| s.clone()).unwrap_or_default()
}

fn set_install(f: impl FnOnce(&mut InstallView)) {
    if let Ok(mut s) = install_state().lock() {
        f(&mut s);
    }
}

/// Starts downloading the newest release into Brum's data folder; progress is in `install_view`
pub fn start_install() -> Result<(), String> {
    if !runtime().config.allow_download {
        return Err("Downloading the Proton Drive program is turned off on this server ([proton] allow_download)".to_string());
    }
    {
        let mut s = install_state().lock().map_err(|_| "busy")?;
        if s.phase == "downloading" || s.phase == "verifying" {
            return Err("An install is already running".to_string());
        }
        *s = InstallView { phase: "downloading".into(), ..Default::default() };
    }
    tokio::spawn(async move {
        if let Err(e) = install().await {
            set_install(|s| {
                s.phase = "failed".into();
                s.error = Some(e);
            });
        }
    });
    Ok(())
}

async fn install() -> Result<(), String> {
    use futures_util::StreamExt;
    use sha2::{Digest, Sha512};
    use tokio::io::AsyncWriteExt;

    let rel = latest_release().await?;
    set_install(|s| s.version = Some(rel.version.clone()));
    let bin_dir = proton_dir().join("bin");
    let _ = ensure_private_dir(&proton_dir());
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("Cannot create {}: {e}", bin_dir.display()))?;
    let tmp = bin_dir.join(format!("{BIN_NAME}.download"));
    let resp = http_client()?
        .get(&rel.url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.content_length();
    set_install(|s| s.total = total);
    let mut file = tokio::fs::File::create(&tmp).await.map_err(|e| format!("Cannot write {}: {e}", tmp.display()))?;
    let mut hasher = Sha512::new();
    let mut received = 0u64;
    let mut stream = resp.bytes_stream();
    let mut last = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Download interrupted: {e}")
        })?;
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| format!("Cannot write the download: {e}"))?;
        received += chunk.len() as u64;
        if last.elapsed() > Duration::from_millis(200) {
            last = Instant::now();
            set_install(|s| s.received = received);
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    set_install(|s| {
        s.received = received;
        s.phase = "verifying".into();
    });
    let got = hex::encode(hasher.finalize());
    if got != rel.sha512 {
        let _ = std::fs::remove_file(&tmp);
        return Err("The download did not match Proton's fingerprint and was thrown away. Try again later.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, installed_bin_path()).map_err(|e| format!("Cannot put the program in place: {e}"))?;
    let installed = cli_version(&installed_bin_path());
    if installed.is_none() {
        return Err(format!(
            "The program was downloaded but does not start on this machine ({}).{}",
            rel.platform,
            if rel.platform.ends_with("-musl") { " On Alpine it needs the libstdc++ package." } else { "" }
        ));
    }
    set_install(|s| {
        s.phase = "done".into();
        s.version = installed;
    });
    Ok(())
}

// ---------------------------------------------------------------- signing in

#[derive(Debug, Clone, Serialize)]
pub struct LoginView {
    /// "starting", "waiting" (link shown), "done", "failed" or "cancelled"
    pub state: String,
    pub url: Option<String>,
    pub error: Option<String>,
    pub started_secs_ago: u64,
}

struct LoginSession {
    state: String,
    url: Option<String>,
    error: Option<String>,
    started: Instant,
    killer: Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
}

type Logins = Mutex<HashMap<String, Arc<Mutex<LoginSession>>>>;

fn logins() -> &'static Logins {
    static L: OnceLock<Logins> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn login_view(user: &str) -> Option<LoginView> {
    let map = logins().lock().ok()?;
    let s = map.get(user)?.lock().ok()?;
    Some(LoginView { state: s.state.clone(), url: s.url.clone(), error: s.error.clone(), started_secs_ago: s.started.elapsed().as_secs() })
}

/// Only the sign-in page Proton prints is passed on as a link
fn extract_login_url(text: &str) -> Option<String> {
    let start = text.find("https://account.proton.me/")?;
    let url: String = text[start..].chars().take_while(|c| !c.is_whitespace() && *c != '\u{1b}').collect();
    if url.len() > 30 { Some(url) } else { None }
}

/// Starts `proton-drive auth login` in a terminal of its own and collects the sign-in link
pub fn start_login(user: &str) -> Result<(), String> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    let (bin, _) = find_cli().ok_or("The Proton Drive program is not installed yet")?;
    if let Some(v) = login_view(user) {
        if v.state == "starting" || v.state == "waiting" {
            return Ok(());
        }
    }
    let home = ensure_user_home(user).map_err(|e| format!("Cannot create the Proton folder for this user: {e}"))?;
    let pair = native_pty_system().openpty(PtySize { rows: 30, cols: 400, pixel_width: 0, pixel_height: 0 }).map_err(|e| format!("Cannot start the sign-in: {e}"))?;
    let mut cmd = CommandBuilder::new(&bin);
    cmd.args(["auth", "login"]);
    cmd.env_clear();
    if let Some(p) = std::env::var_os("PATH") {
        cmd.env("PATH", p);
    }
    for (k, v) in cli_env(&home) {
        cmd.env(k, v);
    }
    cmd.env("TERM", "dumb");
    cmd.cwd(&home);
    let mut child = pair.slave.spawn_command(cmd).map_err(|e| format!("Cannot start the Proton Drive program: {e}"))?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let session = Arc::new(Mutex::new(LoginSession { state: "starting".into(), url: None, error: None, started: Instant::now(), killer: Some(child.clone_killer()) }));
    logins().lock().map_err(|_| "busy")?.insert(user.to_string(), session.clone());

    // Read what the program prints; the link arrives within a few seconds
    let s_read = session.clone();
    std::thread::spawn(move || {
        let mut all = String::new();
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    all.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if all.len() > 64 * 1024 {
                        all.drain(..32 * 1024);
                    }
                    if let Ok(mut s) = s_read.lock() {
                        if s.url.is_none() {
                            if let Some(u) = extract_login_url(&all) {
                                s.url = Some(u);
                                if s.state == "starting" {
                                    s.state = "waiting".into();
                                }
                            }
                        }
                        if all.contains("timed out") {
                            s.error = Some("The sign-in link expired. Start again.".into());
                        }
                    }
                }
            }
        }
        drop(pair.master);
    });

    // Wait for the program to finish: success means the login is saved
    let user_owned = user.to_string();
    std::thread::spawn(move || {
        let ok = child.wait().map(|st| st.success()).unwrap_or(false);
        let mut s = match session.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        s.killer = None;
        if s.state == "cancelled" {
            return;
        }
        if ok && session_file(&user_owned).is_file() {
            s.state = "done".into();
        } else {
            s.state = "failed".into();
            if s.error.is_none() {
                s.error = Some("The sign-in was not completed".into());
            }
        }
    });

    // Give up after 15 minutes so a forgotten sign-in does not run forever
    let s_timeout = session_weak(user);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(15 * 60));
        if let Some(s) = s_timeout.upgrade() {
            if let Ok(mut s) = s.lock() {
                if s.state == "starting" || s.state == "waiting" {
                    if let Some(mut k) = s.killer.take() {
                        let _ = k.kill();
                    }
                    s.state = "failed".into();
                    s.error = Some("The sign-in link expired. Start again.".into());
                }
            }
        }
    });
    Ok(())
}

fn session_weak(user: &str) -> std::sync::Weak<Mutex<LoginSession>> {
    logins().lock().ok().and_then(|m| m.get(user).map(Arc::downgrade)).unwrap_or_default()
}

pub fn cancel_login(user: &str) {
    if let Ok(map) = logins().lock() {
        if let Some(s) = map.get(user) {
            if let Ok(mut s) = s.lock() {
                if let Some(mut k) = s.killer.take() {
                    let _ = k.kill();
                }
                if s.state == "starting" || s.state == "waiting" {
                    s.state = "cancelled".into();
                }
            }
        }
    }
}

/// Forgets a finished sign-in so the window starts fresh next time
pub fn clear_login(user: &str) {
    if let Ok(mut map) = logins().lock() {
        let finished = map.get(user).and_then(|s| s.lock().ok().map(|s| s.state != "starting" && s.state != "waiting")).unwrap_or(false);
        if finished {
            map.remove(user);
        }
    }
}

/// Signs out with Proton, then removes the saved login and caches either way
pub async fn sign_out(user: &str) -> Result<(), String> {
    cancel_login(user);
    if find_cli().is_some() && session_file(user).is_file() {
        let _ = run_cli(user, &["auth", "logout"], Duration::from_secs(60)).await;
    }
    let home = user_home(user);
    for p in [session_file(user), account_file(user)] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_dir_all(home.join(".cache"));
    clear_login(user);
    Ok(())
}

// ---------------------------------------------------------------- reading folders

/// `proton:///my-files/Work` → `/my-files/Work`
pub fn cli_path_of(uri: &str) -> String {
    let rest = uri.trim_start_matches("proton://").trim_start_matches("PROTON://");
    let p = format!("/{}", rest.trim_matches('/'));
    if p == "/" { p } else { p.trim_end_matches('/').to_string() }
}

fn parse_time(s: Option<&str>) -> Option<u64> {
    s.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|d| d.timestamp().max(0) as u64)
}

/// Turns one entry of `filesystem list -j` into a file panel row
fn entry_from_json(parent_uri: &str, v: &serde_json::Value) -> Option<FileEntry> {
    let base = parent_uri.trim_end_matches('/');
    // Top-level sections only have a path
    if v.get("name").is_none() {
        let path = v["path"].as_str()?;
        let name = path.trim_start_matches('/').to_string();
        return Some(dir_entry(&name, format!("proton:///{name}"), None));
    }
    let name = v["name"]["value"].as_str().or_else(|| v["uid"].as_str())?.to_string();
    // A "/" inside a name is written as "\/" in the program's paths
    let escaped = name.replace('/', "\\/");
    let path = format!("{base}/{escaped}");
    let modified = parse_time(v["activeRevision"]["claimedModificationTime"].as_str()).or_else(|| parse_time(v["modificationTime"].as_str()));
    let kind = v["type"].as_str().unwrap_or("file");
    if kind == "folder" || kind == "album" {
        return Some(dir_entry(&name, path, modified));
    }
    let size = v["activeRevision"]["claimedSize"].as_u64().or_else(|| v["totalStorageSize"].as_u64()).unwrap_or(0);
    let lower = name.to_lowercase();
    Some(FileEntry {
        is_archive: lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".7z"),
        name,
        path,
        is_dir: false,
        is_symlink: false,
        is_empty: None,
        size,
        modified,
        permissions: "-rw-------".to_string(),
        mode_octal: "0600".to_string(),
        owner: v["ownedBy"]["email"].as_str().unwrap_or("proton").to_string(),
        group: "proton".to_string(),
        uid: 0,
        gid: 0,
        mime_type: v["mediaType"].as_str().map(|s| s.to_string()),
    })
}

fn dir_entry(name: &str, path: String, modified: Option<u64>) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        path,
        is_dir: true,
        is_symlink: false,
        is_empty: None,
        size: 0,
        modified,
        permissions: "drwx------".to_string(),
        mode_octal: "0700".to_string(),
        owner: "proton".to_string(),
        group: "proton".to_string(),
        uid: 0,
        gid: 0,
        mime_type: None,
        is_archive: false,
    }
}

pub struct ProtonDriveClient;

impl ProtonDriveClient {
    /// Lists a Proton Drive folder for a Brum user
    pub async fn list_directory(user: &str, uri: &str) -> VfsResult<Vec<FileEntry>> {
        let path = cli_path_of(uri);
        let out = run_cli(user, &["filesystem", "list", "-j", &path], Duration::from_secs(120)).await?;
        let items: Vec<serde_json::Value> = serde_json::from_str(out.trim()).map_err(|_| "Unexpected answer from Proton Drive".to_string())?;
        let parent = if path == "/" { "proton://".to_string() } else { format!("proton://{path}") };
        Ok(items.iter().filter_map(|v| entry_from_json(&parent, v)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_read_from_the_banner() {
        assert_eq!(parse_version("Proton Drive CLI cli-drive@0.9.0+75612299\nProton Drive SDK js@0.22.2"), Some("0.9.0".into()));
        assert_eq!(parse_version("nothing"), None);
    }

    #[test]
    fn user_folders_cannot_escape() {
        assert_eq!(safe_user_dir_name("bolt"), "bolt");
        assert_eq!(safe_user_dir_name("first.last-2"), "first.last-2");
        assert!(safe_user_dir_name("../etc").starts_with("u-"));
        assert!(safe_user_dir_name(".hidden").starts_with("u-"));
        assert!(safe_user_dir_name("DOMAIN\\user").starts_with("u-"));
        assert!(!safe_user_dir_name("a/b").contains('/'));
    }

    #[test]
    fn uri_maps_to_program_paths() {
        assert_eq!(cli_path_of("proton://"), "/");
        assert_eq!(cli_path_of("proton:///"), "/");
        assert_eq!(cli_path_of("proton:///my-files/Work/"), "/my-files/Work");
        assert_eq!(cli_path_of("proton://my-files"), "/my-files");
    }

    #[test]
    fn only_proton_sign_in_links_are_taken() {
        let text = "Sign in in your browser.\r\nOpen following URL manually:\r\nhttps://account.proton.me/desktop/login?app=drive&pv=3#payload=abc%3Ddef\r\n";
        assert_eq!(extract_login_url(text).as_deref(), Some("https://account.proton.me/desktop/login?app=drive&pv=3#payload=abc%3Ddef"));
        // Whatever surrounds it, only the part starting at Proton's sign-in site is used
        assert!(extract_login_url("see https://evil.example/x https://account.proton.me/desktop/login?app=drive#payload=1").unwrap().starts_with("https://account.proton.me/"));
        assert_eq!(extract_login_url("https://evil.example/login?app=drive"), None);
        assert_eq!(extract_login_url("no link here"), None);
    }

    #[test]
    fn listing_entries_become_rows() {
        let section = serde_json::json!({"path": "/my-files"});
        let e = entry_from_json("proton://", &section).unwrap();
        assert!(e.is_dir);
        assert_eq!(e.path, "proton:///my-files");

        let file = serde_json::json!({
            "name": {"ok": true, "value": "a/b.txt"}, "type": "file", "mediaType": "text/plain",
            "ownedBy": {"email": "me@proton.me"}, "modificationTime": "2026-11-05T10:13:00.000Z",
            "totalStorageSize": 999, "activeRevision": {"claimedSize": 11, "claimedModificationTime": "2026-11-01T08:00:00.000Z"}
        });
        let e = entry_from_json("proton:///my-files/Brum-dev", &file).unwrap();
        assert_eq!(e.name, "a/b.txt");
        assert_eq!(e.path, "proton:///my-files/Brum-dev/a\\/b.txt");
        assert_eq!(e.size, 11);
        assert_eq!(e.modified, Some(chrono::DateTime::parse_from_rfc3339("2026-11-01T08:00:00Z").unwrap().timestamp() as u64));
        assert_eq!(e.owner, "me@proton.me");

        let folder = serde_json::json!({"name": {"ok": true, "value": "Work"}, "type": "folder", "modificationTime": "2026-08-24T05:52:36.000Z"});
        assert!(entry_from_json("proton:///my-files", &folder).unwrap().is_dir);
    }

    #[test]
    fn platform_names_match_protons_list() {
        if let Some(p) = platform_name() {
            assert!(["linux/x64", "linux/x64-musl", "linux/arm64", "linux/arm64-musl", "macos/x64", "macos/arm64", "windows/x64", "windows/arm64", "freebsd/x64", "freebsd/arm64"].contains(&p.as_str()), "{p}");
        }
    }
}
