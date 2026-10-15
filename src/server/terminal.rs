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
use std::path::PathBuf;
#[cfg(not(windows))]
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, warn, debug, info};

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
    pub instance_token: Option<String>,
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
    if let Some(ref inst) = state.instance_info {
        let has_valid_token = query.instance_token.as_deref()
            .or_else(|| {
                headers.get("x-brum-instance-token")
                    .or_else(|| headers.get("x-instance-token"))
                    .and_then(|v| v.to_str().ok())
            })
            .or_else(|| {
                headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).and_then(|cookie_hdr| {
                    for pair in cookie_hdr.split(';') {
                        let pair = pair.trim();
                        if let Some(tok) = pair.strip_prefix("brum_instance_token=") {
                            return Some(tok.trim());
                        }
                    }
                    None
                })
            })
            .map(|tok| {
                use subtle::ConstantTimeEq;
                bool::from(tok.as_bytes().ct_eq(inst.instance_token.as_bytes()))
            })
            .unwrap_or(false);

        if has_valid_token {
            let home_dir = dirs::home_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".to_string());
            return Ok(Claims {
                sub: inst.owner.clone(),
                role: "admin".to_string(),
                home_dir,
                is_pam: false,
                allowed_roots: Some("[\"*\"]".to_string()),
                token_id: None,
                exp: 9999999999,
                shell: None,
            });
        }
    } else if !state.config.server.enable_auth || state.config.server.standalone {
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
            shell: None,
        });
    }

    // 1. Check Authorization: Bearer <token>
    if let Some(auth_header) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(token_str) = auth_header.strip_prefix("Bearer ").or_else(|| Some(auth_header)) {
            let trimmed = token_str.trim();
            if !trimmed.is_empty() && trimmed != "undefined" && trimmed != "null" {
                if let Ok(claims) = state.auth.verify_token(trimmed) {
                    return Ok(claims);
                }
                let decoded = crate::server::middleware::percent_decode(trimmed);
                if decoded != trimmed {
                    if let Ok(claims) = state.auth.verify_token(&decoded) {
                        return Ok(claims);
                    }
                }
            }
        }
    }

    // 2. Check query parameter ?token=... or ?cd_token=...
    if let Some(tok) = query.token.as_deref().or(query.cd_token.as_deref()) {
        let trimmed = tok.trim();
        if !trimmed.is_empty() && trimmed != "undefined" && trimmed != "null" {
            if let Ok(claims) = state.auth.verify_token(trimmed) {
                return Ok(claims);
            }
            let decoded = crate::server::middleware::percent_decode(trimmed);
            if decoded != trimmed {
                if let Ok(claims) = state.auth.verify_token(&decoded) {
                    return Ok(claims);
                }
            }
        }
    }

    // 3. Check Cookie header for cd_token=... or token=...
    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie_hdr.split(';') {
            let pair = pair.trim();
            if let Some(tok) = pair.strip_prefix("cd_token=").or_else(|| pair.strip_prefix("token=")) {
                let trimmed = tok.trim();
                if !trimmed.is_empty() && trimmed != "undefined" && trimmed != "null" {
                    if let Ok(claims) = state.auth.verify_token(trimmed) {
                        return Ok(claims);
                    }
                    let decoded = crate::server::middleware::percent_decode(trimmed);
                    if decoded != trimmed {
                        if let Ok(claims) = state.auth.verify_token(&decoded) {
                            return Ok(claims);
                        }
                    }
                }
            }
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        "Unauthorized: Valid session token required for terminal access",
    ))
}

pub fn is_ws_origin_allowed(origin: &str, headers: &HeaderMap, state: &AppState) -> bool {
    let origin = origin.trim();
    if origin.is_empty() || origin == "null" {
        return false;
    }

    let norm_origin = origin.trim_end_matches('/');

    let clean_origin = norm_origin
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("ws://")
        .trim_start_matches("wss://");

    // 1. Loopback origins (always permitted to ensure local client/admin access)
    let is_loopback = clean_origin == "localhost"
        || clean_origin.starts_with("localhost:")
        || clean_origin == "127.0.0.1"
        || clean_origin.starts_with("127.0.0.1:")
        || clean_origin == "[::1]"
        || clean_origin.starts_with("[::1]:");

    if is_loopback {
        debug!(origin = %norm_origin, "Origin trusted via loopback address");
        return true;
    }

    // 2. Local Tauri / Desktop webview origins (allowed in standalone/desktop mode)
    if (state.config.server.standalone || cfg!(windows) || cfg!(target_os = "macos"))
        && (origin.starts_with("tauri://")
            || origin.starts_with("http://tauri.localhost")
            || origin.starts_with("https://tauri.localhost"))
    {
        debug!(origin = %norm_origin, "Origin trusted via local desktop webview");
        return true;
    }

    // 3. Explicitly configured allowed_origins whitelist (supports exact, wildcard, and subdomain patterns)
    if !state.config.server.allowed_origins.is_empty() {
        for allowed in &state.config.server.allowed_origins {
            if is_origin_allowed_by_pattern(allowed, norm_origin) {
                debug!(origin = %norm_origin, pattern = %allowed, "Origin trusted via allowed_origins pattern");
                return true;
            }
        }
    }

    // 4. Server's own bound address if specific (not wildcard 0.0.0.0 / ::)
    let cfg_host = state.config.server.host.trim();
    let cfg_port = state.config.server.port;
    if !cfg_host.is_empty() && cfg_host != "0.0.0.0" && cfg_host != "::" && cfg_host != "[::]" {
        if clean_origin == format!("{}:{}", cfg_host, cfg_port) || clean_origin == cfg_host {
            debug!(origin = %norm_origin, "Origin trusted via server bound host/port match");
            return true;
        }
    }

    // 5. Configured server_name if non-empty (resilient to optional http:// or https:// prefix)
    let srv_name = state.config.server.server_name.trim();
    let clean_srv_host = srv_name
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("ws://")
        .trim_start_matches("wss://");
    let (srv_host, _) = clean_srv_host.split_once(':').unwrap_or((clean_srv_host, ""));

    if !srv_host.is_empty() {
        let (origin_host, _) = clean_origin.split_once(':').unwrap_or((clean_origin, ""));
        if origin_host.eq_ignore_ascii_case(srv_host) {
            debug!(origin = %norm_origin, server_name = %srv_name, "Origin trusted via server_name match");
            return true;
        }
    }

    // 5b. System hostname (allow bare system hostname or with port)
    let sys_hostname = crate::server::system_handlers::get_system_hostname();
    if !sys_hostname.is_empty() && sys_hostname != "localhost" {
        let (origin_host, _) = clean_origin.split_once(':').unwrap_or((clean_origin, ""));
        if origin_host.eq_ignore_ascii_case(&sys_hostname) {
            debug!(origin = %norm_origin, hostname = %sys_hostname, "Origin trusted via system hostname match");
            return true;
        }
    }

    // 6. Trusted private/internal LAN & overlay hosts (e.g. *.local, *.lan, *.home.arpa, RFC1918 IPs, or RFC6598 CGNAT/Tailscale/NetBird IPs)
    // Only permit matching Host/X-Forwarded-Host if the host is a local/private network address or trusted proxy
    let host_header = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();

    if !host_header.is_empty() {
        let (origin_host, _) = clean_origin.split_once(':').unwrap_or((clean_origin, ""));
        let (req_host, _) = host_header.split_once(':').unwrap_or((host_header, ""));

        if origin_host.eq_ignore_ascii_case(req_host) {
            // Guard against DNS rebinding (e.g. evil.com -> local IP):
            // Only allow Host matching if host is a local IP, .local, .lan, .home.arpa, localhost, or mesh overlay IP
            let is_safe_local_host = req_host.eq_ignore_ascii_case("localhost")
                || req_host == "127.0.0.1"
                || req_host == "[::1]"
                || req_host.ends_with(".local")
                || req_host.ends_with(".lan")
                || req_host.ends_with(".home.arpa")
                || (!srv_host.is_empty() && req_host.eq_ignore_ascii_case(srv_host))
                || (!sys_hostname.is_empty() && req_host.eq_ignore_ascii_case(&sys_hostname))
                || if let Ok(ip) = req_host.parse::<std::net::IpAddr>() {
                    is_safe_local_or_overlay_ip(ip)
                } else {
                    false
                };

            if is_safe_local_host {
                debug!(origin = %norm_origin, host = %req_host, "Origin trusted via safe local/private network host match");
                return true;
            }
        }
    }

    debug!(origin = %norm_origin, host = %host_header, "Origin rejected: does not match server_name, allowed_origins, or safe local network hosts");
    false
}

/// Helper to check if an IP address belongs to local loopback, RFC 1918 private IPv4,
/// RFC 6598 Shared Address Space (100.64.0.0/10 used by Tailscale / NetBird / CGNAT),
/// or RFC 4193 Unique Local Address (fc00::/7).
pub fn is_safe_local_or_overlay_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                // RFC 6598 Shared Address Space (100.64.0.0/10) used by Tailscale, NetBird, CGNAT
                || (octets[0] == 100 && (octets[1] >= 64 && octets[1] <= 127))
        }
        std::net::IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                // RFC 4193 Unique Local Address (fc00::/7)
                || (ipv6.octets()[0] & 0xfe == 0xfc)
        }
    }
}

/// Matches an origin pattern against a candidate origin string.
/// Supports exact origins, global wildcard "*", wildcard ports (e.g. "https://*.arf.ac:*"),
/// and subdomain wildcard patterns (e.g. "https://*.arf.ac") with strict label boundary checking.
pub fn is_origin_allowed_by_pattern(pattern: &str, candidate: &str) -> bool {
    let pat = pattern.trim().trim_end_matches('/');
    let cand = candidate.trim().trim_end_matches('/');

    if pat == "*" {
        return true;
    }
    if pat.eq_ignore_ascii_case(cand) {
        return true;
    }

    let (pat_scheme, pat_host, pat_port) = split_origin(pat);
    let (cand_scheme, cand_host, cand_port) = split_origin(cand);

    // 1. Verify scheme if specified in pattern
    if let Some(ps) = pat_scheme {
        match cand_scheme {
            Some(cs) => {
                if !ps.eq_ignore_ascii_case(cs) {
                    return false;
                }
            }
            None => return false,
        }
    }

    // 2. Verify port
    if let Some(pp) = pat_port {
        if pp != "*" {
            match cand_port {
                Some(cp) => {
                    if pp != cp {
                        return false;
                    }
                }
                None => {
                    let default_port = match cand_scheme.unwrap_or("") {
                        "http" | "ws" => "80",
                        "https" | "wss" => "443",
                        _ => "",
                    };
                    if pp != default_port {
                        return false;
                    }
                }
            }
        }
    } else if let Some(cp) = cand_port {
        let default_port = match cand_scheme.unwrap_or("") {
            "http" | "ws" => "80",
            "https" | "wss" => "443",
            _ => "",
        };
        if cp != default_port {
            return false;
        }
    }

    // 3. Verify host
    if pat_host == "*" {
        return true;
    }

    if let Some(prefix) = pat_host.strip_suffix('*') {
        if cand_host.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) {
            return true;
        }
    }

    if let Some(base_domain) = pat_host.strip_prefix("*.") {
        let base_clean = base_domain.trim_start_matches('.');
        if cand_host.eq_ignore_ascii_case(base_clean) {
            return true;
        }
        if cand_host.len() > base_clean.len() + 1
            && cand_host.to_ascii_lowercase().ends_with(&base_clean.to_ascii_lowercase())
            && cand_host.as_bytes()[cand_host.len() - base_clean.len() - 1] == b'.'
        {
            return true;
        }
        return false;
    }

    pat_host.eq_ignore_ascii_case(cand_host)
}

fn split_origin(origin: &str) -> (Option<&str>, &str, Option<&str>) {
    let (scheme, rest) = if let Some(idx) = origin.find("://") {
        (Some(&origin[..idx]), &origin[idx + 3..])
    } else {
        (None, origin)
    };

    if rest.starts_with('[') {
        if let Some(end_bracket) = rest.find(']') {
            let host = &rest[1..end_bracket];
            let after = &rest[end_bracket + 1..];
            let port = if let Some(p) = after.strip_prefix(':') {
                Some(p)
            } else {
                None
            };
            (scheme, host, port)
        } else {
            (scheme, rest, None)
        }
    } else if let Some((h, p)) = rest.split_once(':') {
        (scheme, h, Some(p))
    } else {
        (scheme, rest, None)
    }
}

pub async fn handle_terminal_ws(
    State(state): State<AppState>,
    claims_ext: Option<axum::extract::Extension<Claims>>,
    headers: HeaderMap,
    Query(query): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // 0. Verify Origin (Anti-CSWSH Protection)
    if let Some(origin_val) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let origin_str = origin_val.trim();
        if !origin_str.is_empty() && !is_ws_origin_allowed(origin_str, &headers, &state) {
            warn!(
                origin = %origin_str,
                "Terminal WebSocket connection rejected: Origin header does not match Host or allowed origins (Anti-CSWSH)"
            );
            return (
                StatusCode::FORBIDDEN,
                "Cross-origin WebSocket connection forbidden",
            ).into_response();
        }
    }

    // 1. Verify Authentication & Extract Claims (prefer claims already verified by auth_middleware)
    let claims = if let Some(axum::extract::Extension(c)) = claims_ext {
        c
    } else {
        match extract_terminal_claims(&state, &headers, &query) {
            Ok(c) => c,
            Err((code, msg)) => {
                warn!("Terminal WebSocket unauthorized connection attempt: {}", msg);
                return (code, msg).into_response();
            }
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
    let auth = state.auth.clone();
    ws.on_upgrade(move |socket| handle_terminal_socket(socket, query, claims, terminal_config, auth))
}

#[cfg(not(windows))]
fn resolve_unix_shell(
    config: &TerminalConfig,
    username: Option<&str>,
    profile_shell: Option<&str>,
    posix_shell: Option<&str>,
) -> String {
    // 1. Check per-user shell override in config ([terminal.user_shells])
    if let Some(user) = username {
        if let Some(s) = config.user_shells.get(user) {
            if Path::new(s).exists() {
                return s.clone();
            }
        }
    }

    // 2. Check user's DB profile shell (configured via Users & RBAC)
    if let Some(s) = profile_shell {
        if !s.is_empty() && Path::new(s).exists() {
            return s.to_string();
        }
    }

    // 3. Check global default_shell in config ([terminal])
    if let Some(ref s) = config.default_shell {
        if Path::new(s).exists() {
            return s.clone();
        }
    }

    // 4. Check user's POSIX shell from /etc/passwd
    if let Some(s) = posix_shell {
        if !s.is_empty()
            && Path::new(s).exists()
            && s != "/bin/false"
            && s != "/usr/sbin/nologin"
            && s != "/sbin/nologin"
        {
            return s.to_string();
        }
    }

    // 5. Check SHELL environment variable
    if let Ok(shell_env) = std::env::var("SHELL") {
        if Path::new(&shell_env).exists() {
            return shell_env;
        }
    }

    // 6. Fallback to standard system shells
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
    auth: Arc<crate::auth::AuthManager>,
) {
    let _ = &claims;
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

    let db_shell = auth
        .get_user_by_username(&claims.sub)
        .ok()
        .flatten()
        .and_then(|u| u.shell)
        .filter(|s| !s.trim().is_empty());
    let profile_shell = db_shell.as_deref().or(claims.shell.as_deref());

    #[cfg(windows)]
    let mut cmd = {
        let shell = if let Some(s) = terminal_config.user_shells.get(&claims.sub) {
            s.clone()
        } else if let Some(s) = profile_shell {
            s.to_string()
        } else if let Some(ref s) = terminal_config.default_shell {
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
                    let effective_shell = terminal_config
                        .user_shells
                        .get(&pu.name)
                        .or_else(|| terminal_config.user_shells.get(&claims.sub))
                        .map(|s| s.as_str())
                        .or(profile_shell)
                        .or(terminal_config.default_shell.as_deref());
                    if let Some(custom_shell) = effective_shell {
                        if Path::new(custom_shell).exists() {
                            c.arg("-s");
                            c.arg(custom_shell);
                        }
                    }
                    c.env("TERM", "xterm-256color");
                    c.env("COLORTERM", "truecolor");
                    (c, user_home)
                } else {
                    // Authenticated user is root
                    let shell = resolve_unix_shell(&terminal_config, Some(&claims.sub), profile_shell, Some(&pu.shell));
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
                let shell = resolve_unix_shell(&terminal_config, Some(&claims.sub), profile_shell, None);
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
            let shell = resolve_unix_shell(&terminal_config, Some(&claims.sub), profile_shell, user_shell);
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

        let oidc_mgr = Arc::new(crate::auth::oidc::OidcManager::new(config.auth.oidc.clone(), auth_arc.clone()));

        AppState {
            config: Arc::new(config),
            auth: auth_arc,
            oidc: oidc_mgr,
            tasks: task_mgr,
            tags: tag_mgr,
            vaults: vault_mgr,
            backup: backup_mgr,
            plugins: plugin_mgr,
            rate_limiter: Arc::new(crate::server::RateLimiter::new()),
            instance_info: None,
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
            instance_token: None,
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
        let user = state.auth.create_user(
            "testuser",
            "pass123",
            "admin",
            "/home/testuser",
            Some("[\"*\"]"),
        ).unwrap();
        let token = state.auth.generate_token(&user).unwrap();

        // 1. Missing token -> Unauthorized
        let headers = HeaderMap::new();
        let query_empty = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: None,
            cd_token: None,
            instance_token: None,
        };
        assert!(extract_terminal_claims(&state, &headers, &query_empty).is_err());

        // 2. Query param token -> Success
        let query_with_token = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: Some(token.clone()),
            cd_token: None,
            instance_token: None,
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

        // 5. Percent-encoded query token -> Success
        let encoded_token = token.replace('.', "%2E");
        let query_encoded = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: Some(encoded_token),
            cd_token: None,
            instance_token: None,
        };
        let claims = extract_terminal_claims(&state, &headers, &query_encoded).unwrap();
        assert_eq!(claims.sub, "testuser");

        // 6. Stale/invalid cookie + valid query token -> Success
        let mut headers_stale_cookie = HeaderMap::new();
        headers_stale_cookie.insert(header::COOKIE, "cd_token=invalid_stale_token_signature; other=123".parse().unwrap());
        let claims = extract_terminal_claims(&state, &headers_stale_cookie, &query_with_token).unwrap();
        assert_eq!(claims.sub, "testuser");
    }

    #[test]
    fn test_extract_terminal_claims_with_instance_token() {
        let mut config = crate::config::AppConfig::default();
        config.server.standalone = true;
        let tmp = tempfile::tempdir().unwrap();
        config.server.database_path = tmp.path().join("term_inst.db").to_string_lossy().to_string();

        let mut state = create_test_state(config);
        let secret = "secret_instance_handshake_12345".to_string();
        state.instance_info = Some(std::sync::Arc::new(crate::instance::UserInstanceInfo {
            pid: std::process::id(),
            port: 3140,
            owner: "alice".to_string(),
            instance_token: secret.clone(),
            started_at: "2026-10-10T00:00:00Z".to_string(),
        }));

        let headers = HeaderMap::new();
        let query_no_token = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: None,
            cd_token: None,
            instance_token: None,
        };
        // Without instance token -> Unauthorized
        assert!(extract_terminal_claims(&state, &headers, &query_no_token).is_err());

        // With invalid instance token -> Unauthorized
        let query_bad_token = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: None,
            cd_token: None,
            instance_token: Some("wrong_token".to_string()),
        };
        assert!(extract_terminal_claims(&state, &headers, &query_bad_token).is_err());

        // With valid instance token in query -> Success
        let query_good_token = TerminalQuery {
            cwd: None,
            cols: None,
            rows: None,
            token: None,
            cd_token: None,
            instance_token: Some(secret.clone()),
        };
        let claims = extract_terminal_claims(&state, &headers, &query_good_token).unwrap();
        assert_eq!(claims.sub, "alice");
        assert_eq!(claims.role, "admin");

        // With valid instance token in Cookie -> Success
        let mut headers_cookie = HeaderMap::new();
        headers_cookie.insert(header::COOKIE, format!("brum_instance_token={}", secret).parse().unwrap());
        let claims = extract_terminal_claims(&state, &headers_cookie, &query_no_token).unwrap();
        assert_eq!(claims.sub, "alice");
        assert_eq!(claims.role, "admin");
    }

    #[test]
    fn test_is_ws_origin_allowed_with_allowed_origins_and_server_name() {
        let mut config = crate::config::AppConfig::default();
        let tmp = tempfile::tempdir().unwrap();
        config.server.database_path = tmp.path().join("term_origin.db").to_string_lossy().to_string();
        config.server.server_name = "griffon".to_string();
        config.server.port = 3140;
        config.server.allowed_origins = vec![
            "http://10.37.10.*:*".to_string(),
            "http://100.117.*:*".to_string(),
        ];

        let state = create_test_state(config);
        let mut headers = HeaderMap::new();

        // 1. Loopback origin: always allowed
        assert!(is_ws_origin_allowed("http://localhost:3140", &headers, &state));
        assert!(is_ws_origin_allowed("http://127.0.0.1:3140", &headers, &state));

        // 2. Overlay origin matching allowed_origins pattern: allowed
        assert!(is_ws_origin_allowed("http://10.37.10.170:3140", &headers, &state));
        assert!(is_ws_origin_allowed("http://10.37.10.217:3141", &headers, &state));

        // 3. Server name: allowed even though not explicitly in allowed_origins list
        assert!(is_ws_origin_allowed("http://griffon:3140", &headers, &state));
        assert!(is_ws_origin_allowed("http://griffon", &headers, &state));

        // 4. Same-origin Host matching for safe local/server host
        headers.insert(header::HOST, "griffon:3140".parse().unwrap());
        assert!(is_ws_origin_allowed("http://griffon:3140", &headers, &state));

        // 5. Malicious external origin: rejected
        assert!(!is_ws_origin_allowed("http://evil.com", &headers, &state));
        assert!(!is_ws_origin_allowed("http://attacker.local.evil.com", &headers, &state));

        // 6. Custom external FQDN without allowed_origins or server_name: rejected
        let mut ext_headers = HeaderMap::new();
        ext_headers.insert(header::HOST, "fe.nb.arf.ac".parse().unwrap());
        assert!(!is_ws_origin_allowed("https://fe.nb.arf.ac", &ext_headers, &state));

        // 7. Custom external FQDN with allowed_origins (bare domain or full URL or wildcard)
        let mut cfg2 = crate::config::AppConfig::default();
        cfg2.server.database_path = tmp.path().join("term_origin2.db").to_string_lossy().to_string();
        cfg2.server.allowed_origins = vec![
            "fe.nb.arf.ac".to_string(), // bare domain without scheme
            "https://*.nb.arf.ac".to_string(), // with scheme and wildcard
        ];
        let state2 = create_test_state(cfg2);
        assert!(is_ws_origin_allowed("https://fe.nb.arf.ac", &ext_headers, &state2));
        assert!(is_ws_origin_allowed("http://fe.nb.arf.ac", &ext_headers, &state2));
        assert!(is_ws_origin_allowed("https://node1.nb.arf.ac", &ext_headers, &state2));

        // 8. Server name configured as full URL: scheme is gracefully stripped and matches
        let mut cfg3 = crate::config::AppConfig::default();
        cfg3.server.database_path = tmp.path().join("term_origin3.db").to_string_lossy().to_string();
        cfg3.server.server_name = "https://fe.nb.arf.ac".to_string();
        let state3 = create_test_state(cfg3);
        assert!(is_ws_origin_allowed("https://fe.nb.arf.ac", &ext_headers, &state3));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_unix_shell_user_override() {
        let mut config = TerminalConfig::default();
        config.default_shell = Some("/bin/bash".to_string());
        config.user_shells.insert("bolt".to_string(), "/bin/sh".to_string());

        // 1. User with explicit override -> returns user override
        let shell_bolt = resolve_unix_shell(&config, Some("bolt"), None, Some("/bin/bash"));
        assert_eq!(shell_bolt, "/bin/sh");

        // 2. User with DB profile shell -> returns profile shell over default_shell
        let shell_profile = resolve_unix_shell(&config, Some("alice"), Some("/bin/sh"), Some("/bin/dash"));
        assert_eq!(shell_profile, "/bin/sh");

        // 3. User without override or profile -> falls back to default_shell
        let shell_alice = resolve_unix_shell(&config, Some("alice"), None, Some("/bin/dash"));
        assert_eq!(shell_alice, "/bin/bash");

        // 4. User with non-existent shell override -> falls back to default_shell
        config.user_shells.insert("charlie".to_string(), "/nonexistent/path/xyz".to_string());
        let shell_charlie = resolve_unix_shell(&config, Some("charlie"), None, Some("/bin/sh"));
        assert_eq!(shell_charlie, "/bin/bash");
    }
}
