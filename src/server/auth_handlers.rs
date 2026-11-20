use axum::{
    extract::{ConnectInfo, Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use crate::auth::User;
use crate::server::AppState;
use crate::server::session_cookie;
use crate::server::middleware::{extract_claims_or_local, resolve_client_ip, require_admin};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    /// Left out when the Brum page asked for a cookie-only login
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    pub user: User,
    /// The account must choose a new password before anything else (#205)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub must_change_password: bool,
}

pub async fn handle_oidc_config(State(state): State<AppState>) -> Json<crate::auth::oidc::OidcPublicConfig> {
    Json(state.oidc.get_public_config())
}

pub async fn handle_oidc_login(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !state.oidc.is_enabled() {
        return Err((StatusCode::NOT_FOUND, "OIDC authentication is not enabled".to_string()));
    }

    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:8080");

    let dynamic_redirect_uri = format!("{}://{}/api/auth/oidc/callback", scheme, host);

    match state.oidc.generate_auth_url(&dynamic_redirect_uri).await {
        Ok((auth_url, _state)) => {
            Ok(axum::response::Redirect::temporary(&auth_url))
        }
        Err(e) => {
            tracing::error!("Failed to generate OIDC authorization URL: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to initiate SSO login: {}", e)))
        }
    }
}

pub fn is_request_secure_peer(
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
    trusted_proxies: &[String],
) -> bool {
    // If peer_addr is provided, only trust proxy headers if the peer is in trusted_proxies
    if let Some(peer) = peer_addr {
        if !crate::server::middleware::is_ip_trusted(&peer.ip().to_string(), trusted_proxies) {
            return false;
        }
    }

    if let Some(proto) = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) {
        if proto.split(',').next().unwrap_or("").trim().eq_ignore_ascii_case("https") {
            return true;
        }
    }
    if let Some(ssl) = headers.get("x-forwarded-ssl").and_then(|v| v.to_str().ok()) {
        if ssl.trim().eq_ignore_ascii_case("on") {
            return true;
        }
    }
    if let Some(front_end) = headers.get("front-end-https").and_then(|v| v.to_str().ok()) {
        if front_end.trim().eq_ignore_ascii_case("on") {
            return true;
        }
    }
    false
}

pub fn is_request_secure(headers: &HeaderMap) -> bool {
    is_request_secure_peer(headers, None, &[])
}

#[derive(Deserialize)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

pub async fn handle_oidc_callback(
    State(state): State<AppState>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    Query(query): Query<OidcCallbackQuery>,
) -> impl IntoResponse {
    let peer_addr = connect_info.map(|ci| ci.0);
    if let Some(err) = query.error {
        let desc = query.error_description.unwrap_or_default();
        tracing::warn!("OIDC provider returned error: {} - {}", err, desc);
        return axum::response::Redirect::temporary(&format!("/?error={}", urlencoding_simple(&err))).into_response();
    }

    let code = match query.code {
        Some(c) if !c.trim().is_empty() => c,
        _ => return axum::response::Redirect::temporary("/?error=missing_code").into_response(),
    };

    let state_param = match query.state {
        Some(s) if !s.trim().is_empty() => s,
        _ => return axum::response::Redirect::temporary("/?error=missing_state").into_response(),
    };

    match state.oidc.exchange_code_and_login(&code, &state_param).await {
        Ok((token, user)) => {
            tracing::info!(username = %user.username, role = %user.role, "SSO login successful");
            let max_age_secs = state.config.server.session_duration_hours * 3600;
            let secure = is_request_secure_peer(&headers, peer_addr, &state.config.server.trusted_proxies);
            let mut response = axum::response::Redirect::temporary("/?sso_success=1").into_response();
            session_cookie::attach_login_cookie(response.headers_mut(), &headers, &token, max_age_secs, secure);
            response
        }
        Err(e) => {
            tracing::error!("OIDC callback error: {}", e);
            axum::response::Redirect::temporary(&format!("/?error={}", urlencoding_simple(&e.to_string()))).into_response()
        }
    }
}

fn urlencoding_simple(s: &str) -> String {
    let mut encoded = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{:02X}", b));
        }
    }
    encoded
}

/// The key for one account's wrong-password count. The login page and the lock screen share it.
fn account_guard_key(username: &str) -> String {
    format!("login:user:{}", username.trim().to_lowercase())
}

/// Counts a password try for an account before it is checked, or refuses it when the account
/// has had too many wrong passwords lately. A correct password clears the count again.
/// Counting before checking means tries sent at the same moment can't slip past the limit,
/// so the computer never sees more failed logins than the limit allows.
fn account_guard_try(state: &AppState, username: &str) -> Result<(), String> {
    let (tries, window) = state.auth.wrong_password_limit(username.trim());
    state.rate_limiter
        .check_and_record(&account_guard_key(username), tries, window)
        .map_err(|retry_after| {
            tracing::warn!(username = %username, tries, "Too many wrong passwords for this account; pausing sign-in tries");
            let minutes = retry_after.div_ceil(60);
            format!(
                "Too many wrong passwords for this account. To keep the computer from locking the account, \
                 Brum pauses sign-in tries for {} minute{}.",
                minutes,
                if minutes == 1 { "" } else { "s" }
            )
        })
}

pub async fn handle_login(
    State(state): State<AppState>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let peer_addr = connect_info.map(|ci| ci.0);
    let client_ip = resolve_client_ip(&headers, peer_addr, &state.config.server.trusted_proxies);
    let rate_key = format!("login:{}", client_ip);
    if let Err(retry_after) = state.rate_limiter.check_and_record(&rate_key, 10, 60) {
        tracing::error!(client_ip = %client_ip, "Rate limit exceeded on login endpoint: repeated login attempts");
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many login attempts. Please retry after {} seconds.", retry_after),
        ));
    }

    // Per-account limit on wrong passwords, shared with the lock screen
    let user_rate_key = account_guard_key(&payload.username);
    if let Err(msg) = account_guard_try(&state, &payload.username) {
        return Err((StatusCode::TOO_MANY_REQUESTS, msg));
    }

    tracing::info!("Received login request for user '{}'...", payload.username);

    // Enforce private instance owner isolation
    if let Some(ref inst) = state.instance_info {
        if payload.username.to_lowercase() != inst.owner.to_lowercase() {
            tracing::warn!(
                "Rejected login attempt by user '{}' on private instance owned by '{}'",
                payload.username, inst.owner
            );
            return Err((
                StatusCode::FORBIDDEN,
                format!("Access denied: this Brum instance is privately owned by '{}'.", inst.owner),
            ));
        }
    }

    let make_login_response = |token: String, user: User| -> axum::response::Response {
        let max_age_secs = state.config.server.session_duration_hours * 3600;
        let secure = is_request_secure_peer(&headers, peer_addr, &state.config.server.trusted_proxies);
        let body_token = if session_cookie::wants_cookie_only(&headers) { String::new() } else { token.clone() };
        let must_change_password = state.config.server.enable_auth && !state.config.server.standalone && state.auth.must_change_password(&user.username);
        let mut resp = Json(LoginResponse { token: body_token, user, must_change_password }).into_response();
        session_cookie::attach_login_cookie(resp.headers_mut(), &headers, &token, max_age_secs, secure);
        resp
    };

    // Standalone / Auth Disabled bypass
    if !state.config.server.enable_auth || state.config.server.standalone {
        tracing::info!("Authentication disabled or standalone mode active - granting login for '{}'", payload.username);
        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        let user = if let Ok(Some(mut u)) = state.auth.get_user_by_username(&payload.username) {
            u.resolve_avatar();
            u
        } else if let Ok(Some(mut u)) = state.auth.get_user_by_username(&current_user) {
            u.resolve_avatar();
            u
        } else {
            let avatar_url = crate::auth::resolve_system_avatar(&current_user, &home_dir);
            User {
                id: 1,
                username: current_user.clone(),
                nickname: Some(current_user),
                full_name: None,
                bio: None,
                email: None,
                avatar_url,
                role: "admin".to_string(),
                home_dir,
                is_pam: false,
                is_disabled: false,
                allowed_services: "[\"*\"]".to_string(),
                allowed_roots: "[\"*\"]".to_string(),
                can_install_plugins: true,
                allowed_plugins: "[\"*\"]".to_string(),
                blocked_plugins: "[]".to_string(),
                auth_source: None,
                shell: None,
            }
        };

        state.rate_limiter.reset(&rate_key);
        state.rate_limiter.reset(&user_rate_key);
        let token = state.auth.generate_token(&user).unwrap_or_default();
        return Ok(make_login_response(token, user));
    }

    // Offload Argon2 password verification to blocking thread pool
    let auth = state.auth.clone();
    let uname = payload.username.clone();
    let pwd = payload.password.clone();
    let auth_res = tokio::task::spawn_blocking(move || {
        auth.authenticate(&uname, &pwd)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Authentication task failed: {}", e)))?;

    match auth_res {
        Ok(user) => {
            state.rate_limiter.reset(&rate_key);
            state.rate_limiter.reset(&user_rate_key);
            tracing::info!(username = %user.username, is_pam = user.is_pam, role = %user.role, "User successfully authenticated");
            let token = state.auth.generate_token(&user).map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create token: {}", e))
            })?;
            Ok(make_login_response(token, user))
        }
        Err(e) => {
            tracing::warn!(username = %payload.username, error = %e, "Authentication failed");
            Err((StatusCode::UNAUTHORIZED, "Invalid username or password".to_string()))
        }
    }
}

/// Moves a login the page used to keep itself into the protected cookie, once.
/// The page sends its old stored login in the `Authorization` header; afterwards it forgets it.
pub async fn handle_adopt_session_cookie(
    State(state): State<AppState>,
    headers: HeaderMap,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing login".to_string()))?;
    let claims = state.auth.verify_token(&token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Login no longer valid".to_string()))?;
    if claims.token_id.is_some() {
        // API tokens are for scripts, not for the browser's login cookie
        return Err((StatusCode::BAD_REQUEST, "API tokens cannot become a browser login".to_string()));
    }
    let remaining = (claims.exp - chrono::Utc::now().timestamp()).max(0) as u64;
    let secure = is_request_secure_peer(&headers, connect_info.map(|ci| ci.0), &state.config.server.trusted_proxies);
    let mut resp = Json(serde_json::json!({ "success": true })).into_response();
    session_cookie::attach_login_cookie(resp.headers_mut(), &headers, &token, remaining, secure);
    Ok(resp)
}

pub async fn handle_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // End this login on the server too, so a copied token stops working
    if state.config.server.enable_auth && !state.config.server.standalone {
        if let Ok(claims) = crate::server::middleware::extract_claims_or_local(&state, &headers) {
            if claims.token_id.is_none() {
                if let Err(e) = state.auth.revoke_session(&claims) {
                    tracing::warn!(user = %claims.sub, error = %e, "Could not end session on logout");
                }
            }
        }
    }
    let mut resp = Json(serde_json::json!({ "success": true, "message": "Logged out" })).into_response();
    session_cookie::attach_logout_cookies(resp.headers_mut(), &headers);
    resp
}

/// The signed-in account, and whether it must choose a new password first (#205)
pub async fn handle_get_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let login_on = state.config.server.enable_auth && !state.config.server.standalone;
    let Json(user) = handle_get_me_user(State(state.clone()), headers).await?;
    let mut body = serde_json::to_value(&user).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if login_on && state.auth.must_change_password(&user.username) {
        body["must_change_password"] = serde_json::Value::Bool(true);
    }
    Ok(Json(body))
}

async fn handle_get_me_user(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<User>, (StatusCode, String)> {
    if !state.config.server.enable_auth || state.config.server.standalone {
        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        if let Ok(Some(mut user)) = state.auth.get_user_by_username(&current_user) {
            user.resolve_avatar();
            return Ok(Json(user));
        }

        let avatar_url = crate::auth::resolve_system_avatar(&current_user, &home_dir);

        return Ok(Json(User {
            id: 1,
            username: current_user.clone(),
            nickname: Some(current_user),
            full_name: None,
            bio: None,
            email: None,
            avatar_url,
            role: "admin".to_string(),
            home_dir,
            is_pam: false,
            is_disabled: false,
            allowed_services: "[\"*\"]".to_string(),
            allowed_roots: "[\"*\"]".to_string(),
            can_install_plugins: true,
            allowed_plugins: "[\"*\"]".to_string(),
            blocked_plugins: "[]".to_string(),
            auth_source: None,
            shell: None,
        }));
    }

    // The first login that still works: an explicit header, then this Brum's cookie, then older cookies
    let token_opt = {
        let from_header = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer ").or_else(|| Some(h)))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "undefined" && s != "null");
        from_header
            .into_iter()
            .chain(session_cookie::tokens_from_cookies(&headers))
            .find(|t| state.auth.verify_token(t).is_ok())
    };

    if let Some(token_str) = token_opt {
        match state.auth.verify_token(&token_str) {
            Ok(claims) => {
                if let Ok(Some(mut user)) = state.auth.get_user_by_username(&claims.sub) {
                    user.resolve_avatar();
                    return Ok(Json(user));
                }
                // Automatically sync PAM user to database so they appear in Users table & RBAC
                if let Ok(mut synced_user) = state.auth.sync_pam_user_to_db(&claims.sub, &claims.role, &claims.home_dir) {
                    synced_user.resolve_avatar();
                    return Ok(Json(synced_user));
                }
                let is_admin = claims.role == "admin";
                let avatar_url = crate::auth::resolve_system_avatar(&claims.sub, &claims.home_dir);
                Ok(Json(User {
                    id: 0,
                    username: claims.sub,
                    nickname: None,
                    full_name: None,
                    bio: None,
                    email: None,
                    avatar_url,
                    role: claims.role,
                    home_dir: claims.home_dir,
                    is_pam: claims.is_pam,
                    is_disabled: false,
                    allowed_services: "[\"*\"]".to_string(),
                    allowed_roots: claims.allowed_roots.unwrap_or_else(|| "[\"*\"]".to_string()),
                    can_install_plugins: is_admin,
                    allowed_plugins: "[\"*\"]".to_string(),
                    blocked_plugins: "[]".to_string(),
                    auth_source: if claims.is_pam { Some(if cfg!(windows) { "windows".to_string() } else { "pam".to_string() }) } else { None },
                    shell: claims.shell,
                }))
            }
            Err(_) => Err((StatusCode::UNAUTHORIZED, "Invalid token".to_string())),
        }
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

pub fn get_avatars_dir() -> std::path::PathBuf {
    if let Some(dir) = crate::config::private_data_dir("avatars") {
        dir
    } else {
        std::path::PathBuf::from("data").join("avatars")
    }
}

pub async fn handle_get_user_avatar(
    AxumPath(username): AxumPath<String>,
) -> Result<impl axum::response::IntoResponse, (StatusCode, String)> {
    let sanitized: String = username.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.').collect();
    if sanitized.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Invalid username".to_string()));
    }

    let avatars_dir = get_avatars_dir();
    let candidates = [
        (avatars_dir.join(format!("{}.webp", sanitized)), "image/webp"),
        (avatars_dir.join(format!("{}.png", sanitized)), "image/png"),
        (avatars_dir.join(format!("{}.jpg", sanitized)), "image/jpeg"),
        (avatars_dir.join(format!("{}.jpeg", sanitized)), "image/jpeg"),
        (avatars_dir.join(format!("{}.svg", sanitized)), "image/svg+xml"),
    ];

    for (path, content_type) in &candidates {
        if path.exists() {
            if let Ok(bytes) = tokio::fs::read(path).await {
                return Ok((
                    [
                        (header::CONTENT_TYPE, *content_type),
                        (header::CACHE_CONTROL, "public, max-age=86400, stale-while-revalidate=604800"),
                    ],
                    bytes,
                ));
            }
        }
    }

    // Check system avatar fallback
    let home_dir = format!("/home/{}", sanitized);
    if let Some(data_uri) = crate::auth::resolve_system_avatar(&sanitized, &home_dir) {
        if let Some(comma_pos) = data_uri.find(',') {
            let meta = &data_uri[..comma_pos];
            let b64_data = &data_uri[comma_pos + 1..];
            use base64::Engine;
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64_data) {
                let content_type = if meta.contains("image/webp") {
                    "image/webp"
                } else if meta.contains("image/jpeg") {
                    "image/jpeg"
                } else if meta.contains("image/svg") {
                    "image/svg+xml"
                } else {
                    "image/png"
                };
                return Ok((
                    [
                        (header::CONTENT_TYPE, content_type),
                        (header::CACHE_CONTROL, "public, max-age=86400"),
                    ],
                    bytes,
                ));
            }
        }
    }

    Err((StatusCode::NOT_FOUND, "Avatar not found".to_string()))
}

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    pub nickname: Option<String>,
    pub full_name: Option<String>,
    pub bio: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub new_password: Option<String>,
}

pub async fn handle_update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    _connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let username = if !state.config.server.enable_auth || state.config.server.standalone {
        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());
        if state.auth.get_user_by_username(&current_user).ok().flatten().is_none() {
            let home_dir = dirs::home_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".to_string());
            let _ = state.auth.create_user(&current_user, "local_no_password", "admin", &home_dir, Some("[\"*\"]"));
        }
        current_user
    } else {
        let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
        if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
            let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
            if state.auth.get_user_by_username(&claims.sub).ok().flatten().is_none() {
                let _ = state.auth.sync_pam_user_to_db(&claims.sub, &claims.role, &claims.home_dir);
            }
            claims.sub
        } else {
            return Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()));
        }
    };

    if payload.new_password.as_deref().is_some_and(|p| !p.trim().is_empty()) {
        // Changing the password needs the current one (#205): it has its own request now
        return Err((StatusCode::BAD_REQUEST, "Change your password with Change password, which asks for the current one.".into()));
    }

    let mut effective_avatar_url = payload.avatar_url.clone();

    // If payload avatar_url is a base64 data URI, extract and save to server avatars directory
    if let Some(ref av) = payload.avatar_url {
        if av.starts_with("data:image/") {
            if let Some(comma_pos) = av.find(',') {
                let meta = &av[..comma_pos];
                let b64 = &av[comma_pos + 1..];
                use base64::Engine;
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                    let ext = if meta.contains("webp") { "webp" } else if meta.contains("jpeg") || meta.contains("jpg") { "jpg" } else if meta.contains("svg") { "svg" } else { "png" };
                    let avatars_dir = get_avatars_dir();
                    let _ = std::fs::create_dir_all(&avatars_dir);
                    let target_path = avatars_dir.join(format!("{}.{}", username, ext));
                    if std::fs::write(&target_path, bytes).is_ok() {
                        effective_avatar_url = Some(format!("/api/auth/avatar/{}?t={}", username, chrono::Utc::now().timestamp()));
                    }
                }
            }
        } else if av.trim().is_empty() || av == "👤" {
            let avatars_dir = get_avatars_dir();
            for ext in &["webp", "png", "jpg", "jpeg", "svg"] {
                let _ = std::fs::remove_file(avatars_dir.join(format!("{}.{}", username, ext)));
            }
        }
    }

    let _ = state.auth.update_user_profile(
        &username,
        payload.nickname.as_deref(),
        payload.full_name.as_deref(),
        payload.bio.as_deref(),
        payload.email.as_deref(),
        effective_avatar_url.as_deref(),
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update profile: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Profile updated successfully",
        "avatar_url": effective_avatar_url
    })).into_response())
}

#[derive(Deserialize)]
pub struct UpdateUserRbacRequest {
    pub role: String,
    #[serde(default)]
    pub allowed_services: Vec<String>,
    #[serde(default)]
    pub allowed_roots: Option<Vec<String>>,
    #[serde(default)]
    pub home_dir: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub can_install_plugins: Option<bool>,
    #[serde(default)]
    pub allowed_plugins: Option<Vec<String>>,
    #[serde(default)]
    pub blocked_plugins: Option<Vec<String>>,
    #[serde(default)]
    pub is_disabled: Option<bool>,
}

async fn handle_update_user_rbac_inner(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
    Json(payload): Json<UpdateUserRbacRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    // An admin cannot lock themselves out, so the machine always keeps an admin (#205)
    if username.eq_ignore_ascii_case(&claims.sub) {
        if payload.role != "admin" {
            return Err((StatusCode::BAD_REQUEST, "You cannot remove your own admin role. Another admin can.".into()));
        }
        if payload.is_disabled.unwrap_or(false) {
            return Err((StatusCode::BAD_REQUEST, "You cannot disable your own account. Another admin can.".into()));
        }
    }

    let services_json = if payload.allowed_services.is_empty() {
        "[\"*\"]".to_string()
    } else {
        serde_json::to_string(&payload.allowed_services).unwrap_or_else(|_| "[\"*\"]".to_string())
    };
    let roots_json = payload.allowed_roots.map(|r| {
        let clean: Vec<String> = r.into_iter().filter(|s| !s.trim().is_empty()).collect();
        serde_json::to_string(&clean).unwrap_or_else(|_| "[\"*\"]".to_string())
    });
    let allowed_plugins_json = payload.allowed_plugins.map(|p| serde_json::to_string(&p).unwrap_or_else(|_| "[\"*\"]".to_string()));
    let blocked_plugins_json = payload.blocked_plugins.map(|p| serde_json::to_string(&p).unwrap_or_else(|_| "[]".to_string()));

    let shell_opt: Option<Option<&str>> = payload.shell.as_deref().map(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    let updated = state.auth.update_user_rbac(
        &username,
        &payload.role,
        &services_json,
        roots_json.as_deref(),
        payload.home_dir.as_deref(),
        payload.can_install_plugins,
        allowed_plugins_json.as_deref(),
        blocked_plugins_json.as_deref(),
        payload.is_disabled.unwrap_or(false),
        shell_opt,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update user RBAC: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": updated })))
}

pub async fn handle_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<User>>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;

    if claims.is_pam {
        let _ = state.auth.sync_pam_user_to_db(&claims.sub, &claims.role, &claims.home_dir);
    }
    state.auth.list_users().map(Json).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list users: {}", e))
    })
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: String,
    pub home_dir: Option<String>,
    pub allowed_roots: Option<Vec<String>>,
    #[serde(default)]
    pub shell: Option<String>,
    /// Must choose a new password at first sign-in (on unless switched off, #205)
    #[serde(default)]
    pub must_change_password: Option<bool>,
}

async fn handle_create_user_inner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<User>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    crate::auth::check_password_policy(&payload.password, &payload.username, None, state.config.auth.min_password_length)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e}.")))?;
    let must_change = payload.must_change_password.unwrap_or(true);
    let new_name = payload.username.clone();

    let final_home = payload.home_dir.unwrap_or_else(|| {
        state.config.storage.default_user_home_template.replace("{username}", &payload.username)
    });

    // Only a full path on this system becomes a folder here
    if crate::server::middleware::is_local_absolute_path(&final_home) {
        let _ = std::fs::create_dir_all(&final_home);
    }

    let roots_json = payload.allowed_roots.map(|r| {
        let clean: Vec<String> = r.into_iter().filter(|s| !s.trim().is_empty()).collect();
        serde_json::to_string(&clean).unwrap_or_else(|_| "[\"*\"]".to_string())
    });

    let auth = state.auth.clone();
    let u = payload.username;
    let p = payload.password;
    let r = payload.role;
    let r_json = roots_json;
    let fh = final_home;
    let shell_opt = payload.shell.filter(|s| !s.trim().is_empty());
    let created = tokio::task::spawn_blocking(move || -> Result<User, Box<dyn std::error::Error + Send + Sync>> {
        let mut user = auth.create_user(&u, &p, &r, &fh, r_json.as_deref())?;
        if let Some(sh) = shell_opt {
            let _ = auth.update_user_rbac(
                &u,
                &r,
                "[\"*\"]",
                r_json.as_deref(),
                Some(&fh),
                None,
                None,
                None,
                false,
                Some(Some(&sh)),
            );
            user.shell = Some(sh);
        }
        Ok(user)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("User creation task failed: {}", e)))?
    .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create user: {}", e)))?;
    if must_change {
        state.auth.set_must_change_password(&new_name, true).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok(Json(created))
}

async fn handle_delete_user_inner(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    if username.eq_ignore_ascii_case(&claims.sub) {
        return Err((StatusCode::BAD_REQUEST, "You cannot delete your own account. Another admin can.".into()));
    }

    state.auth.delete_user(&username).map(|deleted| {
        Json(serde_json::json!({ "success": deleted }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete user: {}", e)))
}

#[derive(Deserialize)]
pub struct UnlockRequest {
    pub password: String,
    pub username: Option<String>,
}

pub async fn handle_unlock_session(
    State(state): State<AppState>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    Json(payload): Json<UnlockRequest>,
) -> Result<Response, (StatusCode, String)> {
    let peer_addr = connect_info.map(|ci| ci.0);
    let client_ip = resolve_client_ip(&headers, peer_addr, &state.config.server.trusted_proxies);
    let rate_key = format!("unlock:{}", client_ip);
    if let Err(retry_after) = state.rate_limiter.check_and_record(&rate_key, 10, 60) {
        tracing::error!(client_ip = %client_ip, "Rate limit exceeded on session unlock endpoint: repeated unlock attempts");
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many unlock attempts. Please retry after {} seconds.", retry_after),
        ));
    }

    // Standalone / Auth Disabled mode
    if !state.config.server.enable_auth || state.config.server.standalone {
        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        let user = if let Ok(Some(mut u)) = state.auth.get_user_by_username(&current_user) {
            u.resolve_avatar();
            u
        } else {
            let avatar_url = crate::auth::resolve_system_avatar(&current_user, &home_dir);
            User {
                id: 1,
                username: current_user.clone(),
                nickname: Some(current_user),
                full_name: None,
                bio: None,
                email: None,
                avatar_url,
                role: "admin".to_string(),
                home_dir,
                is_pam: false,
                is_disabled: false,
                allowed_services: "[\"*\"]".to_string(),
                allowed_roots: "[\"*\"]".to_string(),
                can_install_plugins: true,
                allowed_plugins: "[\"*\"]".to_string(),
                blocked_plugins: "[]".to_string(),
                auth_source: None,
                shell: None,
            }
        };

        state.rate_limiter.reset(&rate_key);
        let new_token = state.auth.generate_token(&user).unwrap_or_default();
        return Ok(Json(serde_json::json!({
            "success": true,
            "message": "Session unlocked (standalone)",
            "token": new_token,
            "user": user
        })).into_response());
    }

    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let mut username = payload.username.clone();

    // The login being unlocked: an explicit header, else this Brum's cookie, else an older cookie
    let token_opt = auth_header
        .and_then(|h| h.strip_prefix("Bearer ").or_else(|| Some(h)))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "undefined" && *s != "null")
        .map(|s| s.to_string())
        .into_iter()
        .chain(session_cookie::tokens_from_cookies(&headers))
        .find(|t| state.auth.verify_token_allow_expired(t).is_ok());

    // If authorization token is present, extract claims (allowing expired token during lock)
    if let Some(ref token_str) = token_opt {
        if let Ok(claims) = state.auth.verify_token_allow_expired(token_str) {
            username = Some(claims.sub);
        }
    }

    let uname = match username {
        Some(u) if !u.trim().is_empty() => u,
        _ => {
            return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()));
        }
    };

    if let Err(msg) = account_guard_try(&state, &uname) {
        return Err((StatusCode::TOO_MANY_REQUESTS, msg));
    }

    tracing::info!("Processing session unlock request for user '{}'...", uname);
    let auth = state.auth.clone();
    let u = uname.clone();
    let p = payload.password;
    let auth_res = tokio::task::spawn_blocking(move || {
        auth.authenticate(&u, &p)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Authentication task failed: {}", e)))?;

    match auth_res {
        Ok(user) => {
            state.rate_limiter.reset(&rate_key);
            state.rate_limiter.reset(&account_guard_key(&uname));
            tracing::info!("Session unlocked successfully for user '{}'", user.username);
            let new_token = state.auth.generate_token(&user).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let max_age_secs = state.config.server.session_duration_hours * 3600;
            let secure = is_request_secure_peer(&headers, peer_addr, &state.config.server.trusted_proxies);
            let mut body = serde_json::json!({
                "success": true,
                "message": "Session unlocked",
                "user": user
            });
            if !session_cookie::wants_cookie_only(&headers) {
                body["token"] = serde_json::Value::String(new_token.clone());
            }
            let mut resp = Json(body).into_response();
            session_cookie::attach_login_cookie(resp.headers_mut(), &headers, &new_token, max_age_secs, secure);
            Ok(resp)
        }
        Err(e) => {
            tracing::warn!("Failed session unlock attempt for user '{}': {}", uname, e);
            Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()))
        }
    }
}

pub async fn handle_get_user_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let prefs = state.auth.get_user_preferences(&claims.sub)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load user preferences: {}", e)))?;

    if let Some(p_str) = prefs {
        let val: serde_json::Value = serde_json::from_str(&p_str)
            .unwrap_or_else(|_| serde_json::json!({}));
        Ok(Json(val))
    } else {
        Ok(Json(serde_json::json!({})))
    }
}

pub async fn handle_save_user_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let p_str = serde_json::to_string(&payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid preferences JSON: {}", e)))?;

    state.auth.save_user_preferences(&claims.sub, &p_str)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save user preferences: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true })))
}

/// Saves only the settings that changed (#200), so a browser with older copies of other
/// settings does not put those back.
pub async fn handle_patch_user_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let changes = payload
        .as_object()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Expected an object of changed settings".to_string()))?;
    state.auth.merge_user_preferences(&claims.sub, changes)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save user preferences: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn handle_reset_user_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    state.auth.reset_user_preferences(&claims.sub)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to reset user preferences: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true, "message": "User preferences reset to default" })))
}

pub async fn handle_get_security_settings(
    State(state): State<AppState>,
) -> Result<Json<crate::auth::SecuritySettings>, (StatusCode, String)> {
    let settings = state.auth.get_security_settings()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get security settings: {}", e)))?;
    Ok(Json(settings))
}

pub async fn handle_update_security_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::auth::SecuritySettings>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        if claims.role != "admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can update security settings".to_string()));
        }

        state.auth.update_security_settings(&payload)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update security settings: {}", e)))?;

        Ok(Json(serde_json::json!({ "success": true, "message": "Security settings saved" })))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

#[derive(Deserialize)]
pub struct CreateApiTokenRequest {
    pub name: String,
    pub role: Option<String>,
    pub expires_in_days: Option<i64>,
    pub allowed_roots: Option<Vec<String>>,
}

pub async fn handle_list_api_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::auth::ApiTokenInfo>>, (StatusCode, String)> {
    if !state.config.server.standalone && state.config.server.enable_auth {
        let auth_header = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

        let token_str = auth_header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "Invalid authorization scheme".to_string()))?;

        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

        let username_filter = if claims.role == "admin" {
            None
        } else {
            Some(claims.sub.as_str())
        };

        let tokens = state.auth.list_api_tokens(username_filter).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(Json(tokens))
    } else {
        let tokens = state.auth.list_api_tokens(None).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(Json(tokens))
    }
}

async fn handle_create_api_token_inner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateApiTokenRequest>,
) -> Result<Json<crate::auth::GeneratedApiToken>, (StatusCode, String)> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Token name is required".to_string()));
    }

    let (username, default_role) = if !state.config.server.standalone && state.config.server.enable_auth {
        let auth_header = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

        let token_str = auth_header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "Invalid authorization scheme".to_string()))?;

        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        (claims.sub, claims.role)
    } else {
        ("admin".to_string(), "admin".to_string())
    };

    let role = payload.role.unwrap_or(default_role);
    let allowed_roots = payload.allowed_roots.map(|r| serde_json::to_string(&r).unwrap_or_else(|_| "[\"*\"]".to_string()));

    let res = state.auth.create_api_token(
        &username,
        name,
        &role,
        allowed_roots,
        payload.expires_in_days,
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(res))
}

async fn handle_revoke_api_token_inner(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let is_admin_or_owner = if !state.config.server.standalone && state.config.server.enable_auth {
        let auth_header = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

        let token_str = auth_header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "Invalid authorization scheme".to_string()))?;

        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        if claims.role == "admin" {
            None
        } else {
            Some(claims.sub)
        }
    } else {
        None
    };

    let revoked = state.auth.revoke_api_token(&token_id, is_admin_or_owner.as_deref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if revoked {
        Ok(Json(serde_json::json!({ "success": true, "message": "API token revoked" })))
    } else {
        Err((StatusCode::NOT_FOUND, "Token not found or unauthorized".to_string()))
    }
}

// ---------- Activity log (#193) ----------
use crate::server::activity_handlers::{actor, log_result};

fn roots_text(roots: &Option<Vec<String>>) -> String {
    match roots {
        Some(r) if !r.is_empty() => format!(", folders: {}", r.join(", ")),
        _ => String::new(),
    }
}

pub async fn handle_update_user_rbac(State(state): State<AppState>, headers: HeaderMap, AxumPath(username): AxumPath<String>, Json(payload): Json<UpdateUserRbacRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let who = actor(&state, &headers);
    let detail = format!("role {}{}", payload.role, roots_text(&payload.allowed_roots));
    let r = handle_update_user_rbac_inner(State(state), headers, AxumPath(username.clone()), Json(payload)).await;
    log_result(who, "account changed", &username, detail, &r);
    r
}

pub async fn handle_create_user(State(state): State<AppState>, headers: HeaderMap, Json(payload): Json<CreateUserRequest>) -> Result<Json<User>, (StatusCode, String)> {
    let who = actor(&state, &headers);
    let (name, detail) = (payload.username.clone(), format!("role {}{}", payload.role, roots_text(&payload.allowed_roots)));
    let r = handle_create_user_inner(State(state), headers, Json(payload)).await;
    log_result(who, "account created", &name, detail, &r);
    r
}

pub async fn handle_delete_user(State(state): State<AppState>, headers: HeaderMap, AxumPath(username): AxumPath<String>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let who = actor(&state, &headers);
    let r = handle_delete_user_inner(State(state), headers, AxumPath(username.clone())).await;
    log_result(who, "account deleted", &username, String::new(), &r);
    r
}

pub async fn handle_create_api_token(State(state): State<AppState>, headers: HeaderMap, Json(payload): Json<CreateApiTokenRequest>) -> Result<Json<crate::auth::GeneratedApiToken>, (StatusCode, String)> {
    let who = actor(&state, &headers);
    let detail = format!(
        "role {}{}{}",
        payload.role.as_deref().unwrap_or("same as owner"),
        payload.expires_in_days.map(|d| format!(", expires in {d} days")).unwrap_or_default(),
        roots_text(&payload.allowed_roots)
    );
    let name = payload.name.clone();
    let r = handle_create_api_token_inner(State(state), headers, Json(payload)).await;
    log_result(who, "API token created", &name, detail, &r);
    r
}

pub async fn handle_revoke_api_token(State(state): State<AppState>, headers: HeaderMap, AxumPath(token_id): AxumPath<String>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let who = actor(&state, &headers);
    let r = handle_revoke_api_token_inner(State(state), headers, AxumPath(token_id.clone())).await;
    log_result(who, "API token revoked", &token_id, String::new(), &r);
    r
}

// ---------------------------------------------------------------------------------------------
// Passwords of Brum accounts (#205): choose your own (with the current one), admin reset, rules.

/// Why this account's password cannot be changed in Brum, if it cannot
fn managed_elsewhere_reason(user: &User) -> Option<&'static str> {
    if !crate::auth::password_managed_elsewhere(user) {
        return None;
    }
    Some(match user.auth_source.as_deref() {
        Some("windows") => "This is a Windows account: change its password in Windows.",
        Some("oidc") => "This account signs in with single sign-on: change its password there.",
        _ => "This is an account of this computer: change its password on the computer (for example with passwd).",
    })
}

#[derive(Deserialize)]
pub struct ChangeOwnPasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn handle_change_own_password(
    State(state): State<AppState>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    Json(payload): Json<ChangeOwnPasswordRequest>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let who = actor(&state, &headers);
    let r = handle_change_own_password_inner(State(state), connect_info, headers, Json(payload)).await;
    let target = who.clone().unwrap_or_default();
    log_result(who, "password changed", &target, String::new(), &r);
    r
}

async fn handle_change_own_password_inner(
    State(state): State<AppState>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    Json(payload): Json<ChangeOwnPasswordRequest>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !state.config.server.enable_auth || state.config.server.standalone {
        return Err((StatusCode::BAD_REQUEST, "Sign-in is switched off on this machine, so there is no password to change.".into()));
    }
    let claims = extract_claims_or_local(&state, &headers)?;
    if claims.token_id.is_some() {
        return Err((StatusCode::FORBIDDEN, "An API token cannot change the account's password.".into()));
    }
    let user = state.auth.get_user_by_username(&claims.sub)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "This account has no password on this machine.".to_string()))?;
    if let Some(why) = managed_elsewhere_reason(&user) {
        return Err((StatusCode::BAD_REQUEST, why.into()));
    }
    // A wrong current password counts like a wrong password at sign-in
    account_guard_try(&state, &user.username).map_err(|m| (StatusCode::TOO_MANY_REQUESTS, m))?;
    let (auth, name, current) = (state.auth.clone(), user.username.clone(), payload.current_password.clone());
    let current_ok = tokio::task::spawn_blocking(move || auth.verify_own_password(&name, &current))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !current_ok {
        return Err((StatusCode::UNAUTHORIZED, "The current password is not right.".into()));
    }
    crate::auth::check_password_policy(&payload.new_password, &user.username, Some(&payload.current_password), state.config.auth.min_password_length)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e}.")))?;

    let (auth, name, new_pass) = (state.auth.clone(), user.username.clone(), payload.new_password.clone());
    tokio::task::spawn_blocking(move || auth.update_user_password(&name, &new_pass))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update password: {e}")))?;
    state.auth.set_must_change_password(&user.username, false).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state.rate_limiter.reset(&account_guard_key(&user.username));
    // The first-login note goes once the built-in admin has a password of its own
    if user.username.eq_ignore_ascii_case(&state.config.auth.default_admin_user) {
        crate::logging::cleanup_initial_admin_credentials(state.auth.db_path());
    }

    // The new password ended every login, so this browser gets a fresh one
    let token = state.auth.generate_token(&user).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create token: {e}")))?;
    let peer_addr = connect_info.map(|ci| ci.0);
    let secure = is_request_secure_peer(&headers, peer_addr, &state.config.server.trusted_proxies);
    let mut body = serde_json::json!({ "success": true, "message": "Password changed." });
    if !session_cookie::wants_cookie_only(&headers) {
        body["token"] = serde_json::Value::String(token.clone());
    }
    let mut resp = Json(body).into_response();
    session_cookie::attach_login_cookie(resp.headers_mut(), &headers, &token, state.config.server.session_duration_hours * 3600, secure);
    Ok(resp)
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    /// "generate" (a random password, shown once) or "set" (the one in `password`)
    pub mode: String,
    #[serde(default)]
    pub password: Option<String>,
    /// The account must choose a new password at its next sign-in (on unless switched off)
    #[serde(default)]
    pub must_change: Option<bool>,
}

pub async fn handle_reset_user_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
    Json(payload): Json<ResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let who = actor(&state, &headers);
    let detail = format!(
        "{}{}",
        if payload.mode == "generate" { "random password" } else { "password set by admin" },
        if payload.must_change.unwrap_or(true) { ", must choose a new one at next sign-in" } else { "" }
    );
    let r = handle_reset_user_password_inner(State(state), headers, AxumPath(username.clone()), Json(payload)).await;
    log_result(who, "password reset", &username, detail, &r);
    r
}

async fn handle_reset_user_password_inner(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
    Json(payload): Json<ResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    let user = state.auth.get_user_by_username(&username)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "No such account.".to_string()))?;
    if let Some(why) = managed_elsewhere_reason(&user) {
        return Err((StatusCode::BAD_REQUEST, why.into()));
    }
    let (password, generated) = match payload.mode.as_str() {
        "generate" => (crate::auth::generate_admin_password(), true),
        "set" => {
            let p = payload.password.unwrap_or_default();
            crate::auth::check_password_policy(&p, &user.username, None, state.config.auth.min_password_length)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e}.")))?;
            (p, false)
        }
        _ => return Err((StatusCode::BAD_REQUEST, "Choose to generate a password or to set one.".into())),
    };
    let (auth, name, pass) = (state.auth.clone(), user.username.clone(), password.clone());
    tokio::task::spawn_blocking(move || auth.update_user_password(&name, &pass))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update password: {e}")))?;
    state.auth.set_must_change_password(&user.username, payload.must_change.unwrap_or(true))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state.rate_limiter.reset(&account_guard_key(&user.username));
    let mut body = serde_json::json!({ "success": true, "message": "Password reset. All of this user's current logins have ended." });
    if generated {
        body["password"] = serde_json::Value::String(password);
    }
    Ok(Json(body))
}
