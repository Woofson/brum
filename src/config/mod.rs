use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub mod mask;
pub mod saved;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub themes: ThemeConfig,
    #[serde(default)]
    pub paranoid: ParanoidConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub desktop: DesktopConfig,
    #[serde(default = "default_custom_actions")]
    pub custom_actions: Vec<CustomAction>,
    #[serde(default = "default_open_with")]
    pub open_with: Vec<OpenWithRule>,
    #[serde(default = "default_bookmarks")]
    pub bookmarks: Vec<BookmarkConfig>,
    #[serde(default)]
    pub syncthing: crate::tools::syncthing::SyncthingConfig,
    /// `[notes]`; `[notedog]` from older configs is still read
    #[serde(default, alias = "notedog")]
    pub notes: NotesConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub fleet: FleetConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub sftp: Option<SftpConfig>,
    /// `[proton]`: where the Proton Drive program is and whether admins may download it
    #[serde(default)]
    pub proton: crate::vfs::proton::ProtonConfig,
    /// `[rclone]`: where rclone is and whether admins may download it (cloud storage, #189)
    #[serde(default)]
    pub rclone: crate::vfs::rclone::RcloneConfig,
    /// `[activity]`: the record of who did what to which files (#193)
    #[serde(default)]
    pub activity: ActivityConfig,
    /// `[[access.rules]]`: who may read, write or not touch which folders on this machine (#193)
    #[serde(default)]
    pub access: AccessConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccessConfig {
    #[serde(default)]
    pub rules: Vec<crate::tools::access::Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Entries older than this are removed (0 keeps them for good)
    #[serde(default = "default_activity_days")]
    pub retention_days: u32,
}

fn default_activity_days() -> u32 {
    90
}

impl Default for ActivityConfig {
    fn default() -> Self {
        Self { enabled: true, retention_days: default_activity_days() }
    }
}

impl AppConfig {
    /// With login off everyone who reaches Brum is admin, so that is only allowed where nobody else
    /// can reach it: on this machine only, or behind a reverse proxy that does the login (`trusted_proxies`)
    pub fn check_login_off_is_safe(&self) -> Result<(), String> {
        if self.server.enable_auth || self.server.standalone {
            return Ok(());
        }
        if crate::instance::is_loopback_host(&self.server.host) {
            return Ok(());
        }
        if self.server.trusted_proxies.iter().any(|p| !p.trim().is_empty()) {
            tracing::warn!(
                "Login is OFF. Only requests from this machine or the trusted proxies ({}) are accepted; the proxy must handle login",
                self.server.trusted_proxies.join(", ")
            );
            return Ok(());
        }
        Err(format!(
            "Refusing to start: login is off (--no-auth or enable_auth = false) while listening on '{}', which would give everyone on the network admin rights.\n\
             Either turn login back on, listen on 127.0.0.1 only, or set server.trusted_proxies to the reverse proxy that handles login.",
            self.server.host
        ))
    }

    /// Start-up check of the login signing secret: refuses to start when a named secret file
    /// cannot be used, logs where the secret came from (never the secret), warns when it is short,
    /// and keeps the secret file private (Windows: locked to Brum, SYSTEM and Administrators;
    /// elsewhere: a warning when other users can read it).
    pub fn check_signing_secret(&self) -> Result<(), String> {
        if let Some(e) = &self.server.jwt_secret_error {
            return Err(format!(
                "Refusing to start: {}.\n\
                 Fix the file or remove the setting. Brum does not fall back to a secret of its own here, \
                 because that would split the logins of a Fleet sharing this secret.",
                e
            ));
        }
        if self.server.jwt_secret_source.is_empty() {
            return Ok(());
        }
        info!("Login signing secret: from {}", self.server.jwt_secret_source);
        if self.server.jwt_secret.chars().count() < MIN_JWT_SECRET_LEN {
            warn!(
                "The login signing secret is shorter than {} characters; use a longer random one (for example: openssl rand -hex 32)",
                MIN_JWT_SECRET_LEN
            );
        }
        if let Some(path) = &self.server.jwt_secret_path {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = fs::metadata(path) {
                    let mode = meta.permissions().mode() & 0o777;
                    if mode & 0o027 != 0 {
                        warn!(
                            "The secret file '{}' can be read or changed by other users (permissions {:o}); limit it with: chmod 600 '{}'",
                            path.display(),
                            mode,
                            path.display()
                        );
                    }
                }
            }
            #[cfg(windows)]
            if let Err(e) = crate::win_perms::restrict_file_to_owner(path) {
                warn!("Could not limit the secret file '{}' to Brum and administrators: {}", path.display(), e);
            }
        }
        Ok(())
    }

    pub fn get_sftp_config(&self) -> SftpConfig {
        if let Some(ref sftp) = self.sftp {
            sftp.clone()
        } else {
            self.storage.sftp.clone()
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            auth: AuthConfig::default(),
            storage: StorageConfig::default(),
            themes: ThemeConfig::default(),
            paranoid: ParanoidConfig::default(),
            ui: UiConfig::default(),
            desktop: DesktopConfig::default(),
            custom_actions: default_custom_actions(),
            open_with: default_open_with(),
            bookmarks: default_bookmarks(),
            syncthing: crate::tools::syncthing::SyncthingConfig::default(),
            notes: NotesConfig::default(),
            terminal: TerminalConfig::default(),
            plugins: PluginsConfig::default(),
            fleet: FleetConfig::default(),
            logging: LoggingConfig::default(),
            sftp: None,
            proton: crate::vfs::proton::ProtonConfig::default(),
            rclone: crate::vfs::rclone::RcloneConfig::default(),
            activity: ActivityConfig::default(),
            access: AccessConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_root_path")]
    pub root_path: String,
    #[serde(default = "default_upload_max_mb")]
    pub upload_max_size_mb: usize,
    #[serde(default = "default_true")]
    pub enable_auth: bool,
    #[serde(default)]
    pub standalone: bool,
    #[serde(default = "default_jwt_secret", skip_serializing)]
    pub jwt_secret: String,
    /// Path of a file holding the signing secret (`jwt_secret_file`); the secret itself never
    /// appears in the config
    #[serde(default)]
    pub jwt_secret_file: String,
    /// Where the signing secret came from, for the start-up log (empty: derived from the key file)
    #[serde(skip)]
    pub jwt_secret_source: String,
    /// The secret file in use, checked for loose permissions at start-up
    #[serde(skip)]
    pub jwt_secret_path: Option<PathBuf>,
    /// A configured secret file that could not be used; start-up refuses to continue
    #[serde(skip)]
    pub jwt_secret_error: Option<String>,
    #[serde(default = "default_session_hours")]
    pub session_duration_hours: u64,
    #[serde(default = "default_db_path")]
    pub database_path: String,
    #[serde(default)]
    pub server_name: String,
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_true")]
    pub allow_web_config_edit: bool,
    /// The config file this configuration was read from (None: no file), so settings set
    /// there can be shown as "set in config file" and left alone by the web page
    #[serde(skip)]
    pub config_file: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            root_path: default_root_path(),
            upload_max_size_mb: default_upload_max_mb(),
            enable_auth: true,
            standalone: false,
            jwt_secret: default_jwt_secret(),
            jwt_secret_file: String::new(),
            jwt_secret_source: String::new(),
            jwt_secret_path: None,
            jwt_secret_error: None,
            session_duration_hours: default_session_hours(),
            database_path: default_db_path(),
            server_name: String::new(),
            trusted_proxies: Vec::new(),
            allowed_origins: Vec::new(),
            allow_web_config_edit: true,
            config_file: None,
        }
    }
}

/// Expands a leading tilde (`~` or `~/` or `~\`) to the current user's home directory.
pub fn expand_tilde_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let p = path.as_ref();
    let s = p.to_string_lossy();
    if s == "~" {
        if let Some(home) = dirs::home_dir().or_else(|| std::env::var("HOME").ok().map(PathBuf::from)) {
            return home;
        }
    } else if let Some(stripped) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir().or_else(|| std::env::var("HOME").ok().map(PathBuf::from)) {
            return home.join(stripped);
        }
    }
    p.to_path_buf()
}

fn default_host() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 3140 }
fn default_root_path() -> String {
    #[cfg(windows)]
    {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
        "C:\\".to_string()
    }
    #[cfg(not(windows))]
    {
        "/".to_string()
    }
}
fn default_upload_max_mb() -> usize { 20480 } // 20 GB
fn default_true() -> bool { true }
fn default_hostname_size() -> String { "md".to_string() }
fn default_hostname_icon() -> String { "none".to_string() }
fn default_hostname_style() -> String { "text".to_string() }
fn default_hostname_color() -> String { "slate".to_string() }
/// Placeholder signing secret meaning "none configured"
pub const DEFAULT_JWT_SECRET: &str = "brum-super-secret-jwt-key-2026";
fn default_jwt_secret() -> String { DEFAULT_JWT_SECRET.to_string() }

/// Shortest signing secret that does not get a warning
const MIN_JWT_SECRET_LEN: usize = 32;

/// Reads a secret from a file, ignoring surrounding whitespace. A missing, unreadable or empty
/// file is an error.
pub fn read_secret_file(path: &str) -> Result<(String, PathBuf), String> {
    let p = expand_tilde_path(path.trim());
    match fs::read_to_string(&p) {
        Ok(content) => {
            let secret = content.trim().to_string();
            if secret.is_empty() {
                Err(format!("the secret file '{}' is empty", p.display()))
            } else {
                Ok((secret, p))
            }
        }
        Err(e) => Err(format!("cannot read the secret file '{}': {}", p.display(), e)),
    }
}

/// Picks the login signing secret, first match wins: `BRUM_JWT_SECRET`, `BRUM_JWT_SECRET_FILE`,
/// `jwt_secret`, `jwt_secret_file`. Nothing found leaves the placeholder, so the secret is derived
/// from `brum-secrets.key`. A named file that cannot be used is recorded as an error, never
/// skipped: falling back to a node-local secret would split a Fleet's logins.
fn resolve_jwt_secret(server: &mut ServerConfig, env_secret: Option<String>, env_file: Option<String>) {
    let env_secret = env_secret.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let env_file = env_file.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let config_secret = server.jwt_secret.trim().to_string();
    let config_has_secret = !config_secret.is_empty() && config_secret != DEFAULT_JWT_SECRET;
    let config_file = server.jwt_secret_file.trim().to_string();

    server.jwt_secret_source.clear();
    server.jwt_secret_path = None;
    server.jwt_secret_error = None;

    let from_file = |path: &str, label: &str, server: &mut ServerConfig| match read_secret_file(path) {
        Ok((secret, p)) => {
            server.jwt_secret = secret;
            server.jwt_secret_source = format!("{} ({})", label, p.display());
            server.jwt_secret_path = Some(p);
        }
        Err(e) => {
            server.jwt_secret = DEFAULT_JWT_SECRET.to_string();
            server.jwt_secret_error = Some(format!("{} names a file Brum cannot use: {}", label, e));
        }
    };

    if let Some(secret) = env_secret {
        server.jwt_secret = secret;
        server.jwt_secret_source = "the BRUM_JWT_SECRET environment variable".to_string();
    } else if let Some(path) = env_file {
        from_file(&path, "BRUM_JWT_SECRET_FILE", server);
    } else if config_has_secret {
        server.jwt_secret = config_secret;
        server.jwt_secret_source = "jwt_secret in config.toml".to_string();
    } else if !config_file.is_empty() {
        from_file(&config_file, "jwt_secret_file in config.toml", server);
    }
}
fn default_session_hours() -> u64 { 72 }
/// The Program Files folders of this Windows computer.
#[cfg(windows)]
pub(crate) fn windows_program_files_dirs() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn path_key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// True when brum.exe sits in an installed location (Program Files or a Scoop app folder),
/// where data must not live next to it.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_installed_exe_dir(exe_dir: &Path, program_files: &[PathBuf]) -> bool {
    let exe = path_key(exe_dir);
    exe.contains("\\scoop\\apps\\")
        || program_files.iter().any(|pf| {
            let pf = path_key(pf);
            !pf.is_empty() && (exe == pf || exe.starts_with(&format!("{}\\", pf)))
        })
}

/// Where a relative `database_path` (such as the shipped `brum.db`) belongs on Windows, so the
/// service, the command line and desktop mode agree no matter which folder they start in:
/// - installed, as the service or administrator: the shared data folder (`C:\ProgramData\Brum`)
/// - installed, as an ordinary user: that user's own `%APPDATA%\Brum`
/// - portable: next to brum.exe
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_db_target(
    relative: &Path,
    exe_dir: &Path,
    elevated: bool,
    program_files: &[PathBuf],
    program_data: Option<&Path>,
    app_data: Option<&Path>,
) -> PathBuf {
    if is_installed_exe_dir(exe_dir, program_files) {
        let base = if elevated { program_data } else { app_data };
        if let Some(base) = base {
            return base.join("Brum").join(relative);
        }
    }
    exe_dir.join(relative)
}

/// Turns a relative `database_path` into its full location: on Windows see `windows_db_target`,
/// on Linux and macOS see `system_data_dir`. Absolute paths and in-memory databases are left as
/// they are.
pub fn resolve_database_path(path: &str) -> String {
    #[cfg(windows)]
    {
        let p = Path::new(path);
        if path.trim().is_empty() || path == ":memory:" || p.is_absolute() {
            return path.to_string();
        }
        let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else {
            return path.to_string();
        };
        let program_data = std::env::var("ProgramData").ok().map(PathBuf::from);
        let app_data = dirs::data_dir();
        let target = windows_db_target(
            p,
            &exe_dir,
            crate::win_perms::is_elevated(),
            &windows_program_files_dirs(),
            program_data.as_deref(),
            app_data.as_deref(),
        );
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        target.to_string_lossy().to_string()
    }
    #[cfg(not(windows))]
    {
        let p = Path::new(path);
        let resolved = match system_data_dir() {
            Some(dir) if !path.trim().is_empty() && path != ":memory:" && !p.is_absolute() => dir.join(p).to_string_lossy().to_string(),
            _ => path.to_string(),
        };
        move_legacy_system_data(&resolved);
        resolved
    }
}

/// True inside the Brum Docker image (or another container), where `/data` is Brum's storage.
/// Outside a container a `/data` folder is just a folder and means nothing to Brum.
pub fn in_container() -> bool {
    if std::env::var("BRUM_CONTAINER").map(|v| v.trim() == "1" || v.trim().eq_ignore_ascii_case("true")).unwrap_or(false) {
        return true;
    }
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

/// Brum's shared data folder when it runs as the system service, `/var/lib/brum`:
/// - the service: the folder systemd made for it (`StateDirectory=brum` in brum.service)
/// - root on the command line (`sudo brum reset-admin-password`, `sudo brum join`): the same
///   folder, once the service keeps its database there, so both work on the same data
///
/// None in a container (that uses `/data`) and for ordinary users (they use their home folder).
#[cfg(not(windows))]
pub fn system_data_dir() -> Option<PathBuf> {
    if in_container() {
        return None;
    }
    if let Some(dir) = systemd_dir_env("STATE_DIRECTORY") {
        return Some(dir);
    }
    // SAFETY: geteuid has no preconditions and cannot fail
    let is_root = unsafe { libc::geteuid() } == 0;
    let shared = Path::new(SYSTEM_DATA_DIR);
    if is_root && shared.join("brum.db").is_file() {
        return Some(shared.to_path_buf());
    }
    None
}

#[cfg(not(windows))]
const SYSTEM_DATA_DIR: &str = "/var/lib/brum";

/// Whether the Brum system service is installed on this machine: its unit file or its data
/// folder exists. Commands that change the service's data must then run as root.
#[cfg(not(windows))]
pub fn system_service_installed() -> bool {
    if in_container() {
        return false;
    }
    ["/usr/lib/systemd/system/brum.service", "/lib/systemd/system/brum.service", "/etc/systemd/system/brum.service"]
        .iter()
        .any(|p| Path::new(p).exists())
        || Path::new(SYSTEM_DATA_DIR).is_dir()
}

/// Whether this process may create files in a folder
#[cfg(not(windows))]
fn is_writable_dir(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
    // SAFETY: c is a valid NUL-terminated path for the duration of the call
    unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

/// A folder of Brum's own data (profile pictures, SSH host keys, ...): in `/var/lib/brum` for
/// the system service, otherwise in the user's own settings folder (`~/.config/brum`,
/// `%APPDATA%\brum`)
pub fn private_data_dir(name: &str) -> Option<PathBuf> {
    #[cfg(not(windows))]
    if let Some(shared) = system_data_dir() {
        return Some(shared.join(name));
    }
    dirs::config_dir().map(|d| d.join("brum").join(name))
}

/// The folders and files earlier versions of the system service kept in root's home folder
/// (`~/.config/brum`), moved once into `/var/lib/brum` when the service starts. Each one moves
/// on its own, and only when `/var/lib/brum` does not have it yet.
#[cfg(not(windows))]
const HOME_ITEMS: &[&str] = &["plugins", "avatars", "attachments", "known_hosts"];

#[cfg(not(windows))]
fn move_legacy_home_data() {
    let Some(state_dir) = systemd_dir_env("STATE_DIRECTORY") else { return };
    if in_container() {
        return;
    }
    let Some(old) = dirs::config_dir().map(|d| d.join("brum")) else { return };
    move_items_from(&old, &state_dir, HOME_ITEMS);
}

#[cfg(not(windows))]
fn move_items_from(old: &Path, state_dir: &Path, names: &[&str]) {
    let mut moved = Vec::new();
    for name in names {
        let (src, dst) = (old.join(name), state_dir.join(name));
        if !src.exists() || dst.exists() {
            continue;
        }
        let _ = fs::create_dir_all(state_dir);
        match copy_recursive(&src, &dst) {
            Ok(()) => {
                let _ = remove_path(&src);
                moved.push(*name);
            }
            Err(e) => {
                let _ = remove_path(&dst);
                eprintln!("brum: could not move {} to {}: {}; it stays where it is", src.display(), dst.display(), e);
            }
        }
    }
    if !moved.is_empty() {
        eprintln!("brum: moved {} from {} to {}", moved.join(", "), old.display(), state_dir.display());
        // Only removed when nothing else is left in it
        let _ = fs::remove_dir(old);
    }
}

/// A folder systemd hands the service (`STATE_DIRECTORY`, `LOGS_DIRECTORY`); the first one if
/// it lists several
#[cfg(not(windows))]
fn systemd_dir_env(name: &str) -> Option<PathBuf> {
    let v = std::env::var(name).ok()?;
    let first = v.split(':').next()?.trim();
    (!first.is_empty() && Path::new(first).is_absolute()).then(|| PathBuf::from(first))
}

/// The files of one Brum that live beside its database
#[cfg(not(windows))]
const DATA_FILES: &[&str] = &["brum.db", "brum.db-wal", "brum.db-shm", "brum-secrets.key", crate::tools::pairing::SECRET_FILE, "initial_admin_credentials.txt"];
#[cfg(not(windows))]
const DATA_FOLDERS: &[&str] = &["proton", "attachments"];

/// Once, when the system service first starts with its database in `/var/lib/brum`: moves the
/// data an earlier version kept elsewhere into it, so accounts and settings carry over.
/// Earlier versions used `/etc/brum` (the service's working folder, with the shipped config) or
/// `/data` (when that folder existed, which the Debian package created). If both have a
/// database, the one changed most recently is moved and the other is left alone.
/// Only the service itself does this, never the command line, so a running older Brum is never
/// moved out from under itself.
#[cfg(not(windows))]
fn move_legacy_system_data(db_path: &str) {
    let Some(state_dir) = systemd_dir_env("STATE_DIRECTORY") else { return };
    if in_container() || Path::new(db_path) != state_dir.join("brum.db") {
        return;
    }
    move_data_from(&[Path::new("/etc/brum"), Path::new("/data")], &state_dir);
}

/// Moves the newest of the old data folders into `state_dir`, unless it already has a database
#[cfg(not(windows))]
fn move_data_from(old_dirs: &[&Path], state_dir: &Path) {
    if state_dir.join("brum.db").exists() {
        return;
    }
    let newest = |dir: &Path| -> Option<std::time::SystemTime> {
        ["brum.db", "brum.db-wal"].iter().filter_map(|f| fs::metadata(dir.join(f)).and_then(|m| m.modified()).ok()).max()
    };
    let Some((from, _)) = old_dirs
        .iter()
        .copied()
        .filter(|d| d.join("brum.db").is_file())
        .filter_map(|d| newest(d).map(|t| (d, t)))
        .max_by_key(|(_, t)| *t)
    else {
        return;
    };
    let _ = fs::create_dir_all(state_dir);
    // All or nothing: copy everything first, and remove the old files only when every copy
    // worked, so the database and its keys are never split over two folders
    let names: Vec<&str> = DATA_FILES.iter().chain(DATA_FOLDERS).copied().filter(|n| from.join(n).exists() && !state_dir.join(n).exists()).collect();
    for (i, name) in names.iter().enumerate() {
        if let Err(e) = copy_recursive(&from.join(name), &state_dir.join(name)) {
            eprintln!("brum: could not move the data of an earlier version from {} to {} ({}: {}); it stays where it is", from.display(), state_dir.display(), name, e);
            for done in &names[..=i] {
                let _ = remove_path(&state_dir.join(done));
            }
            return;
        }
    }
    for name in &names {
        let _ = remove_path(&from.join(name));
    }
    let moved = names;
    // Start-up messages are not logged yet; the service's console output goes to the journal
    eprintln!("brum: moved the data of an earlier version from {} to {} ({})", from.display(), state_dir.display(), moved.join(", "));
    let note = format!(
        "Brum moved its data from this folder to {} on start-up: {}.\nBrum now keeps its database there and its logs in /var/log/brum. This note can be deleted.\n",
        state_dir.display(),
        moved.join(", ")
    );
    let _ = fs::write(from.join("BRUM-DATA-MOVED.txt"), note);
}

#[cfg(not(windows))]
fn remove_path(p: &Path) -> std::io::Result<()> {
    if p.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

#[cfg(not(windows))]
fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

fn default_db_path() -> String {
    if let Ok(env_path) = std::env::var("BRUM_DATABASE_PATH").or_else(|_| std::env::var("CD_DATABASE_PATH")) {
        if !env_path.trim().is_empty() {
            return env_path;
        }
    }
    #[cfg(windows)]
    {
        // 1. Portable use: a database next to brum.exe (outside Program Files and Scoop).
        //    Returned as a bare name; resolve_database_path() places it.
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            if !is_installed_exe_dir(&exe_dir, &windows_program_files_dirs()) {
                for name in ["brum.db", "commanderdog.db"] {
                    if exe_dir.join(name).is_file() {
                        return name.to_string();
                    }
                }
            }
        }

        // 2. %PROGRAMDATA%\Brum\brum.db (e.g. C:\ProgramData\Brum\brum.db): the shared data of an all-users
        //    install, used by the Windows Service and by Brum started as administrator. Other
        //    accounts keep their own data in their profile (step 3).
        if crate::win_perms::is_elevated() {
            if let Ok(progdata) = std::env::var("ProgramData") {
                let pdata_dir = Path::new(&progdata).join("Brum");
                let pdata_db = pdata_dir.join("brum.db");
                if pdata_db.is_file() {
                    return pdata_db.to_string_lossy().to_string();
                }
                if pdata_dir.is_dir() || std::fs::create_dir_all(&pdata_dir).is_ok() {
                    return pdata_db.to_string_lossy().to_string();
                }
            }
        }

        // 3. Check %APPDATA%\Brum\brum.db
        if let Some(appdata) = dirs::data_dir() {
            let app_dir = appdata.join("Brum");
            let _ = std::fs::create_dir_all(&app_dir);
            let app_db = app_dir.join("brum.db");
            return app_db.to_string_lossy().to_string();
        }

        "brum.db".to_string()
    }
    #[cfg(not(windows))]
    {
        if in_container() && Path::new("/data").is_dir() {
            if Path::new("/data/commanderdog.db").is_file() && !Path::new("/data/brum.db").is_file() {
                "/data/commanderdog.db".to_string()
            } else {
                "/data/brum.db".to_string()
            }
        } else if let Some(dir) = system_data_dir() {
            dir.join("brum.db").to_string_lossy().to_string()
        } else {
            if Path::new("commanderdog.db").is_file() && !Path::new("brum.db").is_file() {
                "commanderdog.db".to_string()
            } else {
                "brum.db".to_string()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_auth_mode")]
    pub mode: String, // "mixed", "builtin", "pam", "none"
    #[serde(default = "default_pam_service")]
    pub pam_service: String,
    #[serde(default = "default_false")]
    pub allow_guest: bool,
    #[serde(default = "default_admin_username")]
    pub default_admin_user: String,
    #[serde(default = "default_admin_password", skip_serializing)]
    pub default_admin_pass: String,
    #[serde(default)]
    pub oidc: OidcConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_oidc_provider_name")]
    pub provider_name: String, // e.g. "Authentik", "Keycloak", "SSO"
    #[serde(default)]
    pub issuer_url: String, // e.g. "https://auth.example.com/application/o/brum/"
    #[serde(default)]
    pub client_id: String,
    #[serde(default, skip_serializing)]
    pub client_secret: String,
    #[serde(default)]
    pub redirect_url: String, // e.g. "https://brum.example.com/api/auth/oidc/callback"
    #[serde(default = "default_oidc_scopes")]
    pub scopes: Vec<String>, // ["openid", "profile", "email", "groups"]
    #[serde(default = "default_true")]
    pub auto_provision: bool,
    #[serde(default = "default_oidc_admin_group")]
    pub admin_group: String, // e.g. "brum-admins" or "authentik Admins"
    #[serde(default = "default_oidc_default_role")]
    pub default_user_role: String, // "user", "readonly", "admin"
    #[serde(default = "default_user_home_template")]
    pub default_home_template: String,
    #[serde(default = "default_false")]
    pub force_sso_only: bool,
    #[serde(default = "default_oidc_button_icon")]
    pub button_icon: String, // e.g. "shield-check", "key-round", "lock"
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider_name: default_oidc_provider_name(),
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            redirect_url: String::new(),
            scopes: default_oidc_scopes(),
            auto_provision: true,
            admin_group: default_oidc_admin_group(),
            default_user_role: default_oidc_default_role(),
            default_home_template: default_user_home_template(),
            force_sso_only: false,
            button_icon: default_oidc_button_icon(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: default_auth_mode(),
            pam_service: default_pam_service(),
            allow_guest: false,
            default_admin_user: default_admin_username(),
            default_admin_pass: default_admin_password(),
            oidc: OidcConfig::default(),
        }
    }
}

fn default_auth_mode() -> String { "mixed".to_string() }
fn default_pam_service() -> String { "login".to_string() }
fn default_false() -> bool { false }
fn default_admin_username() -> String { "admin".to_string() }
fn default_admin_password() -> String { "brum".to_string() }
fn default_oidc_provider_name() -> String { "Authentik".to_string() }
fn default_oidc_scopes() -> Vec<String> { vec!["openid".to_string(), "profile".to_string(), "email".to_string(), "groups".to_string()] }
fn default_oidc_admin_group() -> String { "brum-admins".to_string() }
fn default_oidc_default_role() -> String { "user".to_string() }
fn default_oidc_button_icon() -> String { "shield-check".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_true")]
    pub file_enabled: bool,
    #[serde(default)]
    pub directory: String,
    #[serde(default = "default_log_rotation")]
    pub rotation: String,
    #[serde(default = "default_log_max_files")]
    pub max_files: usize,
    #[serde(default = "default_true")]
    pub console_enabled: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file_enabled: true,
            directory: String::new(),
            rotation: default_log_rotation(),
            max_files: default_log_max_files(),
            console_enabled: true,
        }
    }
}

fn default_log_level() -> String { "info".to_string() }
fn default_log_rotation() -> String { "daily".to_string() }
fn default_log_max_files() -> usize { 14 }

impl LoggingConfig {
    pub fn resolve_log_directory(&self) -> PathBuf {
        if !self.directory.trim().is_empty() {
            return expand_tilde_path(self.directory.trim());
        }

        #[cfg(windows)]
        {
            // The shared data folder is only open to the service and administrators
            if crate::win_perms::is_elevated() {
                if let Ok(progdata) = std::env::var("ProgramData") {
                    let pdata_dir = Path::new(&progdata).join("Brum");
                    if pdata_dir.is_dir() {
                        return pdata_dir.join("logs");
                    }
                }
            }
            if let Some(local_appdata) = dirs::data_local_dir() {
                return local_appdata.join("Brum").join("logs");
            }
            if let Some(appdata) = dirs::data_dir() {
                return appdata.join("Brum").join("logs");
            }
            PathBuf::from("logs")
        }

        #[cfg(not(windows))]
        {
            // The system service: the folder systemd made for it (LogsDirectory=brum)
            if !in_container() {
                if let Some(dir) = systemd_dir_env("LOGS_DIRECTORY") {
                    return dir;
                }
            }
            // The shared log folder, only when this Brum may write there: an ordinary user's Brum
            // on a machine with the service installed keeps its logs in its own home folder
            let var_log = Path::new("/var/log/brum");
            if var_log.is_dir() && is_writable_dir(var_log) {
                return var_log.to_path_buf();
            }
            if let Some(data_dir) = dirs::data_local_dir().or_else(dirs::data_dir) {
                return data_dir.join("brum").join("logs");
            }
            PathBuf::from("logs")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SftpConfig {
    #[serde(default = "default_host_key_checking")]
    pub host_key_checking: String, // "auto_accept", "tofu", "strict"
    #[serde(default)]
    pub known_hosts_file: Option<String>,
}

fn default_host_key_checking() -> String {
    "tofu".to_string()
}

impl Default for SftpConfig {
    fn default() -> Self {
        Self {
            host_key_checking: default_host_key_checking(),
            known_hosts_file: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Admins may browse the whole machine (every package shipped this switched on)
    #[serde(default = "default_true")]
    pub allow_entire_system: bool,
    #[serde(default = "default_false")]
    pub allow_remote_loopback: bool,
    #[serde(default = "default_user_home_template")]
    pub default_user_home_template: String,
    #[serde(default = "default_home_fallback_scheme")]
    pub home_fallback_scheme: String, // "auto", "roots_only", "strict"
    #[serde(default = "default_true")]
    pub auto_create_home_dirs: bool,
    #[serde(default)]
    pub sftp: SftpConfig,
    #[serde(default = "default_storage_roots")]
    pub roots: Vec<StorageRoot>,
}

fn default_home_fallback_scheme() -> String {
    "auto".to_string()
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            allow_entire_system: true,
            allow_remote_loopback: false,
            default_user_home_template: default_user_home_template(),
            home_fallback_scheme: default_home_fallback_scheme(),
            auto_create_home_dirs: true,
            sftp: SftpConfig::default(),
            roots: default_storage_roots(),
        }
    }
}

fn default_user_home_template() -> String {
    #[cfg(windows)]
    {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        let users = windows_users_dir(std::env::var("USERPROFILE").ok().as_deref(), std::env::var("SystemRoot").ok().as_deref(), &drive);
        format!("{users}/{{username}}")
    }
    #[cfg(not(windows))]
    {
        if in_container() && Path::new("/data").exists() {
            "/data/users/{username}".to_string()
        } else {
            "/home/{username}".to_string()
        }
    }
}

/// The folder that holds people's folders on Windows, worked out from Brum's own user folder
/// (`C:\\Users\\bolt` gives `C:/Users`). The Windows service runs as SYSTEM, whose folder is inside
/// the Windows folder (`C:\\Windows\\system32\\config\\systemprofile`): then `<drive>/Users` is used.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_users_dir(userprofile: Option<&str>, system_root: Option<&str>, system_drive: &str) -> String {
    let fallback = format!("{}/Users", system_drive.trim_end_matches(['\\', '/']));
    let Some(profile) = userprofile.map(str::trim).filter(|p| !p.is_empty()) else { return fallback };
    if system_root.is_some_and(|root| path_is_inside(profile, root)) {
        return fallback;
    }
    let profile = profile.replace('\\', "/");
    match profile.trim_end_matches('/').rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() && !parent.ends_with(':') => parent.to_string(),
        _ => fallback,
    }
}

/// Whether a Windows path is the folder `dir` or inside it (letter case and `/` or `\\` ignored)
pub fn path_is_inside(path: &str, dir: &str) -> bool {
    let norm = |p: &str| p.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let (path, dir) = (norm(path), norm(dir));
    !dir.is_empty() && (path == dir || path.starts_with(&format!("{dir}\\")))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StorageRoot {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub allowed_roles: Vec<String>,
}

fn default_storage_roots() -> Vec<StorageRoot> {
    Vec::new()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_theme_name")]
    pub default_theme: String,
    #[serde(default = "default_themes")]
    pub themes: Vec<ThemeDefinition>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            default_theme: default_theme_name(),
            themes: default_themes(),
        }
    }
}

fn default_theme_name() -> String { "amber-charcoal".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDefinition {
    pub id: String,
    pub name: String,
    pub bg_dark: String,
    pub bg_panel: String,
    pub bg_active: String,
    pub accent: String,
    pub accent_hover: String,
    pub text_main: String,
    pub text_muted: String,
    pub border: String,
    #[serde(default)]
    pub folder_color: Option<String>,
    #[serde(default)]
    pub radius: Option<String>,
    #[serde(default)]
    pub panel_gap_x: Option<String>,
    #[serde(default)]
    pub panel_gap_y: Option<String>,
    #[serde(default)]
    pub app_margin: Option<String>,
}

pub fn theme_def(
    id: &str,
    name: &str,
    bg_dark: &str,
    bg_panel: &str,
    bg_active: &str,
    accent: &str,
    accent_hover: &str,
    text_main: &str,
    text_muted: &str,
    border: &str,
) -> ThemeDefinition {
    ThemeDefinition {
        id: id.to_string(),
        name: name.to_string(),
        bg_dark: bg_dark.to_string(),
        bg_panel: bg_panel.to_string(),
        bg_active: bg_active.to_string(),
        accent: accent.to_string(),
        accent_hover: accent_hover.to_string(),
        text_main: text_main.to_string(),
        text_muted: text_muted.to_string(),
        border: border.to_string(),
        folder_color: None,
        radius: None,
        panel_gap_x: None,
        panel_gap_y: None,
        app_margin: None,
    }
}

fn default_themes() -> Vec<ThemeDefinition> {
    vec![
        theme_def(
            "amber-charcoal",
            "Woofsons Amber Charcoal",
            "#121214",
            "#18181b",
            "#27272a",
            "#f59e0b",
            "#fbbf24",
            "#f4f4f5",
            "#a1a1aa",
            "#3f3f46",
        ),
        theme_def(
            "zink",
            "Woofsons Amber Zink",
            "#fafafa",
            "#ffffff",
            "#e4e4e7",
            "#d97706",
            "#b45309",
            "#18181b",
            "#52525b",
            "#d4d4d8",
        ),
        theme_def(
            "gruvbox",
            "Gruvbox Dark",
            "#1d2021",
            "#282828",
            "#3c3836",
            "#fabd2f",
            "#fe8019",
            "#ebdbb2",
            "#a89984",
            "#504945",
        ),
        theme_def(
            "catppuccin-mocha",
            "Catppuccin Mocha",
            "#181825",
            "#1e1e2e",
            "#313244",
            "#cba6f7",
            "#f5c2e7",
            "#cdd6f4",
            "#a6adc8",
            "#45475a",
        ),
        theme_def(
            "catppuccin-latte",
            "Catppuccin Latte (Light)",
            "#dce0e8",
            "#eff1f5",
            "#e6e9ef",
            "#8839ef",
            "#1e66f5",
            "#4c4f69",
            "#6c6f85",
            "#bcc0cc",
        ),
        theme_def(
            "tokyo-night",
            "Tokyo Night",
            "#16161e",
            "#1a1b26",
            "#24283b",
            "#7aa2f7",
            "#7dcfff",
            "#c0caf5",
            "#9aa5ce",
            "#3b4261",
        ),
        theme_def(
            "monokai",
            "Monokai Pro",
            "#1e1f1c",
            "#272822",
            "#3e3d32",
            "#ffd866",
            "#a9dc76",
            "#f8f8f2",
            "#939293",
            "#49483e",
        ),
        theme_def(
            "solarized-dark",
            "Solarized Dark",
            "#00212b",
            "#002b36",
            "#073642",
            "#268bd2",
            "#2aa198",
            "#839496",
            "#657b83",
            "#586e75",
        ),
        theme_def(
            "ayu-dark",
            "Ayu Dark",
            "#0b0e14",
            "#0f1419",
            "#1f2430",
            "#e6b450",
            "#ffb454",
            "#e6e1cf",
            "#707a8c",
            "#252e37",
        ),
        theme_def(
            "nord",
            "Nord Frost",
            "#242933",
            "#2e3440",
            "#3b4252",
            "#88c0d0",
            "#81a1c1",
            "#eceff4",
            "#d8dee9",
            "#4c566a",
        ),
        theme_def(
            "dracula",
            "Dracula Dark",
            "#1e1f29",
            "#282a36",
            "#44475a",
            "#bd93f9",
            "#ff79c6",
            "#f8f8f2",
            "#6272a4",
            "#6272a4",
        ),
        theme_def(
            "midnight-blue",
            "Midnight Commander Blue",
            "#000044",
            "#000088",
            "#0000aa",
            "#00ffff",
            "#ffffff",
            "#ffffff",
            "#a0a0ff",
            "#00aaff",
        ),
        theme_def(
            "skumring",
            "Larvikite Skumring",
            "#0a0e14",
            "#111822",
            "#1e2c3d",
            "#38bdf8",
            "#7dd3fc",
            "#e6edf3",
            "#8b9bb4",
            "#243347",
        ),
        theme_def(
            "demring",
            "Larvikite Demring",
            "#eef2f6",
            "#f7fafc",
            "#cbd5e1",
            "#0e7490",
            "#155e75",
            "#0f172a",
            "#475569",
            "#cbd5e1",
        ),
        theme_def(
            "trollnatt",
            "Larvikite Trollnatt",
            "#0b100d",
            "#121914",
            "#222f26",
            "#4ade80",
            "#86efac",
            "#edf4ee",
            "#93a797",
            "#25342a",
        ),
        theme_def(
            "myrtaake",
            "Larvikite Myrtåke",
            "#edf2ee",
            "#f5f9f6",
            "#cad5cc",
            "#15803d",
            "#166534",
            "#0f1712",
            "#49594d",
            "#cbd7cd",
        ),
        theme_def(
            "bergtatt",
            "Kittelsen Bergtatt",
            "#0a0c0f",
            "#11141a",
            "#222935",
            "#d9a042",
            "#f1b759",
            "#e8e2d8",
            "#8e8d89",
            "#262e3d",
        ),
        theme_def(
            "soria-moria",
            "Kittelsen Soria Moria",
            "#ebe5dc",
            "#f5f0e6",
            "#cbbead",
            "#b87a1f",
            "#8f5a0e",
            "#1c1815",
            "#5d554a",
            "#c6bbaa",
        ),
        theme_def(
            "pestanatt",
            "Kittelsen Pestanatt",
            "#0b090a",
            "#141011",
            "#261e20",
            "#dc2626",
            "#ef4444",
            "#e6dede",
            "#948285",
            "#2b2023",
        ),
        theme_def(
            "sotslette",
            "Kittelsen Sotslette",
            "#ece6dc",
            "#f5f0e6",
            "#cec3b2",
            "#991b1b",
            "#b91c1c",
            "#1c1517",
            "#5c4f52",
            "#c7bcab",
        ),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParanoidConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_checksum_algo")]
    pub checksum_algorithm: String, // "sha256", "md5", "sha1"
    #[serde(default = "default_true")]
    pub verify_after_transfer: bool,
    #[serde(default = "default_true")]
    pub atomic_writes: bool,
    #[serde(default = "default_true")]
    pub trash_enabled: bool,
    pub custom_trash_dir: Option<String>,
    #[serde(default = "default_true")]
    pub confirm_delete: bool,
    #[serde(default = "default_true")]
    pub confirm_overwrite: bool,
    #[serde(default = "default_true")]
    pub windows_native_file_ops: bool,
    #[serde(default = "default_true")]
    pub detect_locking_processes: bool,
}

impl Default for ParanoidConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            checksum_algorithm: default_checksum_algo(),
            verify_after_transfer: true,
            atomic_writes: true,
            trash_enabled: true,
            custom_trash_dir: None,
            confirm_delete: true,
            confirm_overwrite: true,
            windows_native_file_ops: true,
            detect_locking_processes: true,
        }
    }
}

fn default_checksum_algo() -> String { "sha256".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_pane_count")]
    pub default_pane_count: usize, // 1 to 4
    #[serde(default = "default_layout_name")]
    pub default_layout: String, // "quad", "dual-vertical", "dual-horizontal", "single", "triple"
    #[serde(default = "default_true")]
    pub show_hidden_files: bool,
    #[serde(default = "default_view_mode")]
    pub default_view_mode: String, // "details", "compact", "grid"
    #[serde(default = "default_false")]
    pub show_global_refresh: bool, // Global header refresh button toggle
    #[serde(default = "default_true")]
    pub show_hostname_badge: bool, // Top header hostname badge toggle
    #[serde(default)]
    pub hostname_badge: String, // Custom label for hostname badge (empty = auto)
    #[serde(default = "default_hostname_color")]
    pub hostname_color: String, // "amber", "emerald", "sky", "purple", "rose", "orange", "cyan", "slate"
    #[serde(default = "default_hostname_style")]
    pub hostname_style: String, // "subtle", "solid", "outline", "pill", "glow"
    #[serde(default = "default_hostname_icon")]
    pub hostname_icon: String,  // "server", "hard-drive", "cpu", "terminal", "cloud", "shield", "box", "home", "globe", "radio", "none"
    #[serde(default = "default_hostname_size")]
    pub hostname_size: String,  // "sm", "md", "lg"
    #[serde(default)]
    pub window_title: String,   // Custom document / window title (empty = default "Brum - Multi-Pane Web Environment")
    #[serde(default)]
    pub document_title_template: Option<String>,
    #[serde(default)]
    pub panel_gap_x: Option<String>,
    #[serde(default)]
    pub panel_gap_y: Option<String>,
    #[serde(default)]
    pub app_margin: Option<String>,
    #[serde(default)]
    pub corner_radius: Option<String>,
    #[serde(default)]
    pub folder_color: Option<String>,
    #[serde(default)]
    pub login_title: Option<String>,
    #[serde(default)]
    pub login_subtitle_template: Option<String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            default_pane_count: default_pane_count(),
            default_layout: default_layout_name(),
            show_hidden_files: true,
            default_view_mode: default_view_mode(),
            show_global_refresh: false,
            show_hostname_badge: true,
            hostname_badge: String::new(),
            hostname_color: "slate".to_string(),
            hostname_style: "text".to_string(),
            hostname_icon: "none".to_string(),
            hostname_size: "md".to_string(),
            window_title: String::new(),
            document_title_template: None,
            panel_gap_x: None,
            panel_gap_y: None,
            app_margin: None,
            corner_radius: None,
            folder_color: None,
            login_title: None,
            login_subtitle_template: None,
        }
    }
}

fn default_pane_count() -> usize { 2 }
fn default_layout_name() -> String { "dual-vertical".to_string() }
fn default_view_mode() -> String { "details".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotesConfig {
    #[serde(default = "default_notes_folder")]
    pub notes_folder: String,
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            notes_folder: default_notes_folder(),
        }
    }
}

pub fn default_notes_folder() -> String {
    "~/Notes".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_terminal_allow_roles")]
    pub allow_roles: Vec<String>,
    #[serde(default = "default_terminal_allow_virtual_users")]
    pub allow_virtual_users: bool,
    #[serde(default = "default_terminal_drop_privileges")]
    pub drop_privileges: bool,
    #[serde(default)]
    pub default_shell: Option<String>,
    #[serde(default, alias = "user_shell")]
    pub user_shells: HashMap<String, String>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_roles: default_terminal_allow_roles(),
            allow_virtual_users: false,
            drop_privileges: true,
            default_shell: None,
            user_shells: HashMap::new(),
        }
    }
}

fn default_terminal_allow_roles() -> Vec<String> {
    vec!["admin".to_string(), "root".to_string()]
}
fn default_terminal_allow_virtual_users() -> bool {
    false
}
fn default_terminal_drop_privileges() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_plugins_system_dir")]
    pub directory: String,
    #[serde(default = "default_plugins_user_dir")]
    pub user_directory: String,
    #[serde(default = "default_false")]
    pub allow_user_installs: bool,
    #[serde(default = "default_plugin_policy")]
    pub default_policy: String, // "allow_all", "whitelist", "blacklist"
    #[serde(default = "default_global_whitelist")]
    pub global_whitelist: Vec<String>,
    #[serde(default = "default_global_blacklist")]
    pub global_blacklist: Vec<String>,
}

fn default_plugins_system_dir() -> String {
    #[cfg(windows)]
    {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let p = parent.join("plugins");
                if p.is_dir() {
                    return p.to_string_lossy().to_string();
                }
            }
        }
        if std::path::Path::new("plugins").exists() {
            return "plugins".to_string();
        }
        if let Ok(app_data) = std::env::var("PROGRAMDATA") {
            return format!("{}\\Brum\\plugins", app_data);
        }
        "C:\\ProgramData\\Brum\\plugins".to_string()
    }
    #[cfg(not(windows))]
    {
        if std::path::Path::new("plugins").exists() {
            return "plugins".to_string();
        }
        if std::path::Path::new("/usr/share/brum/plugins").exists() {
            return "/usr/share/brum/plugins".to_string();
        }
        "/etc/brum/plugins".to_string()
    }
}

fn default_plugins_user_dir() -> String {
    #[cfg(windows)]
    {
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local_appdata).join("brum").join("plugins").to_string_lossy().to_string();
        }
        if let Some(local_dir) = dirs::data_local_dir() {
            return local_dir.join("brum").join("plugins").to_string_lossy().to_string();
        }
        if let Some(app_data) = dirs::config_dir() {
            return app_data.join("brum").join("plugins").to_string_lossy().to_string();
        }
        "C:\\Users\\Default\\AppData\\Local\\brum\\plugins".to_string()
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(".config/brum/plugins").to_string_lossy().to_string();
        }
        "/data/plugins".to_string()
    }
}

fn default_plugin_policy() -> String {
    "allow_all".to_string()
}
fn default_global_whitelist() -> Vec<String> {
    vec!["*".to_string()]
}
fn default_global_blacklist() -> Vec<String> {
    vec![]
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            directory: default_plugins_system_dir(),
            user_directory: default_plugins_user_dir(),
            allow_user_installs: false,
            default_policy: default_plugin_policy(),
            global_whitelist: default_global_whitelist(),
            global_blacklist: default_global_blacklist(),
        }
    }
}

impl PluginsConfig {
    pub fn resolve_system_directory(&self) -> PathBuf {
        expand_tilde_path(&self.directory)
    }

    pub fn resolve_user_directory(&self) -> PathBuf {
        // The system service keeps Chewtoys with its other data, not in root's home folder.
        // The shipped config.toml names the personal folder, so that value counts as unset.
        #[cfg(not(windows))]
        {
            let dir = self.user_directory.trim();
            if dir.is_empty() || dir == "~/.config/brum/plugins" {
                if let Some(shared) = system_data_dir() {
                    return shared.join("plugins");
                }
            }
        }
        expand_tilde_path(&self.user_directory)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub nodes: Vec<FleetNodeConfig>,
    /// How many copies between Fleet machines run at the same time; the others wait their turn
    #[serde(default = "default_transfer_jobs")]
    pub transfer_jobs: usize,
}

fn default_transfer_jobs() -> usize {
    2
}

impl Default for FleetConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            nodes: Vec::new(),
            transfer_jobs: default_transfer_jobs(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FleetNodeConfig {
    pub id: String,
    pub name: String,
    /// The node's address. A machine reachable under several (a multi-boot box with one
    /// hostname per system) lists them in `urls` instead, or as well.
    #[serde(default)]
    pub url: String,
    /// More addresses for the same machine, tried in order after `url`; the first that answers wins
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default, skip_serializing)]
    pub token: Option<String>,
    #[serde(default)]
    pub start_path: Option<String>,
    /// Start folder per system the node reports (`linux`, `windows`, `macos`…); `start_path`
    /// is used when none matches
    #[serde(default)]
    pub start_paths: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub color_accent: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl FleetNodeConfig {
    /// Every address of the node in the order to try them: `url` first, then `urls`, without
    /// blanks, trailing slashes or repeats
    pub fn addresses(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for raw in std::iter::once(&self.url).chain(self.urls.iter()) {
            let a = raw.trim().trim_end_matches('/').to_string();
            if !a.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&a)) {
                out.push(a);
            }
        }
        out
    }

    /// The start folder for the system the node runs now, else `start_path`
    pub fn start_path_for(&self, os: &str) -> Option<String> {
        self.start_paths
            .iter()
            .find(|(k, v)| k.eq_ignore_ascii_case(os.trim()) && !v.trim().is_empty())
            .map(|(_, v)| v.trim().to_string())
            .or_else(|| self.start_path.clone().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Windows: show the Brum tray icon (`brum tray`, and desktop mode with `--minimized`)
    #[serde(default = "default_true")]
    pub enable_tray: bool,
    #[serde(default)]
    pub external_editor: Option<String>,
    #[serde(default)]
    pub external_viewer: Option<String>,
    #[serde(default)]
    pub external_terminal: Option<String>,
    #[serde(default = "default_false")]
    pub use_external_editor_f4: bool,
    #[serde(default = "default_false")]
    pub use_external_viewer_f3: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            enable_tray: true,
            external_editor: None,
            external_viewer: None,
            external_terminal: None,
            use_external_editor_f4: false,
            use_external_viewer_f3: false,
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomAction {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub command: String,
    pub applicable_to: String, // "file", "folder", "archive", "all"
    #[serde(default)]
    pub in_background: bool,
}

fn default_custom_actions() -> Vec<CustomAction> {
    vec![
        CustomAction {
            id: "sha256-calc".to_string(),
            label: "Calculate SHA-256 Checksum".to_string(),
            icon: "shield-check".to_string(),
            command: "builtin:checksum:sha256".to_string(),
            applicable_to: "file".to_string(),
            in_background: false,
        },
        CustomAction {
            id: "folder-diff".to_string(),
            label: "Compare with Other Pane (Diff)".to_string(),
            icon: "columns-2".to_string(),
            command: "builtin:diff".to_string(),
            applicable_to: "all".to_string(),
            in_background: false,
        },
        CustomAction {
            id: "compress-zip".to_string(),
            label: "Compress to .zip".to_string(),
            icon: "archive".to_string(),
            command: "builtin:archive:zip".to_string(),
            applicable_to: "all".to_string(),
            in_background: true,
        },
        CustomAction {
            id: "compress-targz".to_string(),
            label: "Compress to .tar.gz".to_string(),
            icon: "archive".to_string(),
            command: "builtin:archive:targz".to_string(),
            applicable_to: "all".to_string(),
            in_background: true,
        },
        CustomAction {
            id: "compress-7z".to_string(),
            label: "Compress to .7z".to_string(),
            icon: "archive".to_string(),
            command: "builtin:archive:7z".to_string(),
            applicable_to: "all".to_string(),
            in_background: true,
        },
        CustomAction {
            id: "extract-here".to_string(),
            label: "Extract Archive Here".to_string(),
            icon: "unarchive".to_string(),
            command: "builtin:archive:extract".to_string(),
            applicable_to: "archive".to_string(),
            in_background: true,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenWithRule {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    pub command: String,
    pub icon: String,
    #[serde(default)]
    pub is_default: bool,
}

pub fn default_open_with() -> Vec<OpenWithRule> {
    vec![
        OpenWithRule {
            id: "editor-code".to_string(),
            name: "VS Code / Cursor".to_string(),
            extensions: vec![
                "rs".to_string(), "js".to_string(), "ts".to_string(), "py".to_string(),
                "json".to_string(), "toml".to_string(), "md".to_string(), "txt".to_string(),
                "html".to_string(), "css".to_string(), "sh".to_string(), "c".to_string(), "cpp".to_string(),
            ],
            command: "code \"%1\"".to_string(),
            icon: "code".to_string(),
            is_default: false,
        },
        OpenWithRule {
            id: "media-vlc".to_string(),
            name: "VLC Media Player".to_string(),
            extensions: vec![
                "mp4".to_string(), "mkv".to_string(), "avi".to_string(), "webm".to_string(),
                "mov".to_string(), "mp3".to_string(), "flac".to_string(), "wav".to_string(),
                "ogg".to_string(), "m4a".to_string(),
            ],
            command: "vlc \"%1\"".to_string(),
            icon: "film".to_string(),
            is_default: false,
        },
        OpenWithRule {
            id: "image-viewer".to_string(),
            name: "System Default Viewer".to_string(),
            extensions: vec![
                "png".to_string(), "jpg".to_string(), "jpeg".to_string(), "webp".to_string(),
                "svg".to_string(), "gif".to_string(), "bmp".to_string(), "ico".to_string(),
            ],
            command: "open \"%1\"".to_string(),
            icon: "image".to_string(),
            is_default: false,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkConfig {
    pub id: String,
    pub name: String,
    pub protocol: String, // "local", "sftp", "webdav"
    pub path: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
}

fn default_bookmarks() -> Vec<BookmarkConfig> {
    #[cfg(windows)]
    let (root_name, root_path) = ("System Drive (C:)", "C:\\");
    #[cfg(not(windows))]
    let (root_name, root_path) = ("Root Filesystem", "/");

    vec![
        BookmarkConfig {
            id: "home".to_string(),
            name: "Home Directory".to_string(),
            protocol: "local".to_string(),
            path: dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| {
                #[cfg(windows)]
                { "C:\\Users".to_string() }
                #[cfg(not(windows))]
                { "/home".to_string() }
            }),
            host: None,
            port: None,
            username: None,
        },
        BookmarkConfig {
            id: "root".to_string(),
            name: root_name.to_string(),
            protocol: "local".to_string(),
            path: root_path.to_string(),
            host: None,
            port: None,
            username: None,
        },
    ]
}

// ----------------------------------------------------------------------------
// 🛡️ Public Client Configuration DTO (Sanitized, No Secrets)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfigDto {
    pub version: String,
    pub server: ClientServerConfigDto,
    pub auth: ClientAuthConfigDto,
    pub storage: StorageConfig,
    pub themes: ThemeConfig,
    pub paranoid: ParanoidConfig,
    pub ui: UiConfig,
    pub desktop: DesktopConfig,
    pub custom_actions: Vec<CustomAction>,
    pub open_with: Vec<OpenWithRule>,
    pub bookmarks: Vec<BookmarkConfig>,
    pub syncthing: ClientSyncthingConfigDto,
    pub notes: NotesConfig,
    pub terminal: TerminalConfig,
    pub plugins: PluginsConfig,
    pub fleet: ClientFleetConfigDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientServerConfigDto {
    pub host: String,
    pub port: u16,
    pub root_path: String,
    pub upload_max_size_mb: usize,
    pub enable_auth: bool,
    pub standalone: bool,
    pub session_duration_hours: u64,
    pub server_name: String,
    pub allow_web_config_edit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientAuthConfigDto {
    pub mode: String,
    pub pam_service: String,
    pub allow_guest: bool,
    pub default_admin_user: String,
    /// Where the first admin password note is, while it still exists
    #[serde(default)]
    pub first_login_note: Option<String>,
    pub oidc: ClientOidcConfigDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientOidcConfigDto {
    pub enabled: bool,
    pub provider_name: String,
    pub issuer_url: String,
    pub client_id: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
    pub auto_provision: bool,
    pub admin_group: String,
    pub default_user_role: String,
    pub default_home_template: String,
    pub force_sso_only: bool,
    pub button_icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSyncthingConfigDto {
    pub enabled: bool,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFleetConfigDto {
    pub enabled: bool,
    pub nodes: Vec<ClientFleetNodeConfigDto>,
    /// The machines are left out for visitors who are not signed in (the page then keeps the
    /// list it has instead of tidying it)
    #[serde(default)]
    pub nodes_hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFleetNodeConfigDto {
    pub id: String,
    pub name: String,
    /// The first address (kept for older pages)
    pub url: String,
    /// All addresses, in the order to try them
    pub urls: Vec<String>,
    pub start_path: Option<String>,
    pub start_paths: std::collections::BTreeMap<String, String>,
    pub read_only: bool,
    pub color_accent: Option<String>,
    pub tags: Vec<String>,
}

impl ClientFleetNodeConfigDto {
    pub fn from_node(n: &FleetNodeConfig) -> Self {
        Self {
            id: n.id.clone(),
            name: n.name.clone(),
            url: n.addresses().into_iter().next().unwrap_or_default(),
            urls: n.addresses(),
            start_path: n.start_path.clone(),
            start_paths: n.start_paths.clone(),
            read_only: n.read_only,
            color_accent: n.color_accent.clone(),
            tags: n.tags.clone(),
        }
    }
}

impl AppConfig {
    pub fn to_client_dto(&self) -> ClientConfigDto {
        ClientConfigDto {
            version: env!("CARGO_PKG_VERSION").to_string(),
            server: ClientServerConfigDto {
                host: self.server.host.clone(),
                port: self.server.port,
                root_path: self.server.root_path.clone(),
                upload_max_size_mb: self.server.upload_max_size_mb,
                enable_auth: self.server.enable_auth,
                standalone: self.server.standalone,
                session_duration_hours: self.server.session_duration_hours,
                server_name: self.server.server_name.clone(),
                allow_web_config_edit: self.server.allow_web_config_edit,
            },
            auth: ClientAuthConfigDto {
                mode: self.auth.mode.clone(),
                pam_service: self.auth.pam_service.clone(),
                allow_guest: self.auth.allow_guest,
                default_admin_user: self.auth.default_admin_user.clone(),
                first_login_note: None,
                oidc: ClientOidcConfigDto {
                    enabled: self.auth.oidc.enabled,
                    provider_name: self.auth.oidc.provider_name.clone(),
                    issuer_url: self.auth.oidc.issuer_url.clone(),
                    client_id: self.auth.oidc.client_id.clone(),
                    redirect_url: self.auth.oidc.redirect_url.clone(),
                    scopes: self.auth.oidc.scopes.clone(),
                    auto_provision: self.auth.oidc.auto_provision,
                    admin_group: self.auth.oidc.admin_group.clone(),
                    default_user_role: self.auth.oidc.default_user_role.clone(),
                    default_home_template: self.auth.oidc.default_home_template.clone(),
                    force_sso_only: self.auth.oidc.force_sso_only,
                    button_icon: self.auth.oidc.button_icon.clone(),
                },
            },
            storage: self.storage.clone(),
            themes: self.themes.clone(),
            paranoid: self.paranoid.clone(),
            ui: self.ui.clone(),
            desktop: self.desktop.clone(),
            custom_actions: self.custom_actions.clone(),
            open_with: self.open_with.clone(),
            bookmarks: self.bookmarks.clone(),
            syncthing: ClientSyncthingConfigDto {
                enabled: self.syncthing.enabled,
                url: self.syncthing.url.clone(),
            },
            notes: self.notes.clone(),
            terminal: self.terminal.clone(),
            plugins: self.plugins.clone(),
            fleet: ClientFleetConfigDto {
                enabled: self.fleet.enabled,
                nodes: self.fleet.nodes.iter().map(ClientFleetNodeConfigDto::from_node).collect(),
                nodes_hidden: false,
            },
        }
    }
}

/// Preprocesses raw TOML text to automatically repair unescaped Windows backslashes in double-quoted strings.
pub fn sanitize_toml_content(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 64);

    for line in input.lines() {
        let trimmed = line.trim();
        // Skip comment lines or empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            output.push_str(line);
            output.push('\n');
            continue;
        }

        let mut repaired_line = String::with_capacity(line.len() + 16);
        let mut in_double_quote = false;
        let mut in_single_quote = false;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        let len = chars.len();

        while i < len {
            let c = chars[i];

            if in_single_quote {
                repaired_line.push(c);
                if c == '\'' {
                    in_single_quote = false;
                }
                i += 1;
                continue;
            }

            if !in_double_quote {
                if c == '#' {
                    // Comment until end of line
                    repaired_line.push_str(&chars[i..].iter().collect::<String>());
                    break;
                } else if c == '\'' {
                    in_single_quote = true;
                    repaired_line.push(c);
                    i += 1;
                } else if c == '"' {
                    // Check if triple quote
                    if i + 2 < len && chars[i + 1] == '"' && chars[i + 2] == '"' {
                        // Skip multi-line strings verbatim
                        repaired_line.push_str("\"\"\"");
                        i += 3;
                    } else {
                        in_double_quote = true;
                        repaired_line.push(c);
                        i += 1;
                    }
                } else {
                    repaired_line.push(c);
                    i += 1;
                }
                continue;
            }

            // We are inside a double-quoted string
            if c == '"' {
                in_double_quote = false;
                repaired_line.push(c);
                i += 1;
                continue;
            }

            if c == '\\' {
                if i + 1 < len {
                    let next_c = chars[i + 1];
                    if next_c == '"' {
                        // Check if this is a trailing backslash at the end of a Windows path e.g. "D:\"
                        // If there is no other quote later in the line before a comment, this quote is the closing quote!
                        let remaining = &chars[i + 2..];
                        let has_another_quote = remaining.iter().take_while(|&&ch| ch != '#').any(|&ch| ch == '"');
                        if !has_another_quote {
                            // This is a trailing backslash before closing quote! Repair: \\"
                            repaired_line.push_str("\\\\\"");
                            in_double_quote = false;
                            i += 2;
                            continue;
                        } else {
                            // Intended escaped quote \" inside string
                            repaired_line.push_str("\\\"");
                            i += 2;
                            continue;
                        }
                    } else if next_c == 'u' || next_c == 'U' {
                        // Check if valid unicode escape (4 hex for \u, 8 hex for \U)
                        let hex_len = if next_c == 'u' { 4 } else { 8 };
                        let is_hex = i + 1 + hex_len < len && chars[i + 2..=i + 1 + hex_len].iter().all(|ch| ch.is_ascii_hexdigit());
                        if is_hex {
                            repaired_line.push('\\');
                            repaired_line.push(next_c);
                            i += 2;
                            continue;
                        } else {
                            // Windows folder starting with \u... (e.g. \users) -> escape as \\
                            repaired_line.push_str("\\\\");
                            i += 1;
                            continue;
                        }
                    } else {
                        // Escape single backslash as \\ so TOML parser treats it as literal \
                        repaired_line.push_str("\\\\");
                        i += 1;
                        continue;
                    }
                } else {
                    // Backslash at end of line
                    repaired_line.push_str("\\\\");
                    i += 1;
                    continue;
                }
            } else {
                repaired_line.push(c);
                i += 1;
            }
        }

        output.push_str(&repaired_line);
        output.push('\n');
    }

    output
}

/// Repairs unescaped Windows paths (such as `"C:\\Users\\bolt"`, where `\\U` would start a
/// Unicode code) only on the lines TOML rejects, one at a time, so correctly written lines are
/// never touched. Returns the repaired text and how many lines changed, or the original error
/// when the problem is not something this repair can fix.
pub fn repair_toml_windows_paths(content: &str) -> Result<(String, usize), String> {
    let mut text = content.to_string();
    let mut repaired = 0usize;
    for _ in 0..500 {
        let err = match toml::from_str::<toml::Table>(&text) {
            Ok(_) => return Ok((text, repaired)),
            Err(e) => e,
        };
        let Some(span) = err.span() else { return Err(err.to_string()) };
        let at = span.start.min(text.len());
        let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        let line = &text[start..end];
        let fixed = sanitize_toml_content(line);
        let fixed = fixed.trim_end_matches('\n');
        if fixed == line {
            return Err(err.to_string());
        }
        text.replace_range(start..end, fixed);
        repaired += 1;
    }
    Err("too many lines to repair".to_string())
}

/// Config & External Theme Manager
pub struct ConfigManager;

/// The config file the last load used (None: built-in defaults), for the start-up log line.
static LOADED_FROM: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
/// How many settings saved in Brum the last load used, for the same log line
static SAVED_USED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl ConfigManager {
    /// Parses and normalizes configuration string with smart backslash repair fallback
    pub fn parse_config_str(content: &str) -> Result<AppConfig, String> {
        match toml::from_str::<AppConfig>(content) {
            Ok(mut cfg) => {
                Self::normalize_config_paths(&mut cfg);
                Ok(cfg)
            }
            Err(e) => {
                // Repair only the lines with unescaped Windows paths, never the correct ones
                let repaired = repair_toml_windows_paths(content)
                    .map_err(|re| format!("Failed to parse config: {} (after repairing Windows paths: {})", e, re))?;
                match toml::from_str::<AppConfig>(&repaired.0) {
                    Ok(mut cfg) => {
                        info!("Parsed configuration after repairing {} line(s) with unescaped Windows paths", repaired.1);
                        Self::normalize_config_paths(&mut cfg);
                        Ok(cfg)
                    }
                    Err(repaired_err) => {
                        Err(format!("Failed to parse config: {} (after repairing Windows paths: {})", e, repaired_err))
                    }
                }
            }
        }
    }

    /// Normalizes configured storage paths and expands environment variables
    pub fn normalize_config_paths(config: &mut AppConfig) {
        config.server.root_path = crate::vfs::local::expand_windows_env_vars(&config.server.root_path);
        config.storage.default_user_home_template = crate::vfs::local::expand_windows_env_vars(&config.storage.default_user_home_template);
        if let Some(ref mut trash) = config.paranoid.custom_trash_dir {
            *trash = crate::vfs::local::expand_windows_env_vars(trash);
        }

        for root in &mut config.storage.roots {
            root.path = crate::vfs::local::expand_windows_env_vars(&root.path);
            let trimmed = root.path.trim().to_string();
            if !trimmed.is_empty() {
                root.path = trimmed;
            }
            if root.id.trim().is_empty() {
                if !root.name.trim().is_empty() {
                    root.id = root.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-");
                } else if !root.path.trim().is_empty() {
                    root.id = Path::new(&root.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "storage".to_string());
                } else {
                    root.id = "storage".to_string();
                }
            }
            if root.name.trim().is_empty() {
                root.name = if !root.path.trim().is_empty() {
                    Path::new(&root.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| root.id.clone())
                } else {
                    root.id.clone()
                };
            }
        }
        config.storage.roots.retain(|r| !r.path.trim().is_empty());
    }

    /// Returns all candidate config paths in priority order:
    /// 1. Environment variable override: $BRUM_CONFIG, $CD_CONFIG, $CONFIG_PATH, $CONFIG_FILE
    /// 2. User Roaming AppData: %APPDATA%/brum/config.toml (or ~/.config/brum/config.toml)
    /// 3. User Local AppData: %LOCALAPPDATA%/brum/config.toml
    /// 4. User Roaming AppData (legacy): %APPDATA%/commanderdog/config.toml
    /// 5. User Local AppData (legacy): %LOCALAPPDATA%/commanderdog/config.toml
    /// 6. Executable directory: <exe_dir>/config.toml, <exe_dir>/brum.toml (portable mode)
    /// 7. Current working directory: ./brum.toml, ./config.toml
    /// 8. System-wide & container config: /data/config.toml, /etc/brum/config.toml, /etc/commanderdog/config.toml
    pub fn candidate_config_paths() -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        // 1. Environment Variable Overrides
        for env_var in &["BRUM_CONFIG", "CD_CONFIG", "CONFIG_PATH", "CONFIG_FILE"] {
            if let Ok(val) = std::env::var(env_var) {
                let trimmed = val.trim();
                if !trimmed.is_empty() {
                    candidates.push(PathBuf::from(trimmed));
                }
            }
        }

        // 2. User Config Dir (~/.config or %APPDATA%)
        if let Some(d) = dirs::config_dir() {
            candidates.push(d.join("brum").join("config.toml"));
            candidates.push(d.join("commanderdog").join("config.toml"));
        }

        // 3. User Local Data Dir (%LOCALAPPDATA% on Windows, ~/.local/share on Linux)
        if let Some(d) = dirs::data_local_dir() {
            candidates.push(d.join("brum").join("config.toml"));
            candidates.push(d.join("commanderdog").join("config.toml"));
        }

        // 4. Windows Explicit %LOCALAPPDATA%
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let p = PathBuf::from(local_appdata);
            candidates.push(p.join("brum").join("config.toml"));
            candidates.push(p.join("commanderdog").join("config.toml"));
        }

        // 5. Executable Directory (Portable installations)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                candidates.push(parent.join("config.toml"));
                candidates.push(parent.join("brum.toml"));
            }
        }

        // 6. Working Directory & Container Mounts
        candidates.push(PathBuf::from("./brum.toml"));
        candidates.push(PathBuf::from("./config.toml"));
        candidates.push(PathBuf::from("/data/config.toml"));
        candidates.push(PathBuf::from("/etc/brum/config.toml"));
        candidates.push(PathBuf::from("/etc/commanderdog/config.toml"));

        candidates
    }

    /// One log line saying which config file is in use. Config is read before logging starts,
    /// so start-up writes this once logging is on.
    /// The settings file in use, for the start-up messages
    pub fn settings_location() -> String {
        let file = match LOADED_FROM.lock().ok().and_then(|l| l.clone()) {
            Some(p) => std::fs::canonicalize(&p).unwrap_or(p).display().to_string(),
            None => "built-in defaults (no config.toml found)".to_string(),
        };
        match SAVED_USED.load(std::sync::atomic::Ordering::Relaxed) {
            0 => file,
            1 => format!("{file} + 1 setting saved in Brum"),
            n => format!("{file} + {n} settings saved in Brum"),
        }
    }

    /// Reads a config file into its sections, repairing unescaped Windows paths like loading does
    pub fn parse_table_str(content: &str) -> Option<toml::Table> {
        toml::from_str::<toml::Table>(content).ok().or_else(|| {
            let (repaired, _) = repair_toml_windows_paths(content).ok()?;
            toml::from_str::<toml::Table>(&repaired).ok()
        })
    }

    /// The sections of the config file a running Brum was started with (None: no file, or it
    /// cannot be read now)
    pub fn read_config_file_table(path: Option<&Path>) -> Option<toml::Table> {
        Self::parse_table_str(&fs::read_to_string(path?).ok()?)
    }

    /// Resolves the currently active configuration path, or the preferred writable location if none exists
    pub fn active_config_path() -> PathBuf {
        for candidate in Self::candidate_config_paths() {
            if candidate.is_file() {
                return candidate;
            }
        }

        // Default write location if no existing config file was found
        if let Some(user_config) = dirs::config_dir().map(|d| d.join("brum").join("config.toml")) {
            user_config
        } else if let Some(local_config) = dirs::data_local_dir().map(|d| d.join("brum").join("config.toml")) {
            local_config
        } else {
            PathBuf::from("./config.toml")
        }
    }

    /// Loads configuration with sub-millisecond fast-path resolution across all standard paths.
    /// Also scans external theme directories (~/.config/brum/themes/*.toml, %LOCALAPPDATA%/brum/themes/*.toml).
    pub fn load_all() -> AppConfig {
        // Ensure user config and themes directories exist (for a person, not for the system
        // service: nothing of the service belongs in root's home folder)
        #[cfg(not(windows))]
        let personal = system_data_dir().is_none();
        #[cfg(windows)]
        let personal = true;
        #[cfg(not(windows))]
        move_legacy_home_data();
        if let Some(user_config_dir) = dirs::config_dir().map(|d| d.join("brum")).filter(|_| personal) {
            let _ = fs::create_dir_all(user_config_dir.join("themes"));
        }
        if let Some(local_data_dir) = dirs::data_local_dir().map(|d| d.join("brum")).filter(|_| personal) {
            let _ = fs::create_dir_all(local_data_dir.join("themes"));
        }
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let _ = fs::create_dir_all(PathBuf::from(local_appdata).join("brum").join("themes"));
        }

        let mut config = AppConfig::default();
        let mut file_table: Option<toml::Table> = None;

        for candidate in Self::candidate_config_paths() {
            if candidate.is_file() {
                info!("Loading master configuration: {}", candidate.display());
                match fs::read_to_string(&candidate) {
                    Ok(content) => match Self::parse_config_str(&content) {
                        Ok(parsed) => {
                            config = parsed;
                            config.server.config_file = Some(candidate.clone());
                            file_table = Self::parse_table_str(&content);
                            if let Ok(mut l) = LOADED_FROM.lock() {
                                *l = Some(candidate.clone());
                            }
                            break; // Stop immediately on first matching priority config
                        }
                        Err(e) => {
                            warn!("Failed to parse config {}: {}, falling back to defaults", candidate.display(), e);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to read config {}: {}", candidate.display(), e);
                    }
                }
            }
        }

        // Settings saved in Brum (#195) go under the file: anything the file sets wins
        Self::apply_saved_settings(&mut config, file_table.as_ref());

        // Ensure all built-in default themes (e.g. zink, amber-charcoal) are present
        // even if the user has an existing older config.toml file on disk.
        for dt in default_themes() {
            if !config.themes.themes.iter().any(|t| t.id == dt.id) {
                config.themes.themes.push(dt);
            }
        }

        // Discover and load external themes from themes/ directories
        Self::load_external_themes(&mut config);

        // Environment Variable Overrides for Docker & Cloud Deployments
        if let Ok(p) = std::env::var("BRUM_PORT").or_else(|_| std::env::var("CD_PORT")).or_else(|_| std::env::var("PORT")) {
            if let Ok(port_num) = p.parse::<u16>() {
                config.server.port = port_num;
            }
        }
        if let Ok(h) = std::env::var("BRUM_BIND").or_else(|_| std::env::var("BRUM_HOST")).or_else(|_| std::env::var("CD_BIND")).or_else(|_| std::env::var("CD_HOST")).or_else(|_| std::env::var("HOST")) {
            if !h.trim().is_empty() {
                config.server.host = h.trim().to_string();
            }
        }
        if let Ok(db) = std::env::var("BRUM_DATABASE_PATH").or_else(|_| std::env::var("CD_DATABASE_PATH")).or_else(|_| std::env::var("DATABASE_PATH")) {
            if !db.trim().is_empty() {
                config.server.database_path = db.trim().to_string();
            }
        }
        resolve_jwt_secret(
            &mut config.server,
            std::env::var("BRUM_JWT_SECRET").or_else(|_| std::env::var("CD_JWT_SECRET")).or_else(|_| std::env::var("JWT_SECRET")).ok(),
            std::env::var("BRUM_JWT_SECRET_FILE").ok(),
        );
        if let Ok(v) = std::env::var("BRUM_ALLOW_WEB_CONFIG_EDIT").or_else(|_| std::env::var("ALLOW_WEB_CONFIG_EDIT")) {
            config.server.allow_web_config_edit = v.eq_ignore_ascii_case("true") || v == "1";
        }

        // OIDC / SSO Environment Variable Overrides
        if let Ok(v) = std::env::var("BRUM_OIDC_ENABLED").or_else(|_| std::env::var("OIDC_ENABLED")) {
            config.auth.oidc.enabled = v.eq_ignore_ascii_case("true") || v == "1";
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_ISSUER_URL").or_else(|_| std::env::var("OIDC_ISSUER_URL")) {
            if !v.trim().is_empty() { config.auth.oidc.issuer_url = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_CLIENT_ID").or_else(|_| std::env::var("OIDC_CLIENT_ID")) {
            if !v.trim().is_empty() { config.auth.oidc.client_id = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_CLIENT_SECRET").or_else(|_| std::env::var("OIDC_CLIENT_SECRET")) {
            if !v.trim().is_empty() { config.auth.oidc.client_secret = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_REDIRECT_URL").or_else(|_| std::env::var("OIDC_REDIRECT_URL")) {
            if !v.trim().is_empty() { config.auth.oidc.redirect_url = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_PROVIDER_NAME").or_else(|_| std::env::var("OIDC_PROVIDER_NAME")) {
            if !v.trim().is_empty() { config.auth.oidc.provider_name = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_ADMIN_GROUP").or_else(|_| std::env::var("OIDC_ADMIN_GROUP")) {
            if !v.trim().is_empty() { config.auth.oidc.admin_group = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_FORCE_SSO").or_else(|_| std::env::var("OIDC_FORCE_SSO")) {
            config.auth.oidc.force_sso_only = v.eq_ignore_ascii_case("true") || v == "1";
        }

        // Windows: a bare database name never depends on the folder Brum was started from
        config.server.database_path = resolve_database_path(&config.server.database_path);

        config
    }

    /// Where the database is, as start-up will decide it (the file, then the environment)
    fn boot_database_path(config: &AppConfig) -> String {
        let db = std::env::var("BRUM_DATABASE_PATH")
            .or_else(|_| std::env::var("CD_DATABASE_PATH"))
            .or_else(|_| std::env::var("DATABASE_PATH"))
            .ok()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| config.server.database_path.clone());
        resolve_database_path(&db)
    }

    /// Puts the settings saved in Brum under the config file's. If they cannot be used, Brum
    /// starts with the file alone and says why.
    fn apply_saved_settings(config: &mut AppConfig, file_table: Option<&toml::Table>) {
        SAVED_USED.store(0, std::sync::atomic::Ordering::Relaxed);
        if config.server.config_file.is_some() && file_table.is_none() {
            return; // the file was read by repairing it in a way we could not repeat; keep it as is
        }
        let saved = saved::read_from_db_file(&Self::boot_database_path(config));
        if saved.is_empty() {
            return;
        }
        match saved::compose(file_table, &saved) {
            Ok(mut merged) => {
                merged.server.config_file = config.server.config_file.clone();
                *config = merged;
                SAVED_USED.store(saved.len(), std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => warn!("The settings saved in Brum could not be used ({e}); starting with the config file alone"),
        }
    }

    fn collect_theme_files_sorted(dir: &Path, files: &mut Vec<PathBuf>) {
        if dir.exists() && dir.is_dir() {
            if let Ok(entries) = fs::read_dir(dir) {
                let mut dir_files: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension().map_or(false, |ext| {
                                ext.eq_ignore_ascii_case("toml") || ext.eq_ignore_ascii_case("conf")
                            })
                    })
                    .collect();
                dir_files.sort();
                files.extend(dir_files);
            }
        }
    }

    /// Scans external theme directories and loads theme definitions
    fn load_external_themes(config: &mut AppConfig) {
        let mut theme_dirs = vec![
            PathBuf::from("/etc/brum/themes"),
            PathBuf::from("/etc/commanderdog/themes"),
            PathBuf::from("./themes"),
        ];
        if let Some(d) = dirs::config_dir() {
            theme_dirs.push(d.join("brum").join("themes"));
            theme_dirs.push(d.join("commanderdog").join("themes"));
        }
        if let Some(d) = dirs::data_local_dir() {
            theme_dirs.push(d.join("brum").join("themes"));
            theme_dirs.push(d.join("commanderdog").join("themes"));
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            theme_dirs.push(PathBuf::from(&local_app_data).join("brum").join("themes"));
            theme_dirs.push(PathBuf::from(local_app_data).join("commanderdog").join("themes"));
        }
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                theme_dirs.push(parent.join("themes"));
            }
        }

        let mut theme_files = Vec::new();
        for dir in theme_dirs {
            Self::collect_theme_files_sorted(&dir, &mut theme_files);
        }

        for file_path in theme_files {
            info!("Loading external theme definition: {}", file_path.display());
            if let Ok(content) = fs::read_to_string(&file_path) {
                let stem = file_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "custom".to_string());
                Self::parse_and_insert_themes(config, &content, &stem);
            }
        }
    }

    fn parse_and_insert_themes(config: &mut AppConfig, content: &str, file_stem: &str) {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ThemeListOrNested {
            List(Vec<ThemeDefinition>),
            Nested { themes: Vec<ThemeDefinition> },
        }

        // Try parsing as multi-theme array struct: [[themes]], [themes] themes = [...], [dark], [light]
        #[derive(Deserialize)]
        struct MultiThemeContainer {
            themes: Option<ThemeListOrNested>,
            theme: Option<ThemeDefinition>,
            dark: Option<ThemeDefinition>,
            light: Option<ThemeDefinition>,
        }

        if let Ok(container) = toml::from_str::<MultiThemeContainer>(content) {
            let mut matched = false;
            if let Some(list_or_nested) = container.themes {
                let list = match list_or_nested {
                    ThemeListOrNested::List(l) => l,
                    ThemeListOrNested::Nested { themes } => themes,
                };
                for t in list {
                    Self::upsert_theme(&mut config.themes.themes, t);
                }
                matched = true;
            }
            if let Some(t) = container.theme {
                Self::upsert_theme(&mut config.themes.themes, t);
                matched = true;
            }
            if let Some(mut d) = container.dark {
                if d.id.is_empty() {
                    d.id = format!("{}-dark", file_stem);
                }
                Self::upsert_theme(&mut config.themes.themes, d);
                matched = true;
            }
            if let Some(mut l) = container.light {
                if l.id.is_empty() {
                    l.id = format!("{}-light", file_stem);
                }
                Self::upsert_theme(&mut config.themes.themes, l);
                matched = true;
            }
            if matched {
                return;
            }
        }

        // Try parsing directly as a single flat ThemeDefinition:
        // id = "...", name = "...", bg_dark = "...", ...
        #[derive(Deserialize)]
        struct FlatTheme {
            id: Option<String>,
            name: Option<String>,
            bg_dark: String,
            bg_panel: String,
            bg_active: String,
            accent: String,
            accent_hover: Option<String>,
            text_main: String,
            text_muted: String,
            border: String,
            folder_color: Option<String>,
            radius: Option<String>,
            panel_gap_x: Option<String>,
            panel_gap_y: Option<String>,
            app_margin: Option<String>,
        }

        if let Ok(flat) = toml::from_str::<FlatTheme>(content) {
            let id = flat.id.unwrap_or_else(|| file_stem.to_string());
            let name = flat.name.unwrap_or_else(|| {
                file_stem
                    .split(['-', '_'])
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        }
                    })
                    .collect::<Vec<String>>()
                    .join(" ")
            });
            let accent = flat.accent.clone();
            let accent_hover = flat.accent_hover.unwrap_or(accent);

            let theme = ThemeDefinition {
                id,
                name,
                bg_dark: flat.bg_dark,
                bg_panel: flat.bg_panel,
                bg_active: flat.bg_active,
                accent: flat.accent,
                accent_hover,
                text_main: flat.text_main,
                text_muted: flat.text_muted,
                border: flat.border,
                folder_color: flat.folder_color,
                radius: flat.radius,
                panel_gap_x: flat.panel_gap_x,
                panel_gap_y: flat.panel_gap_y,
                app_margin: flat.app_margin,
            };
            Self::upsert_theme(&mut config.themes.themes, theme);
        }
    }

    fn upsert_theme(themes: &mut Vec<ThemeDefinition>, new_theme: ThemeDefinition) {
        if let Some(existing) = themes.iter_mut().find(|t| t.id == new_theme.id) {
            *existing = new_theme;
        } else {
            themes.push(new_theme);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_personal_folders_never_go_inside_the_windows_folder() {
        use super::{path_is_inside, windows_users_dir};
        let root = Some(r"C:\Windows");
        // The service runs as SYSTEM: its own folder is inside the Windows folder
        assert_eq!(windows_users_dir(Some(r"C:\Windows\system32\config\systemprofile"), root, "C:"), "C:/Users");
        assert_eq!(windows_users_dir(Some(r"c:\windows\System32\Config\systemprofile\"), root, "C:"), "C:/Users");
        // Started by a person: the folder next to theirs
        assert_eq!(windows_users_dir(Some(r"C:\Users\terje"), root, "C:"), "C:/Users");
        assert_eq!(windows_users_dir(Some(r"D:\Profiles\terje"), root, "C:"), "D:/Profiles");
        // Nothing usable
        assert_eq!(windows_users_dir(None, root, "C:"), "C:/Users");
        assert_eq!(windows_users_dir(Some(r"C:\"), root, "C:"), "C:/Users");
        assert!(path_is_inside(r"C:/Windows/system32/config/bolt", r"C:\Windows"));
        assert!(path_is_inside(r"C:\WINDOWS", r"C:\Windows\"));
        assert!(!path_is_inside(r"C:\WindowsApps\x", r"C:\Windows"));
        assert!(!path_is_inside(r"C:\Users\bolt", r"C:\Windows"));
    }

    #[cfg(not(windows))]
    #[test]
    fn old_home_items_move_one_by_one() {
        use std::fs;
        let tmp = tempfile::tempdir().unwrap();
        let (old, state) = (tmp.path().join("home/.config/brum"), tmp.path().join("state"));
        fs::create_dir_all(old.join("plugins/calculator")).unwrap();
        fs::create_dir_all(old.join("themes")).unwrap();
        fs::create_dir_all(state.join("avatars")).unwrap();
        fs::create_dir_all(old.join("avatars")).unwrap();
        fs::write(old.join("plugins/calculator/plugin.toml"), "x").unwrap();
        fs::write(old.join("known_hosts"), "host").unwrap();
        fs::write(old.join("avatars/old.png"), "a").unwrap();

        super::move_items_from(&old, &state, super::HOME_ITEMS);
        assert!(state.join("plugins/calculator/plugin.toml").is_file() && !old.join("plugins").exists());
        assert_eq!(fs::read_to_string(state.join("known_hosts")).unwrap(), "host");
        // Already there: left alone on both sides; not one of Brum's data folders: not touched
        assert!(old.join("avatars/old.png").is_file() && !state.join("avatars/old.png").exists());
        assert!(old.join("themes").is_dir());
    }

    #[cfg(not(windows))]
    #[test]
    fn old_system_data_moves_once_and_newest_wins() {
        use std::fs;
        let tmp = tempfile::tempdir().unwrap();
        let (etc, data, state) = (tmp.path().join("etc"), tmp.path().join("data"), tmp.path().join("state"));
        fs::create_dir_all(&etc).unwrap();
        fs::create_dir_all(data.join("attachments")).unwrap();
        fs::write(etc.join("brum.db"), "old").unwrap();
        fs::write(etc.join("config.toml"), "kept").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(data.join("brum.db"), "new").unwrap();
        fs::write(data.join("brum-secrets.key"), "key").unwrap();
        fs::write(data.join("attachments/a.png"), "img").unwrap();
        fs::write(data.join("other.txt"), "not Brum's").unwrap();

        super::move_data_from(&[&etc, &data], &state);
        assert_eq!(fs::read_to_string(state.join("brum.db")).unwrap(), "new");
        assert_eq!(fs::read_to_string(state.join("brum-secrets.key")).unwrap(), "key");
        assert_eq!(fs::read_to_string(state.join("attachments/a.png")).unwrap(), "img");
        assert!(!data.join("brum.db").exists() && !data.join("attachments").exists());
        assert!(data.join("other.txt").exists() && data.join("BRUM-DATA-MOVED.txt").exists());
        // The older one is left alone, and nothing happens a second time
        assert_eq!(fs::read_to_string(etc.join("brum.db")).unwrap(), "old");
        super::move_data_from(&[&etc, &data], &state);
        assert_eq!(fs::read_to_string(state.join("brum.db")).unwrap(), "new");
        assert!(etc.join("brum.db").exists() && etc.join("config.toml").exists());
    }

    #[test]
    fn test_windows_db_target_by_install_kind() {
        use super::{is_installed_exe_dir, windows_db_target};
        use std::path::{Path, PathBuf};
        let pf = vec![PathBuf::from(r"C:\Program Files"), PathBuf::from(r"C:\Program Files (x86)")];
        let pd = Path::new(r"C:\ProgramData");
        let ad = Path::new(r"C:\Users\alice\AppData\Roaming");
        let db = Path::new("brum.db");
        let key = |p: PathBuf| p.to_string_lossy().replace('/', "\\").to_lowercase();

        // Installed in Program Files: service/administrator share the data folder, users keep their own
        let inst = Path::new(r"C:\Program Files\Brum");
        assert!(is_installed_exe_dir(inst, &pf));
        assert_eq!(key(windows_db_target(db, inst, true, &pf, Some(pd), Some(ad))), r"c:\programdata\brum\brum.db");
        assert_eq!(key(windows_db_target(db, inst, false, &pf, Some(pd), Some(ad))), r"c:\users\alice\appdata\roaming\brum\brum.db");

        // Scoop counts as installed (its app folder is replaced on every update)
        let scoop = Path::new(r"C:\Users\alice\scoop\apps\brum\current");
        assert!(is_installed_exe_dir(scoop, &pf));
        assert_eq!(key(windows_db_target(db, scoop, false, &pf, Some(pd), Some(ad))), r"c:\users\alice\appdata\roaming\brum\brum.db");

        // Portable folder: next to brum.exe, whoever runs it and from wherever
        let port = Path::new(r"D:\Tools\Brum");
        assert!(!is_installed_exe_dir(port, &pf));
        assert_eq!(key(windows_db_target(db, port, true, &pf, Some(pd), Some(ad))), r"d:\tools\brum\brum.db");

        // A folder whose name merely starts like Program Files is not inside it
        assert!(!is_installed_exe_dir(Path::new(r"C:\Program Files Extra\Brum"), &pf));
    }

    use super::*;

    #[test]
    fn notes_section_accepts_old_and_new_names() {
        let old: AppConfig = toml::from_str("[notedog]\nnotes_folder = \"~/OldNotes\"\n").unwrap();
        assert_eq!(old.notes.notes_folder, "~/OldNotes");
        let new: AppConfig = toml::from_str("[notes]\nnotes_folder = \"~/NewNotes\"\n").unwrap();
        assert_eq!(new.notes.notes_folder, "~/NewNotes");
    }

    #[test]
    fn test_external_theme_flat_parsing() {
        let mut config = AppConfig::default();
        let sample_toml = r##"
            id = "hyprland-cyan"
            name = "Hyprland Cyan"
            bg_dark = "#0b0f14"
            bg_panel = "#111822"
            bg_active = "#1b2533"
            accent = "#00e5ff"
            accent_hover = "#33ebff"
            text_main = "#e1e7ec"
            text_muted = "#7a889b"
            border = "#00e5ff"
        "##;

        ConfigManager::parse_and_insert_themes(&mut config, sample_toml, "hyprland-cyan");
        let theme = config.themes.themes.iter().find(|t| t.id == "hyprland-cyan");
        assert!(theme.is_some());
        let t = theme.unwrap();
        assert_eq!(t.name, "Hyprland Cyan");
        assert_eq!(t.accent, "#00e5ff");
    }

    #[test]
    fn test_external_theme_multi_parsing() {
        let mut config = AppConfig::default();
        let sample_toml = r##"
            [[themes]]
            id = "custom-one"
            name = "Custom One"
            bg_dark = "#101010"
            bg_panel = "#202020"
            bg_active = "#303030"
            accent = "#ff0055"
            accent_hover = "#ff3377"
            text_main = "#ffffff"
            text_muted = "#888888"
            border = "#444444"
        "##;

        ConfigManager::parse_and_insert_themes(&mut config, sample_toml, "themes");
        let theme = config.themes.themes.iter().find(|t| t.id == "custom-one");
        assert!(theme.is_some());
        assert_eq!(theme.unwrap().accent, "#ff0055");
    }

    #[test]
    fn test_master_config_parsing() {
        let sample_toml = r##"
            [server]
            host = "127.0.0.1"
            port = 9090

            [ui]
            window_decorations = false
            show_global_refresh = false

            [desktop]
            enable_tray = false
            # Removed settings: older config files that still have them keep loading
            global_summon_hotkey = "Super+C"
            start_minimized = true
            external_editor = "code \"%1\""
            use_external_editor_f4 = true

            [themes]
            default_theme = "catppuccin-mocha"
        "##;

        let config: AppConfig = toml::from_str(sample_toml).unwrap();
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.ui.show_global_refresh, false);
        assert!(!config.desktop.enable_tray);
        assert_eq!(config.desktop.external_editor, Some("code \"%1\"".to_string()));
        assert_eq!(config.desktop.use_external_editor_f4, true);
        assert_eq!(config.themes.default_theme, "catppuccin-mocha");
    }

    #[test]
    fn test_storage_config_parsing() {
        let sample_toml = r##"
            [storage]
            allow_entire_system = false
            default_user_home_template = "/users/{username}"
            home_fallback_scheme = "roots_only"
            auto_create_home_dirs = false

            [[storage.roots]]
            id = "vault"
            name = "Secure Vault"
            path = "/mnt/vault"
            read_only = true
            allowed_roles = ["admin"]

            [[storage.roots]]
            id = "share"
            name = "Public Share"
            path = "/mnt/share"
            read_only = false
        "##;

        let config: AppConfig = toml::from_str(sample_toml).unwrap();
        assert_eq!(config.storage.allow_entire_system, false);
        assert_eq!(config.storage.default_user_home_template, "/users/{username}");
        assert_eq!(config.storage.home_fallback_scheme, "roots_only");
        assert_eq!(config.storage.auto_create_home_dirs, false);
        assert_eq!(config.storage.roots.len(), 2);
        assert_eq!(config.storage.roots[0].id, "vault");
        assert_eq!(config.storage.roots[0].read_only, true);
        assert_eq!(config.storage.roots[1].name, "Public Share");
    }

    #[test]
    fn test_open_with_and_custom_actions_parsing() {
        let sample_toml = r##"
            [[open_with]]
            id = "custom-vlc"
            name = "VLC Player"
            extensions = ["mp4", "mkv"]
            command = "vlc %1"
            icon = "film"
            is_default = true

            [[custom_actions]]
            id = "git-pull"
            label = "Git Pull"
            icon = "git-pull-request"
            command = "git -C {dir} pull"
            applicable_to = "folder"
            in_background = false
        "##;

        let config: AppConfig = toml::from_str(sample_toml).unwrap();
        assert_eq!(config.open_with.len(), 1);
        assert_eq!(config.open_with[0].id, "custom-vlc");
        assert_eq!(config.open_with[0].extensions, vec!["mp4", "mkv"]);
        assert_eq!(config.custom_actions.len(), 1);
        assert_eq!(config.custom_actions[0].command, "git -C {dir} pull");
    }

    #[test]
    fn test_terminal_config_parsing() {
        let sample_toml = r#"
            [terminal]
            enabled = true
            allow_roles = ["admin", "operator"]
            allow_virtual_users = false
            drop_privileges = true
            default_shell = "/bin/bash"

            [terminal.user_shells]
            bolt = "/usr/bin/fish"
            alice = "/bin/zsh"
        "#;

        let config: AppConfig = toml::from_str(sample_toml).unwrap();
        assert_eq!(config.terminal.enabled, true);
        assert_eq!(config.terminal.allow_roles, vec!["admin", "operator"]);
        assert_eq!(config.terminal.allow_virtual_users, false);
        assert_eq!(config.terminal.drop_privileges, true);
        assert_eq!(config.terminal.default_shell, Some("/bin/bash".to_string()));
        assert_eq!(config.terminal.user_shells.get("bolt"), Some(&"/usr/bin/fish".to_string()));
        assert_eq!(config.terminal.user_shells.get("alice"), Some(&"/bin/zsh".to_string()));

        // Also accepts singular [terminal.user_shell] alias
        let singular_toml = r#"
            [terminal.user_shell]
            bolt = "/usr/bin/fish"
        "#;
        let singular_cfg: AppConfig = toml::from_str(singular_toml).unwrap();
        assert_eq!(singular_cfg.terminal.user_shells.get("bolt"), Some(&"/usr/bin/fish".to_string()));
    }

    #[test]
    fn test_repair_only_touches_broken_lines() {
        let raw = r#"
[server]
port = 3140
root_path = "C:\\Data"
note = "tab\there"

[[fleet.nodes]]
id = "pc"
name = "PC"
url = "http://pc:3140"
start_path = "C:\Users\user"
"#;
        let (fixed, n) = super::repair_toml_windows_paths(raw).expect("repairable");
        assert_eq!(n, 1, "only the start_path line needs repair");
        assert!(fixed.contains(r#"root_path = "C:\\Data""#), "correct escapes stay as they were");
        assert!(fixed.contains(r#"note = "tab\there""#), "valid escape sequences stay as they were");
        assert!(fixed.contains(r#"start_path = "C:\\Users\\user""#));
        let cfg = ConfigManager::parse_config_str(raw).expect("loads");
        assert_eq!(cfg.server.root_path, r"C:\Data");
        assert_eq!(cfg.fleet.nodes[0].start_path.as_deref(), Some(r"C:\Users\user"));
        // Real syntax errors are still reported, not "repaired"
        assert!(super::repair_toml_windows_paths("port = = 1").is_err());
    }

    #[test]
    fn test_windows_unescaped_backslashes_auto_repair() {
        let raw_toml = r#"
            [storage]
            allow_entire_system = true
            default_user_home_template = "C:\Users\{username}"

            [[storage.roots]]
            id = "d-drive"
            name = "D Drive"
            path = "D:\Storage\Media"
            read_only = false

            [[storage.roots]]
            id = "samba-share"
            name = "NAS Samba"
            path = "\\192.168.1.100\share\data"
            read_only = true

            [[storage.roots]]
            id = "d-root"
            name = "D Root"
            path = "D:\"
            read_only = false

            [[storage.roots]]
            id = "literal-single"
            name = "Single Quoted"
            path = 'C:\Users\Photos'
            read_only = false
        "#;

        let parsed = ConfigManager::parse_config_str(raw_toml).expect("Should parse despite unescaped backslashes");
        assert_eq!(parsed.storage.default_user_home_template, "C:\\Users\\{username}");
        assert_eq!(parsed.storage.roots.len(), 4);
        assert_eq!(parsed.storage.roots[0].path, "D:\\Storage\\Media");
        assert_eq!(parsed.storage.roots[1].path, "\\\\192.168.1.100\\share\\data");
        assert_eq!(parsed.storage.roots[2].path, "D:\\");
        assert_eq!(parsed.storage.roots[3].path, "C:\\Users\\Photos");
    }

    #[test]
    fn test_sftp_config_parsing() {
        // 1. Default fallback
        let cfg_default: AppConfig = toml::from_str("").unwrap();
        assert_eq!(cfg_default.get_sftp_config().host_key_checking, "tofu");
        assert_eq!(cfg_default.get_sftp_config().known_hosts_file, None);

        // 2. [storage.sftp] nested format
        let toml_storage = r#"
            [storage.sftp]
            host_key_checking = "strict"
            known_hosts_file = "/etc/ssh/ssh_known_hosts"
        "#;
        let cfg_storage: AppConfig = toml::from_str(toml_storage).unwrap();
        assert_eq!(cfg_storage.get_sftp_config().host_key_checking, "strict");
        assert_eq!(cfg_storage.get_sftp_config().known_hosts_file, Some("/etc/ssh/ssh_known_hosts".to_string()));

        // 3. Top-level [sftp] format
        let toml_toplevel = r#"
            [sftp]
            host_key_checking = "auto_accept"
            known_hosts_file = "~/.ssh/known_hosts"
        "#;
        let cfg_top: AppConfig = toml::from_str(toml_toplevel).unwrap();
        assert_eq!(cfg_top.get_sftp_config().host_key_checking, "auto_accept");
        assert_eq!(cfg_top.get_sftp_config().known_hosts_file, Some("~/.ssh/known_hosts".to_string()));
    }

    #[test]
    fn test_storage_roots_resilient_parsing() {
        let raw_toml = r#"
            [[storage.roots]]
            name = "Incomplete Root Without Path"

            [[storage.roots]]
            path = "/var/log"

            [[storage.roots]]
            id = "custom_id"
            name = "Custom Name"
            path = "/var/data"
        "#;

        let mut parsed = ConfigManager::parse_config_str(raw_toml).expect("Should parse even with missing fields");
        ConfigManager::normalize_config_paths(&mut parsed);
        assert_eq!(parsed.storage.roots.len(), 2);
        assert_eq!(parsed.storage.roots[0].path, "/var/log");
        assert_eq!(parsed.storage.roots[0].name, "log");
        assert_eq!(parsed.storage.roots[0].id, "log");
        assert_eq!(parsed.storage.roots[1].id, "custom_id");
        assert_eq!(parsed.storage.roots[1].name, "Custom Name");
        assert_eq!(parsed.storage.roots[1].path, "/var/data");
    }

    #[test]
    fn test_root_config_toml_parses_cleanly() {
        if Path::new("config.toml").is_file() {
            let content = fs::read_to_string("config.toml").expect("Should read config.toml");
            let parsed = ConfigManager::parse_config_str(&content).expect("config.toml should parse cleanly into AppConfig");
            assert_eq!(parsed.server.port, 3140);
            assert!(parsed.fleet.enabled);
            assert!(parsed.plugins.enabled);
        }
    }

    #[test]
    fn test_expand_tilde_path_and_plugins_dir_resolution() {
        if let Some(home) = dirs::home_dir().or_else(|| std::env::var("HOME").ok().map(PathBuf::from)) {
            assert_eq!(expand_tilde_path("~"), home);
            assert_eq!(expand_tilde_path("~/"), home);
            assert_eq!(expand_tilde_path("~/.config/brum/plugins"), home.join(".config/brum/plugins"));
            assert_eq!(expand_tilde_path("~/custom/dir"), home.join("custom/dir"));

            let plugins = PluginsConfig::default();
            assert_eq!(plugins.resolve_user_directory(), home.join(".config/brum/plugins"));
        }

        // Non-tilde absolute paths should be unchanged
        assert_eq!(expand_tilde_path("/etc/brum/plugins"), PathBuf::from("/etc/brum/plugins"));
        assert_eq!(expand_tilde_path("relative/path"), PathBuf::from("relative/path"));
    }
}


#[cfg(test)]
mod jwt_secret_tests {
    use super::*;

    fn secret_file(dir: &Path, name: &str, content: &str) -> String {
        let p = dir.join(name);
        fs::write(&p, content).unwrap();
        p.to_string_lossy().to_string()
    }

    #[test]
    fn order_env_then_env_file_then_config_then_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = secret_file(dir.path(), "env.secret", "  from-env-file\n");
        let cfg_file = secret_file(dir.path(), "cfg.secret", "from-config-file\n");

        let mut s = ServerConfig { jwt_secret: "from-config".into(), jwt_secret_file: cfg_file.clone(), ..Default::default() };
        resolve_jwt_secret(&mut s, Some("from-env".into()), Some(env_file.clone()));
        assert_eq!(s.jwt_secret, "from-env");
        assert!(s.jwt_secret_source.contains("BRUM_JWT_SECRET environment"));

        let mut s = ServerConfig { jwt_secret: "from-config".into(), jwt_secret_file: cfg_file.clone(), ..Default::default() };
        resolve_jwt_secret(&mut s, Some("   ".into()), Some(env_file.clone()));
        assert_eq!(s.jwt_secret, "from-env-file", "surrounding whitespace is ignored");
        assert!(s.jwt_secret_source.starts_with("BRUM_JWT_SECRET_FILE"));
        assert!(s.jwt_secret_path.is_some());

        let mut s = ServerConfig { jwt_secret: "from-config".into(), jwt_secret_file: cfg_file.clone(), ..Default::default() };
        resolve_jwt_secret(&mut s, None, None);
        assert_eq!(s.jwt_secret, "from-config");

        let mut s = ServerConfig { jwt_secret_file: cfg_file, ..Default::default() };
        resolve_jwt_secret(&mut s, None, None);
        assert_eq!(s.jwt_secret, "from-config-file");
        assert!(s.jwt_secret_source.starts_with("jwt_secret_file"));
        assert!(s.jwt_secret_error.is_none());
    }

    #[test]
    fn nothing_configured_keeps_the_placeholder() {
        let mut s = ServerConfig::default();
        resolve_jwt_secret(&mut s, None, Some(String::new()));
        assert_eq!(s.jwt_secret, DEFAULT_JWT_SECRET);
        assert!(s.jwt_secret_source.is_empty());
        assert!(s.jwt_secret_error.is_none());
        let cfg = AppConfig { server: s, ..Default::default() };
        assert!(cfg.check_signing_secret().is_ok());
    }

    #[test]
    fn missing_or_empty_file_refuses_to_start() {
        let dir = tempfile::tempdir().unwrap();
        let empty = secret_file(dir.path(), "empty.secret", " \n\t\n");
        let missing = dir.path().join("nope.secret").to_string_lossy().to_string();

        for (env_file, cfg_file) in [(Some(missing.clone()), String::new()), (None, empty.clone()), (None, missing.clone())] {
            let mut s = ServerConfig { jwt_secret_file: cfg_file, ..Default::default() };
            resolve_jwt_secret(&mut s, None, env_file);
            assert!(s.jwt_secret_error.is_some());
            assert_eq!(s.jwt_secret, DEFAULT_JWT_SECRET, "never signs with a half-read value");
            let cfg = AppConfig { server: s, ..Default::default() };
            let err = cfg.check_signing_secret().unwrap_err();
            assert!(err.starts_with("Refusing to start"), "{err}");
        }
    }

    #[test]
    fn secret_file_path_is_shown_in_the_config_editor() {
        assert!(!crate::config::mask::is_sensitive_key("jwt_secret_file"));
        assert!(crate::config::mask::is_sensitive_key("jwt_secret"));
        let redacted = crate::config::mask::redact_sensitive_config(
            "[server]\njwt_secret = \"abc\"\njwt_secret_file = \"/run/secrets/brum_jwt\"\n",
        )
        .unwrap();
        assert!(redacted.contains("jwt_secret = \"********\""));
        assert!(redacted.contains("jwt_secret_file = \"/run/secrets/brum_jwt\""));
    }

    #[test]
    fn config_key_is_read_from_toml() {
        let cfg = ConfigManager::parse_config_str("[server]\njwt_secret_file = \"/run/secrets/brum_jwt\"\n").unwrap();
        assert_eq!(cfg.server.jwt_secret_file, "/run/secrets/brum_jwt");
    }
}
