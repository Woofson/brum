use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use crate::auth::User;
use crate::server::AppState;
use crate::server::middleware::{extract_claims_or_local, extract_client_ip, require_admin};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: User,
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

#[derive(Deserialize)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

pub async fn handle_oidc_callback(
    State(state): State<AppState>,
    Query(query): Query<OidcCallbackQuery>,
) -> impl IntoResponse {
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
            let cookie_header = format!(
                "cd_token={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000",
                token
            );
            let mut response = axum::response::Redirect::temporary(&format!("/?token={}&sso_success=1", token)).into_response();
            if let Ok(hv) = axum::http::HeaderValue::from_str(&cookie_header) {
                response.headers_mut().insert(axum::http::header::SET_COOKIE, hv);
            }
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

pub async fn handle_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    let client_ip = extract_client_ip(&headers);
    let rate_key = format!("login:{}", client_ip);
    if let Err(retry_after) = state.rate_limiter.check_and_record(&rate_key, 10, 60) {
        tracing::warn!(client_ip = %client_ip, "Rate limit exceeded on login endpoint");
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many login attempts. Please retry after {} seconds.", retry_after),
        ));
    }

    tracing::info!("Received login request for user '{}'...", payload.username);

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
            }
        };

        state.rate_limiter.reset(&rate_key);
        let token = state.auth.generate_token(&user).unwrap_or_default();
        return Ok(Json(LoginResponse { token, user }));
    }

    match state.auth.authenticate(&payload.username, &payload.password) {
        Ok(user) => {
            state.rate_limiter.reset(&rate_key);
            tracing::info!(username = %user.username, is_pam = user.is_pam, role = %user.role, "User successfully authenticated");
            let token = state.auth.generate_token(&user).map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create token: {}", e))
            })?;
            Ok(Json(LoginResponse { token, user }))
        }
        Err(e) => {
            tracing::warn!(username = %payload.username, error = %e, "Authentication failed");
            Err((StatusCode::UNAUTHORIZED, "Invalid username or password".to_string()))
        }
    }
}

pub async fn handle_logout() -> impl IntoResponse {
    let mut resp = Json(serde_json::json!({ "success": true, "message": "Logged out" })).into_response();
    let clear_cookie = "cd_token=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT";
    if let Ok(hv) = axum::http::HeaderValue::from_str(clear_cookie) {
        resp.headers_mut().insert(axum::http::header::SET_COOKIE, hv);
    }
    resp
}

pub async fn handle_get_me(
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
        }));
    }

    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        match state.auth.verify_token(token_str) {
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
                }))
            }
            Err(_) => Err((StatusCode::UNAUTHORIZED, "Invalid token".to_string())),
        }
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))
    }
}

pub fn get_avatars_dir() -> std::path::PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("brum").join("avatars")
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
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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

    if let Some(ref new_pass) = payload.new_password {
        if !new_pass.trim().is_empty() {
            let _ = state.auth.update_user_password(&username, new_pass)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update password: {}", e)))?;
        }
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Profile updated successfully",
        "avatar_url": effective_avatar_url
    })))
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
    pub can_install_plugins: Option<bool>,
    #[serde(default)]
    pub allowed_plugins: Option<Vec<String>>,
    #[serde(default)]
    pub blocked_plugins: Option<Vec<String>>,
    #[serde(default)]
    pub is_disabled: Option<bool>,
}

pub async fn handle_update_user_rbac(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
    Json(payload): Json<UpdateUserRbacRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;

    let services_json = if payload.allowed_services.is_empty() {
        "[\"*\"]".to_string()
    } else {
        serde_json::to_string(&payload.allowed_services).unwrap_or_else(|_| "[\"*\"]".to_string())
    };
    let roots_json = payload.allowed_roots.map(|r| serde_json::to_string(&r).unwrap_or_else(|_| "[\"*\"]".to_string()));
    let allowed_plugins_json = payload.allowed_plugins.map(|p| serde_json::to_string(&p).unwrap_or_else(|_| "[\"*\"]".to_string()));
    let blocked_plugins_json = payload.blocked_plugins.map(|p| serde_json::to_string(&p).unwrap_or_else(|_| "[]".to_string()));

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
}

pub async fn handle_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<User>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;

    let final_home = payload.home_dir.unwrap_or_else(|| {
        state.config.storage.default_user_home_template.replace("{username}", &payload.username)
    });

    let _ = std::fs::create_dir_all(&final_home);

    let roots_json = payload.allowed_roots.map(|r| serde_json::to_string(&r).unwrap_or_else(|_| "[\"*\"]".to_string()));

    state
        .auth
        .create_user(&payload.username, &payload.password, &payload.role, &final_home, roots_json.as_deref())
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create user: {}", e)))
}

pub async fn handle_delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;

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
    headers: HeaderMap,
    Json(payload): Json<UnlockRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let client_ip = extract_client_ip(&headers);
    let rate_key = format!("unlock:{}", client_ip);
    if let Err(retry_after) = state.rate_limiter.check_and_record(&rate_key, 10, 60) {
        tracing::warn!(client_ip = %client_ip, "Rate limit exceeded on session unlock endpoint");
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
            }
        };

        state.rate_limiter.reset(&rate_key);
        let new_token = state.auth.generate_token(&user).unwrap_or_default();
        return Ok(Json(serde_json::json!({
            "success": true,
            "message": "Session unlocked (standalone)",
            "token": new_token,
            "user": user
        })));
    }

    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let mut username = payload.username.clone();

    // If authorization token is present, extract claims (allowing expired token during lock)
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
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

    tracing::info!("Processing session unlock request for user '{}'...", uname);
    match state.auth.authenticate(&uname, &payload.password) {
        Ok(user) => {
            state.rate_limiter.reset(&rate_key);
            tracing::info!("Session unlocked successfully for user '{}'", user.username);
            let new_token = state.auth.generate_token(&user).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(Json(serde_json::json!({
                "success": true,
                "message": "Session unlocked",
                "token": new_token,
                "user": user
            })))
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

pub async fn handle_create_api_token(
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

pub async fn handle_revoke_api_token(
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
