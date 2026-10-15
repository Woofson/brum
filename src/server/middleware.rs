use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use parking_lot::Mutex;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Json, Response},
};
use crate::server::AppState;

#[derive(Debug, Clone)]
pub struct RateLimiter {
    attempts: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if key is currently rate limited without recording an attempt
    pub fn check(&self, key: &str, max_attempts: usize, window_secs: u64) -> Result<(), u64> {
        let mut map = self.attempts.lock();
        let now = Instant::now();
        let window = std::time::Duration::from_secs(window_secs);

        if let Some(entry) = map.get_mut(key) {
            entry.retain(|&t| now.duration_since(t) < window);
            if entry.len() >= max_attempts {
                let oldest = entry[0];
                let elapsed = now.duration_since(oldest);
                let retry_after = if elapsed < window {
                    (window - elapsed).as_secs().max(1)
                } else {
                    1
                };
                return Err(retry_after);
            }
        }
        Ok(())
    }

    /// Record an attempt under key with bounded map capacity and automatic pruning
    pub fn record(&self, key: &str, window_secs: u64) {
        let mut map = self.attempts.lock();
        let now = Instant::now();
        let window = std::time::Duration::from_secs(window_secs);

        // Opportunistic pruning to prevent memory exhaustion under rotating keys
        if map.len() > 1024 {
            map.retain(|_, timestamps| {
                timestamps.retain(|&t| now.duration_since(t) < window);
                !timestamps.is_empty()
            });
            if map.len() >= 5000 {
                let excess = map.len() - 4000;
                let keys_to_remove: Vec<String> = map.keys().take(excess).cloned().collect();
                for k in keys_to_remove {
                    map.remove(&k);
                }
            }
        }

        let entry = map.entry(key.to_string()).or_insert_with(Vec::new);
        entry.retain(|&t| now.duration_since(t) < window);
        entry.push(now);
    }

    pub fn check_and_record(&self, key: &str, max_attempts: usize, window_secs: u64) -> Result<(), u64> {
        let mut map = self.attempts.lock();
        let now = Instant::now();
        let window = std::time::Duration::from_secs(window_secs);

        // Opportunistic pruning to prevent memory exhaustion
        if map.len() > 1024 {
            map.retain(|_, timestamps| {
                timestamps.retain(|&t| now.duration_since(t) < window);
                !timestamps.is_empty()
            });
            if map.len() >= 5000 {
                let excess = map.len() - 4000;
                let keys_to_remove: Vec<String> = map.keys().take(excess).cloned().collect();
                for k in keys_to_remove {
                    map.remove(&k);
                }
            }
        }

        let entry = map.entry(key.to_string()).or_insert_with(Vec::new);
        entry.retain(|&t| now.duration_since(t) < window);

        if entry.len() >= max_attempts {
            let oldest = entry[0];
            let elapsed = now.duration_since(oldest);
            let retry_after = if elapsed < window {
                (window - elapsed).as_secs().max(1)
            } else {
                1
            };
            return Err(retry_after);
        }

        entry.push(now);
        Ok(())
    }

    pub fn reset(&self, key: &str) {
        let mut map = self.attempts.lock();
        map.remove(key);
    }

    pub fn len(&self) -> usize {
        self.attempts.lock().len()
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn security_headers_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();

    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        axum::http::HeaderValue::from_static("SAMEORIGIN"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        header::HeaderName::from_static("x-xss-protection"),
        axum::http::HeaderValue::from_static("1; mode=block"),
    );
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        axum::http::HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        axum::http::HeaderValue::from_static(
            "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob: ws: wss:; frame-ancestors 'self';"
        ),
    );

    resp
}

#[allow(dead_code)]
pub fn require_admin(claims: &crate::auth::Claims) -> Result<(), (StatusCode, String)> {
    if !claims.role.eq_ignore_ascii_case("admin") && !claims.role.eq_ignore_ascii_case("root") {
        return Err((
            StatusCode::FORBIDDEN,
            "Forbidden: Administrative privileges required".to_string(),
        ));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn require_write(claims: &crate::auth::Claims) -> Result<(), (StatusCode, String)> {
    if claims.role.eq_ignore_ascii_case("readonly") {
        return Err((
            StatusCode::FORBIDDEN,
            "Forbidden: Write operations not permitted for read-only user".to_string(),
        ));
    }
    Ok(())
}

pub fn is_public_route(path: &str) -> bool {
    // Non-API routes are frontend SPA routes and embedded static assets
    if !path.starts_with("/api/") {
        return true;
    }

    if path == "/api/health"
        || path == "/api/system/status"
        || path == "/api/config"
        || path == "/api/auth/login"
        || path == "/api/auth/unlock"
        || path == "/api/auth/oidc/config"
        || path == "/api/auth/oidc/login"
        || path == "/api/auth/oidc/callback"
        || path == "/api/auth/logout"
    {
        return true;
    }

    if path.starts_with("/api/public/shares/") || path.starts_with("/share/") {
        return true;
    }

    if path.starts_with("/api/auth/avatar/") {
        return true;
    }

    false
}

/// Safely percent-decodes a URL-encoded string without panicking on malformed sequences.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes_in = s.as_bytes();
    let mut bytes_out = Vec::with_capacity(bytes_in.len());
    let mut i = 0;
    while i < bytes_in.len() {
        if bytes_in[i] == b'%' && i + 2 < bytes_in.len() {
            let h1 = match bytes_in[i + 1] {
                b'0'..=b'9' => Some(bytes_in[i + 1] - b'0'),
                b'a'..=b'f' => Some(bytes_in[i + 1] - b'a' + 10),
                b'A'..=b'F' => Some(bytes_in[i + 1] - b'A' + 10),
                _ => None,
            };
            let h2 = match bytes_in[i + 2] {
                b'0'..=b'9' => Some(bytes_in[i + 2] - b'0'),
                b'a'..=b'f' => Some(bytes_in[i + 2] - b'a' + 10),
                b'A'..=b'F' => Some(bytes_in[i + 2] - b'A' + 10),
                _ => None,
            };
            if let (Some(d1), Some(d2)) = (h1, h2) {
                bytes_out.push((d1 << 4) | d2);
                i += 3;
                continue;
            }
        }
        bytes_out.push(bytes_in[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes_out).to_string()
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let path = req.uri().path().to_string();

    // 1. Loopback Token Handshake for Private Per-User Desktop Instances
    let mut query_has_instance_token = false;
    let provided_instance_token = {
        let headers = req.headers();
        let from_header = headers
            .get("x-brum-instance-token")
            .or_else(|| headers.get("x-instance-token"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string());

        let from_cookie = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|cookie_str| {
                for pair in cookie_str.split(';') {
                    let pair = pair.trim();
                    if let Some(tok) = pair.strip_prefix("brum_instance_token=") {
                        return Some(tok.trim().to_string());
                    }
                }
                None
            });

        let from_query = req.uri().query().and_then(|q| {
            for pair in q.split('&') {
                if let Some(tok) = pair.strip_prefix("instance_token=") {
                    query_has_instance_token = true;
                    return Some(tok.trim().to_string());
                }
            }
            None
        });

        from_header.or(from_cookie).or(from_query)
    };

    if let Some(ref inst) = state.instance_info {
        if !crate::instance::is_loopback_host(&state.config.server.host) {
            tracing::warn!("Instance token rejected: server is not bound to a loopback address");
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "Access denied: Instance tokens are only valid on loopback bindings"
                })),
            ));
        }

        let is_valid_token = provided_instance_token.as_deref().map_or(false, |tok| {
            use subtle::ConstantTimeEq;
            bool::from(tok.as_bytes().ct_eq(inst.instance_token.as_bytes()))
        });

        if is_valid_token {
            let home_dir = dirs::home_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".to_string());
            let claims = crate::auth::Claims {
                sub: inst.owner.clone(),
                role: "admin".to_string(),
                home_dir,
                is_pam: false,
                allowed_roots: Some("[\"*\"]".to_string()),
                token_id: None,
                exp: 9999999999,
                shell: None,
            };
            req.extensions_mut().insert(claims);

            let mut resp = next.run(req).await;
            if query_has_instance_token {
                let cookie_val = format!(
                    "brum_instance_token={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=31536000",
                    inst.instance_token
                );
                if let Ok(hv) = axum::http::HeaderValue::from_str(&cookie_val) {
                    resp.headers_mut().insert(axum::http::header::SET_COOKIE, hv);
                }
            }
            return Ok(resp);
        }

        // Token invalid or missing on private per-user instance:
        // Allow public discovery and login endpoints, but block all protected data
        if is_public_route(&path) {
            return Ok(next.run(req).await);
        }

        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Unauthorized loopback access: instance token missing or invalid",
                "owner": inst.owner
            })),
        ));
    }

    // 2. Standalone desktop or auth-disabled instances bypass auth checks with local admin claims
    if !state.config.server.enable_auth || state.config.server.standalone {
        let current_user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        let claims = crate::auth::Claims {
            sub: current_user,
            role: "admin".to_string(),
            home_dir,
            is_pam: false,
            allowed_roots: Some("[\"*\"]".to_string()),
            token_id: None,
            exp: 9999999999,
            shell: None,
        };
        req.extensions_mut().insert(claims);
        return Ok(next.run(req).await);
    }

    let is_pub = is_public_route(&path);

    // 2. Collect candidate tokens in prioritized order.
    // On WebSocket upgrade requests, the browser cannot attach custom Authorization headers
    // and sends ambient cookies automatically. If ambient cookies are stale, an explicit
    // client-provided query token (?token=... or ?cd_token=...) MUST take precedence over cookies.
    // In all cases, multiple candidate tokens are tried so a stale cookie does not block a valid token.
    let is_ws_upgrade = path.starts_with("/api/ws/")
        || path.contains("/api/ws/")
        || path == "/api/terminal/ws"
        || path.ends_with("/api/terminal/ws")
        || path.ends_with("/ws")
        || req.headers().get(header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false);

    let mut query_token = None;
    if let Some(q) = req.uri().query() {
        for pair in q.split('&') {
            if let Some(tok) = pair.strip_prefix("token=").or_else(|| pair.strip_prefix("cd_token=")) {
                let trimmed = tok.trim();
                if !trimmed.is_empty() && trimmed != "undefined" && trimmed != "null" {
                    query_token = Some(trimmed.to_string());
                    break;
                }
            }
        }
    }

    let mut candidates: Vec<String> = Vec::new();

    // A. Explicit Authorization header (Bearer ...)
    if let Some(h) = req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").or_else(|| Some(h)))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "undefined" && *s != "null")
    {
        let decoded = percent_decode(h);
        if decoded != h {
            candidates.push(decoded);
        }
        candidates.push(h.to_string());
    }

    // B. Query parameter (prioritized before ambient cookies on WebSocket upgrade requests)
    if is_ws_upgrade {
        if let Some(ref tok) = query_token {
            let decoded = percent_decode(tok);
            if decoded != *tok {
                candidates.push(decoded);
            }
            candidates.push(tok.clone());
        }
    }

    // C. Cookie header (cd_token=... or token=...)
    if let Some(cookie_str) = req.headers().get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie_str.split(';') {
            let pair = pair.trim();
            if let Some(tok) = pair.strip_prefix("cd_token=").or_else(|| pair.strip_prefix("token=")) {
                let trimmed = tok.trim();
                if !trimmed.is_empty() && trimmed != "undefined" && trimmed != "null" {
                    let decoded = percent_decode(trimmed);
                    if decoded != trimmed {
                        candidates.push(decoded);
                    }
                    candidates.push(trimmed.to_string());
                }
            }
        }
    }

    // D. Query parameter fallback for non-WS requests (if applicable)
    if !is_ws_upgrade {
        if let Some(ref tok) = query_token {
            let decoded = percent_decode(tok);
            if decoded != *tok {
                candidates.push(decoded);
            }
            candidates.push(tok.clone());
        }
    }

    let client_ip = resolve_client_ip(req.headers(), None, &state.config.server.trusted_proxies);
    let origin_str = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()).unwrap_or("-");

    let mut last_err = None;
    let mut verified_claims = None;

    for candidate in &candidates {
        match state.auth.verify_token(candidate) {
            Ok(claims) => {
                verified_claims = Some(claims);
                break;
            }
            Err(err) => {
                last_err = Some(err);
            }
        }
    }

    if let Some(claims) = verified_claims {
        tracing::debug!(
            client_ip = %client_ip,
            user = %claims.sub,
            role = %claims.role,
            origin = %origin_str,
            path = %path,
            "Request authenticated"
        );
        req.extensions_mut().insert(claims);
        return Ok(next.run(req).await);
    }

    if let Some(err) = last_err {
        if is_pub {
            tracing::trace!(
                client_ip = %client_ip,
                path = %path,
                origin = %origin_str,
                "Public route accessed with unverified token"
            );
            Ok(next.run(req).await)
        } else {
            tracing::warn!(
                client_ip = %client_ip,
                path = %path,
                origin = %origin_str,
                error = %err,
                "Unauthorized request: invalid token"
            );
            Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": format!("Unauthorized: {}", err),
                    "code": "UNAUTHORIZED"
                })),
            ))
        }
    } else {
        if is_pub {
            tracing::trace!(
                client_ip = %client_ip,
                path = %path,
                origin = %origin_str,
                "Public route accessed"
            );
            Ok(next.run(req).await)
        } else {
            tracing::warn!(
                client_ip = %client_ip,
                path = %path,
                origin = %origin_str,
                "Unauthorized request: authentication credentials missing"
            );
            Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Unauthorized: Authentication required",
                    "code": "UNAUTHORIZED"
                })),
            ))
        }
    }
}

pub fn extract_claims_or_local(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::auth::Claims, (StatusCode, String)> {
    if !state.config.server.enable_auth || state.config.server.standalone {
        let current_user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        return Ok(crate::auth::Claims {
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

    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ").or_else(|| Some(h))) {
        let trimmed = token_str.trim();
        if !trimmed.is_empty() && trimmed != "undefined" && trimmed != "null" {
            if let Ok(claims) = state.auth.verify_token(trimmed) {
                return Ok(claims);
            }
            let decoded = percent_decode(trimmed);
            if decoded != trimmed {
                if let Ok(claims) = state.auth.verify_token(&decoded) {
                    return Ok(claims);
                }
            }
        }
    }

    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie_hdr.split(';') {
            let pair = pair.trim();
            if let Some(tok) = pair.strip_prefix("cd_token=").or_else(|| pair.strip_prefix("token=")) {
                let trimmed = tok.trim();
                if !trimmed.is_empty() && trimmed != "undefined" && trimmed != "null" {
                    if let Ok(claims) = state.auth.verify_token(trimmed) {
                        return Ok(claims);
                    }
                    let decoded = percent_decode(trimmed);
                    if decoded != trimmed {
                        if let Ok(claims) = state.auth.verify_token(&decoded) {
                            return Ok(claims);
                        }
                    }
                }
            }
        }
    }

    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

pub fn normalize_path(path: &Path) -> PathBuf {
    let clean = crate::vfs::local::clean_path_buf(path);
    let clean_str = clean.to_string_lossy();
    if clean_str.starts_with(r"\\") || clean_str.starts_with("//") {
        let is_forward = clean_str.starts_with("//");
        let sep = if is_forward { '/' } else { '\\' };
        let prefix = if is_forward { "//" } else { r"\\" };
        let trimmed = clean_str.trim_start_matches(['\\', '/']);
        let mut segments: Vec<&str> = Vec::new();
        for seg in trimmed.split(['\\', '/']) {
            if seg.is_empty() || seg == "." {
                continue;
            } else if seg == ".." {
                segments.pop();
            } else {
                segments.push(seg);
            }
        }
        if segments.is_empty() {
            return PathBuf::from(prefix);
        }
        let joined = segments.join(&sep.to_string());
        return PathBuf::from(format!("{}{}", prefix, joined));
    }

    let mut components = Vec::new();
    for component in clean.components() {
        match component {
            std::path::Component::Prefix(..) => {
                components.clear();
                components.push(component);
            }
            std::path::Component::RootDir => {
                if let Some(std::path::Component::Prefix(..)) = components.first() {
                    components.truncate(1);
                    components.push(component);
                } else {
                    components.clear();
                    components.push(component);
                }
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if let Some(last) = components.last() {
                    if last != &std::path::Component::RootDir && !matches!(last, std::path::Component::Prefix(..)) {
                        components.pop();
                    }
                }
            }
            std::path::Component::Normal(..) => components.push(component),
        }
    }
    if components.len() == 1 && matches!(components.first(), Some(std::path::Component::Prefix(..))) {
        components.push(std::path::Component::RootDir);
    }
    components.into_iter().collect()
}

pub fn sanitize_uploaded_file_name(raw: &str) -> String {
    let clean = raw.trim();
    // Strip Windows drive letter prefix (e.g. "C:", "D:")
    let without_drive = if clean.len() >= 2 && clean.as_bytes()[1] == b':' && clean.as_bytes()[0].is_ascii_alphabetic() {
        &clean[2..]
    } else {
        clean
    };
    // Split on both '/' and '\\' and take the last component
    let filename = without_drive
        .split(|c| c == '/' || c == '\\')
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .last()
        .unwrap_or("upload.bin");

    // Filter out control characters
    let sanitized: String = filename.chars().filter(|c| !c.is_control()).collect();
    if sanitized.is_empty() {
        "upload.bin".to_string()
    } else {
        sanitized
    }
}

pub fn sanitize_uploaded_relative_path(raw: &str) -> String {
    let clean = raw.trim();
    // Strip Windows drive letter prefix (e.g. "C:", "D:")
    let without_drive = if clean.len() >= 2 && clean.as_bytes()[1] == b':' && clean.as_bytes()[0].is_ascii_alphabetic() {
        &clean[2..]
    } else {
        clean
    };

    let mut parts = Vec::new();
    for seg in without_drive.split(|c| c == '/' || c == '\\') {
        let seg_clean = seg.trim();
        if seg_clean.is_empty() || seg_clean == "." || seg_clean == ".." {
            continue;
        }
        // Filter out control characters
        let sanitized: String = seg_clean.chars().filter(|c| !c.is_control()).collect();
        if !sanitized.is_empty() && sanitized != "." && sanitized != ".." {
            parts.push(sanitized);
        }
    }

    if parts.is_empty() {
        "upload.bin".to_string()
    } else {
        parts.join("/")
    }
}

pub fn path_starts_with_case_insensitive(path: &Path, prefix: &Path) -> bool {
    let path = crate::vfs::local::clean_path_buf(path);
    let prefix = crate::vfs::local::clean_path_buf(prefix);

    let path_comps: Vec<_> = path.components().collect();
    let prefix_comps: Vec<_> = prefix.components().collect();

    if prefix_comps.len() > path_comps.len() {
        return false;
    }

    // Check if this is a Windows path (drive prefix or UNC prefix or Windows OS)
    let is_windows_path = cfg!(windows)
        || path_comps.first().map_or(false, |c| match c {
            std::path::Component::Prefix(..) => true,
            std::path::Component::Normal(n) => {
                let s = n.to_string_lossy();
                s.len() == 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':'
            }
            _ => false,
        })
        || prefix_comps.first().map_or(false, |c| match c {
            std::path::Component::Prefix(..) => true,
            std::path::Component::Normal(n) => {
                let s = n.to_string_lossy();
                s.len() == 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':'
            }
            _ => false,
        });

    for (p_comp, pre_comp) in path_comps.iter().zip(prefix_comps.iter()) {
        match (p_comp, pre_comp) {
            (std::path::Component::Prefix(p_prefix), std::path::Component::Prefix(pre_prefix)) => {
                let p_kind = p_prefix.kind();
                let pre_kind = pre_prefix.kind();
                match (p_kind, pre_kind) {
                    (std::path::Prefix::Disk(d1), std::path::Prefix::Disk(d2))
                    | (std::path::Prefix::VerbatimDisk(d1), std::path::Prefix::Disk(d2))
                    | (std::path::Prefix::Disk(d1), std::path::Prefix::VerbatimDisk(d2))
                    | (std::path::Prefix::VerbatimDisk(d1), std::path::Prefix::VerbatimDisk(d2)) => {
                        if !d1.eq_ignore_ascii_case(&d2) {
                            return false;
                        }
                    }
                    (std::path::Prefix::UNC(s1, sh1), std::path::Prefix::UNC(s2, sh2))
                    | (std::path::Prefix::VerbatimUNC(s1, sh1), std::path::Prefix::UNC(s2, sh2))
                    | (std::path::Prefix::UNC(s1, sh1), std::path::Prefix::VerbatimUNC(s2, sh2))
                    | (std::path::Prefix::VerbatimUNC(s1, sh1), std::path::Prefix::VerbatimUNC(s2, sh2)) => {
                        if !s1.to_string_lossy().eq_ignore_ascii_case(&s2.to_string_lossy())
                            || !sh1.to_string_lossy().eq_ignore_ascii_case(&sh2.to_string_lossy())
                        {
                            return false;
                        }
                    }
                    _ => {
                        if p_prefix.as_os_str().to_string_lossy().to_ascii_lowercase()
                            != pre_prefix.as_os_str().to_string_lossy().to_ascii_lowercase()
                        {
                            return false;
                        }
                    }
                }
            }
            (std::path::Component::RootDir, std::path::Component::RootDir) => {}
            (std::path::Component::Normal(p_str), std::path::Component::Normal(pre_str)) => {
                if is_windows_path {
                    if !p_str.to_string_lossy().eq_ignore_ascii_case(&pre_str.to_string_lossy()) {
                        return false;
                    }
                } else {
                    if p_str != pre_str {
                        return false;
                    }
                }
            }
            _ => {
                if p_comp != pre_comp {
                    return false;
                }
            }
        }
    }

    true
}

pub fn is_root_path(path: &Path) -> bool {
    let clean = crate::vfs::local::clean_path_buf(path);
    let s = clean.to_string_lossy();
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed == "/" || trimmed == "\\" {
        return true;
    }
    let comps: Vec<_> = clean.components().collect();
    if comps.is_empty() {
        return true;
    }
    let has_normal = comps.iter().any(|c| matches!(c, std::path::Component::Normal(..)));
    if !has_normal {
        return true;
    }
    if trimmed.len() <= 3 && trimmed.as_bytes().get(1) == Some(&b':') {
        return true;
    }
    false
}

pub fn resolve_effective_home(claims: &crate::auth::Claims, state: &AppState) -> Option<String> {
    let scheme = state.config.storage.home_fallback_scheme.to_lowercase();
    let is_admin = claims.role.eq_ignore_ascii_case("admin");

    // 1. "roots_only" scheme: Do not inject personal home directory if storage roots are configured
    if scheme == "roots_only" && !state.config.storage.roots.is_empty() {
        return None;
    }

    let raw_home = claims.home_dir.trim();

    // Sanitize username for template substitution
    let safe_username: String = claims.sub.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.').collect();
    let sub_name = if safe_username.is_empty() { "user" } else { &safe_username };

    // Determine candidate path
    let candidate = if !raw_home.is_empty() && (!is_root_path(Path::new(raw_home)) || state.config.storage.allow_entire_system) {
        raw_home.to_string()
    } else {
        state.config.storage.default_user_home_template.replace("{username}", sub_name)
    };

    let candidate_path = Path::new(&candidate);

    // If candidate path is valid, check if it exists or auto-create it
    if !is_root_path(candidate_path) || state.config.storage.allow_entire_system {
        if candidate_path.is_dir() {
            return Some(candidate);
        }
        if state.config.storage.auto_create_home_dirs {
            let _ = std::fs::create_dir_all(candidate_path);
            if candidate_path.is_dir() {
                return Some(candidate);
            }
        }
    }

    // 2. "strict" scheme: Do not fall back
    if scheme == "strict" {
        if candidate_path.is_dir() {
            return Some(candidate);
        }
        return None;
    }

    // 3. "auto" scheme: Graceful fallback cascade
    // Fallback A: First accessible configured storage root
    for root in &state.config.storage.roots {
        let role_ok = root.allowed_roles.is_empty() || root.allowed_roles.iter().any(|r| r.eq_ignore_ascii_case(&claims.role));
        let user_ok = is_admin || claims.allowed_roots.as_deref().map_or(false, |r| r.contains('*') || r.contains(&root.id) || r.contains(&root.path));
        if role_ok && user_ok && Path::new(&root.path).is_dir() {
            return Some(root.path.clone());
        }
    }

    // Fallback B: Container /data directory
    if Path::new("/data").is_dir() {
        let container_user_dir = format!("/data/users/{}", sub_name);
        if state.config.storage.auto_create_home_dirs {
            let _ = std::fs::create_dir_all(&container_user_dir);
        }
        if Path::new(&container_user_dir).is_dir() {
            return Some(container_user_dir);
        }
        return Some("/data".to_string());
    }

    // Fallback C: Real process user home if accessible and not root
    if let Some(home) = dirs::home_dir() {
        if !is_root_path(&home) && home.is_dir() {
            return Some(home.to_string_lossy().to_string());
        }
    }

    // Fallback D: App local data directory (~/.local/share/brum/users/{username} or %LOCALAPPDATA%/brum/users/{username})
    if let Some(data_dir) = dirs::data_local_dir() {
        let app_user_dir = data_dir.join("brum").join("users").join(sub_name);
        if state.config.storage.auto_create_home_dirs {
            let _ = std::fs::create_dir_all(&app_user_dir);
        }
        if app_user_dir.is_dir() {
            return Some(app_user_dir.to_string_lossy().to_string());
        }
    }

    if candidate_path.is_dir() || state.config.storage.allow_entire_system {
        Some(candidate)
    } else {
        None
    }
}

pub fn is_path_or_canonical_within(target: &Path, root: &Path) -> bool {
    let norm_target = normalize_path(target);
    let norm_root = normalize_path(root);
    if path_starts_with_case_insensitive(&norm_target, &norm_root) {
        let canonical_target = if let Ok(canonical) = target.canonicalize() {
            Some(canonical)
        } else {
            let mut tail_components = Vec::new();
            let mut curr = target;
            while !curr.exists() {
                // If curr exists as a symlink (dangling symlink), resolve its target and enforce boundary checks
                if let Ok(meta) = std::fs::symlink_metadata(curr) {
                    if meta.file_type().is_symlink() {
                        let link_target = match std::fs::read_link(curr) {
                            Ok(t) => t,
                            Err(_) => return false,
                        };

                        let mut resolved = if link_target.is_relative() {
                            curr.parent().map(|p| p.join(&link_target)).unwrap_or(link_target)
                        } else {
                            link_target
                        };

                        // Follow chained symlinks up to max depth 32
                        let mut depth = 0;
                        while depth < 32 {
                            if let Ok(m) = std::fs::symlink_metadata(&resolved) {
                                if m.file_type().is_symlink() {
                                    depth += 1;
                                    if let Ok(next) = std::fs::read_link(&resolved) {
                                        resolved = if next.is_relative() {
                                            resolved.parent().map(|p| p.join(&next)).unwrap_or(next)
                                        } else {
                                            next
                                        };
                                        continue;
                                    } else {
                                        return false;
                                    }
                                }
                            }
                            break;
                        }
                        if depth >= 32 {
                            return false;
                        }

                        // Append any trailing components that followed this dangling symlink
                        for part in tail_components.into_iter().rev() {
                            resolved.push(part);
                        }

                        let resolved_norm = normalize_path(&resolved);
                        let canonical_resolved = if let Ok(can) = resolved_norm.canonicalize() {
                            can
                        } else {
                            let mut d_tail = Vec::new();
                            let mut d_curr = resolved_norm.as_path();
                            while !d_curr.exists() {
                                if let Some(name) = d_curr.file_name() {
                                    d_tail.push(name.to_os_string());
                                } else {
                                    break;
                                }
                                if let Some(parent) = d_curr.parent() {
                                    d_curr = parent;
                                } else {
                                    break;
                                }
                            }
                            if d_curr.exists() {
                                if let Ok(mut can_anc) = d_curr.canonicalize() {
                                    for part in d_tail.into_iter().rev() {
                                        can_anc.push(part);
                                    }
                                    can_anc
                                } else {
                                    crate::vfs::local::clean_path_buf(&resolved_norm)
                                }
                            } else {
                                crate::vfs::local::clean_path_buf(&resolved_norm)
                            }
                        };

                        let canonical_root = root.canonicalize().unwrap_or_else(|_| norm_root.clone());
                        let clean_target = crate::vfs::local::clean_path_buf(&canonical_resolved);
                        let clean_root = crate::vfs::local::clean_path_buf(&canonical_root);
                        return path_starts_with_case_insensitive(&clean_target, &clean_root);
                    }
                }

                if let Some(file_name) = curr.file_name() {
                    tail_components.push(file_name.to_os_string());
                } else {
                    break;
                }
                if let Some(parent) = curr.parent() {
                    curr = parent;
                } else {
                    break;
                }
            }

            if curr.exists() {
                if let Ok(mut canonical_ancestor) = curr.canonicalize() {
                    for part in tail_components.into_iter().rev() {
                        canonical_ancestor.push(part);
                    }
                    Some(canonical_ancestor)
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(canonical_target) = canonical_target {
            let canonical_root = root.canonicalize().unwrap_or_else(|_| norm_root.clone());
            let clean_target = crate::vfs::local::clean_path_buf(&canonical_target);
            let clean_root = crate::vfs::local::clean_path_buf(&canonical_root);
            if !path_starts_with_case_insensitive(&clean_target, &clean_root) {
                return false;
            }
        }
        return true;
    }
    false
}

/// Determines if an IP address belongs to loopback, link-local, cloud metadata, or unspecified ranges.
pub fn is_prohibited_remote_ip(ip: &IpAddr, allow_loopback: bool) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // Loopback: 127.0.0.0/8
            if !allow_loopback && (octets[0] == 127 || v4.is_loopback()) {
                return true;
            }
            // Link-local / Cloud Metadata: 169.254.0.0/16
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            // Current network / Unspecified: 0.0.0.0/8
            if octets[0] == 0 || v4.is_unspecified() {
                return true;
            }
            // Broadcast: 255.255.255.255
            if v4.is_broadcast() {
                return true;
            }
            false
        }
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            // Loopback: ::1
            if !allow_loopback && v6.is_loopback() {
                return true;
            }
            // Unspecified: ::
            if v6.is_unspecified() {
                return true;
            }
            // Link-local: fe80::/10
            if octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80 {
                return true;
            }
            // IPv4-mapped IPv6 (::ffff:x.x.x.x)
            if octets[0..10] == [0; 10] && octets[10] == 0xff && octets[11] == 0xff {
                let v4 = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                return is_prohibited_remote_ip(&IpAddr::V4(v4), allow_loopback);
            }
            // IPv4-compatible IPv6 (::x.x.x.x)
            if octets[0..12] == [0; 12] && (octets[12] != 0 || octets[13] != 0 || octets[14] != 0 || octets[15] > 1) {
                let v4 = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                return is_prohibited_remote_ip(&IpAddr::V4(v4), allow_loopback);
            }
            false
        }
    }
}

/// Validates whether a remote hostname/IP targets internal infrastructure (anti-SSRF check).
pub fn is_prohibited_remote_host(host: &str, allow_loopback: bool) -> bool {
    let clean_host = host
        .trim()
        .trim_matches('[')
        .trim_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();

    if clean_host.is_empty() {
        return true;
    }

    // 1. Hostname checks for loopback and cloud metadata
    if !allow_loopback && (clean_host == "localhost" || clean_host.ends_with(".localhost")) {
        return true;
    }
    if clean_host == "metadata.google.internal"
        || clean_host.ends_with(".metadata.google.internal")
        || clean_host == "instance-data"
        || clean_host.ends_with(".instance-data")
    {
        return true;
    }

    // 2. Direct IP parsing (standard IPv4 / IPv6)
    if let Ok(ip) = clean_host.parse::<IpAddr>() {
        return is_prohibited_remote_ip(&ip, allow_loopback);
    }

    // 3. Decimal integer notation (e.g. 2130706433 -> 127.0.0.1)
    if let Ok(num) = clean_host.parse::<u32>() {
        let v4 = Ipv4Addr::from(num);
        return is_prohibited_remote_ip(&IpAddr::V4(v4), allow_loopback);
    }

    // 4. Hex / Octal dotted IPv4 parsing (e.g. 0177.0.0.1, 0x7f.0.0.1)
    let parts: Vec<&str> = clean_host.split('.').collect();
    if parts.len() == 4 {
        let mut octets = [0u8; 4];
        let mut parsed_all = true;
        for (i, p) in parts.iter().enumerate() {
            let val = if p.starts_with("0x") || p.starts_with("0X") {
                u8::from_str_radix(&p[2..], 16).ok()
            } else if p.starts_with('0') && p.len() > 1 {
                u8::from_str_radix(&p[1..], 8).ok()
            } else {
                p.parse::<u8>().ok()
            };
            if let Some(v) = val {
                octets[i] = v;
            } else {
                parsed_all = false;
                break;
            }
        }
        if parsed_all {
            let v4 = Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
            if is_prohibited_remote_ip(&IpAddr::V4(v4), allow_loopback) {
                return true;
            }
        }
    }

    // 5. DNS resolution check (detect domains resolving to loopback/metadata targets)
    if let Ok(addrs) = (clean_host.as_str(), 80).to_socket_addrs() {
        for addr in addrs {
            if is_prohibited_remote_ip(&addr.ip(), allow_loopback) {
                return true;
            }
        }
    }

    false
}

/// Extracts the scheme and target host from a remote network URI.
/// Returns Some((scheme, host)) if the path starts with a recognized network protocol.
pub fn extract_remote_scheme_host(raw_path: &str) -> Option<(String, String)> {
    let (scheme, remainder) = raw_path.split_once("://")?;
    let scheme_lower = scheme.to_ascii_lowercase();
    match scheme_lower.as_str() {
        "sftp" | "ssh" | "smb" | "samba" | "webdav" | "webdavs" | "s3" | "s3s" | "nfs" | "http" | "https" => {}
        _ => return None,
    }

    let authority = remainder.split(['/', '?', '#']).next().unwrap_or(remainder);
    let host_port = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);

    let host = if host_port.starts_with('[') {
        if let Some(end_bracket) = host_port.find(']') {
            &host_port[1..end_bracket]
        } else {
            host_port
        }
    } else if let Some(colon_idx) = host_port.find(':') {
        &host_port[..colon_idx]
    } else {
        host_port
    };

    Some((scheme_lower, host.trim().to_string()))
}

pub fn validate_path_access(
    state: &AppState,
    headers: &HeaderMap,
    raw_path: &str,
    is_write: bool,
) -> Result<String, (StatusCode, String)> {
    let claims = extract_claims_or_local(state, headers)?;
    let user_role = claims.role.as_str();
    let effective_home_opt = resolve_effective_home(&claims, state);
    let mut allowed_roots: Vec<String> = claims.allowed_roots.as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    allowed_roots.retain(|r| !r.trim().is_empty());

    if user_role.eq_ignore_ascii_case("readonly") && is_write {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot modify or delete files".to_string()));
    }

    // 1. Remote network scheme check: enforce admin or explicit entitlement authorization, plus anti-SSRF protections
    if let Some((scheme, host)) = extract_remote_scheme_host(raw_path) {
        let is_admin = user_role.eq_ignore_ascii_case("admin");
        let has_explicit_remote_perm = allowed_roots.iter().any(|r| {
            let r_clean = r.trim();
            if r_clean.is_empty() {
                return false;
            }
            r_clean == "remote"
                || r_clean == "*://*"
                || r_clean.eq_ignore_ascii_case(&scheme)
                || r_clean.eq_ignore_ascii_case(&format!("{}://*", scheme))
                || raw_path.starts_with(r_clean)
        });

        if !is_admin && !has_explicit_remote_perm {
            return Err((
                StatusCode::FORBIDDEN,
                format!("Access denied: Remote network scheme '{}' is restricted to administrators or explicitly authorized users", scheme),
            ));
        }

        let allow_loopback = state.config.storage.allow_remote_loopback;
        if is_prohibited_remote_host(&host, allow_loopback) {
            return Err((
                StatusCode::FORBIDDEN,
                format!("Access denied: Remote destination '{}' is a prohibited loopback, link-local, or cloud metadata target", host),
            ));
        }

        return Ok(raw_path.to_string());
    }

    // 2. Local virtual schemes (vault://, archive://, image://, trash://)
    let is_local_virtual_scheme = raw_path.starts_with("vault://")
        || raw_path.starts_with("archive://")
        || raw_path.starts_with("image://")
        || raw_path.starts_with("trash://");

    if is_local_virtual_scheme {
        return Ok(raw_path.to_string());
    }

    let default_home = effective_home_opt.as_deref().unwrap_or("/");

    let expanded = if raw_path == "~" {
        default_home.to_string()
    } else if let Some(stripped) = raw_path.strip_prefix("~/") {
        Path::new(default_home).join(stripped).to_string_lossy().to_string()
    } else if let Some(stripped) = raw_path.strip_prefix(r"~\") {
        Path::new(default_home).join(stripped).to_string_lossy().to_string()
    } else {
        #[cfg(windows)]
        {
            if (raw_path == "/" || raw_path == "\\") && default_home != "/" {
                default_home.to_string()
            } else if (raw_path.starts_with('/') || raw_path.starts_with('\\'))
                && raw_path.len() >= 3
                && raw_path.as_bytes()[1].is_ascii_alphabetic()
                && raw_path.as_bytes()[2] == b':'
            {
                raw_path[1..].to_string()
            } else {
                raw_path.to_string()
            }
        }
        #[cfg(not(windows))]
        {
            if (raw_path.starts_with('/') || raw_path.starts_with('\\'))
                && raw_path.len() >= 3
                && raw_path.as_bytes()[1].is_ascii_alphabetic()
                && raw_path.as_bytes()[2] == b':'
            {
                raw_path[1..].to_string()
            } else {
                raw_path.to_string()
            }
        }
    };

    let normalized = normalize_path(Path::new(&expanded));
    let norm_str = normalized.to_string_lossy().to_string();
    let is_admin = user_role.eq_ignore_ascii_case("admin");

    // Unrestricted system root access if enabled in config AND user has permission
    let allow_system = state.config.storage.allow_entire_system
        && (is_admin || allowed_roots.contains(&"*".to_string()) || allowed_roots.contains(&"/".to_string()));
    if allow_system {
        return Ok(norm_str);
    }

    // Permit user within their designated home directory (if resolved and home is not root or allow_entire_system is enabled)
    if let Some(ref effective_home) = effective_home_opt {
        let norm_home = normalize_path(Path::new(effective_home));
        if (!is_root_path(&norm_home) || state.config.storage.allow_entire_system)
            && is_path_or_canonical_within(&normalized, &norm_home)
        {
            return Ok(norm_str);
        }
    }

    // Check configured storage roots
    for root in &state.config.storage.roots {
        let role_ok = root.allowed_roles.is_empty() || root.allowed_roles.iter().any(|r| r.eq_ignore_ascii_case(user_role));
        let user_ok = is_admin || allowed_roots.contains(&"*".to_string()) || allowed_roots.contains(&root.id) || allowed_roots.contains(&root.path);

        if role_ok && user_ok {
            let norm_root = normalize_path(Path::new(&root.path));
            if is_path_or_canonical_within(&normalized, &norm_root) {
                if is_write && root.read_only {
                    return Err((StatusCode::FORBIDDEN, format!("Storage root '{}' is configured as read-only", root.name)));
                }
                return Ok(norm_str);
            }
        }
    }

    Err((
        StatusCode::FORBIDDEN,
        format!("Access denied: Path '{}' is outside your authorized storage roots", raw_path),
    ))
}

pub fn expand_tilde(path_str: &str) -> String {
    crate::config::expand_tilde_path(path_str)
        .to_string_lossy()
        .to_string()
}

/// Evaluates whether an IP address matches the trusted proxies whitelist,
/// supporting both exact IP matches (e.g. "127.0.0.1", "::1") and CIDR subnets (e.g. "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16").
pub fn is_ip_trusted(ip_str: &str, trusted_proxies: &[String]) -> bool {
    let clean_ip = ip_str.trim();
    if clean_ip.is_empty() {
        return false;
    }
    let target_ip: std::net::IpAddr = match clean_ip.parse() {
        Ok(ip) => ip,
        Err(_) => return false,
    };

    for proxy in trusted_proxies {
        let p = proxy.trim();
        if p.is_empty() {
            continue;
        }
        if p.eq_ignore_ascii_case(clean_ip) {
            return true;
        }
        if let Some((cidr_ip_str, prefix_str)) = p.split_once('/') {
            if let Ok(prefix) = prefix_str.parse::<u8>() {
                match (target_ip, cidr_ip_str.trim().parse::<std::net::IpAddr>()) {
                    (std::net::IpAddr::V4(t4), Ok(std::net::IpAddr::V4(c4))) => {
                        if prefix <= 32 {
                            let mask = if prefix == 0 { 0 } else { !0u32 << (32 - prefix) };
                            if (u32::from(t4) & mask) == (u32::from(c4) & mask) {
                                return true;
                            }
                        }
                    }
                    (std::net::IpAddr::V6(t6), Ok(std::net::IpAddr::V6(c6))) => {
                        if prefix <= 128 {
                            let mask = if prefix == 0 { 0 } else { !0u128 << (128 - prefix) };
                            if (u128::from(t6) & mask) == (u128::from(c6) & mask) {
                                return true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    false
}

fn find_rightmost_untrusted_ip<'a>(
    forwarded: &'a str,
    trusted_proxies: &[String],
) -> Option<&'a str> {
    for raw in forwarded.rsplit(',') {
        let ip = raw.trim();
        if ip.is_empty() {
            continue;
        }
        if !is_ip_trusted(ip, trusted_proxies) {
            return Some(ip);
        }
    }
    None
}

pub fn resolve_client_ip(
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
    trusted_proxies: &[String],
) -> String {
    if let Some(peer) = peer_addr {
        let peer_ip = peer.ip().to_string();
        let is_trusted = is_ip_trusted(&peer_ip, trusted_proxies);

        if is_trusted {
            if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
                if let Some(client_ip) = find_rightmost_untrusted_ip(forwarded, trusted_proxies) {
                    return client_ip.to_string();
                }
            }
            if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
                let trimmed = real_ip.trim();
                let is_real_trusted = is_ip_trusted(trimmed, trusted_proxies);
                if !trimmed.is_empty() && !is_real_trusted {
                    return trimmed.to_string();
                }
            }
        }
        return peer_ip;
    }

    // Fallback when peer socket address is not directly provided (e.g. unit tests)
    if !trusted_proxies.is_empty() {
        if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(client_ip) = find_rightmost_untrusted_ip(forwarded, trusted_proxies) {
                return client_ip.to_string();
            }
        }
        if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            let trimmed = real_ip.trim();
            let is_real_trusted = is_ip_trusted(trimmed, trusted_proxies);
            if !trimmed.is_empty() && !is_real_trusted {
                return trimmed.to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_percent_decode_basic() {
        assert_eq!(percent_decode(""), "");
        assert_eq!(percent_decode("simple_token_123"), "simple_token_123");
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("foo%2Bbar%3Dbaz%2Fqux"), "foo+bar=baz/qux");
        assert_eq!(percent_decode("token%2Ewith%2Edots"), "token.with.dots");
    }

    #[test]
    fn test_percent_decode_malformed() {
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("%2"), "%2");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("test%2Gextra"), "test%2Gextra");
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
    fn test_extract_claims_or_local_and_percent_decoding() {
        let mut config = crate::config::AppConfig::default();
        let tmp = tempfile::tempdir().unwrap();
        config.server.database_path = tmp.path().join("mid_test.db").to_string_lossy().to_string();

        let state = create_test_state(config);
        let user = state.auth.create_user(
            "bob",
            "secretpass",
            "admin",
            "/home/bob",
            Some("[\"*\"]"),
        ).unwrap();
        let token = state.auth.generate_token(&user).unwrap();

        // 1. Missing token -> Err
        let headers_empty = HeaderMap::new();
        assert!(extract_claims_or_local(&state, &headers_empty).is_err());

        // 2. Authorization header -> Ok
        let mut headers_auth = HeaderMap::new();
        headers_auth.insert(header::AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
        let claims = extract_claims_or_local(&state, &headers_auth).unwrap();
        assert_eq!(claims.sub, "bob");

        // 3. Percent-encoded authorization header -> Ok
        let mut headers_encoded = HeaderMap::new();
        let enc_token = token.replace('.', "%2E");
        headers_encoded.insert(header::AUTHORIZATION, format!("Bearer {}", enc_token).parse().unwrap());
        let claims = extract_claims_or_local(&state, &headers_encoded).unwrap();
        assert_eq!(claims.sub, "bob");

        // 4. Stale cookie + valid Authorization header -> Ok
        let mut headers_stale = HeaderMap::new();
        headers_stale.insert(header::AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
        headers_stale.insert(header::COOKIE, "cd_token=stale_invalid_signature".parse().unwrap());
        let claims = extract_claims_or_local(&state, &headers_stale).unwrap();
        assert_eq!(claims.sub, "bob");
    }
}


