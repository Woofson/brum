use std::path::{Path, PathBuf};
use std::fs;
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use crate::server::{AppState, Asset, ManualsAsset};
use crate::auth::GlobalMount;
use crate::config::{AppConfig, ConfigManager};
use crate::server::middleware::{
    extract_claims_or_local, require_admin,
    validate_path_access, resolve_effective_home, is_root_path,
};

pub fn get_system_hostname() -> String {
    if let Ok(h) = std::env::var("CD_HOSTNAME") {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(h) = std::env::var("HOSTNAME") {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        let res = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
        if res == 0 {
            if let Ok(s) = std::ffi::CStr::from_bytes_until_nul(&buf) {
                if let Ok(str_slice) = s.to_str() {
                    let trimmed = str_slice.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
        }
    }
    #[cfg(windows)]
    {
        if let Ok(h) = std::env::var("COMPUTERNAME") {
            let trimmed = h.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "localhost".to_string()
}

#[derive(Serialize)]
pub struct SystemStatusResponse {
    pub version: String,
    pub build_number: String,
    pub build_commit: String,
    pub build_timestamp: String,
    pub build_target: String,
    pub semver_full: String,
    pub standalone: bool,
    pub auth_enabled: bool,
    pub current_user: String,
    pub home_dir: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub auth_engine: String,
    pub custom_hostname: Option<String>,
    pub show_hostname_badge: bool,
    pub hostname_color: Option<String>,
    pub hostname_style: Option<String>,
    pub hostname_icon: Option<String>,
    pub hostname_size: Option<String>,
    pub window_title: Option<String>,
    pub login_title: Option<String>,
    pub login_subtitle_template: Option<String>,
}

pub async fn handle_health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let hostname = get_system_hostname();
    let node_name = if !state.config.ui.hostname_badge.trim().is_empty() {
        state.config.ui.hostname_badge.clone()
    } else if !state.config.server.server_name.trim().is_empty() {
        state.config.server.server_name.clone()
    } else {
        hostname.clone()
    };

    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "build_number": env!("BRUM_BUILD_NUMBER"),
        "build_commit": env!("BRUM_BUILD_COMMIT"),
        "build_timestamp": env!("BRUM_BUILD_TIMESTAMP"),
        "build_target": env!("BRUM_BUILD_TARGET"),
        "semver_full": env!("BRUM_SEMVER_FULL"),
        "hostname": hostname,
        "node_name": node_name,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "standalone": state.config.server.standalone,
        "auth_enabled": state.config.server.enable_auth && !state.config.server.standalone,
        "time": chrono::Utc::now().to_rfc3339()
    }))
}

pub async fn handle_system_status(State(state): State<AppState>) -> Json<SystemStatusResponse> {
    let current_user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".to_string());
    let home_dir = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());
    let hostname = get_system_hostname();
    let custom_hostname = if !state.config.ui.hostname_badge.trim().is_empty() {
        Some(state.config.ui.hostname_badge.clone())
    } else if !state.config.server.server_name.trim().is_empty() {
        Some(state.config.server.server_name.clone())
    } else {
        None
    };

    let window_title = if !state.config.ui.window_title.trim().is_empty() {
        Some(state.config.ui.window_title.clone())
    } else {
        None
    };

    Json(SystemStatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_number: env!("BRUM_BUILD_NUMBER").to_string(),
        build_commit: env!("BRUM_BUILD_COMMIT").to_string(),
        build_timestamp: env!("BRUM_BUILD_TIMESTAMP").to_string(),
        build_target: env!("BRUM_BUILD_TARGET").to_string(),
        semver_full: env!("BRUM_SEMVER_FULL").to_string(),
        standalone: state.config.server.standalone,
        auth_enabled: state.config.server.enable_auth && !state.config.server.standalone,
        current_user,
        home_dir,
        hostname,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        auth_engine: if cfg!(windows) {
            "Windows SAM / AD + SQLite RBAC".to_string()
        } else if cfg!(target_os = "macos") {
            "macOS PAM + SQLite RBAC".to_string()
        } else {
            "Linux PAM + SQLite RBAC".to_string()
        },
        custom_hostname,
        show_hostname_badge: state.config.ui.show_hostname_badge,
        hostname_color: if !state.config.ui.hostname_color.trim().is_empty() { Some(state.config.ui.hostname_color.clone()) } else { None },
        hostname_style: if !state.config.ui.hostname_style.trim().is_empty() { Some(state.config.ui.hostname_style.clone()) } else { None },
        hostname_icon: if !state.config.ui.hostname_icon.trim().is_empty() { Some(state.config.ui.hostname_icon.clone()) } else { None },
        hostname_size: if !state.config.ui.hostname_size.trim().is_empty() { Some(state.config.ui.hostname_size.clone()) } else { None },
        window_title,
        login_title: state.config.ui.login_title.clone(),
        login_subtitle_template: state.config.ui.login_subtitle_template.clone(),
    })
}



pub async fn handle_get_storage_roots(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::config::StorageRoot>>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let user_role = claims.role.as_str();
    let effective_home_opt = resolve_effective_home(&claims, &state);
    let allowed_roots_json = claims.allowed_roots.as_deref().unwrap_or("[\"*\"]");
    let allowed_roots: Vec<String> = serde_json::from_str(allowed_roots_json).unwrap_or_else(|_| vec!["*".to_string()]);

    let mut accessible = Vec::new();
    let is_admin = user_role.eq_ignore_ascii_case("admin");
    let is_readonly = user_role.eq_ignore_ascii_case("readonly");

    // 1. Personal Home Directory (if resolved and not roots_only)
    if let Some(ref effective_home) = effective_home_opt {
        if state.config.storage.allow_entire_system || !is_root_path(Path::new(effective_home)) {
            accessible.push(crate::config::StorageRoot {
                id: "home".to_string(),
                name: "Home".to_string(),
                path: effective_home.clone(),
                read_only: is_readonly,
                allowed_roles: vec![],
            });
        }
    }

    // 2. Configured Storage Roots
    for root in &state.config.storage.roots {
        let role_ok = root.allowed_roles.is_empty() || root.allowed_roles.iter().any(|r| r.eq_ignore_ascii_case(user_role));
        let user_ok = is_admin || allowed_roots.contains(&"*".to_string()) || allowed_roots.contains(&root.id) || allowed_roots.contains(&root.path);

        if role_ok && user_ok {
            let mut r = root.clone();
            if is_readonly {
                r.read_only = true;
            }
            if !accessible.iter().any(|existing| existing.path == r.path) {
                accessible.push(r);
            }
        }
    }

    // 3. System Root (if allowed in config, AND user has permission)
    let allow_system = state.config.storage.allow_entire_system
        && (is_admin || allowed_roots.contains(&"*".to_string()) || allowed_roots.contains(&"/".to_string()));
    if allow_system {
        #[cfg(windows)]
        {
            for b in b'A'..=b'Z' {
                let drive = format!("{}:\\", b as char);
                if std::path::Path::new(&drive).exists() {
                    let drive_lower = (b as char).to_ascii_lowercase();
                    let drive_id = format!("drive-{}", drive_lower);
                    if !accessible.iter().any(|existing| existing.path.eq_ignore_ascii_case(&drive)) {
                        accessible.push(crate::config::StorageRoot {
                            id: drive_id,
                            name: format!("Local Disk ({}:)", b as char),
                            path: drive,
                            read_only: false,
                            allowed_roles: vec!["admin".to_string()],
                        });
                    }
                }
            }
        }
        #[cfg(not(windows))]
        {
            if !accessible.iter().any(|existing| existing.path == "/") {
                accessible.push(crate::config::StorageRoot {
                    id: "system-root".to_string(),
                    name: "Root Filesystem (/)".to_string(),
                    path: "/".to_string(),
                    read_only: false,
                    allowed_roles: vec!["admin".to_string()],
                });
            }
        }
    }

    // 4. Mounted Removable / USB Storage
    for usb in crate::tools::usb::list_usb_devices() {
        for part in usb.partitions {
            if let Some(ref mnt) = part.mount_point {
                if Path::new(mnt).exists() && !accessible.iter().any(|existing| existing.path == *mnt) {
                    let display_name = part.label.clone()
                        .filter(|l| !l.trim().is_empty())
                        .or_else(|| usb.model.clone())
                        .unwrap_or_else(|| format!("USB Storage ({})", part.name));
                    accessible.push(crate::config::StorageRoot {
                        id: format!("usb-{}", part.name),
                        name: display_name,
                        path: mnt.clone(),
                        read_only: part.is_read_only,
                        allowed_roles: vec![],
                    });
                }
            }
        }
    }

    Ok(Json(accessible))
}



pub async fn handle_system_exit(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.config.server.standalone {
        let claims = extract_claims_or_local(&state, &headers)?;
        if claims.role != "admin" && claims.role != "Admin" {
            return Err((
                StatusCode::FORBIDDEN,
                "Only administrators or standalone desktop sessions can terminate the process".to_string(),
            ));
        }
    }

    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        std::process::exit(0);
    });

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Brum is exiting cleanly"
    })))
}



// ---------------- GLOBAL NETWORK MOUNTS & RBAC HANDLERS ----------------

#[derive(Deserialize)]
pub struct CreateMountRequest {
    name: String,
    protocol: String,
    target_uri: String,
    options_json: Option<String>,
    allowed_users: Option<Vec<String>>,
}

pub async fn handle_list_accessible_mounts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<GlobalMount>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let is_admin = claims.role == "admin";
        let mounts = state.auth.list_accessible_mounts(&claims.sub, is_admin)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list accessible mounts: {}", e)))?;
        Ok(Json(mounts))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

pub async fn handle_list_all_mounts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<GlobalMount>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        if claims.role != "admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can view all mounts".to_string()));
        }
        let mounts = state.auth.list_all_mounts()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list all mounts: {}", e)))?;
        Ok(Json(mounts))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

pub async fn handle_create_or_update_mount(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateMountRequest>,
) -> Result<Json<GlobalMount>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        if claims.role != "admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can create or configure global mounts".to_string()));
        }

        let allowed_users_str = if let Some(users) = payload.allowed_users {
            serde_json::to_string(&users).unwrap_or_else(|_| "[\"*\"]".to_string())
        } else {
            "[\"*\"]".to_string()
        };

        let options_str = payload.options_json.unwrap_or_else(|| "{}".to_string());

        let mount = state.auth.create_or_update_mount(
            &payload.name,
            &payload.protocol,
            &payload.target_uri,
            &options_str,
            &allowed_users_str,
        ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save global mount: {}", e)))?;

        Ok(Json(mount))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

pub async fn handle_delete_mount(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        if claims.role != "admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can delete global mounts".to_string()));
        }

        state.auth.delete_mount(id)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete mount: {}", e)))?;

        Ok(Json(serde_json::json!({ "success": true, "message": "Mount removed" })))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

#[derive(Deserialize)]
pub struct CreateBookmarkRequest {
    name: String,
    protocol: String,
    path: String,
    password: Option<String>,
}





// ---------------- TRANSPARENT ENCRYPTED VAULT HANDLERS ----------------

#[derive(Deserialize)]
pub struct CreateVaultRequest {
    path: String,
    password: String,
}

pub async fn handle_create_vault(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateVaultRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, true)?;
    let p = Path::new(&valid_path);
    crate::vfs::vault::VaultManager::create_vault(p, &payload.password)
        .map(|_| Json(serde_json::json!({ "success": true, "path": valid_path })))
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
pub struct UnlockVaultRequest {
    path: String,
    password: String,
    auto_lock_secs: Option<u64>,
}

pub async fn handle_unlock_vault(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UnlockVaultRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, false)?;
    let p = Path::new(&valid_path);
    let auto_lock = payload.auto_lock_secs.unwrap_or(900);
    state.vaults.unlock_vault(p, &payload.password, auto_lock)
        .map(|norm_path| Json(serde_json::json!({ "success": true, "path": norm_path })))
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))
}

#[derive(Deserialize)]
pub struct LockVaultRequest {
    path: String,
}

pub async fn handle_lock_vault(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LockVaultRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, false)?;
    state.vaults.lock_vault(&valid_path);
    Ok(Json(serde_json::json!({ "success": true, "path": valid_path })))
}

#[derive(Deserialize)]
pub struct VaultStatusQuery {
    path: String,
}

pub async fn handle_vault_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<VaultStatusQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &query.path, false)?;
    let is_unlocked = state.vaults.is_unlocked(&valid_path);
    Ok(Json(serde_json::json!({ "unlocked": is_unlocked, "path": valid_path })))
}



pub async fn handle_list_bookmarks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::auth::UserBookmark>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let username = if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        match state.auth.verify_token(token_str) {
            Ok(c) => c.sub,
            Err(_) => "bolt".to_string(),
        }
    } else {
        "bolt".to_string()
    };

    let bookmarks = state.auth.list_bookmarks(&username)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list bookmarks: {}", e)))?;
    Ok(Json(bookmarks))
}

pub async fn handle_create_bookmark(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateBookmarkRequest>,
) -> Result<Json<crate::auth::UserBookmark>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let username = if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        match state.auth.verify_token(token_str) {
            Ok(c) => c.sub,
            Err(_) => "bolt".to_string(),
        }
    } else {
        "bolt".to_string()
    };

    let bm = state.auth.create_bookmark(
        &username,
        &payload.name,
        &payload.protocol,
        &payload.path,
        payload.password.as_deref(),
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save bookmark: {}", e)))?;

    Ok(Json(bm))
}

pub async fn handle_delete_bookmark(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let (username, is_admin) = if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        match state.auth.verify_token(token_str) {
            Ok(c) => (c.sub.clone(), c.role == "admin"),
            Err(_) => ("bolt".to_string(), false),
        }
    } else {
        ("bolt".to_string(), false)
    };

    state.auth.delete_bookmark(id, &username, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete bookmark: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true, "message": "Bookmark removed" })))
}



// ---------------- MANUALS & DOCUMENTATION HANDLERS ----------------

pub async fn handle_list_manuals() -> Json<crate::tools::notedog::NoteDogNotebook> {
    Json(crate::tools::notedog::get_builtin_manuals_notebook())
}

pub async fn handle_get_manual(
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<crate::vfs::FileContentResponse>, (StatusCode, String)> {
    let clean_name = name.trim_start_matches('/').replace("..", "");
    let disk_path = Path::new("manuals").join(&clean_name);
    let content_str = if disk_path.is_file() {
        fs::read_to_string(&disk_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else if let Some(file) = ManualsAsset::get(&clean_name) {
        String::from_utf8_lossy(&file.data).to_string()
    } else {
        return Err((StatusCode::NOT_FOUND, format!("Manual '{}' not found", clean_name)));
    };

    Ok(Json(crate::vfs::FileContentResponse {
        path: format!("manual://{}", clean_name),
        name: clean_name,
        content: content_str.clone(),
        is_binary: false,
        size: content_str.len() as u64,
        mime_type: "text/markdown".to_string(),
    }))
}



// ---------------- CONFIG & SYSTEM HANDLERS ----------------

pub async fn handle_get_config(
    State(state): State<AppState>,
) -> Json<crate::config::ClientConfigDto> {
    Json(state.config.to_client_dto())
}

#[derive(Serialize)]
pub struct SystemUsersGroups {
    users: Vec<String>,
    groups: Vec<String>,
}

pub async fn handle_get_system_users_groups(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SystemUsersGroups>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;

    let mut users = Vec::new();
    let mut groups = Vec::new();

    if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                users.push(parts[0].to_string());
            }
        }
    }

    if let Ok(grp) = fs::read_to_string("/etc/group") {
        for line in grp.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                groups.push(parts[0].to_string());
            }
        }
    }

    users.sort();
    groups.sort();

    Ok(Json(SystemUsersGroups { users, groups }))
}

#[derive(Serialize)]
pub struct ConfigFileResponse {
    path: String,
    content: String,
    is_writable: bool,
}

#[derive(Deserialize)]
pub struct SaveConfigFileRequest {
    content: String,
}

pub fn resolve_active_config_path() -> PathBuf {
    crate::config::ConfigManager::active_config_path()
}

pub async fn handle_get_config_file(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ConfigFileResponse>, (StatusCode, String)> {
    if !state.config.server.standalone {
        let claims = extract_claims_or_local(&state, &headers)?;
        if claims.role != "admin" && claims.role != "Admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can view raw configuration".to_string()));
        }
    }

    let path = resolve_active_config_path();
    let is_writable = match fs::OpenOptions::new().write(true).open(&path) {
        Ok(_) => true,
        Err(_) => false,
    };

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            // Return active config serialized as toml
            toml::to_string_pretty(&*state.config).unwrap_or_default()
        }
    };

    Ok(Json(ConfigFileResponse {
        path: path.to_string_lossy().to_string(),
        content,
        is_writable,
    }))
}

pub async fn handle_save_config_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SaveConfigFileRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.config.server.standalone {
        let claims = extract_claims_or_local(&state, &headers)?;
        if claims.role != "admin" && claims.role != "Admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can edit configuration".to_string()));
        }
    }

    // Validate TOML syntax before saving
    let _: AppConfig = toml::from_str(&payload.content).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("Invalid TOML syntax: {}", e))
    })?;

    let path = resolve_active_config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    fs::write(&path, &payload.content).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write config file {}: {}", path.display(), e))
    })?;

    tracing::info!("Configuration file saved to {}", path.display());

    Ok(Json(serde_json::json!({
        "success": true,
        "path": path.to_string_lossy().to_string(),
        "message": "Configuration saved successfully. Server restart required for some changes."
    })))
}

pub async fn handle_reload_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.config.server.standalone {
        let claims = extract_claims_or_local(&state, &headers)?;
        if claims.role != "admin" && claims.role != "Admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can reload configuration".to_string()));
        }
    }

    let new_cfg = ConfigManager::load_all();
    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Configuration reloaded into memory",
        "config": new_cfg
    })))
}

pub async fn handle_system_restart(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.config.server.standalone {
        let claims = extract_claims_or_local(&state, &headers)?;
        if claims.role != "admin" && claims.role != "Admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can restart the server".to_string()));
        }
    }

    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            if let Ok(exe) = std::env::current_exe() {
                let args: Vec<String> = std::env::args().skip(1).collect();
                let _ = std::process::Command::new(exe).args(args).exec();
            }
        }
        #[cfg(windows)]
        {
            let args: Vec<String> = std::env::args().collect();
            let is_service = args.iter().any(|a| a == "--windows-service" || a == "--service" || a == "service");
            if is_service {
                // When running under Windows SCM, trigger SCM restart via detached cmd.exe so SCM cycles the service
                use std::os::windows::process::CommandExt;
                let _ = std::process::Command::new("cmd.exe")
                    .args(["/c", "timeout /t 1 /nobreak >nul & net stop Brum & net start Brum"])
                    .creation_flags(0x08000000) // CREATE_NO_WINDOW
                    .spawn();
            } else if let Ok(exe) = std::env::current_exe() {
                use std::os::windows::process::CommandExt;
                let pass_args: Vec<String> = std::env::args().skip(1).collect();
                let _ = std::process::Command::new(exe)
                    .args(pass_args)
                    .creation_flags(0x08000000)
                    .spawn();
                std::process::exit(0);
            }
        }
    });

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Brum server is restarting..."
    })))
}



// ---------------- STATIC ASSET EMBEDDED HANDLER ----------------

pub async fn handle_static_asset(headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if uri.path().starts_with("/api/") || path.starts_with("api/") {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"error":"Not Found","code":"NOT_FOUND"}"#))
            .unwrap_or_else(|_| StatusCode::NOT_FOUND.into_response());
    }
    let file_path = if path.is_empty() { "index.html" } else { path };

    let is_font_or_media = file_path.starts_with("assets/fonts/")
        || file_path.starts_with("assets/vendor/")
        || file_path.ends_with(".woff2")
        || file_path.ends_with(".woff")
        || file_path.ends_with(".ttf")
        || file_path.ends_with(".svg")
        || file_path.ends_with(".png")
        || file_path.ends_with(".webp")
        || file_path.ends_with(".ico");

    let cache_control = if is_font_or_media {
        "public, max-age=31536000, immutable"
    } else if file_path == "index.html" {
        "no-cache, no-store, must-revalidate"
    } else {
        "no-cache, must-revalidate"
    };

    // 1. In development / when ./frontend directory exists on disk, prefer live disk reading for instant browser refresh
    let local_file_path = Path::new("frontend").join(file_path);
    if local_file_path.is_file() {
        if let Ok(bytes) = fs::read(&local_file_path) {
            use sha2::{Digest, Sha256};
            let sha = Sha256::digest(&bytes);
            let etag = format!("\"{}\"", hex::encode(sha));
            if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
                if let Ok(val) = if_none_match.to_str() {
                    if val.trim() == etag {
                        return Response::builder()
                            .status(StatusCode::NOT_MODIFIED)
                            .header(header::ETAG, etag)
                            .header(header::CACHE_CONTROL, cache_control)
                            .body(Body::empty())
                            .unwrap_or_else(|_| StatusCode::NOT_MODIFIED.into_response());
                    }
                }
            }

            let mime = mime_guess::from_path(&local_file_path).first_or_octet_stream().to_string();
            let data_len = bytes.len();
            return Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .header(header::CONTENT_LENGTH, data_len.to_string())
                .header(header::ETAG, etag)
                .header(header::CACHE_CONTROL, cache_control)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Asset load error").into_response());
        }
    }

    // 2. Fall back to embedded RustEmbed static assets (for standalone/installed binaries)
    match Asset::get(file_path) {
        Some(content) => {
            let etag = format!("\"{}\"", hex::encode(content.metadata.sha256_hash()));
            if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
                if let Ok(val) = if_none_match.to_str() {
                    if val.trim() == etag {
                        return Response::builder()
                            .status(StatusCode::NOT_MODIFIED)
                            .header(header::ETAG, etag)
                            .header(header::CACHE_CONTROL, cache_control)
                            .body(Body::empty())
                            .unwrap_or_else(|_| StatusCode::NOT_MODIFIED.into_response());
                    }
                }
            }

            let mime = mime_guess::from_path(file_path).first_or_octet_stream().to_string();
            let data_len = content.data.len();
            Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .header(header::CONTENT_LENGTH, data_len.to_string())
                .header(header::ETAG, etag)
                .header(header::CACHE_CONTROL, cache_control)
                .body(Body::from(content.data))
                .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Asset load error").into_response())
        }
        None => {
            let local_index = Path::new("frontend").join("index.html");
            if local_index.is_file() {
                if let Ok(bytes) = fs::read(&local_index) {
                    use sha2::{Digest, Sha256};
                    let sha = Sha256::digest(&bytes);
                    let etag = format!("\"{}\"", hex::encode(sha));
                    if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
                        if let Ok(val) = if_none_match.to_str() {
                            if val.trim() == etag {
                                return Response::builder()
                                    .status(StatusCode::NOT_MODIFIED)
                                    .header(header::ETAG, etag)
                                    .header(header::CACHE_CONTROL, "no-cache")
                                    .body(Body::empty())
                                    .unwrap_or_else(|_| StatusCode::NOT_MODIFIED.into_response());
                            }
                        }
                    }

                    let data_len = bytes.len();
                    return Response::builder()
                        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                        .header(header::CONTENT_LENGTH, data_len.to_string())
                        .header(header::ETAG, etag)
                        .header(header::CACHE_CONTROL, "no-cache")
                        .body(Body::from(bytes))
                        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Index load error").into_response());
                }
            }

            if let Some(index) = Asset::get("index.html") {
                let etag = format!("\"{}\"", hex::encode(index.metadata.sha256_hash()));
                if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
                    if let Ok(val) = if_none_match.to_str() {
                        if val.trim() == etag {
                            return Response::builder()
                                .status(StatusCode::NOT_MODIFIED)
                                .header(header::ETAG, etag)
                                .header(header::CACHE_CONTROL, "no-cache")
                                .body(Body::empty())
                                .unwrap_or_else(|_| StatusCode::NOT_MODIFIED.into_response());
                        }
                    }
                }

                let data_len = index.data.len();
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(header::CONTENT_LENGTH, data_len.to_string())
                    .header(header::ETAG, etag)
                    .header(header::CACHE_CONTROL, "no-cache")
                    .body(Body::from(index.data))
                    .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Index load error").into_response())
            } else {
                (StatusCode::NOT_FOUND, "Resource not found").into_response()
            }
        }
    }
}



// ---------------- SYSTEM AUTOSTART HANDLERS ----------------

#[derive(Serialize, Deserialize)]
pub struct AutostartStatus {
    enabled: bool,
    platform: String,
    target_path: Option<String>,
}

#[derive(Deserialize)]
pub struct SetAutostartRequest {
    enabled: bool,
    minimized: Option<bool>,
}

pub async fn handle_get_autostart(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AutostartStatus>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;

    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("reg")
            .args(["query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "Brum"])
            .output();
        let enabled = output.map_or(false, |o| o.status.success());
        Ok(Json(AutostartStatus {
            enabled,
            platform: "windows".to_string(),
            target_path: std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()),
        }))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let autostart_file = dirs::config_dir()
            .map(|c| c.join("autostart/brum.desktop"));
        let enabled = autostart_file.as_ref().map_or(false, |p| p.exists());
        Ok(Json(AutostartStatus {
            enabled,
            platform: std::env::consts::OS.to_string(),
            target_path: autostart_file.map(|p| p.to_string_lossy().to_string()),
        }))
    }
}

pub async fn handle_set_autostart(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SetAutostartRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;

    let exe_path = std::env::current_exe().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let flags = if payload.minimized.unwrap_or(false) { " --minimized" } else { "" };
    let exec_cmd = format!("\"{}\"{}", exe_path.display(), flags);

    #[cfg(target_os = "windows")]
    {
        if payload.enabled {
            let status = std::process::Command::new("reg")
                .args(["add", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "Brum", "/t", "REG_SZ", "/d", &exec_cmd, "/f"])
                .status()
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            if !status.success() {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to set Windows autostart registry key".to_string()));
            }
        } else {
            let _ = std::process::Command::new("reg")
                .args(["delete", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "Brum", "/f"])
                .status();
            let _ = std::process::Command::new("reg")
                .args(["delete", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "CommanderDog", "/f"])
                .status();
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(config_dir) = dirs::config_dir() {
            let auto_dir = config_dir.join("autostart");
            let desktop_path = auto_dir.join("brum.desktop");
            let legacy_path = auto_dir.join("commanderdog.desktop");
            if payload.enabled {
                let _ = std::fs::create_dir_all(&auto_dir);
                let content = format!(
                    "[Desktop Entry]\nType=Application\nName=Brum\nComment=Multi-Pane Web Environment (File Commander/Manager)\nExec={}\nIcon=brum\nTerminal=false\nCategories=Utility;FileManager;\n",
                    exec_cmd
                );
                std::fs::write(&desktop_path, content).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                if legacy_path.exists() {
                    let _ = std::fs::remove_file(legacy_path);
                }
            } else {
                if desktop_path.exists() {
                    let _ = std::fs::remove_file(&desktop_path);
                }
                if legacy_path.exists() {
                    let _ = std::fs::remove_file(&legacy_path);
                }
            }
        }
    }

    Ok(Json(serde_json::json!({ "success": true, "enabled": payload.enabled })))
}

#[derive(Deserialize)]
pub struct OpenWithRequest {
    file_path: String,
    command: Option<String>,
}

pub async fn handle_open_with(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<OpenWithRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let valid_path = validate_path_access(&state, &headers, &payload.file_path, false)?;
    let local_path = crate::vfs::local::LocalFs::resolve_local_path(&valid_path);

    if !local_path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Target file does not exist: {}", local_path.display())));
    }

    let path_str = local_path.to_string_lossy().to_string();
    let dir_str = if local_path.is_dir() {
        path_str.clone()
    } else {
        local_path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| path_str.clone())
    };

    if let Some(cmd) = payload.command {
        if !cmd.trim().is_empty() {
            require_admin(&claims)?;
            let replaced = cmd
                .replace("%1", &format!("\"{}\"", path_str))
                .replace("{file}", &format!("\"{}\"", path_str))
                .replace("{dir}", &format!("\"{}\"", dir_str));
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("cmd")
                    .args(["/C", &replaced])
                    .current_dir(&dir_str)
                    .spawn()
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to spawn process: {}", e)))?;
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = std::process::Command::new("sh")
                    .args(["-c", &replaced])
                    .current_dir(&dir_str)
                    .spawn()
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to spawn process: {}", e)))?;
            }
            return Ok(Json(serde_json::json!({ "success": true, "command": replaced, "working_dir": dir_str })));
        }
    }

    open::that_detached(&local_path).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open file with default system handler: {}", e))
    })?;

    Ok(Json(serde_json::json!({ "success": true, "path": path_str })))
}

#[derive(Deserialize)]
pub struct RunCustomActionRequest {
    command: String,
    target_path: String,
    selection: Option<Vec<String>>,
    target_pane_path: Option<String>,
}

pub async fn handle_run_custom_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RunCustomActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;

    let valid_path = validate_path_access(&state, &headers, &payload.target_path, false)?;
    let local_path = crate::vfs::local::LocalFs::resolve_local_path(&valid_path);
    let target_str = local_path.to_string_lossy().to_string();
    let dir_str = if local_path.is_dir() {
        target_str.clone()
    } else {
        local_path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| target_str.clone())
    };

    let selection_joined = payload.selection
        .as_ref()
        .map(|list| list.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| format!("\"{}\"", target_str));

    let target_pane_str = payload.target_pane_path.unwrap_or_default();

    let exec_cmd = payload.command
        .replace("{file}", &format!("\"{}\"", target_str))
        .replace("{dir}", &format!("\"{}\"", dir_str))
        .replace("{selection}", &selection_joined)
        .replace("{target_pane}", &format!("\"{}\"", target_pane_str))
        .replace("%1", &format!("\"{}\"", target_str));

    #[cfg(target_os = "windows")]
    let child = std::process::Command::new("cmd")
        .args(["/C", &exec_cmd])
        .current_dir(&dir_str)
        .spawn();

    #[cfg(not(target_os = "windows"))]
    let child = std::process::Command::new("sh")
        .args(["-c", &exec_cmd])
        .current_dir(&dir_str)
        .spawn();

    child.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to run action: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "executed": exec_cmd,
        "working_dir": dir_str
    })))
}

// ============================================================================
// 🧩 ChewToy Plugins Architecture & Add-ons Handlers
// ============================================================================
