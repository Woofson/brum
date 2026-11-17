//! Cloud storage through rclone (`rclone://remote/folder`), for panes, copies and settings (#189).
//!
//! * The program is found in `[rclone] cli_path`, in Brum's data folder (where the Install button
//!   puts it), or on the system (`PATH`).
//! * Install and Update read rclone's current version from downloads.rclone.org, download the build
//!   for this machine and check its SHA-256 against rclone's published `SHA256SUMS` before keeping it.
//! * Remotes live in rclone's settings file. A shared install (the Linux service, Docker, the
//!   Windows service) keeps its own file in Brum's data folder, readable by Brum only; Brum started
//!   by an ordinary user uses that user's normal rclone file, shared with `rclone config`.
//! * One rclone helper (`rclone rcd`) serves all panes on a private local port with a random
//!   password; it is restarted when the program or the settings file changes.

use crate::vfs::{DirectoryListing, FileEntry, VfsResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const DOWNLOADS: &str = "https://downloads.rclone.org";
pub const DOWNLOAD_PAGE: &str = "https://rclone.org/downloads/";
#[cfg(windows)]
const BIN_NAME: &str = "rclone.exe";
#[cfg(not(windows))]
const BIN_NAME: &str = "rclone";

/// `[rclone]` in config.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RcloneConfig {
    /// A program to use instead of searching for one
    #[serde(default)]
    pub cli_path: String,
    /// Whether admins may download rclone from rclone.org with the Install button
    #[serde(default = "default_true")]
    pub allow_download: bool,
}

fn default_true() -> bool {
    true
}

impl Default for RcloneConfig {
    fn default() -> Self {
        Self { cli_path: String::new(), allow_download: true }
    }
}

struct Runtime {
    data_dir: PathBuf,
    config: RcloneConfig,
    /// Keep remotes in Brum's own settings file (shared installs) instead of the user's
    own_config: bool,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Called once at start-up with the database path, so rclone's files live next to it.
/// `own_config`: a shared install (service, Docker) that keeps its own rclone settings file.
pub fn init(database_path: &str, config: &RcloneConfig, own_config: bool) {
    let parent = Path::new(database_path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let data_dir = if parent.as_os_str().is_empty() { std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")) } else { parent };
    if RUNTIME.set(Runtime { data_dir, config: config.clone(), own_config }).is_ok() && own_config {
        adopt_root_config();
    }
}

/// Whether this Brum should keep its own rclone settings file: the Linux system service and Docker
pub fn wants_own_config() -> bool {
    #[cfg(not(windows))]
    {
        crate::config::system_data_dir().is_some() || crate::config::in_container()
    }
    #[cfg(windows)]
    {
        false
    }
}

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime { data_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")), config: RcloneConfig::default(), own_config: false })
}

fn rclone_dir() -> PathBuf {
    runtime().data_dir.join("rclone")
}

/// Where the Install button puts the program
pub fn installed_bin_path() -> PathBuf {
    rclone_dir().join("bin").join(BIN_NAME)
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

/// Brum's own rclone settings file, for shared installs; None: rclone's normal file for this user
pub fn config_file() -> Option<PathBuf> {
    runtime().own_config.then(|| rclone_dir().join("rclone.conf"))
}

/// Makes sure Brum's own settings file exists and only Brum can read it (it holds logins)
fn prepare_config_file() {
    let Some(file) = config_file() else { return };
    let _ = ensure_private_dir(&rclone_dir());
    if !file.exists() {
        let _ = std::fs::write(&file, b"");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
    }
}

/// Earlier versions used the rclone remotes of the account Brum runs as (root for the Linux
/// service, Brum's user in Docker). On the first start with its own file, Brum takes those over
/// once, so existing backups to rclone places keep working.
fn adopt_root_config() {
    let Some(file) = config_file() else { return };
    if file.exists() {
        return;
    }
    let old = std::env::var_os("RCLONE_CONFIG").map(PathBuf::from).or_else(|| dirs::config_dir().map(|d| d.join("rclone").join("rclone.conf")));
    let Some(old) = old.filter(|p| p.is_file()) else { return };
    prepare_config_file();
    match std::fs::copy(&old, &file) {
        Ok(_) => {
            prepare_config_file();
            tracing::info!("rclone: copied the remotes in {} to Brum's own rclone settings {}", old.display(), file.display());
        }
        Err(e) => tracing::warn!("rclone: could not copy {}: {e}", old.display()),
    }
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
    for d in ["/usr/local/bin", "/usr/bin"] {
        out.push((Path::new(d).join(BIN_NAME), "system"));
    }
    out
}

pub fn find_cli() -> Option<(PathBuf, String)> {
    candidates().into_iter().find(|(p, _)| is_executable(p)).map(|(p, s)| (p, s.to_string()))
}

/// "rclone v1.60.1-DEV" → "1.60.1-DEV"
fn parse_version(text: &str) -> Option<String> {
    let line = text.lines().next()?.trim();
    let v = line.strip_prefix("rclone v").or_else(|| line.strip_prefix("rclone "))?;
    let v = v.split_whitespace().next()?.trim_start_matches('v');
    (!v.is_empty()).then(|| v.to_string())
}

/// Version of a program, remembered by path and file time
fn cli_version(path: &Path) -> Option<String> {
    type Versions = Mutex<std::collections::HashMap<PathBuf, (SystemTime, Option<String>)>>;
    static CACHE: OnceLock<Versions> = OnceLock::new();
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some((t, v)) = cache.lock().ok()?.get(path) {
        if *t == mtime {
            return v.clone();
        }
    }
    let out = Command::new(path).arg("version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let v = parse_version(&String::from_utf8_lossy(&out.stdout));
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

pub fn is_available() -> bool {
    find_cli().is_some()
}

/// The rclone program with Brum's settings file, for one-off commands
pub fn command() -> Result<Command, String> {
    let (bin, _) = find_cli().ok_or_else(not_installed)?;
    let mut c = Command::new(bin);
    if let Some(file) = config_file() {
        prepare_config_file();
        c.arg("--config").arg(file);
        // rclone's own cache next to its settings, not in the service account's home folder
        c.arg("--cache-dir").arg(rclone_dir().join("cache"));
    }
    c.stdin(Stdio::null());
    Ok(c)
}

fn not_installed() -> String {
    "rclone is not installed. An administrator can install it in Settings → Cloud Storage.".to_string()
}

// ---------------------------------------------------------------- the rclone helper

pub struct Helper {
    pub child: Child,
    pub base: String,
    pub pass: String,
    pub client: reqwest::blocking::Client,
}

/// Starts `rclone rcd` on a free local port with a random password and waits until it answers.
/// Must run outside the async runtime (blocking HTTP).
pub fn start_helper() -> Result<Helper, String> {
    let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())?;
    let pass: String = {
        use rand::Rng;
        rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(32).map(char::from).collect()
    };
    let child = command()?
        .args(["rcd", "--rc-serve", "--rc-user", "brum", "--rc-pass", &pass, "--rc-addr"])
        .arg(format!("127.0.0.1:{port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start rclone: {e}"))?;
    let client = reqwest::blocking::Client::builder().timeout(None).connect_timeout(Duration::from_secs(5)).build().map_err(|e| e.to_string())?;
    let mut helper = Helper { child, base: format!("http://127.0.0.1:{port}"), pass, client };
    let started = Instant::now();
    loop {
        let ok = helper.client.post(format!("{}/core/version", helper.base)).basic_auth("brum", Some(&helper.pass)).json(&json!({})).send().map(|r| r.status().is_success()).unwrap_or(false);
        if ok {
            return Ok(helper);
        }
        if let Ok(Some(_)) = helper.child.try_wait() {
            return Err("rclone stopped right after starting".into());
        }
        if started.elapsed() > Duration::from_secs(15) {
            let _ = helper.child.kill();
            return Err("rclone did not start in time".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Shared {
    helper: Helper,
    /// The program it was started with; a new program (Install, Update) starts a new helper.
    /// rclone itself reloads its settings file when it changes, so `rclone config` edits show up.
    stamp: (PathBuf, Option<SystemTime>),
}

fn shared() -> &'static Mutex<Option<Shared>> {
    static S: OnceLock<Mutex<Option<Shared>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn current_stamp() -> Option<(PathBuf, Option<SystemTime>)> {
    let (bin, _) = find_cli()?;
    let bin_time = std::fs::metadata(&bin).and_then(|m| m.modified()).ok();
    Some((bin, bin_time))
}

/// The shared helper's client, address and password, (re)starting it when needed
fn connection() -> Result<(reqwest::blocking::Client, String, String), String> {
    let stamp = current_stamp().ok_or_else(not_installed)?;
    let mut guard = shared().lock().map_err(|_| "rclone helper is unusable".to_string())?;
    let reuse = match guard.as_mut() {
        Some(s) => s.stamp == stamp && matches!(s.helper.child.try_wait(), Ok(None)),
        None => false,
    };
    if !reuse {
        if let Some(mut old) = guard.take() {
            let _ = old.helper.child.kill();
            let _ = old.helper.child.wait();
        }
        let helper = start_helper()?;
        *guard = Some(Shared { helper, stamp });
    }
    let h = &guard.as_ref().expect("set above").helper;
    Ok((h.client.clone(), h.base.clone(), h.pass.clone()))
}

/// Forgets the shared helper (after installing a new program)
pub fn stop_helper() {
    if let Ok(mut g) = shared().lock() {
        if let Some(mut s) = g.take() {
            let _ = s.helper.child.kill();
            let _ = s.helper.child.wait();
        }
    }
}

/// One remote-control call to the shared helper. Blocking: call from a blocking thread.
pub fn rc(method: &str, body: Value, wait: Duration) -> Result<Value, String> {
    let (client, base, pass) = connection()?;
    let resp = client.post(format!("{base}/{method}")).basic_auth("brum", Some(&pass)).json(&body).timeout(wait).send().map_err(|e| format!("rclone: {e}"))?;
    let ok = resp.status().is_success();
    let v: Value = resp.json().unwrap_or(Value::Null);
    if ok {
        Ok(v)
    } else {
        Err(explain(v.get("error").and_then(|e| e.as_str()).unwrap_or("request failed")))
    }
}

/// A longer step (copy, move, folder listing) run as a job in rclone, so it can be stopped and
/// its progress followed: `progress(bytes, total_bytes)`.
pub fn rc_job(method: &str, mut body: Value, is_cancelled: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64, u64)) -> Result<(Value, u64, u64), String> {
    let (client, base, pass) = connection()?;
    let call = |m: &str, b: &Value, wait: Duration| -> Result<Value, String> {
        let resp = client.post(format!("{base}/{m}")).basic_auth("brum", Some(&pass)).json(b).timeout(wait).send().map_err(|e| format!("rclone: {e}"))?;
        let ok = resp.status().is_success();
        let v: Value = resp.json().unwrap_or(Value::Null);
        if ok { Ok(v) } else { Err(explain(v.get("error").and_then(|e| e.as_str()).unwrap_or("request failed"))) }
    };
    body["_async"] = json!(true);
    let job = call(method, &body, Duration::from_secs(30))?.get("jobid").and_then(|j| j.as_u64()).ok_or("rclone did not start the step")?;
    let group = json!({ "group": format!("job/{job}") });
    let (mut bytes, mut files) = (0u64, 0u64);
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let st = call("job/status", &json!({ "jobid": job }), Duration::from_secs(10)).map_err(|e| format!("rclone stopped answering: {e}"))?;
        if let Ok(stats) = call("core/stats", &group, Duration::from_secs(10)) {
            bytes = stats.get("bytes").and_then(|b| b.as_u64()).unwrap_or(bytes);
            files = stats.get("transfers").and_then(|b| b.as_u64()).unwrap_or(files);
            let total = stats.get("totalBytes").and_then(|b| b.as_u64()).unwrap_or(0);
            progress(bytes, total.max(bytes));
        }
        if st.get("finished").and_then(|f| f.as_bool()).unwrap_or(false) {
            let _ = call("core/stats-delete", &group, Duration::from_secs(5));
            if st.get("success").and_then(|f| f.as_bool()).unwrap_or(false) {
                return Ok((st.get("output").cloned().unwrap_or(Value::Null), files, bytes));
            }
            let raw = st.get("error").and_then(|e| e.as_str()).filter(|e| !e.is_empty()).unwrap_or("the step failed");
            return Err(explain(raw));
        }
        if is_cancelled() {
            let _ = call("job/stop", &json!({ "jobid": job }), Duration::from_secs(5));
            return Err("Cancelled".into());
        }
    }
}

/// Turns rclone's error text into a plain reason
pub fn explain(raw: &str) -> String {
    let low = raw.to_lowercase();
    let reason = if low.contains("didn't find section in config file") {
        "rclone has no remote by that name. Remotes are set up in Settings → Cloud Storage."
    } else if low.contains("directory not found") || low.contains("object not found") || low.contains("file not found") {
        "Not found"
    } else if low.contains("invalid_grant") || low.contains("unauthorized") || low.contains("token expired") || low.contains("couldn't fetch token") {
        "The service did not accept the saved login. Remove the remote and add it again with a new login."
    } else if low.contains("permission denied") || low.contains("access denied") || low.contains("forbidden") {
        "No permission for this"
    } else if low.contains("no such host") || low.contains("connection refused") || low.contains("i/o timeout") || low.contains("network is unreachable") {
        "Could not reach the service"
    } else {
        return format!("rclone: {raw}");
    };
    format!("{reason} (rclone: {raw})")
}

// ---------------------------------------------------------------- addresses

/// A place inside a remote: `rclone://remote/path`
#[derive(Debug, Clone, PartialEq)]
pub struct Loc {
    pub remote: String,
    /// Without slashes at the ends; "" is the remote's top
    pub path: String,
}

pub fn valid_remote_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.starts_with('-') && !name.starts_with(' ') && !name.ends_with(' ') && name.chars().all(|c| c.is_alphanumeric() || "._-@ ".contains(c))
}

/// Splits an address. None: the list of remotes (`rclone://`). Steps up (`..`) are refused, so
/// access granted to one folder cannot be used to reach another.
pub fn parse(uri: &str) -> Result<Option<Loc>, String> {
    let rest = if uri.len() >= 9 && uri[..9].eq_ignore_ascii_case("rclone://") { &uri[9..] } else { return Err("An rclone address looks like rclone://remote/folder".into()) };
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return Ok(None);
    }
    let (remote, path) = rest.split_once('/').unwrap_or((rest, ""));
    let remote = remote.trim_end_matches(':');
    if !valid_remote_name(remote) {
        return Err(format!("\"{remote}\" is not a valid rclone remote name"));
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    if parts.contains(&"..") {
        return Err("rclone addresses cannot contain \"..\"".into());
    }
    Ok(Some(Loc { remote: remote.to_string(), path: parts.join("/") }))
}

impl Loc {
    pub fn fs(&self) -> String {
        format!("{}:", self.remote)
    }
    /// The place as an rclone "remote:path" string (a whole folder as its own fs)
    pub fn full(&self) -> String {
        format!("{}:{}", self.remote, self.path)
    }
    pub fn uri(&self) -> String {
        if self.path.is_empty() { format!("rclone://{}", self.remote) } else { format!("rclone://{}/{}", self.remote, self.path) }
    }
    pub fn join(&self, name: &str) -> Loc {
        Loc { remote: self.remote.clone(), path: if self.path.is_empty() { name.to_string() } else { format!("{}/{}", self.path, name) } }
    }
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or("")
    }
    pub fn parent(&self) -> Option<Loc> {
        if self.path.is_empty() {
            return None;
        }
        let p = self.path.rsplit_once('/').map(|(a, _)| a).unwrap_or("");
        Some(Loc { remote: self.remote.clone(), path: p.to_string() })
    }
}

fn need_loc(uri: &str) -> Result<Loc, String> {
    parse(uri)?.ok_or_else(|| "Choose a remote first".to_string())
}

// ---------------------------------------------------------------- listing

fn parse_time(s: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp().max(0) as u64)
}

fn entry(name: &str, path: String, is_dir: bool, size: u64, modified: Option<u64>, owner: &str, mime: Option<String>) -> FileEntry {
    let lower = name.to_lowercase();
    FileEntry {
        name: name.to_string(),
        path,
        is_dir,
        is_symlink: false,
        is_empty: None,
        size: if is_dir { 0 } else { size },
        modified,
        permissions: if is_dir { "drwx------" } else { "-rw-------" }.to_string(),
        mode_octal: if is_dir { "0700" } else { "0600" }.to_string(),
        owner: owner.to_string(),
        group: "rclone".to_string(),
        uid: 0,
        gid: 0,
        mime_type: mime,
        is_archive: !is_dir && crate::vfs::is_archive_file(&lower),
    }
}

/// The remotes and their kind ("jottacloud", "sftp", ...)
pub fn list_remotes_blocking() -> Result<Vec<(String, String)>, String> {
    let names = rc("config/listremotes", json!({}), Duration::from_secs(30))?;
    let mut out = Vec::new();
    for n in names.get("remotes").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
        let Some(name) = n.as_str() else { continue };
        let kind = rc("config/get", json!({ "name": name }), Duration::from_secs(10)).ok().and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from)).unwrap_or_default();
        out.push((name.to_string(), kind));
    }
    out.sort_by_key(|(n, _)| n.to_lowercase());
    Ok(out)
}

fn listing(current: String, parent: Option<String>, entries: Vec<FileEntry>) -> DirectoryListing {
    DirectoryListing {
        total_files: entries.iter().filter(|e| !e.is_dir).count(),
        total_dirs: entries.iter().filter(|e| e.is_dir).count(),
        total_size: entries.iter().map(|e| e.size).sum(),
        current_path: current,
        parent_path: parent,
        entries,
        protocol: "rclone".to_string(),
        is_truncated: None,
        max_limit: None,
    }
}

/// Lists a folder, or the remotes this user may open (`may_open(remote)`) at `rclone://`
pub fn list_blocking(uri: &str, may_open: &dyn Fn(&str) -> bool) -> VfsResult<DirectoryListing> {
    let Some(loc) = parse(uri)? else {
        let entries = list_remotes_blocking()?.into_iter().filter(|(n, _)| may_open(n)).map(|(n, kind)| entry(&n, format!("rclone://{n}"), true, 0, None, &kind, None)).collect();
        return Ok(listing("rclone://".into(), None, entries));
    };
    let v = rc("operations/list", json!({ "fs": loc.fs(), "remote": loc.path, "opt": { "noMimeType": true } }), Duration::from_secs(300))?;
    let mut entries: Vec<FileEntry> = v
        .get("list")
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            let name = e.get("Name")?.as_str()?.to_string();
            let is_dir = e.get("IsDir").and_then(|d| d.as_bool()).unwrap_or(false);
            let size = e.get("Size").and_then(|s| s.as_i64()).unwrap_or(0).max(0) as u64;
            let modified = e.get("ModTime").and_then(|t| t.as_str()).and_then(parse_time);
            Some(entry(&name, loc.join(&name).uri(), is_dir, size, modified, &loc.remote, None))
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let parent = Some(loc.parent().map(|p| p.uri()).unwrap_or_else(|| "rclone://".to_string()));
    Ok(listing(loc.uri(), parent, entries))
}

pub async fn list_directory(uri: &str, may_open: impl Fn(&str) -> bool + Send + 'static) -> VfsResult<DirectoryListing> {
    let uri = uri.to_string();
    blocking(move || list_blocking(&uri, &may_open)).await
}

/// One item: (is_dir, size, modified). None when it does not exist.
pub fn stat_blocking(loc: &Loc) -> Result<Option<(bool, u64, Option<u64>)>, String> {
    if loc.path.is_empty() {
        return Ok(Some((true, 0, None)));
    }
    let v = rc("operations/stat", json!({ "fs": loc.fs(), "remote": loc.path, "opt": { "noMimeType": true } }), Duration::from_secs(120))?;
    let Some(item) = v.get("item").filter(|i| !i.is_null()) else { return Ok(None) };
    let is_dir = item.get("IsDir").and_then(|d| d.as_bool()).unwrap_or(false);
    let size = item.get("Size").and_then(|s| s.as_i64()).unwrap_or(0).max(0) as u64;
    Ok(Some((is_dir, size, item.get("ModTime").and_then(|t| t.as_str()).and_then(parse_time))))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("rclone task failed: {e}"))?
}

// ---------------------------------------------------------------- changes

pub async fn mkdir(uri: &str) -> Result<(), String> {
    let loc = need_loc(uri)?;
    blocking(move || rc("operations/mkdir", json!({ "fs": loc.fs(), "remote": loc.path }), Duration::from_secs(120)).map(|_| ())).await
}

pub async fn delete(uri: &str) -> Result<(), String> {
    let loc = need_loc(uri)?;
    if loc.path.is_empty() {
        return Err("A whole remote cannot be deleted here; remove it in Settings → Cloud Storage".into());
    }
    blocking(move || {
        let is_dir = stat_blocking(&loc)?.ok_or("Not found")?.0;
        if is_dir {
            rc("operations/purge", json!({ "fs": loc.fs(), "remote": loc.path }), Duration::from_secs(3600)).map(|_| ())
        } else {
            rc("operations/deletefile", json!({ "fs": loc.fs(), "remote": loc.path }), Duration::from_secs(120)).map(|_| ())
        }
    })
    .await
}

/// Renames or moves an item inside one remote (the service does it itself where it can)
pub async fn rename(from_uri: &str, to_uri: &str) -> Result<(), String> {
    let from = need_loc(from_uri)?;
    let to = need_loc(to_uri)?;
    if from.path.is_empty() || to.path.is_empty() {
        return Err("A whole remote cannot be renamed here".into());
    }
    blocking(move || {
        if stat_blocking(&to)?.is_some() {
            return Err(format!("{} already exists", to.name()));
        }
        let is_dir = stat_blocking(&from)?.ok_or("Not found")?.0;
        if is_dir {
            rc("sync/move", json!({ "srcFs": from.full(), "dstFs": to.full(), "createEmptySrcDirs": true, "deleteEmptySrcDirs": true }), Duration::from_secs(3600))?;
            let _ = rc("operations/rmdir", json!({ "fs": from.fs(), "remote": from.path }), Duration::from_secs(60));
            Ok(())
        } else {
            rc("operations/movefile", json!({ "srcFs": from.fs(), "srcRemote": from.path, "dstFs": to.fs(), "dstRemote": to.path }), Duration::from_secs(3600)).map(|_| ())
        }
    })
    .await
}

/// A free name in a folder: "photo.jpg" → "photo (1).jpg"
fn free_name(taken: &dyn Fn(&str) -> bool, name: &str) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..10_000).map(|n| format!("{stem} ({n}){ext}")).find(|c| !taken(c)).unwrap_or_else(|| format!("{stem} ({}){ext}", rand_suffix()))
}

fn rand_suffix() -> String {
    use rand::Rng;
    rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(6).map(char::from).collect()
}

/// The name to write to in a folder, by the conflict choice; None: skip
fn target_name(dir: &Loc, name: &str, conflict: &str) -> Result<Option<String>, String> {
    let exists = |n: &str| stat_blocking(&dir.join(n)).ok().flatten().is_some();
    if !exists(name) {
        return Ok(Some(name.to_string()));
    }
    Ok(match conflict {
        "skip" => None,
        "rename" | "keep_both" => Some(free_name(&exists, name)),
        _ => Some(name.to_string()),
    })
}

/// Uploads one file's bytes into a folder
pub async fn upload_bytes(dir_uri: &str, name: &str, data: Vec<u8>, conflict: &str) -> Result<(), String> {
    let dir = need_loc(dir_uri)?;
    let name = name.to_string();
    let conflict = conflict.to_string();
    blocking(move || {
        let Some(name) = target_name(&dir, &name, &conflict)? else { return Ok(()) };
        // Written to a private scratch file first, then copied by rclone like any local file
        let scratch = rclone_dir().join("upload-tmp");
        ensure_private_dir(&rclone_dir()).map_err(|e| e.to_string())?;
        ensure_private_dir(&scratch).map_err(|e| e.to_string())?;
        let work = tempfile::tempdir_in(&scratch).map_err(|e| format!("Cannot make a scratch folder: {e}"))?;
        let local = work.path().join(&name);
        std::fs::write(&local, &data).map_err(|e| format!("Cannot write the upload: {e}"))?;
        copy_between(&Side::Local(local), &Side::Remote(dir.join(&name)), false, false, &|| false, &mut |_, _| {}).map(|_| ())
    })
    .await
}

/// Uploads local files and folders (an unpacked dropped folder) into a folder
pub async fn upload_local_paths(dir_uri: &str, paths: Vec<PathBuf>, conflict: &str) -> Result<(), String> {
    let dir = need_loc(dir_uri)?;
    let conflict = conflict.to_string();
    blocking(move || {
        for p in paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let Some(target) = target_name(&dir, &name, &conflict)? else { continue };
            copy_between(&Side::Local(p.clone()), &Side::Remote(dir.join(&target)), p.is_dir(), false, &|| false, &mut |_, _| {})?;
        }
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------- viewing and downloading

fn view_cache_dir() -> Result<PathBuf, String> {
    let d = rclone_dir().join("view-cache");
    ensure_private_dir(&rclone_dir()).map_err(|e| e.to_string())?;
    ensure_private_dir(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Downloaded copies for viewing are kept for an hour, then cleared
const VIEW_CACHE_TTL: Duration = Duration::from_secs(3600);

fn clean_view_cache(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|age| age > VIEW_CACHE_TTL).unwrap_or(true);
            if old {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// Downloads a file (or folder) to a private local copy for viewing or downloading in the
/// browser; the copy is reused while the item is unchanged
pub async fn fetch_to_cache(uri: &str) -> Result<PathBuf, String> {
    let loc = need_loc(uri)?;
    blocking(move || {
        let (is_dir, size, modified) = stat_blocking(&loc)?.ok_or("Not found")?;
        let cache = view_cache_dir()?;
        clean_view_cache(&cache);
        use sha2::{Digest, Sha256};
        let key = hex::encode(Sha256::digest(format!("{}\n{modified:?}\n{size}\n{is_dir}", loc.full()).as_bytes()));
        let slot = cache.join(&key[..32]);
        let name = if loc.path.is_empty() { loc.remote.clone() } else { loc.name().to_string() };
        let target = slot.join(&name);
        if target.exists() && !is_dir {
            if let Ok(f) = std::fs::File::open(&slot) {
                let _ = f.set_modified(SystemTime::now());
            }
            return Ok(target);
        }
        let _ = std::fs::remove_dir_all(&slot);
        ensure_private_dir(&slot).map_err(|e| e.to_string())?;
        let r = copy_between(&Side::Remote(loc.clone()), &Side::Local(target.clone()), is_dir, false, &|| false, &mut |_, _| {});
        if let Err(e) = r {
            let _ = std::fs::remove_dir_all(&slot);
            return Err(e);
        }
        if !target.exists() {
            let _ = std::fs::remove_dir_all(&slot);
            return Err("rclone did not deliver the file".into());
        }
        Ok(target)
    })
    .await
}

// ---------------------------------------------------------------- copies and moves

/// One end of a copy
#[derive(Debug, Clone)]
enum Side {
    Local(PathBuf),
    Remote(Loc),
}

impl Side {
    /// (fs, remote) naming a single file for operations/copyfile
    fn file_parts(&self) -> (String, String) {
        match self {
            Side::Local(p) => {
                let parent = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                (parent, name)
            }
            Side::Remote(l) => (l.fs(), l.path.clone()),
        }
    }
    /// The whole folder as an fs for sync/copy
    fn dir_fs(&self) -> String {
        match self {
            Side::Local(p) => p.to_string_lossy().to_string(),
            Side::Remote(l) => l.full(),
        }
    }
}

/// Copies or moves one file or folder from `src` to exactly `dst` (the new item's own path)
fn copy_between(src: &Side, dst: &Side, is_dir: bool, is_move: bool, is_cancelled: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64, u64)) -> Result<(u64, u64), String> {
    if let Side::Local(p) = dst {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
    }
    if is_dir {
        let method = if is_move { "sync/move" } else { "sync/copy" };
        let mut body = json!({ "srcFs": src.dir_fs(), "dstFs": dst.dir_fs(), "createEmptySrcDirs": true });
        if is_move {
            body["deleteEmptySrcDirs"] = json!(true);
        }
        let (_, files, bytes) = rc_job(method, body, is_cancelled, progress)?;
        if is_move {
            // The emptied folder itself
            match src {
                Side::Local(p) => {
                    let _ = std::fs::remove_dir(p);
                }
                Side::Remote(l) => {
                    let _ = rc("operations/rmdir", json!({ "fs": l.fs(), "remote": l.path }), Duration::from_secs(60));
                }
            }
        }
        Ok((files.max(1), bytes))
    } else {
        let (sf, sr) = src.file_parts();
        let (df, dr) = dst.file_parts();
        let method = if is_move { "operations/movefile" } else { "operations/copyfile" };
        let (_, _, bytes) = rc_job(method, json!({ "srcFs": sf, "srcRemote": sr, "dstFs": df, "dstRemote": dr }), is_cancelled, progress)?;
        Ok((1, bytes))
    }
}

/// Copy or move between rclone and this machine's disks, or between remotes, for the task list.
/// Returns (note, files, bytes) like the other transfers.
#[allow(clippy::too_many_arguments)]
pub fn transfer_item(
    src: &str,
    dest_dir: &str,
    is_move: bool,
    conflict: Option<&str>,
    tm: &crate::tools::tasks::TaskManager,
    task_id: &str,
    files_done_before: u64,
    total_files: u64,
    bytes_done_before: u64,
    start_time: Instant,
) -> Result<(Option<String>, u64, u64), String> {
    let is_rclone = |s: &str| s.len() >= 9 && s[..9].eq_ignore_ascii_case("rclone://");
    let is_other_remote = |s: &str| s.contains("://") && !is_rclone(s);
    if is_other_remote(src) || is_other_remote(dest_dir) {
        return Err("Copy between cloud storage (rclone) and other remote storage through a folder on this machine".into());
    }
    // The work uses rclone's helper over blocking HTTP, which cannot run on the async runtime's threads
    std::thread::scope(|s| {
        s.spawn(|| {
            let conflict = conflict.unwrap_or("overwrite");
            let src_side = if is_rclone(src) { Side::Remote(need_loc(src)?) } else { Side::Local(PathBuf::from(src)) };
            let (name, is_dir) = match &src_side {
                Side::Remote(l) => {
                    if l.path.is_empty() {
                        return Err("Open the remote and choose what to copy".to_string());
                    }
                    let (d, _, _) = stat_blocking(l)?.ok_or_else(|| format!("{} was not found", l.uri()))?;
                    (l.name().to_string(), d)
                }
                Side::Local(p) => (p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), p.is_dir()),
            };
            let dst_side = if is_rclone(dest_dir) {
                let dir = need_loc(dest_dir)?;
                let Some(target) = target_name(&dir, &name, conflict)? else { return Ok((Some(format!("{name} skipped (already there)")), 0, 0)) };
                Side::Remote(dir.join(&target))
            } else {
                let dir = Path::new(dest_dir);
                let mut target = dir.join(&name);
                if target.exists() {
                    match conflict {
                        "skip" => return Ok((Some(format!("{name} skipped (already there)")), 0, 0)),
                        "rename" | "keep_both" => target = dir.join(free_name(&|n| dir.join(n).exists(), &name)),
                        _ => {}
                    }
                }
                Side::Local(target)
            };
            if let (Side::Remote(a), Side::Remote(b)) = (&src_side, &dst_side) {
                if a == b || (a.remote == b.remote && b.path.starts_with(&format!("{}/", a.path))) {
                    return Err("A folder cannot be copied into itself".into());
                }
            }
            let progress = &mut |cur: u64, total: u64| {
                let elapsed = start_time.elapsed().as_secs_f64();
                let speed = if elapsed > 0.05 { ((bytes_done_before + cur) as f64 / elapsed) as u64 } else { 0 };
                tm.sync_update_stream_progress(task_id, Some(&name), cur, total, files_done_before, total_files, bytes_done_before + cur, speed);
            };
            // What the item holds, so the task counts it even when the service moves it itself
            // without sending data (a move inside one service)
            let (exp_files, exp_bytes) = match &src_side {
                Side::Local(p) if p.is_dir() => walkdir::WalkDir::new(p).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()).fold((0u64, 0u64), |(c, b), e| (c + 1, b + e.metadata().map(|m| m.len()).unwrap_or(0))),
                Side::Local(p) => (1, std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)),
                Side::Remote(_) => totals(src),
            };
            let (files, bytes) = copy_between(&src_side, &dst_side, is_dir, is_move, &|| tm.sync_is_cancelled(task_id), progress)?;
            Ok((None, files.max(exp_files).max(1), bytes.max(exp_bytes)))
        })
        .join()
        .unwrap_or_else(|_| Err("The rclone copy stopped unexpectedly".into()))
    })
}

/// Files and bytes in an rclone item, for the task's totals (0, 0 when unknown)
pub fn totals(uri: &str) -> (u64, u64) {
    let Ok(Some(loc)) = parse(uri) else { return (0, 0) };
    std::thread::scope(|s| {
        s.spawn(|| match stat_blocking(&loc) {
            Ok(Some((false, size, _))) => (1, size),
            Ok(Some((true, _, _))) => rc("operations/size", json!({ "fs": loc.full() }), Duration::from_secs(120))
                .map(|v| (v.get("count").and_then(|c| c.as_u64()).unwrap_or(0), v.get("bytes").and_then(|c| c.as_u64()).unwrap_or(0)))
                .unwrap_or((0, 0)),
            _ => (0, 0),
        })
        .join()
        .unwrap_or((0, 0))
    })
}

// ---------------------------------------------------------------- settings: remotes

#[derive(Debug, Clone, Serialize)]
pub struct RemoteInfo {
    pub name: String,
    pub kind: String,
}

pub async fn remotes() -> Result<Vec<RemoteInfo>, String> {
    if !is_available() {
        return Ok(Vec::new());
    }
    blocking(|| Ok(list_remotes_blocking()?.into_iter().map(|(name, kind)| RemoteInfo { name, kind }).collect())).await
}

/// Storage used and free (where the service tells), and proof the login works
pub async fn about(name: &str) -> Result<Value, String> {
    if !valid_remote_name(name) {
        return Err("Not a valid remote name".into());
    }
    let fs = format!("{name}:");
    blocking(move || {
        // Listing the top proves the login works, also for services that do not tell their size
        rc("operations/list", json!({ "fs": fs, "remote": "", "opt": { "noMimeType": true, "dirsOnly": true } }), Duration::from_secs(60))?;
        Ok(rc("operations/about", json!({ "fs": fs }), Duration::from_secs(60)).unwrap_or(json!({})))
    })
    .await
}

pub async fn remove_remote(name: &str) -> Result<(), String> {
    if !valid_remote_name(name) {
        return Err("Not a valid remote name".into());
    }
    let name = name.to_string();
    blocking(move || rc("config/delete", json!({ "name": name }), Duration::from_secs(30)).map(|_| ())).await
}

/// Adds a Jottacloud remote from the one-time personal login token (jottacloud.com → Settings →
/// Security). The token goes to rclone over the private helper connection, never on a command line.
pub async fn add_jottacloud(name: &str, token: &str) -> Result<(), String> {
    if !valid_remote_name(name) {
        return Err("Use letters, digits, dots, dashes or underscores for the name".into());
    }
    let token: String = token.chars().filter(|c| !c.is_whitespace()).collect();
    if token.len() < 20 {
        return Err("Paste the whole personal login token from Jottacloud".into());
    }
    let name = name.to_string();
    blocking(move || {
        let existing = list_remotes_blocking()?;
        if existing.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            return Err(format!("A remote called \"{name}\" already exists"));
        }
        let mut answer = rc("config/create", json!({ "name": name, "type": "jottacloud", "parameters": { "config_type": "standard" }, "opt": { "nonInteractive": true, "obscure": true } }), Duration::from_secs(60));
        // Follow rclone's questions: the token first, then its suggestions for the rest
        let mut steps = 0;
        let result = loop {
            let v = match answer {
                Ok(v) => v,
                Err(e) => break Err(e),
            };
            let state = v.get("State").and_then(|s| s.as_str()).unwrap_or("").to_string();
            if let Some(err) = v.get("Error").and_then(|e| e.as_str()).filter(|e| !e.is_empty()) {
                break Err(explain(err));
            }
            if state.is_empty() {
                break Ok(());
            }
            steps += 1;
            if steps > 8 {
                break Err("rclone kept asking questions; set this remote up with rclone config in a terminal".into());
            }
            let option = v.get("Option").cloned().unwrap_or(Value::Null);
            let opt_name = option.get("Name").and_then(|n| n.as_str()).unwrap_or("");
            let reply = if opt_name == "config_login_token" {
                token.clone()
            } else {
                option.get("DefaultStr").and_then(|d| d.as_str()).map(String::from).or_else(|| option.get("Default").map(|d| d.to_string().trim_matches('"').to_string())).unwrap_or_default()
            };
            answer = rc("config/update", json!({ "name": name, "parameters": {}, "opt": { "nonInteractive": true, "obscure": true, "continue": true, "state": state, "result": reply } }), Duration::from_secs(60));
        };
        match result {
            Ok(()) => {
                // Only a remote that can be listed counts as set up
                if let Err(e) = rc("operations/list", json!({ "fs": format!("{name}:"), "remote": "", "opt": { "dirsOnly": true } }), Duration::from_secs(60)) {
                    let _ = rc("config/delete", json!({ "name": name }), Duration::from_secs(30));
                    return Err(e);
                }
                prepare_config_file();
                Ok(())
            }
            Err(e) => {
                let _ = rc("config/delete", json!({ "name": name }), Duration::from_secs(30));
                if e.contains("invalid_grant") {
                    // Seen with rclone 1.60 (Debian 13) and fresh tokens; 1.76 logs in fine
                    return Err(format!("Jottacloud did not accept the token. A token works once and only for a short time, so make a new one and add it straight away. If a fresh token is refused too, this rclone is probably too old for Jottacloud's login: install or update rclone in Cloud Storage. ({e})"));
                }
                Err(e)
            }
        }
    })
    .await
}

// ---------------------------------------------------------------- settings: install

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

#[derive(Debug, Clone, Serialize)]
pub struct RcloneStatus {
    pub cli: Option<CliInfo>,
    pub install: InstallView,
    pub allow_download: bool,
    /// Brum's own settings file, when it keeps one
    pub config_file: Option<String>,
    pub platform: Option<String>,
    pub download_page: &'static str,
}

pub fn status() -> RcloneStatus {
    RcloneStatus {
        cli: cli_info(),
        install: install_view(),
        allow_download: runtime().config.allow_download,
        config_file: config_file().map(|p| p.to_string_lossy().to_string()),
        platform: platform_name(),
        download_page: DOWNLOAD_PAGE,
    }
}

/// rclone's name for this machine's build: "linux-amd64", "windows-arm64", "osx-arm64", ...
pub fn platform_name() -> Option<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        "arm" => "arm-v7",
        _ => return None,
    };
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "osx",
        "windows" => "windows",
        "freebsd" => "freebsd",
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder().user_agent(concat!("Brum/", env!("CARGO_PKG_VERSION"))).connect_timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct LatestRelease {
    pub version: String,
    pub url: String,
    pub sha256: String,
}

/// The checksum of `file` in rclone's SHA256SUMS (a signed text file of "hash  name" lines)
fn checksum_of(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, name) = l.trim().split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == file && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_lowercase())
    })
}

/// The newest rclone for this machine, with its published checksum
pub async fn latest_release() -> Result<LatestRelease, String> {
    let platform = platform_name().ok_or("rclone has no download for this kind of computer")?;
    let client = http_client()?;
    let get_text = |url: String| {
        let client = client.clone();
        async move {
            client.get(&url).timeout(Duration::from_secs(30)).send().await.map_err(|e| format!("Could not reach rclone.org: {e}"))?.error_for_status().map_err(|e| format!("rclone.org answered with an error: {e}"))?.text().await.map_err(|e| e.to_string())
        }
    };
    let version_txt = get_text(format!("{DOWNLOADS}/version.txt")).await?;
    let version = version_txt.trim().strip_prefix("rclone ").unwrap_or(version_txt.trim()).to_string();
    if !version.starts_with('v') || !version[1..].chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err("rclone.org did not say which version is current".into());
    }
    let file = format!("rclone-{version}-{platform}.zip");
    let sums = get_text(format!("{DOWNLOADS}/{version}/SHA256SUMS")).await?;
    let sha256 = checksum_of(&sums, &file).ok_or_else(|| format!("rclone's checksum list has no {file}"))?;
    Ok(LatestRelease { version: version.trim_start_matches('v').to_string(), url: format!("{DOWNLOADS}/{version}/{file}"), sha256 })
}

/// Starts downloading the newest rclone into Brum's data folder; progress is in `install_view`
pub fn start_install() -> Result<(), String> {
    if !runtime().config.allow_download {
        return Err("Downloading rclone is turned off on this server ([rclone] allow_download)".to_string());
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
    use sha2::{Digest, Sha256};

    let rel = latest_release().await?;
    set_install(|s| s.version = Some(rel.version.clone()));
    let bin_dir = rclone_dir().join("bin");
    ensure_private_dir(&rclone_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("Cannot create {}: {e}", bin_dir.display()))?;
    let resp = http_client()?.get(&rel.url).send().await.map_err(|e| format!("Download failed: {e}"))?.error_for_status().map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.content_length();
    set_install(|s| s.total = total);
    let mut data: Vec<u8> = Vec::with_capacity(total.unwrap_or(0).min(200_000_000) as usize);
    let mut hasher = Sha256::new();
    let mut stream = resp.bytes_stream();
    let mut last = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
        hasher.update(&chunk);
        data.extend_from_slice(&chunk);
        if data.len() > 500_000_000 {
            return Err("The download is far larger than rclone should be; stopped".into());
        }
        if last.elapsed() > Duration::from_millis(200) {
            last = Instant::now();
            let n = data.len() as u64;
            set_install(|s| s.received = n);
        }
    }
    let n = data.len() as u64;
    set_install(|s| {
        s.received = n;
        s.phase = "verifying".into();
    });
    if hex::encode(hasher.finalize()) != rel.sha256 {
        return Err("The download did not match rclone's published checksum and was thrown away. Try again later.".to_string());
    }
    let target = installed_bin_path();
    let tmp = bin_dir.join(format!("{BIN_NAME}.download"));
    tokio::task::spawn_blocking({
        let tmp = tmp.clone();
        move || extract_program(&data, &tmp)
    })
    .await
    .map_err(|e| e.to_string())??;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot put the program in place: {e}")
    })?;
    stop_helper();
    let installed = cli_version(&target);
    if installed.is_none() {
        return Err("rclone was downloaded but does not start on this machine".into());
    }
    tracing::info!("rclone {} installed in {}", installed.clone().unwrap_or_default(), target.display());
    set_install(|s| {
        s.phase = "done".into();
        s.version = installed;
    });
    Ok(())
}

/// Takes the program out of rclone's zip ("rclone-v1.76.0-linux-amd64/rclone")
fn extract_program(zip_bytes: &[u8], to: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("The download is not a valid zip: {e}"))?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let is_program = f.is_file() && f.name().ok().and_then(|n| n.rsplit('/').next().map(|b| b == BIN_NAME)).unwrap_or(false);
        if is_program {
            let mut out = std::fs::File::create(to).map_err(|e| format!("Cannot write {}: {e}", to.display()))?;
            std::io::copy(&mut f, &mut out).map_err(|e| {
                let _ = std::fs::remove_file(to);
                format!("Cannot unpack rclone: {e}")
            })?;
            return Ok(());
        }
    }
    Err(format!("The download has no {BIN_NAME} in it"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse("rclone://").unwrap(), None);
        assert_eq!(parse("rclone:///").unwrap(), None);
        let l = parse("rclone://jotta/Archive//Photos/").unwrap().unwrap();
        assert_eq!((l.remote.as_str(), l.path.as_str()), ("jotta", "Archive/Photos"));
        assert_eq!(l.full(), "jotta:Archive/Photos");
        assert_eq!(l.uri(), "rclone://jotta/Archive/Photos");
        assert_eq!(l.parent().unwrap().uri(), "rclone://jotta/Archive");
        assert_eq!(l.parent().unwrap().parent().unwrap().uri(), "rclone://jotta");
        assert_eq!(l.parent().unwrap().parent().unwrap().parent(), None);
        assert_eq!(l.join("a b.txt").uri(), "rclone://jotta/Archive/Photos/a b.txt");
        assert_eq!(parse("RCLONE://box:/x").unwrap().unwrap().remote, "box");
        assert!(parse("rclone://jotta/a/../b").is_err());
        assert!(parse("rclone://-x/y").is_err());
        assert!(parse("rclone://a;b/y").is_err());
        assert!(parse("sftp://x").is_err());
    }

    #[test]
    fn versions_and_checksums() {
        assert_eq!(parse_version("rclone v1.60.1-DEV\n- os/version: debian"), Some("1.60.1-DEV".into()));
        assert_eq!(parse_version("rclone v1.76.0\n"), Some("1.76.0".into()));
        let sums = "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA1\n\nc047a1090c30086da45b7e2a91119947bb3d3e55712171b0e5b009ed4a8855fa  rclone-v1.76.0-aix-ppc64.zip\n0209b6a7edc1808b4a7e3b76c13e38fbabf870d51e293b834b124c3cfd0da2e5  rclone-v1.76.0-linux-amd64.zip\n";
        assert_eq!(checksum_of(sums, "rclone-v1.76.0-linux-amd64.zip").as_deref(), Some("0209b6a7edc1808b4a7e3b76c13e38fbabf870d51e293b834b124c3cfd0da2e5"));
        assert_eq!(checksum_of(sums, "rclone-v1.76.0-linux-arm64.zip"), None);
    }

    #[test]
    fn free_names() {
        let taken = |n: &str| n == "a.txt" || n == "a (1).txt" || n == "dir";
        assert_eq!(free_name(&taken, "b.txt"), "b.txt");
        assert_eq!(free_name(&taken, "a.txt"), "a (2).txt");
        assert_eq!(free_name(&taken, "dir"), "dir (1)");
    }

    /// Panes and copies against an offline remote (rclone's "alias" of a scratch folder, which
    /// behaves like a cloud service with folders; no account, no network). rclone's in-memory
    /// storage is listed too, but not used for copies: rclone 1.60 loses its subfolders.
    /// Skipped where rclone is not installed.
    #[tokio::test(flavor = "multi_thread")]
    async fn offline_remote_end_to_end() {
        let home = tempfile::tempdir().unwrap().keep();
        let cloud = home.join("cloud");
        std::fs::create_dir_all(home.join("rclone")).unwrap();
        std::fs::create_dir_all(&cloud).unwrap();
        std::fs::write(home.join("rclone/rclone.conf"), format!("[mem]\ntype = memory\n\n[box]\ntype = alias\nremote = {}\n", cloud.display())).unwrap();
        init(&home.join("brum.db").to_string_lossy(), &RcloneConfig::default(), true);
        if !is_available() {
            eprintln!("rclone not installed; skipping");
            return;
        }
        let all = |_: &str| true;
        // The list of remotes
        let top = list_directory("rclone://", all).await.unwrap();
        assert!(top.entries.iter().any(|e| e.name == "box" && e.is_dir && e.path == "rclone://box") && top.entries.iter().any(|e| e.name == "mem"), "{:?}", top.entries);
        assert!(list_directory("rclone://", |_: &str| false).await.unwrap().entries.is_empty());
        // New folder, upload, list
        mkdir("rclone://box/data/docs").await.unwrap();
        upload_bytes("rclone://box/data/docs", "a.txt", b"hello".to_vec(), "overwrite").await.unwrap();
        upload_bytes("rclone://box/data/docs", "a.txt", b"other".to_vec(), "rename").await.unwrap();
        let l = list_directory("rclone://box/data/docs", all).await.unwrap();
        let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a (1).txt", "a.txt"]);
        assert_eq!(l.parent_path.as_deref(), Some("rclone://box/data"));
        assert_eq!(list_directory("rclone://box", all).await.unwrap().parent_path.as_deref(), Some("rclone://"));
        assert_eq!(l.entries[1].size, 5);
        // Viewing fetches a private copy
        let local = fetch_to_cache("rclone://box/data/docs/a.txt").await.unwrap();
        assert_eq!(std::fs::read(&local).unwrap(), b"hello");
        // Rename a file and a folder
        rename("rclone://box/data/docs/a (1).txt", "rclone://box/data/docs/b.txt").await.unwrap();
        rename("rclone://box/data/docs", "rclone://box/data/papers").await.unwrap();
        let l = list_directory("rclone://box/data", all).await.unwrap();
        assert_eq!(l.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["papers"]);
        assert!(rename("rclone://box/data/papers/b.txt", "rclone://box/data/papers/a.txt").await.is_err(), "renaming onto an existing item must fail");
        // Copy a local folder in and a remote folder out
        let disk = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(disk.path().join("src/sub")).unwrap();
        std::fs::write(disk.path().join("src/sub/c.txt"), b"ccc").unwrap();
        tokio::task::spawn_blocking({
            let p = disk.path().join("src");
            move || copy_between(&Side::Local(p), &Side::Remote(parse("rclone://box/data/src").unwrap().unwrap()), true, false, &|| false, &mut |_, _| {})
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(totals("rclone://box/data/src"), (1, 3));
        let out = disk.path().join("out");
        let moved = tokio::task::spawn_blocking({
            let out = out.clone();
            move || copy_between(&Side::Remote(parse("rclone://box/data/src").unwrap().unwrap()), &Side::Local(out), true, true, &|| false, &mut |_, _| {})
        })
        .await
        .unwrap()
        .unwrap();
        assert!(moved.0 >= 1);
        assert_eq!(std::fs::read(out.join("sub/c.txt")).unwrap(), b"ccc");
        assert!(stat_blocking_async("rclone://box/data/src").await.is_none(), "a moved folder is gone from the remote");
        // Delete a file and a folder
        delete("rclone://box/data/papers/b.txt").await.unwrap();
        delete("rclone://box/data/papers").await.unwrap();
        assert!(list_directory("rclone://box/data", all).await.unwrap().entries.is_empty());
        assert!(delete("rclone://box").await.is_err(), "a whole remote is not deleted from a pane");
        stop_helper();
    }

    async fn stat_blocking_async(uri: &str) -> Option<(bool, u64, Option<u64>)> {
        let loc = parse(uri).unwrap().unwrap();
        tokio::task::spawn_blocking(move || stat_blocking(&loc)).await.unwrap().unwrap()
    }

    #[test]
    fn zip_extraction() {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("rclone-v1.0.0-x/README.txt", o).unwrap();
            z.write_all(b"readme").unwrap();
            z.start_file(format!("rclone-v1.0.0-x/{BIN_NAME}"), o).unwrap();
            z.write_all(b"program").unwrap();
            z.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("out");
        extract_program(buf.get_ref(), &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"program");
    }
}
