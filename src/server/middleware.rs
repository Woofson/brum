use std::collections::HashMap;
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

    pub fn check_and_record(&self, key: &str, max_attempts: usize, window_secs: u64) -> Result<(), u64> {
        let mut map = self.attempts.lock();
        let now = Instant::now();
        let window = std::time::Duration::from_secs(window_secs);

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

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let path = req.uri().path().to_string();

    // 1. Standalone desktop or auth-disabled instances bypass auth checks with local admin claims
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
        };
        req.extensions_mut().insert(claims);
        return Ok(next.run(req).await);
    }

    let is_pub = is_public_route(&path);

    // 2. Attempt to extract token from Authorization header, Cookie, or query
    let token_opt = {
        let headers = req.headers();
        let from_header = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer ").or_else(|| Some(h)))
            .map(|s| s.trim().to_string());

        let from_cookie = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|cookie_str| {
                for pair in cookie_str.split(';') {
                    let pair = pair.trim();
                    if let Some(tok) = pair.strip_prefix("cd_token=").or_else(|| pair.strip_prefix("token=")) {
                        return Some(tok.trim().to_string());
                    }
                }
                None
            });

        let from_query = req.uri().query().and_then(|q| {
            for pair in q.split('&') {
                if let Some(tok) = pair.strip_prefix("token=").or_else(|| pair.strip_prefix("cd_token=")) {
                    return Some(tok.trim().to_string());
                }
            }
            None
        });

        from_header.or(from_cookie).or(from_query)
    };

    match token_opt {
        Some(token) if !token.is_empty() => {
            match state.auth.verify_token(&token) {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    Ok(next.run(req).await)
                }
                Err(err) => {
                    if is_pub {
                        Ok(next.run(req).await)
                    } else {
                        Err((
                            StatusCode::UNAUTHORIZED,
                            Json(serde_json::json!({
                                "error": format!("Unauthorized: {}", err),
                                "code": "UNAUTHORIZED"
                            })),
                        ))
                    }
                }
            }
        }
        _ => {
            if is_pub {
                Ok(next.run(req).await)
            } else {
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
        });
    }

    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        if let Ok(claims) = state.auth.verify_token(token_str) {
            return Ok(claims);
        }
    }

    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie_hdr.split(';') {
            let pair = pair.trim();
            if let Some(tok) = pair.strip_prefix("cd_token=").or_else(|| pair.strip_prefix("token=")) {
                if let Ok(claims) = state.auth.verify_token(tok) {
                    return Ok(claims);
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
        if let Ok(canonical_target) = target.canonicalize() {
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

pub fn validate_path_access(
    state: &AppState,
    headers: &HeaderMap,
    raw_path: &str,
    is_write: bool,
) -> Result<String, (StatusCode, String)> {
    let claims = extract_claims_or_local(state, headers)?;
    let user_role = claims.role.as_str();
    let effective_home_opt = resolve_effective_home(&claims, state);
    let allowed_roots: Vec<String> = claims.allowed_roots.as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();

    if user_role.eq_ignore_ascii_case("readonly") && is_write {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot modify or delete files".to_string()));
    }

    // Remote protocols bypass local storage root checks only if they start with a valid scheme prefix
    let is_remote_or_special_scheme = raw_path.starts_with("sftp://")
        || raw_path.starts_with("webdav://")
        || raw_path.starts_with("webdavs://")
        || raw_path.starts_with("s3://")
        || raw_path.starts_with("s3s://")
        || raw_path.starts_with("smb://")
        || raw_path.starts_with("samba://")
        || raw_path.starts_with("vault://")
        || raw_path.starts_with("archive://")
        || raw_path.starts_with("image://")
        || raw_path.starts_with("trash://");

    if is_remote_or_special_scheme {
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
    if path_str == "~" {
        dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|| "~".to_string())
    } else if let Some(stripped) = path_str.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(stripped).to_string_lossy().to_string()
        } else {
            path_str.to_string()
        }
    } else {
        path_str.to_string()
    }
}

pub fn extract_client_ip(headers: &HeaderMap) -> String {
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first_ip) = forwarded.split(',').next() {
            let trimmed = first_ip.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let trimmed = real_ip.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "127.0.0.1".to_string()
}

