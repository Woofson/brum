use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::auth::Claims;
use crate::config::TerminalConfig;
use crate::server::AppState;

#[derive(Deserialize, Debug, Clone)]
pub struct TerminalQuery {
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub token: Option<String>,
    pub cd_token: Option<String>,
}

#[cfg(unix)]
#[derive(Debug, Clone)]
pub struct PosixUser {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub dir: PathBuf,
    pub shell: String,
}

#[cfg(unix)]
pub fn lookup_posix_user(username: &str) -> Option<PosixUser> {
    use std::ffi::CString;
    let c_user = CString::new(username).ok()?;
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 4096];

    let res = unsafe {
        libc::getpwnam_r(
            c_user.as_ptr(),
            pwd.as_mut_ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };

    if res == 0 && !result.is_null() {
        let pw_ref = unsafe { &*result };
        let name = unsafe { std::ffi::CStr::from_ptr(pw_ref.pw_name) }
            .to_string_lossy()
            .into_owned();
        let uid = pw_ref.pw_uid as u32;
        let gid = pw_ref.pw_gid as u32;
        let dir = unsafe { std::ffi::CStr::from_ptr(pw_ref.pw_dir) }
            .to_string_lossy()
            .into_owned();
        let shell = unsafe { std::ffi::CStr::from_ptr(pw_ref.pw_shell) }
            .to_string_lossy()
            .into_owned();
        Some(PosixUser {
            name,
            uid,
            gid,
            dir: PathBuf::from(dir),
            shell,
        })
    } else {
        None
    }
}

pub fn is_role_permitted(user_role: &str, allowed_roles: &[String]) -> bool {
    if allowed_roles.is_empty() {
        return true;
    }
    let clean_user_role = user_role.trim().to_lowercase();
    allowed_roles.iter().any(|r| {
        let r_clean = r.trim().to_lowercase();
        r_clean == "*" || r_clean == clean_user_role
    })
}

pub fn extract_terminal_claims(
    state: &AppState,
    headers: &HeaderMap,
    query: &TerminalQuery,
) -> Result<Claims, (StatusCode, &'static str)> {
    if !state.config.server.enable_auth || state.config.server.standalone {
        let current_user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        return Ok(Claims {
            sub: current_user,
            role: "admin".to_string(),
            home_dir,
            is_pam: false,
            allowed_roots: Some("[\"*\"]".to_string()),
            token_id: None,
            exp: 9999999999,
        });
    }

    // 1. Check Authorization: Bearer <token>
    if let Some(auth_header) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(token_str) = auth_header.strip_prefix("Bearer ") {
            if let Ok(claims) = state.auth.verify_token(token_str.trim()) {
                return Ok(claims);
            }
        }
    }

    // 2. Check query parameter ?token=... or ?cd_token=...
    if let Some(tok) = query.token.as_deref().or(query.cd_token.as_deref()) {
        if let Ok(claims) = state.auth.verify_token(tok.trim()) {
            return Ok(claims);
        }
    }

    // 3. Check Cookie header for cd_token=... or token=...
    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie_hdr.split(';') {
            let pair = pair.trim();
            if let Some(tok) = pair.strip_prefix("cd_token=").or_else(|| pair.strip_prefix("token=")) {
                if let Ok(claims) = state.auth.verify_token(tok.trim()) {
                    return Ok(claims);
                }
            }
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        "Unauthorized: Valid session token required for terminal access",
    ))
}

pub async fn handle_terminal_ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // 1. Verify Authentication & Extract Claims
    let claims = match extract_terminal_claims(&state, &headers, &query) {
        Ok(c) => c,
        Err((code, msg)) => {
            warn!("Terminal WebSocket unauthorized connection attempt: {}", msg);
            return (code, msg).into_response();
        }
    };

    // 2. Verify Terminal Feature is Enabled
    if !state.config.terminal.enabled {
        warn!(
            "Terminal WebSocket rejected: terminal is disabled in server configuration (user: {})",
            claims.sub
        );
        return (
            StatusCode::FORBIDDEN,
            "Terminal feature is disabled by server configuration",
        )
            .into_response();
    }

    // 3. Verify RBAC Permissions
    if !is_role_permitted(&claims.role, &state.config.terminal.allow_roles) {
        warn!(
            "Terminal WebSocket rejected: user '{}' with role '{}' is not permitted by allow_roles ({:?})",
            claims.sub, claims.role, state.config.terminal.allow_roles
        );
        return (
            StatusCode::FORBIDDEN,
            "Terminal access is restricted for your user role",
        )
            .into_response();
    }

    // 4. POSIX User & Privilege Dropping Evaluation
    #[cfg(unix)]
    {
        let is_root = unsafe { libc::getuid() == 0 };
        let posix_user = lookup_posix_user(&claims.sub);

        if is_root {
            if posix_user.is_none() {
                // Virtual/DB account not present in /etc/passwd
                if !state.config.terminal.allow_virtual_users {
                    warn!(
                        "Terminal WebSocket rejected: user '{}' is a virtual account without a local POSIX user while daemon runs as root",
                        claims.sub
                    );
                    return (
                        StatusCode::FORBIDDEN,
                        "Terminal access is restricted to local system accounts when daemon runs as root",
                    )
                        .into_response();
                } else if claims.role.to_lowercase() != "admin" && claims.role.to_lowercase() != "root" {
                    warn!(
                        "Terminal WebSocket rejected: non-admin virtual user '{}' attempted to spawn shell while daemon runs as root",
                        claims.sub
                    );
                    return (
                        StatusCode::FORBIDDEN,
                        "Non-admin virtual accounts cannot spawn root terminal shells",
                    )
                        .into_response();
                } else {
                    info!(
                        "Terminal WebSocket: Spawning root shell for virtual admin '{}' (allow_virtual_users=true)",
                        claims.sub
                    );
                }
            }
        }
    }

    let terminal_config = state.config.terminal.clone();
    ws.on_upgrade(move |socket| handle_terminal_socket(socket, query, claims, terminal_config))
}

#[cfg(not(windows))]
fn resolve_unix_shell(config: &TerminalConfig, user_shell: Option<&str>) -> String {
    if let Some(ref s) = config.default_shell {
        if Path::new(s).exists() {
            return s.clone();
        }
    }
    if let Some(s) = user_shell {
        if !s.is_empty()
            && Path::new(s).exists()
            && s != "/bin/false"
            && s != "/usr/sbin/nologin"
            && s != "/sbin/nologin"
        {
            return s.to_string();
        }
    }
    if let Ok(shell_env) = std::env::var("SHELL") {
        if Path::new(&shell_env).exists() {
            return shell_env;
        }
    }
    if Path::new("/bin/bash").exists() {
        "/bin/bash".to_string()
    } else if Path::new("/usr/bin/bash").exists() {
        "/usr/bin/bash".to_string()
    } else if Path::new("/bin/sh").exists() {
        "/bin/sh".to_string()
    } else {
        "/bin/sh".to_string()
    }
}

async fn handle_terminal_socket(
    socket: WebSocket,
    query: TerminalQuery,
    claims: Claims,
    terminal_config: TerminalConfig,
) {
    let pty_system = native_pty_system();
    let cols = query.cols.unwrap_or(100).max(10);
    let rows = query.rows.unwrap_or(24).max(2);

    let pair = match pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to open PTY: {}", e);
            return;
        }
    };

    #[cfg(windows)]
    let mut cmd = {
        let shell = if let Some(ref s) = terminal_config.default_shell {
            s.clone()
        } else if let Ok(comspec) = std::env::var("COMSPEC") {
            comspec
        } else {
            "powershell.exe".to_string()
        };
        let mut c = CommandBuilder::new(&shell);
        c.env("TERM", "xterm-256color");
        c.env("COLORTERM", "truecolor");
        c
    };

    #[cfg(not(windows))]
    let (mut cmd, target_user_home) = {
        let is_root = unsafe { libc::getuid() == 0 };
        let posix_user = lookup_posix_user(&claims.sub);

        let user_home = posix_user
            .as_ref()
            .map(|u| u.dir.clone())
            .unwrap_or_else(|| {
                if !claims.home_dir.trim().is_empty() {
                    PathBuf::from(&claims.home_dir)
                } else {
                    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
                }
            });

        if is_root && terminal_config.drop_privileges {
            if let Some(ref pu) = posix_user {
                if pu.uid != 0 {
                    // Running as root daemon & authenticated user is a non-root POSIX user -> drop privileges via su -l
                    info!(
                        "Terminal PTY: Dropping root privileges to POSIX user '{}' (UID {}, GID {})",
                        pu.name, pu.uid, pu.gid
                    );
                    let su_bin = if Path::new("/usr/bin/su").exists() {
                        "/usr/bin/su"
                    } else if Path::new("/bin/su").exists() {
                        "/bin/su"
                    } else {
                        "su"
                    };
                    let mut c = CommandBuilder::new(su_bin);
                    c.arg("-l");
                    c.arg(&pu.name);
                    if let Some(ref custom_shell) = terminal_config.default_shell {
                        c.arg("-s");
                        c.arg(custom_shell);
                    }
                    c.env("TERM", "xterm-256color");
                    c.env("COLORTERM", "truecolor");
                    (c, user_home)
                } else {
                    // Authenticated user is root
                    let shell = resolve_unix_shell(&terminal_config, Some(&pu.shell));
                    let mut c = CommandBuilder::new(&shell);
                    c.env("TERM", "xterm-256color");
                    c.env("COLORTERM", "truecolor");
                    c.env("LANG", "C.UTF-8");
                    c.env("LC_ALL", "C.UTF-8");
                    c.env("SHELL", &shell);
                    (c, user_home)
                }
            } else {
                // Virtual user when allow_virtual_users is true
                let shell = resolve_unix_shell(&terminal_config, None);
                let mut c = CommandBuilder::new(&shell);
                c.env("TERM", "xterm-256color");
                c.env("COLORTERM", "truecolor");
                c.env("LANG", "C.UTF-8");
                c.env("LC_ALL", "C.UTF-8");
                c.env("SHELL", &shell);
                (c, user_home)
            }
        } else {
            // Running as regular unprivileged daemon or non-root user
            let user_shell = posix_user.as_ref().map(|u| u.shell.as_str());
            let shell = resolve_unix_shell(&terminal_config, user_shell);
            let mut c = CommandBuilder::new(&shell);
            c.env("TERM", "xterm-256color");
            c.env("COLORTERM", "truecolor");
            c.env("LANG", "C.UTF-8");
            c.env("LC_ALL", "C.UTF-8");
            c.env("SHELL", &shell);
            (c, user_home)
        }
    };

    // Working directory resolution
    #[cfg(not(windows))]
    {
        if let Some(ref cwd) = query.cwd {
            let clean = cwd.trim();
            let expanded = if clean == "~" || clean.starts_with("~/") {
                if clean == "~" {
                    target_user_home.clone()
                } else {
                    target_user_home.join(&clean[2..])
                }
            } else {
                PathBuf::from(clean)
            };
            if expanded.exists() && expanded.is_dir() {
                cmd.cwd(expanded);
            } else if target_user_home.exists() && target_user_home.is_dir() {
                cmd.cwd(target_user_home);
            }
        } else if target_user_home.exists() && target_user_home.is_dir() {
            cmd.cwd(target_user_home);
        }
    }

    #[cfg(windows)]
    {
        if let Some(ref cwd) = query.cwd {
            let clean = cwd.trim();
            let expanded = if clean == "~" || clean.starts_with("~/") {
                if let Some(home) = dirs::home_dir() {
                    if clean == "~" {
                        home
                    } else {
                        home.join(&clean[2..])
                    }
                } else {
                    PathBuf::from(clean)
                }
            } else {
                PathBuf::from(clean)
            };
            if expanded.exists() && expanded.is_dir() {
                cmd.cwd(expanded);
            } else if let Some(home) = dirs::home_dir() {
                if home.exists() && home.is_dir() {
                    cmd.cwd(home);
                }
            }
        } else if let Some(home) = dirs::home_dir() {
            if home.exists() && home.is_dir() {
                cmd.cwd(home);
            }
        }
    }

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to spawn shell process in PTY: {}", e);
            return;
        }
    };

    drop(pair.slave);

    let mut reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to clone PTY reader: {}", e);
            return;
        }
    };

    let mut writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to take PTY writer: {}", e);
            return;
        }
    };

    let (ws_sender, mut ws_receiver) = socket.split();
    let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));

    let (pty_out_tx, mut pty_out_rx) = mpsc::channel::<Vec<u8>>(100);

    // Blocking thread for reading from PTY master
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if pty_out_tx.blocking_send(buffer[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Task forwarding PTY output -> WebSocket
    let ws_sender_clone = ws_sender.clone();
    let pty_to_ws = tokio::spawn(async move {
        while let Some(data) = pty_out_rx.recv().await {
            let mut sender = ws_sender_clone.lock().await;
            if sender.send(Message::Binary(data)).await.is_err() {
                break;
            }
        }
    });

    #[derive(Deserialize, Debug)]
    struct TerminalResizePayload {
        cols: u16,
        rows: u16,
        #[serde(default)]
        #[allow(dead_code)]
        resize: Option<bool>,
    }

    let master_pty = Arc::new(parking_lot::Mutex::new(pair.master));
    let master_pty_clone = master_pty.clone();

    let mut cur_cols = cols;
    let mut cur_rows = rows;

    // Task forwarding WebSocket input -> PTY master writer
    let ws_to_pty = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            match msg {
                Message::Text(text) => {
                    // Check for JSON control message (e.g. resize)
                    if text.starts_with('{') {
                        if let Ok(resize_cmd) = serde_json::from_str::<TerminalResizePayload>(&text) {
                            let target_cols = resize_cmd.cols.max(10);
                            let target_rows = resize_cmd.rows.max(2);
                            if target_cols != cur_cols || target_rows != cur_rows {
                                cur_cols = target_cols;
                                cur_rows = target_rows;
                                let _ = master_pty_clone.lock().resize(PtySize {
                                    rows: target_rows,
                                    cols: target_cols,
                                    pixel_width: 0,
                                    pixel_height: 0,
                                });
                            }
                            continue;
                        }
                    }
                    if writer.write_all(text.as_bytes()).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Message::Binary(bytes) => {
                    if bytes.starts_with(b"{") {
                        if let Ok(text) = std::str::from_utf8(&bytes) {
                            if let Ok(resize_cmd) = serde_json::from_str::<TerminalResizePayload>(text) {
                                let target_cols = resize_cmd.cols.max(10);
                                let target_rows = resize_cmd.rows.max(2);
                                if target_cols != cur_cols || target_rows != cur_rows {
                                    cur_cols = target_cols;
                                    cur_rows = target_rows;
                                    let _ = master_pty_clone.lock().resize(PtySize {
                                        rows: target_rows,
                                        cols: target_cols,
                                        pixel_width: 0,
                                        pixel_height: 0,
                                    });
                                }
                                continue;
                            }
                        }
                    }
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = pty_to_ws => {},
        _ = ws_to_pty => {},
    }

    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_role_permitted() {
        let allowed = vec!["admin".to_string(), "root".to_string()];
        assert!(is_role_permitted("admin", &allowed));
        assert!(is_role_permitted("Admin", &allowed));
        assert!(is_role_permitted("ROOT", &allowed));
        assert!(!is_role_permitted("user", &allowed));
        assert!(!is_role_permitted("guest", &allowed));

        // Wildcard allow
        let wildcard = vec!["*".to_string()];
        assert!(is_role_permitted("user", &wildcard));
        assert!(is_role_permitted("guest", &wildcard));
        assert!(is_role_permitted("admin", &wildcard));

        // Empty list defaults to allowing all
        assert!(is_role_permitted("user", &[]));
    }

    #[test]
    #[cfg(unix)]
    fn test_lookup_posix_user_root_and_current() {
        let root = lookup_posix_user("root");
        assert!(root.is_some());
        let root = root.unwrap();
        assert_eq!(root.name, "root");
        assert_eq!(root.uid, 0);

        let non_existent = lookup_posix_user("this_user_definitely_does_not_exist_xyz_123");
        assert!(non_existent.is_none());
    }

    fn create_test_state(config: crate::config::AppConfig) -> AppState {
        let auth = crate::auth::AuthManager::new(
            &config.server.database_path,
            &config.server.jwt_secret,
            config.server.session_duration_hours,
            &config.auth.mode,
            &config.auth.pam_service,
            &config.auth.default_admin_user,
            &config.auth.default_admin_pass,
        ).unwrap();
        let db = auth.db();
        let auth_arc = Arc::new(auth);
        let task_mgr = Arc::new(crate::tools::tasks::TaskManager::new());
        let tag_mgr = Arc::new(crate::tools::tags::TagManager::new(db.clone()).unwrap());
        let vault_mgr = Arc::new(crate::vfs::vault::VaultManager::new());
        let backup_mgr = Arc::new(crate::tools::sync::BackupManager::new(db).unwrap());
        let plugin_mgr = Arc::new(crate::plugins::PluginManager::new(
            std::path::PathBuf::from("/tmp/system_plugins"),
            std::path::PathBuf::from("/tmp/user_plugins"),
            false,
            "allow_all".to_string(),
            vec!["*".to_string()],
            vec![],
        ));

        AppState {
            config: Arc::new(config),
            auth: auth_arc,
            tasks: task_mgr,
            tags: tag_mgr,
            vaults: vault_mgr,
            backup: backup_mgr,
            plugins: plugin_mgr,
        }
    }

    #[test]
    fn test_extract_terminal_claims_standalone() {
        let mut config = crate::config::AppConfig::default();
        config.server.enable_auth = false;
        let tmp = tempfile::tempdir().unwrap();
        config.server.database_path = tmp.path().join("term_test.db").to_string_lossy().to_string();

        let state = create_test_state(config);

        let headers = HeaderMap::new();
        let query = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: None,
            cd_token: None,
        };

        let claims = extract_terminal_claims(&state, &headers, &query).unwrap();
        assert_eq!(claims.role, "admin");
    }

    #[test]
    fn test_extract_terminal_claims_with_token_and_query() {
        let mut config = crate::config::AppConfig::default();
        let tmp = tempfile::tempdir().unwrap();
        config.server.database_path = tmp.path().join("term_test2.db").to_string_lossy().to_string();

        let state = create_test_state(config);
        let user = crate::auth::User {
            id: 1,
            username: "testuser".to_string(),
            nickname: None,
            email: None,
            avatar_url: None,
            role: "admin".to_string(),
            home_dir: "/home/testuser".to_string(),
            is_pam: false,
            is_disabled: false,
            allowed_services: "[\"*\"]".to_string(),
            allowed_roots: "[\"*\"]".to_string(),
            can_install_plugins: true,
            allowed_plugins: "[\"*\"]".to_string(),
            blocked_plugins: "[]".to_string(),
        };
        let token = state.auth.generate_token(&user).unwrap();

        // 1. Missing token -> Unauthorized
        let headers = HeaderMap::new();
        let query_empty = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: None,
            cd_token: None,
        };
        assert!(extract_terminal_claims(&state, &headers, &query_empty).is_err());

        // 2. Query param token -> Success
        let query_with_token = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: Some(token.clone()),
            cd_token: None,
        };
        let claims = extract_terminal_claims(&state, &headers, &query_with_token).unwrap();
        assert_eq!(claims.sub, "testuser");
        assert_eq!(claims.role, "admin");

        // 3. Authorization Bearer header -> Success
        let mut headers_bearer = HeaderMap::new();
        headers_bearer.insert(header::AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
        let claims = extract_terminal_claims(&state, &headers_bearer, &query_empty).unwrap();
        assert_eq!(claims.sub, "testuser");

        // 4. Cookie cd_token -> Success
        let mut headers_cookie = HeaderMap::new();
        headers_cookie.insert(header::COOKIE, format!("cd_token={}; other=123", token).parse().unwrap());
        let claims = extract_terminal_claims(&state, &headers_cookie, &query_empty).unwrap();
        assert_eq!(claims.sub, "testuser");
    }
}
