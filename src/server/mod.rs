pub mod terminal;

use crate::auth::{AuthManager, GlobalMount, User};
use crate::config::{AppConfig, ConfigManager};
use crate::tools::diff::compare_files_text;
use crate::tools::paranoid::ParanoidEngine;
use crate::tools::tasks::{TaskInfo, TaskManager};
use crate::vfs::archive::ArchiveHandler;
use crate::vfs::checksum::calculate_checksum;
use crate::vfs::local::LocalFs;
use crate::vfs::sftp::SftpClient;
use crate::vfs::webdav::WebDavClient;
use crate::vfs::DirectoryListing;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(RustEmbed)]
#[folder = "frontend/"]
struct Asset;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub auth: Arc<AuthManager>,
    pub tasks: Arc<TaskManager>,
    pub tags: Arc<crate::tools::tags::TagManager>,
    pub vaults: Arc<crate::vfs::vault::VaultManager>,
    pub backup: Arc<crate::tools::sync::BackupManager>,
}

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // System & Platform Status
        .route("/api/system/status", get(handle_system_status))
        .route("/api/system/exit", post(handle_system_exit))
        .route("/api/system/restart", post(handle_system_restart))
        // Auth & Security API
        .route("/api/auth/login", post(handle_login))
        .route("/api/auth/logout", post(handle_logout))
        .route("/api/auth/unlock", post(handle_unlock_session))
        .route("/api/auth/security-settings", get(handle_get_security_settings).post(handle_update_security_settings))
        .route("/api/auth/me", get(handle_get_me))
        .route("/api/auth/profile", post(handle_update_profile))
        .route("/api/auth/users", get(handle_list_users).post(handle_create_user))
        .route("/api/auth/users/:username", delete(handle_delete_user).post(handle_update_user_rbac))
        // Transparent Encrypted Vaults API
        .route("/api/vault/create", post(handle_create_vault))
        .route("/api/vault/unlock", post(handle_unlock_vault))
        .route("/api/vault/lock", post(handle_lock_vault))
        .route("/api/vault/status", get(handle_vault_status))
        // Global Network Mounts, Storage Roots & Bookmarks API
        .route("/api/storage/roots", get(handle_get_storage_roots))
        .route("/api/mounts/accessible", get(handle_list_accessible_mounts))
        .route("/api/mounts/all", get(handle_list_all_mounts))
        .route("/api/mounts", post(handle_create_or_update_mount))
        .route("/api/mounts/:id", delete(handle_delete_mount))
        .route("/api/bookmarks", get(handle_list_bookmarks).post(handle_create_bookmark))
        .route("/api/bookmarks/:id", delete(handle_delete_bookmark))
        .route("/api/user/preferences", get(handle_get_user_preferences).post(handle_save_user_preferences).delete(handle_reset_user_preferences))
        // Public Link Sharing & Guest Dropbox
        .route("/share/:token", get(handle_public_share_page))
        .route("/api/shares", get(handle_list_shares).post(handle_create_share))
        .route("/api/shares/:id", delete(handle_delete_share))
        .route("/api/public/shares/:token", get(handle_public_get_share_meta))
        .route("/api/public/shares/:token/verify", post(handle_public_verify_share))
        .route("/api/public/shares/:token/download", get(handle_public_download_share))
        .route("/api/public/shares/:token/upload", post(handle_public_upload_share))
        // VFS File Operations
        .route("/api/fs/list", get(handle_list_dir))
        .route("/api/fs/tags", get(handle_get_file_tags))
        .route("/api/fs/tags/all", get(handle_get_all_tags))
        .route("/api/fs/tags/set", post(handle_set_tags))
        .route("/api/fs/read", get(handle_read_file))
        .route("/api/fs/write", post(handle_write_file))
        .route("/api/fs/mkdir", post(handle_mkdir))
        .route("/api/fs/rename", post(handle_rename))
        .route("/api/fs/batch-rename", post(handle_batch_rename))
        .route("/api/fs/delete", post(handle_delete))
        .route("/api/fs/copy", post(handle_copy))
        .route("/api/fs/deltacopy", post(handle_deltacopy))
        .route("/api/fs/move", post(handle_move))
        .route("/api/fs/chmod", post(handle_chmod))
        .route("/api/fs/chown", post(handle_chown))
        .route("/api/fs/upload", post(handle_upload))
        .route("/api/fs/download", get(handle_download))
        .route("/api/fs/download/batch", post(handle_download_batch))
        .route("/api/fs/archive/create", post(handle_archive_create))
        .route("/api/fs/archive/extract", post(handle_archive_extract))
        .route("/api/fs/checksum", post(handle_calculate_checksum))
        // Remote Connections & Test
        .route("/api/remotes/test", post(handle_test_remote))
        .route("/api/remotes/proton/status", get(handle_proton_status))
        // Tools, Comparison & Tasks
        .route("/api/tools/diff/files", post(handle_diff_files))
        .route("/api/tools/diff/folders", post(handle_diff_folders))
        .route("/api/tools/convert", post(handle_convert_file))
        .route("/api/tools/paranoid/dry-run", post(handle_paranoid_dry_run))
        .route("/api/tools/sync/analyze", post(handle_sync_analyze))
        .route("/api/tools/sync/execute", post(handle_sync_execute))
        .route("/api/tools/sync/profiles", get(handle_list_backup_profiles).post(handle_save_backup_profile))
        .route("/api/tools/sync/profiles/:id", delete(handle_delete_backup_profile))
        .route("/api/tools/sync/profiles/:id/run", post(handle_run_backup_profile))
        .route("/api/tools/sync/profiles/:id/toggle", post(handle_toggle_backup_profile))
        .route("/api/tools/sync/history", get(handle_get_backup_history))
        .route("/api/tools/disk-usage", get(handle_disk_usage))
        .route("/api/tools/disks", get(handle_get_disks))
        .route("/api/system/disks", get(handle_get_disks))
        .route("/api/tools/split", post(handle_split_file))
        .route("/api/tools/combine", post(handle_combine_files))
        .route("/api/tools/pdf/info", get(handle_pdf_info))
        .route("/api/tools/pdf/merge", post(handle_pdf_merge))
        .route("/api/tools/pdf/split", post(handle_pdf_split))
        .route("/api/tools/pdf/reorder", post(handle_pdf_reorder))
        .route("/api/tools/syncthing/status", get(handle_syncthing_status))
        .route("/api/tools/syncthing/scan", post(handle_syncthing_scan))
        .route("/api/tools/search", post(handle_search))
        .route("/api/actions/run", post(handle_run_action))
        // NoteDog Notes & Markdown Studio Chewtoy
        .route("/api/tools/notedog/info", get(handle_notedog_info))
        .route("/api/tools/notedog/templates", get(handle_notedog_templates))
        .route("/api/tools/notedog/versions", get(handle_notedog_versions))
        .route("/api/tools/notedog/version/save", post(handle_notedog_save_version))
        .route("/api/tools/notedog/decrypt", post(handle_notedog_decrypt))
        .route("/api/tools/notedog/encrypt", post(handle_notedog_encrypt))
        .route("/api/tools/notedog/create", post(handle_notedog_create_note))
        .route("/api/tools/notedog/section/encrypt", post(handle_notedog_section_encrypt))
        .route("/api/tools/notedog/section/decrypt", post(handle_notedog_section_decrypt))
        .route("/api/tools/notedog/notebook/encrypt", post(handle_notedog_notebook_encrypt))
        .route("/api/tools/notedog/notebook/decrypt", post(handle_notedog_notebook_decrypt))
        // TetraDog Classic Arcade ChewToy & Leaderboard API
        .route("/api/tools/tetradog/scores", get(handle_tetradog_get_scores).post(handle_tetradog_submit_score).delete(handle_tetradog_clear_scores))
        .route("/api/chewtoys/tetradog/scores", get(handle_tetradog_get_scores).post(handle_tetradog_submit_score))
        // Git Client & Version Control API
        .route("/api/git/status", get(handle_git_status))
        .route("/api/git/diff", get(handle_git_diff))
        .route("/api/git/stage", post(handle_git_stage))
        .route("/api/git/unstage", post(handle_git_unstage))
        .route("/api/git/commit", post(handle_git_commit))
        .route("/api/git/push", post(handle_git_push))
        .route("/api/git/pull", post(handle_git_pull))
        .route("/api/git/log", get(handle_git_log))
        .route("/api/tasks", get(handle_list_tasks))
        .route("/api/tasks/:id", get(handle_get_task))
        .route("/api/tasks/:id/cancel", post(handle_cancel_task))
        .route("/api/tasks/:id/pause", post(handle_pause_task))
        .route("/api/tasks/:id/resume", post(handle_resume_task))
        .route("/api/tasks/clear-completed", post(handle_clear_completed_tasks))
        // Terminal WebSocket
        .route("/api/ws/terminal", get(terminal::handle_terminal_ws))
        // System & Config Information
        .route("/api/config", get(handle_get_config))
        .route("/api/system/users-groups", get(handle_get_system_users_groups))
        .route("/api/system/config-file", get(handle_get_config_file).post(handle_save_config_file))
        .route("/api/system/reload-config", post(handle_reload_config))
        .route("/api/system/autostart", get(handle_get_autostart).post(handle_set_autostart))
        .route("/api/system/open-with", post(handle_open_with))
        .route("/api/system/run-custom-action", post(handle_run_custom_action))
        // Embedded Frontend Fallback
        .fallback(handle_static_asset)
        .layer(DefaultBodyLimit::max(state.config.server.upload_max_size_mb * 1024 * 1024))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .with_state(state)
}

// ---------------- AUTH HANDLERS ----------------

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    token: String,
    user: User,
}

async fn handle_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    match state.auth.authenticate(&payload.username, &payload.password) {
        Ok(user) => {
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
    pub standalone: bool,
    pub auth_enabled: bool,
    pub current_user: String,
    pub home_dir: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub custom_hostname: Option<String>,
    pub show_hostname_badge: bool,
    pub hostname_color: Option<String>,
    pub hostname_style: Option<String>,
    pub hostname_icon: Option<String>,
    pub hostname_size: Option<String>,
}

async fn handle_system_status(State(state): State<AppState>) -> Json<SystemStatusResponse> {
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

    Json(SystemStatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        standalone: state.config.server.standalone,
        auth_enabled: state.config.server.enable_auth && !state.config.server.standalone,
        current_user,
        home_dir,
        hostname,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        custom_hostname,
        show_hostname_badge: state.config.ui.show_hostname_badge,
        hostname_color: if !state.config.ui.hostname_color.trim().is_empty() { Some(state.config.ui.hostname_color.clone()) } else { None },
        hostname_style: if !state.config.ui.hostname_style.trim().is_empty() { Some(state.config.ui.hostname_style.clone()) } else { None },
        hostname_icon: if !state.config.ui.hostname_icon.trim().is_empty() { Some(state.config.ui.hostname_icon.clone()) } else { None },
        hostname_size: if !state.config.ui.hostname_size.trim().is_empty() { Some(state.config.ui.hostname_size.clone()) } else { None },
    })
}

#[allow(dead_code)]
fn extract_claims_or_local(
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
    components.into_iter().collect()
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

pub fn validate_path_access(
    state: &AppState,
    headers: &HeaderMap,
    raw_path: &str,
    is_write: bool,
) -> Result<String, (StatusCode, String)> {
    let claims = extract_claims_or_local(state, headers)?;
    let user_role = claims.role.as_str();
    let user_home = &claims.home_dir;
    let allowed_roots_json = claims.allowed_roots.as_deref().unwrap_or("[\"*\"]");
    let allowed_roots: Vec<String> = serde_json::from_str(allowed_roots_json).unwrap_or_else(|_| vec!["*".to_string()]);

    if (user_role.eq_ignore_ascii_case("readonly")) && is_write {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot modify or delete files".to_string()));
    }

    // Remote protocols bypass local storage root checks
    if raw_path.contains("://") {
        return Ok(raw_path.to_string());
    }

    let expanded = if raw_path == "~" {
        user_home.clone()
    } else if let Some(stripped) = raw_path.strip_prefix("~/") {
        Path::new(user_home).join(stripped).to_string_lossy().to_string()
    } else if let Some(stripped) = raw_path.strip_prefix(r"~\") {
        Path::new(user_home).join(stripped).to_string_lossy().to_string()
    } else {
        #[cfg(windows)]
        {
            if (raw_path == "/" || raw_path == "\\") && user_home != "/" {
                user_home.clone()
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

    // Unrestricted system root access if enabled in config and user is admin
    if state.config.storage.allow_entire_system && is_admin {
        return Ok(norm_str);
    }

    // Always permit user within their designated home directory
    let norm_home = normalize_path(Path::new(user_home));
    if path_starts_with_case_insensitive(&normalized, &norm_home) {
        return Ok(norm_str);
    }

    // Check configured storage roots
    for root in &state.config.storage.roots {
        let role_ok = root.allowed_roles.is_empty() || root.allowed_roles.iter().any(|r| r.eq_ignore_ascii_case(user_role));
        let user_ok = is_admin || allowed_roots.contains(&"*".to_string()) || allowed_roots.contains(&root.id) || allowed_roots.contains(&root.path);

        if role_ok && user_ok {
            let norm_root = normalize_path(Path::new(&root.path));
            if path_starts_with_case_insensitive(&normalized, &norm_root) {
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

async fn handle_get_storage_roots(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::config::StorageRoot>>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let user_role = claims.role.as_str();
    let user_home = claims.home_dir.clone();
    let allowed_roots_json = claims.allowed_roots.as_deref().unwrap_or("[\"*\"]");
    let allowed_roots: Vec<String> = serde_json::from_str(allowed_roots_json).unwrap_or_else(|_| vec!["*".to_string()]);

    let mut accessible = Vec::new();
    let is_admin = user_role.eq_ignore_ascii_case("admin");
    let is_readonly = user_role.eq_ignore_ascii_case("readonly");

    // 1. Personal Home Directory
    accessible.push(crate::config::StorageRoot {
        id: "home".to_string(),
        name: "Home".to_string(),
        path: user_home.clone(),
        read_only: is_readonly,
        allowed_roles: vec![],
    });

    // 2. Configured Storage Roots
    for root in &state.config.storage.roots {
        let role_ok = root.allowed_roles.is_empty() || root.allowed_roles.iter().any(|r| r.eq_ignore_ascii_case(user_role));
        let user_ok = is_admin || allowed_roots.contains(&"*".to_string()) || allowed_roots.contains(&root.id) || allowed_roots.contains(&root.path);

        if role_ok && user_ok {
            let mut r = root.clone();
            if is_readonly {
                r.read_only = true;
            }
            if r.path != user_home && !accessible.iter().any(|existing| existing.path == r.path) {
                accessible.push(r);
            }
        }
    }

    // 3. System Root (if allowed in config and user is admin)
    if state.config.storage.allow_entire_system && is_admin {
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

    Ok(Json(accessible))
}

async fn handle_logout() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "success": true, "message": "Logged out" }))
}

async fn handle_system_exit(
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

async fn handle_get_me(
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

        if let Ok(Some(user)) = state.auth.get_user_by_username(&current_user) {
            return Ok(Json(user));
        }

        #[allow(unused_mut)]
        let mut avatar_url = None;
        #[cfg(unix)]
        {
            use base64::Engine;
            if let Some(ref home) = dirs::home_dir() {
                let face_path = home.join(".face");
                let face_icon = home.join(".face.icon");
                if face_path.exists() {
                    if let Ok(bytes) = std::fs::read(&face_path) {
                        let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" };
                        avatar_url = Some(format!("data:{};base64,{}", mime, base64::engine::general_purpose::STANDARD.encode(&bytes)));
                    }
                } else if face_icon.exists() {
                    if let Ok(bytes) = std::fs::read(&face_icon) {
                        let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" };
                        avatar_url = Some(format!("data:{};base64,{}", mime, base64::engine::general_purpose::STANDARD.encode(&bytes)));
                    }
                }
            }
        }

        return Ok(Json(User {
            id: 1,
            username: current_user.clone(),
            nickname: Some(current_user),
            email: None,
            avatar_url,
            role: "admin".to_string(),
            home_dir,
            is_pam: false,
            is_disabled: false,
            allowed_services: "[\"*\"]".to_string(),
            allowed_roots: "[\"*\"]".to_string(),
        }));
    }

    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        match state.auth.verify_token(token_str) {
            Ok(claims) => {
                if let Ok(Some(user)) = state.auth.get_user_by_username(&claims.sub) {
                    return Ok(Json(user));
                }
                // Automatically sync PAM user to database so they appear in Users table & RBAC
                if let Ok(synced_user) = state.auth.sync_pam_user_to_db(&claims.sub, &claims.role, &claims.home_dir) {
                    return Ok(Json(synced_user));
                }
                Ok(Json(User {
                    id: 0,
                    username: claims.sub,
                    nickname: None,
                    email: None,
                    avatar_url: None,
                    role: claims.role,
                    home_dir: claims.home_dir,
                    is_pam: claims.is_pam,
                    is_disabled: false,
                    allowed_services: "[\"*\"]".to_string(),
                    allowed_roots: claims.allowed_roots.unwrap_or_else(|| "[\"*\"]".to_string()),
                }))
            }
            Err(_) => Err((StatusCode::UNAUTHORIZED, "Invalid token".to_string())),
        }
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))
    }
}

#[derive(Deserialize)]
struct UpdateProfileRequest {
    nickname: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
    new_password: Option<String>,
}

async fn handle_update_profile(
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
            claims.sub
        } else {
            return Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()));
        }
    };

    let _ = state.auth.update_user_profile(
        &username,
        payload.nickname.as_deref(),
        payload.email.as_deref(),
        payload.avatar_url.as_deref(),
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update profile: {}", e)))?;

    if let Some(ref new_pass) = payload.new_password {
        if !new_pass.trim().is_empty() {
            let _ = state.auth.update_user_password(&username, new_pass)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update password: {}", e)))?;
        }
    }

    Ok(Json(serde_json::json!({ "success": true, "message": "Profile updated successfully" })))
}

#[derive(Deserialize)]
struct UpdateUserRbacRequest {
    role: String,
    allowed_services: Vec<String>,
    allowed_roots: Option<Vec<String>>,
    home_dir: Option<String>,
    #[serde(default)]
    is_disabled: Option<bool>,
}

async fn handle_update_user_rbac(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
    Json(payload): Json<UpdateUserRbacRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        if claims.role != "admin" {
            return Err((StatusCode::FORBIDDEN, "Only administrators can modify user roles and permissions".to_string()));
        }

        let services_json = serde_json::to_string(&payload.allowed_services).unwrap_or_else(|_| "[\"*\"]".to_string());
        let roots_json = payload.allowed_roots.map(|r| serde_json::to_string(&r).unwrap_or_else(|_| "[\"*\"]".to_string()));

        let updated = state.auth.update_user_rbac(
            &username,
            &payload.role,
            &services_json,
            roots_json.as_deref(),
            payload.home_dir.as_deref(),
            payload.is_disabled.unwrap_or(false),
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update user RBAC: {}", e)))?;

        return Ok(Json(serde_json::json!({ "success": updated })));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

async fn handle_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<User>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        if let Ok(claims) = state.auth.verify_token(token_str) {
            if claims.is_pam {
                let _ = state.auth.sync_pam_user_to_db(&claims.sub, &claims.role, &claims.home_dir);
            }
        }
    }
    state.auth.list_users().map(Json).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list users: {}", e))
    })
}

#[derive(Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    role: String,
    home_dir: Option<String>,
    allowed_roots: Option<Vec<String>>,
}

async fn handle_create_user(
    State(state): State<AppState>,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<User>, (StatusCode, String)> {
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

async fn handle_delete_user(
    State(state): State<AppState>,
    AxumPath(username): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state.auth.delete_user(&username).map(|deleted| {
        Json(serde_json::json!({ "success": deleted }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete user: {}", e)))
}

// ---------------- GLOBAL NETWORK MOUNTS & RBAC HANDLERS ----------------

#[derive(Deserialize)]
struct CreateMountRequest {
    name: String,
    protocol: String,
    target_uri: String,
    options_json: Option<String>,
    allowed_users: Option<Vec<String>>,
}

async fn handle_list_accessible_mounts(
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

async fn handle_list_all_mounts(
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

async fn handle_create_or_update_mount(
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

async fn handle_delete_mount(
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
struct CreateBookmarkRequest {
    name: String,
    protocol: String,
    path: String,
    password: Option<String>,
}

#[derive(Deserialize)]
struct UnlockRequest {
    password: String,
}

async fn handle_unlock_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UnlockRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        match state.auth.authenticate(&claims.sub, &payload.password) {
            Ok(_) => Ok(Json(serde_json::json!({ "success": true, "message": "Session unlocked" }))),
            Err(e) => Err((StatusCode::UNAUTHORIZED, e.to_string())),
        }
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

// ---------------- TRANSPARENT ENCRYPTED VAULT HANDLERS ----------------

#[derive(Deserialize)]
struct CreateVaultRequest {
    path: String,
    password: String,
}

async fn handle_create_vault(
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
struct UnlockVaultRequest {
    path: String,
    password: String,
    auto_lock_secs: Option<u64>,
}

async fn handle_unlock_vault(
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
struct LockVaultRequest {
    path: String,
}

async fn handle_lock_vault(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LockVaultRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, false)?;
    state.vaults.lock_vault(&valid_path);
    Ok(Json(serde_json::json!({ "success": true, "path": valid_path })))
}

#[derive(Deserialize)]
struct VaultStatusQuery {
    path: String,
}

async fn handle_vault_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<VaultStatusQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &query.path, false)?;
    let is_unlocked = state.vaults.is_unlocked(&valid_path);
    Ok(Json(serde_json::json!({ "unlocked": is_unlocked, "path": valid_path })))
}

async fn handle_list_bookmarks(
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

async fn handle_create_bookmark(
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

async fn handle_delete_bookmark(
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

// ---------------- USER PREFERENCES HANDLERS (CROSS-DEVICE SYNC) ----------------

async fn handle_get_user_preferences(
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

async fn handle_save_user_preferences(
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

async fn handle_reset_user_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    state.auth.reset_user_preferences(&claims.sub)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to reset user preferences: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true, "message": "User preferences reset to default" })))
}

async fn handle_get_security_settings(
    State(state): State<AppState>,
) -> Result<Json<crate::auth::SecuritySettings>, (StatusCode, String)> {
    let settings = state.auth.get_security_settings()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get security settings: {}", e)))?;
    Ok(Json(settings))
}

async fn handle_update_security_settings(
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

// ---------------- LINK SHARING & DROPBOX HANDLERS ----------------

#[derive(Deserialize)]
struct CreateShareRequest {
    path: String,
    name: Option<String>,
    is_dir: bool,
    allow_upload: Option<bool>,
    password: Option<String>,
    expires_in_hours: Option<u64>,
    max_downloads: Option<u64>,
}

async fn handle_create_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateShareRequest>,
) -> Result<Json<crate::auth::ShareItem>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let name = payload.name.unwrap_or_else(|| {
        Path::new(&payload.path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    let expires_at = payload.expires_in_hours.map(|hrs| {
        (chrono::Utc::now() + chrono::Duration::hours(hrs as i64)).to_rfc3339()
    });

    let share = state.auth.create_share(
        &claims.sub,
        &payload.path,
        &name,
        payload.is_dir,
        payload.allow_upload.unwrap_or(false),
        payload.password.as_deref(),
        expires_at.as_deref(),
        payload.max_downloads.unwrap_or(0),
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create share: {}", e)))?;

    Ok(Json(share))
}

async fn handle_list_shares(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::auth::ShareItem>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let is_admin = claims.role == "admin";
    let list = state.auth.list_shares(&claims.sub, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list shares: {}", e)))?;

    Ok(Json(list))
}

async fn handle_delete_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let is_admin = claims.role == "admin";
    state.auth.delete_share(id, &claims.sub, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete share: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true })))
}

async fn handle_public_get_share_meta(
    State(state): State<AppState>,
    AxumPath(token): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let share = state.auth.get_share_by_token(&token)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or((StatusCode::NOT_FOUND, "Share not found or expired".to_string()))?;

    if let Some(ref exp) = share.expires_at {
        if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
            if chrono::Utc::now() > exp_time {
                return Err((StatusCode::GONE, "This share link has expired".to_string()));
            }
        }
    }

    if share.max_downloads > 0 && share.download_count >= share.max_downloads {
        return Err((StatusCode::GONE, "This share link has reached its maximum download limit".to_string()));
    }

    let p = Path::new(&share.path);
    let size = if p.is_file() {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    Ok(Json(serde_json::json!({
        "token": share.token,
        "name": share.name,
        "is_dir": share.is_dir,
        "allow_upload": share.allow_upload,
        "has_password": share.has_password,
        "size": size,
        "created_at": share.created_at,
        "expires_at": share.expires_at,
    })))
}

#[derive(Deserialize)]
struct VerifyPassRequest {
    password: String,
}

async fn handle_public_verify_share(
    State(state): State<AppState>,
    AxumPath(token): AxumPath<String>,
    Json(payload): Json<VerifyPassRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid = state.auth.verify_share_password(&token, &payload.password)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Verification error: {}", e)))?;

    if valid {
        Ok(Json(serde_json::json!({ "valid": true })))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Incorrect password for this share".to_string()))
    }
}

async fn handle_public_download_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, (StatusCode, String)> {
    let share = state.auth.get_share_by_token(&token)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or((StatusCode::NOT_FOUND, "Share not found or expired".to_string()))?;

    if let Some(ref exp) = share.expires_at {
        if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
            if chrono::Utc::now() > exp_time {
                return Err((StatusCode::GONE, "This share link has expired".to_string()));
            }
        }
    }

    if share.max_downloads > 0 && share.download_count >= share.max_downloads {
        return Err((StatusCode::GONE, "This share link has reached its maximum download limit".to_string()));
    }

    if share.has_password {
        let pass = query.get("password").map(|s| s.as_str()).unwrap_or("");
        let valid = state.auth.verify_share_password(&token, pass).unwrap_or(false);
        if !valid {
            return Err((StatusCode::UNAUTHORIZED, "Password required for this share".to_string()));
        }
    }

    let path = Path::new(&share.path);
    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "Target file or folder not found on server".to_string()));
    }

    let _ = state.auth.increment_share_downloads(&token);

    if path.is_dir() {
        let temp_zip = tempfile::Builder::new()
            .prefix("brum_share_")
            .suffix(".zip")
            .tempfile()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Temp file error: {}", e)))?;
        let temp_path = temp_zip.path().to_str().unwrap().to_string();

        ArchiveHandler::create_zip(&[share.path.clone()], &temp_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Zip creation failed: {}", e)))?;

        let file_bytes = tokio::fs::read(&temp_path).await.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read generated zip: {}", e))
        })?;

        let zip_name = format!("{}.zip", share.name);
        let response = Response::builder()
            .header(header::CONTENT_TYPE, "application/zip")
            .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", zip_name))
            .header(header::CONTENT_LENGTH, file_bytes.len().to_string())
            .body(Body::from(file_bytes))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

        return Ok(response);
    }

    let metadata = tokio::fs::metadata(path).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file metadata: {}", e))
    })?;

    let mtime_sec = metadata.modified().ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let etag = format!("\"{:x}-{:x}\"", metadata.len(), mtime_sec);

    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    let disposition = format!("attachment; filename=\"{}\"", share.name);
    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

    build_local_file_range_response(path, metadata.len(), mime, disposition, etag, range_header).await
}

async fn handle_public_upload_share(
    State(state): State<AppState>,
    AxumPath(token): AxumPath<String>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let share = state.auth.get_share_by_token(&token)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or((StatusCode::NOT_FOUND, "Share not found or expired".to_string()))?;

    if !share.is_dir || !share.allow_upload {
        return Err((StatusCode::FORBIDDEN, "Guest uploads are not enabled for this share".to_string()));
    }

    if let Some(ref exp) = share.expires_at {
        if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
            if chrono::Utc::now() > exp_time {
                return Err((StatusCode::GONE, "This share link has expired".to_string()));
            }
        }
    }

    let target_dir = Path::new(&share.path);
    if !target_dir.exists() || !target_dir.is_dir() {
        return Err((StatusCode::NOT_FOUND, "Target dropbox folder does not exist on server".to_string()));
    }

    let mut saved_count = 0;
    while let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))? {
        let file_name = field.file_name().unwrap_or("uploaded_file").to_string();
        let safe_name = Path::new(&file_name).file_name().unwrap_or_default().to_string_lossy().to_string();
        if safe_name.is_empty() {
            continue;
        }

        let target_path = target_dir.join(&safe_name);
        let data = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        std::fs::write(&target_path, &data).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write file {}: {}", safe_name, e)))?;
        saved_count += 1;
    }

    Ok(Json(serde_json::json!({ "success": true, "uploaded_files": saved_count })))
}

async fn handle_public_share_page(
    AxumPath(token): AxumPath<String>,
) -> Response {
    let html = format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Brum Share Portal</title>
  <link rel="icon" type="image/png" href="/assets/favicon.png">
  <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;600;700&family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
  <style>
    :root {{
      --bg-dark: #121214;
      --bg-panel: #18181b;
      --bg-header: #202024;
      --accent: #f59e0b;
      --accent-hover: #fbbf24;
      --text-main: #f4f4f5;
      --text-muted: #a1a1aa;
      --border: #3f3f46;
      --radius: 8px;
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
      background: var(--bg-dark);
      color: var(--text-main);
      min-height: 100vh;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      padding: 20px;
    }}
    .share-card {{
      background: var(--bg-panel);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      width: 100%;
      max-width: 480px;
      padding: 28px;
      box-shadow: 0 12px 32px rgba(0,0,0,0.5);
    }}
    .share-header {{
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 20px;
      padding-bottom: 16px;
      border-bottom: 1px solid var(--border);
    }}
    .share-logo {{
      width: 36px;
      height: 36px;
      border-radius: 8px;
      background: rgba(245, 158, 11, 0.15);
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 20px;
    }}
    .share-title {{
      font-size: 16px;
      font-weight: 700;
      color: var(--text-main);
    }}
    .share-subtitle {{
      font-size: 12px;
      color: var(--text-muted);
    }}
    .file-info-box {{
      background: var(--bg-header);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 16px;
      margin-bottom: 20px;
      display: flex;
      align-items: center;
      gap: 14px;
    }}
    .file-icon {{
      font-size: 32px;
    }}
    .file-name {{
      font-size: 14px;
      font-weight: 600;
      word-break: break-all;
    }}
    .file-meta {{
      font-size: 12px;
      color: var(--text-muted);
      margin-top: 4px;
    }}
    .btn {{
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 8px;
      width: 100%;
      padding: 12px;
      font-size: 14px;
      font-weight: 600;
      border-radius: 6px;
      cursor: pointer;
      border: none;
      transition: all 0.15s ease;
      text-decoration: none;
    }}
    .btn-accent {{
      background: var(--accent);
      color: #121214;
    }}
    .btn-accent:hover {{
      background: var(--accent-hover);
    }}
    .dropzone {{
      border: 2px dashed var(--border);
      border-radius: 6px;
      padding: 24px;
      text-align: center;
      margin-top: 20px;
      cursor: pointer;
      transition: all 0.2s ease;
      background: rgba(0,0,0,0.15);
    }}
    .dropzone.drag-over {{
      border-color: var(--accent);
      background: rgba(245, 158, 11, 0.08);
    }}
    .input-field {{
      width: 100%;
      padding: 10px 12px;
      background: var(--bg-dark);
      border: 1px solid var(--border);
      border-radius: 6px;
      color: var(--text-main);
      font-size: 13px;
      margin-bottom: 12px;
      outline: none;
    }}
    .input-field:focus {{
      border-color: var(--accent);
    }}
    .footer-text {{
      margin-top: 16px;
      font-size: 11px;
      color: var(--text-muted);
      text-align: center;
    }}
  </style>
</head>
<body>
  <div class="share-card" id="share-card">
    <div class="share-header">
      <div class="share-logo">📦</div>
      <div>
        <div class="share-title">Brum Public Share</div>
        <div class="share-subtitle">Multi-Pane Web Environment (File Commander/Manager) - By Woofson</div>
      </div>
    </div>

    <div id="loading-state" style="text-align: center; padding: 24px; color: var(--text-muted);">
      Loading shared file information...
    </div>

    <div id="error-state" style="display: none; text-align: center; padding: 20px; color: #ef4444;"></div>

    <div id="password-state" style="display: none;">
      <p style="font-size: 13px; color: var(--text-muted); margin-bottom: 12px;">🔒 This share is password protected. Enter the password to unlock download:</p>
      <input type="password" id="share-pass-input" class="input-field" placeholder="Enter share password...">
      <button class="btn btn-accent" onclick="unlockWithPassword()">Unlock Share</button>
    </div>

    <div id="content-state" style="display: none;">
      <div class="file-info-box">
        <div class="file-icon" id="item-icon">📄</div>
        <div style="flex: 1; min-width: 0;">
          <div class="file-name" id="item-name">Loading...</div>
          <div class="file-meta" id="item-meta">Calculating size...</div>
        </div>
      </div>

      <button class="btn btn-accent" id="btn-download" onclick="startDownload()">
        ⬇️ Download File
      </button>

      <div id="dropbox-section" style="display: none;">
        <div class="dropzone" id="dropzone" onclick="document.getElementById('file-input').click()">
          <input type="file" id="file-input" multiple style="display:none;" onchange="handleFileInput(this.files)">
          <div style="font-size: 24px; margin-bottom: 6px;">📥</div>
          <div style="font-weight: 600; font-size: 13px;">Guest Upload Dropbox</div>
          <div style="font-size: 11px; color: var(--text-muted); margin-top: 4px;">Drag & drop files here or click to browse</div>
        </div>
        <div id="upload-status" style="margin-top: 10px; font-size: 12px; text-align: center; color: var(--accent);"></div>
      </div>
    </div>

    <div class="footer-text">
      Powered by Brum • High Performance Fast File Transport
    </div>
  </div>

  <script>
    const token = "{token}";
    let shareMeta = null;
    let unlockedPass = "";

    function formatBytes(bytes) {{
      if (!bytes || bytes === 0) return '0 B';
      const k = 1024;
      const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i];
    }}

    async function loadMeta() {{
      try {{
        const res = await fetch(`/api/public/shares/${{token}}`);
        if (!res.ok) {{
          const err = await res.text();
          showError(err || 'Share not found or expired');
          return;
        }}
        shareMeta = await res.json();
        document.getElementById('loading-state').style.display = 'none';

        if (shareMeta.has_password && !unlockedPass) {{
          document.getElementById('password-state').style.display = 'block';
        }} else {{
          renderContent();
        }}
      }} catch (e) {{
        showError('Network error loading share');
      }}
    }}

    function showError(msg) {{
      document.getElementById('loading-state').style.display = 'none';
      document.getElementById('password-state').style.display = 'none';
      document.getElementById('content-state').style.display = 'none';
      const errEl = document.getElementById('error-state');
      errEl.style.display = 'block';
      errEl.textContent = '❌ ' + msg;
    }}

    async function unlockWithPassword() {{
      const pass = document.getElementById('share-pass-input').value;
      if (!pass) return;
      try {{
        const res = await fetch(`/api/public/shares/${{token}}/verify`, {{
          method: 'POST',
          headers: {{ 'Content-Type': 'application/json' }},
          body: JSON.stringify({{ password: pass }})
        }});
        if (res.ok) {{
          unlockedPass = pass;
          document.getElementById('password-state').style.display = 'none';
          renderContent();
        }} else {{
          alert('Incorrect password');
        }}
      }} catch (e) {{
        alert('Verification failed');
      }}
    }}

    function renderContent() {{
      document.getElementById('content-state').style.display = 'block';
      document.getElementById('item-name').textContent = shareMeta.name;
      document.getElementById('item-icon').textContent = shareMeta.is_dir ? '📁' : '📄';
      document.getElementById('item-meta').textContent = shareMeta.is_dir ? 'Folder (Downloads as Zip)' : formatBytes(shareMeta.size);
      
      const btn = document.getElementById('btn-download');
      btn.textContent = shareMeta.is_dir ? '⬇️ Download Folder (.zip)' : '⬇️ Download File';

      if (shareMeta.is_dir && shareMeta.allow_upload) {{
        document.getElementById('dropbox-section').style.display = 'block';
        setupDropzone();
      }}
    }}

    function startDownload() {{
      const url = `/api/public/shares/${{token}}/download${{unlockedPass ? '?password=' + encodeURIComponent(unlockedPass) : ''}}`;
      window.location.href = url;
    }}

    function setupDropzone() {{
      const dz = document.getElementById('dropzone');
      dz.ondragover = (e) => {{ e.preventDefault(); dz.classList.add('drag-over'); }};
      dz.ondragleave = () => dz.classList.remove('drag-over');
      dz.ondrop = (e) => {{
        e.preventDefault();
        dz.classList.remove('drag-over');
        if (e.dataTransfer.files) handleFileInput(e.dataTransfer.files);
      }};
    }}

    async function handleFileInput(files) {{
      if (!files || files.length === 0) return;
      const status = document.getElementById('upload-status');
      status.textContent = `Uploading ${{files.length}} file(s)...`;

      const fd = new FormData();
      for (let f of files) fd.append('files', f);

      try {{
        const res = await fetch(`/api/public/shares/${{token}}/upload`, {{
          method: 'POST',
          body: fd
        }});
        if (res.ok) {{
          status.textContent = `✅ Successfully uploaded ${{files.length}} file(s)!`;
        }} else {{
          status.textContent = `❌ Upload failed: ${{await res.text()}}`;
        }}
      }} catch (e) {{
        status.textContent = '❌ Upload failed due to network error';
      }}
    }}

    loadMeta();
  </script>
</body>
</html>"#, token = token);

    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "HTML load error").into_response())
}

// ---------------- VFS HANDLERS ----------------

#[derive(Deserialize)]
#[allow(dead_code)]
struct ListQuery {
    path: Option<String>,
    show_hidden: Option<bool>,
    flat: Option<bool>,
    max_depth: Option<usize>,
    max_entries: Option<usize>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    pass: Option<String>,
}

#[derive(Deserialize)]
struct GetTagsQuery {
    path: Option<String>,
}

async fn handle_get_file_tags(
    State(state): State<AppState>,
    Query(query): Query<GetTagsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Some(p) = query.path {
        let tag_info = state.tags.get_tags_for_path(&p);
        Ok(Json(serde_json::json!({ "success": true, "tag": tag_info })))
    } else {
        let all = state.tags.get_all_tags().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(Json(serde_json::json!({ "success": true, "tags": all })))
    }
}

async fn handle_get_all_tags(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let all = state.tags.get_all_tags().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "success": true, "tags": all })))
}

async fn handle_set_tags(
    State(state): State<AppState>,
    Json(payload): Json<crate::tools::tags::SetTagsRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let count = state.tags.set_tags(payload).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "success": true, "count": count })))
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

async fn handle_list_dir(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<DirectoryListing>, (StatusCode, String)> {
    let raw_path = query.path.unwrap_or_else(|| state.config.server.root_path.clone());
    let target_path = validate_path_access(&state, &headers, &raw_path, false)?;
    let show_hidden = query.show_hidden.unwrap_or(state.config.ui.show_hidden_files);

    if target_path.starts_with("vault://") {
        let rest = target_path.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        state.vaults.list_vault_contents(vault_file, subpath)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))
    } else if target_path.starts_with("archive://") {
        let rest = target_path.strip_prefix("archive://").unwrap();
        let parts: Vec<&str> = rest.split('#').collect();
        let archive_file = parts[0];
        let subpath = if parts.len() > 1 { parts[1] } else { "" };
        ArchiveHandler::list_archive_contents(archive_file, subpath)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to list archive: {}", e)))
    } else if target_path.starts_with("sftp://") {
        let params = SftpClient::parse_uri(&target_path, query.user.as_deref(), query.pass.as_deref())
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?;
        SftpClient::list_dir(&params)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("SFTP list failed: {}", e)))
    } else if target_path.starts_with("webdav://") || target_path.starts_with("http://") || target_path.starts_with("https://") {
        let url = if target_path.starts_with("webdav://") {
            format!("http://{}", target_path.strip_prefix("webdav://").unwrap())
        } else {
            target_path
        };
        WebDavClient::list_dir(&url, query.user.as_deref(), query.pass.as_deref()).await
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to list WebDAV: {}", e)))
    } else if target_path.starts_with("proton://") {
        let entries = crate::vfs::proton::ProtonDriveClient::list_directory(&target_path).await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Proton Drive list failed: {}", e)))?;
        let total_files = entries.iter().filter(|e| !e.is_dir).count();
        let total_dirs = entries.iter().filter(|e| e.is_dir).count();
        let total_size = entries.iter().map(|e| e.size).sum();
        let parent_path = if target_path == "proton://" || target_path == "proton:///" {
            None
        } else {
            let clean = target_path.trim_start_matches("proton://").trim_start_matches('/');
            let p = std::path::Path::new(clean);
            p.parent().and_then(|par| {
                let s = par.to_string_lossy().to_string();
                if s.is_empty() { Some("proton:///".to_string()) } else { Some(format!("proton:///{}", s)) }
            })
        };

        Ok(Json(DirectoryListing {
            current_path: target_path,
            parent_path,
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "proton".to_string(),
            is_truncated: None,
            max_limit: None,
        }))
    } else if target_path.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&target_path, query.user.as_deref(), query.pass.as_deref())
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        crate::vfs::smb::SmbClient::list_dir(&params)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("SMB list failed: {}", e)))
    } else if target_path.starts_with("nfs://") {
        let params = crate::vfs::nfs::NfsClient::parse_uri(&target_path)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid NFS URI: {}", e)))?;
        crate::vfs::nfs::NfsClient::list_dir(&params, show_hidden)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("NFS list failed: {}", e)))
    } else if query.flat.unwrap_or(false) {
        let max_d = query.max_depth;
        let max_e = query.max_entries;
        let p = target_path.clone();
        tokio::task::spawn_blocking(move || {
            LocalFs::list_branch_view(&p, show_hidden, max_d, max_e)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Branch view task failed: {}", e)))?
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to list branch view: {}", e)))
    } else {
        LocalFs::list_dir(&target_path, show_hidden)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to list local directory: {}", e)))
    }
}

#[derive(Deserialize)]
struct ReadFileQuery {
    path: String,
    max_bytes: Option<usize>,
}

async fn handle_read_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReadFileQuery>,
) -> Result<Json<crate::vfs::FileContentResponse>, (StatusCode, String)> {
    let target_path = validate_path_access(&state, &headers, &query.path, false)?;
    let max_b = query.max_bytes.unwrap_or(10_000_000);

    if target_path.starts_with("vault://") {
        let rest = target_path.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        state.vaults.read_vault_file(vault_file, subpath)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))
    } else if target_path.starts_with("archive://") {
        let rest = target_path.strip_prefix("archive://").unwrap();
        let parts: Vec<&str> = rest.split('#').collect();
        let archive_file = parts[0];
        let subpath = if parts.len() > 1 { parts[1] } else { "" };
        ArchiveHandler::read_archive_entry(archive_file, subpath, max_b)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read archive item: {}", e)))
    } else if target_path.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        crate::vfs::smb::SmbClient::read_file(&params)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read SMB file: {}", e)))
    } else if target_path.starts_with("sftp://") {
        let params = SftpClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?;
        let bytes = SftpClient::download_file(&params.host, params.port, &params.user, params.password.as_deref(), &params.remote_path, max_b)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read SFTP file: {}", e)))?;
        let mime = mime_guess::from_path(&params.remote_path).first_or_octet_stream().to_string();
        let is_text = mime.starts_with("text/") || mime.contains("json") || mime.contains("javascript") || mime.contains("xml") || mime.contains("yaml") || mime.contains("toml");
        use base64::Engine;
        let content = if is_text {
            String::from_utf8(bytes.clone()).unwrap_or_else(|_| base64::engine::general_purpose::STANDARD.encode(&bytes))
        } else {
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        };
        let file_name = params.remote_path.rsplit('/').next().unwrap_or(&params.remote_path).to_string();
        Ok(Json(crate::vfs::FileContentResponse {
            path: target_path,
            name: file_name,
            content,
            size: bytes.len() as u64,
            mime_type: mime,
            is_binary: !is_text,
        }))
    } else {
        LocalFs::read_file(&target_path, max_b)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read file: {}", e)))
    }
}

#[derive(Deserialize)]
struct WriteFileRequest {
    path: String,
    content: String,
    atomic: Option<bool>,
}

async fn handle_write_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<WriteFileRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let target_path = validate_path_access(&state, &headers, &payload.path, true)?;

    if target_path.starts_with("vault://") {
        let rest = target_path.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        state.vaults.write_vault_file(vault_file, subpath, payload.content.as_bytes())
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
    } else if target_path.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        crate::vfs::smb::SmbClient::write_file(&params, payload.content.as_bytes())
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save SMB file: {}", e)))
    } else if target_path.starts_with("sftp://") {
        let params = SftpClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?;
        SftpClient::write_file(&params, payload.content.as_bytes())
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save SFTP file: {}", e)))
    } else {
        let atomic = payload.atomic.unwrap_or(state.config.paranoid.atomic_writes);
        LocalFs::write_file(&target_path, payload.content.as_bytes(), atomic)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save file: {}", e)))
    }
}

#[derive(Deserialize)]
struct MkdirRequest {
    path: String,
}

async fn handle_mkdir(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<MkdirRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let target_path = validate_path_access(&state, &headers, &payload.path, true)?;

    if target_path.starts_with("vault://") {
        let rest = target_path.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        state.vaults.mkdir_vault(vault_file, subpath)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
    } else if target_path.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        crate::vfs::smb::SmbClient::mkdir(&params)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create SMB folder: {}", e)))
    } else if target_path.starts_with("sftp://") {
        let params = crate::vfs::sftp::SftpClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?;
        crate::vfs::sftp::SftpClient::mkdir(&params)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create SFTP folder: {}", e)))
    } else if target_path.starts_with("nfs://") {
        let params = crate::vfs::nfs::NfsClient::parse_uri(&target_path)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid NFS URI: {}", e)))?;
        let mount = crate::vfs::nfs::NfsClient::ensure_mounted(&params)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("NFS mount error: {}", e)))?;
        let local_target = mount.join(params.subpath.trim_start_matches('/'));
        LocalFs::create_dir(&local_target.to_string_lossy())
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create NFS folder: {}", e)))
    } else {
        LocalFs::create_dir(&target_path)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create folder: {}", e)))
    }
}

#[derive(Deserialize)]
struct RenameRequest {
    from: String,
    to: String,
}

async fn handle_rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RenameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let from_path = validate_path_access(&state, &headers, &payload.from, true)?;
    let to_path = validate_path_access(&state, &headers, &payload.to, true)?;

    if from_path.starts_with("smb://") {
        let params_from = crate::vfs::smb::SmbClient::parse_uri(&from_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        let target_subpath = if to_path.starts_with("smb://") {
            let params_to = crate::vfs::smb::SmbClient::parse_uri(&to_path, None, None)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
            params_to.subpath
        } else {
            to_path
        };
        crate::vfs::smb::SmbClient::rename(&params_from, &target_subpath)
            .map(|_| Json(serde_json::json!({ "success": true })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to rename SMB item: {}", e)))
    } else if from_path.starts_with("sftp://") {
        let params_from = crate::vfs::sftp::SftpClient::parse_uri(&from_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?;
        let target_remote = if to_path.starts_with("sftp://") {
            let params_to = crate::vfs::sftp::SftpClient::parse_uri(&to_path, None, None)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?;
            params_to.remote_path
        } else {
            to_path
        };
        crate::vfs::sftp::SftpClient::rename(&params_from, &target_remote)
            .map(|_| Json(serde_json::json!({ "success": true })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to rename SFTP item: {}", e)))
    } else if from_path.starts_with("nfs://") {
        let params_from = crate::vfs::nfs::NfsClient::parse_uri(&from_path)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid NFS URI: {}", e)))?;
        let mount = crate::vfs::nfs::NfsClient::ensure_mounted(&params_from)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("NFS mount error: {}", e)))?;
        let local_from = mount.join(params_from.subpath.trim_start_matches('/'));
        let local_to = if to_path.starts_with("nfs://") {
            let params_to = crate::vfs::nfs::NfsClient::parse_uri(&to_path)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid NFS URI: {}", e)))?;
            mount.join(params_to.subpath.trim_start_matches('/'))
        } else {
            std::path::PathBuf::from(&to_path)
        };
        LocalFs::rename_entry(&local_from.to_string_lossy(), &local_to.to_string_lossy())
            .map(|_| Json(serde_json::json!({ "success": true })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to rename NFS item: {}", e)))
    } else {
        LocalFs::rename_entry(&from_path, &to_path)
            .map(|_| Json(serde_json::json!({ "success": true })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to rename: {}", e)))
    }
}

#[derive(Deserialize)]
struct BatchRenameItem {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct BatchRenameRequest {
    renames: Vec<BatchRenameItem>,
}

async fn handle_batch_rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchRenameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut renamed = 0;
    let mut errors = Vec::new();

    for item in payload.renames {
        if item.from != item.to {
            let from_res = validate_path_access(&state, &headers, &item.from, true);
            let to_res = validate_path_access(&state, &headers, &item.to, true);
            match (from_res, to_res) {
                (Ok(from_path), Ok(to_path)) => {
                    match LocalFs::rename_entry(&from_path, &to_path) {
                        Ok(_) => renamed += 1,
                        Err(e) => errors.push(format!("{}: {}", item.from, e)),
                    }
                }
                (Err((_, e)), _) | (_, Err((_, e))) => {
                    errors.push(format!("{}: {}", item.from, e));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(Json(serde_json::json!({ "success": true, "renamed_count": renamed })))
    } else {
        Err((StatusCode::MULTI_STATUS, format!("Encountered errors: {}", errors.join("; "))))
    }
}

#[derive(Deserialize)]
struct DeleteRequest {
    paths: Vec<String>,
    use_trash: Option<bool>,
    custom_trash_dir: Option<String>,
}

async fn handle_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DeleteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let use_trash = payload.use_trash.unwrap_or(state.config.paranoid.trash_enabled);
    let custom_trash = payload.custom_trash_dir.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
    let mut deleted = Vec::new();
    let mut errors = Vec::new();

    for path in &payload.paths {
        let valid_path = match validate_path_access(&state, &headers, path, true) {
            Ok(p) => p,
            Err((_, e)) => {
                errors.push(format!("{}: {}", path, e));
                continue;
            }
        };

        if valid_path.starts_with("vault://") {
            let rest = valid_path.strip_prefix("vault://").unwrap();
            let (vault_file, subpath) = match rest.split_once('#') {
                Some((v, s)) => (v, s),
                None => (rest, ""),
            };
            match state.vaults.delete_vault_file(vault_file, subpath) {
                Ok(_) => deleted.push(path.clone()),
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        } else if valid_path.starts_with("smb://") {
            match crate::vfs::smb::SmbClient::parse_uri(&valid_path, None, None) {
                Ok(params) => {
                    match crate::vfs::smb::SmbClient::delete(&params, false) {
                        Ok(_) => deleted.push(path.clone()),
                        Err(e) => errors.push(format!("{}: {}", path, e)),
                    }
                }
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        } else if valid_path.starts_with("sftp://") {
            match crate::vfs::sftp::SftpClient::parse_uri(&valid_path, None, None) {
                Ok(params) => {
                    match crate::vfs::sftp::SftpClient::delete(&params, false) {
                        Ok(_) => deleted.push(path.clone()),
                        Err(e) => errors.push(format!("{}: {}", path, e)),
                    }
                }
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        } else if valid_path.starts_with("nfs://") {
            match crate::vfs::nfs::NfsClient::parse_uri(&valid_path) {
                Ok(params) => {
                    match crate::vfs::nfs::NfsClient::ensure_mounted(&params) {
                        Ok(mount) => {
                            let local_target = mount.join(params.subpath.trim_start_matches('/'));
                            match LocalFs::delete_entry(&local_target.to_string_lossy(), false, None) {
                                Ok(_) => deleted.push(path.clone()),
                                Err(e) => errors.push(format!("{}: {}", path, e)),
                            }
                        }
                        Err(e) => errors.push(format!("{}: {}", path, e)),
                    }
                }
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        } else {
            match LocalFs::delete_entry(&valid_path, use_trash, custom_trash) {
                Ok(_) => deleted.push(path.clone()),
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        }
    }

    if errors.is_empty() {
        Ok(Json(serde_json::json!({ "success": true, "deleted": deleted })))
    } else {
        Err((StatusCode::MULTI_STATUS, format!("Encountered errors: {}", errors.join("; "))))
    }
}

#[derive(Deserialize)]
struct ChmodRequest {
    paths: Vec<String>,
    mode: u32,
    recursive: Option<bool>,
}

async fn handle_chmod(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChmodRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rec = payload.recursive.unwrap_or(false);
    for p in &payload.paths {
        let valid_path = validate_path_access(&state, &headers, p, true)?;
        LocalFs::chmod_entry(&valid_path, payload.mode, rec)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Chmod failed for {}: {}", p, e)))?;
    }
    Ok(Json(serde_json::json!({ "success": true, "mode": format!("{:04o}", payload.mode) })))
}

#[derive(Deserialize)]
struct ChownRequest {
    paths: Vec<String>,
    owner: Option<String>,
    group: Option<String>,
    recursive: Option<bool>,
}

async fn handle_chown(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChownRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rec = payload.recursive.unwrap_or(false);

    let uid_val: Option<u32> = if let Some(ref o) = payload.owner {
        if let Ok(u) = o.parse::<u32>() {
            Some(u)
        } else if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
            passwd.lines().find_map(|l| {
                let parts: Vec<&str> = l.split(':').collect();
                if parts.len() >= 3 && parts[0] == o {
                    parts[2].parse::<u32>().ok()
                } else {
                    None
                }
            })
        } else {
            None
        }
    } else {
        None
    };

    let gid_val: Option<u32> = if let Some(ref g) = payload.group {
        if let Ok(gid) = g.parse::<u32>() {
            Some(gid)
        } else if let Ok(grp) = fs::read_to_string("/etc/group") {
            grp.lines().find_map(|l| {
                let parts: Vec<&str> = l.split(':').collect();
                if parts.len() >= 3 && parts[0] == g {
                    parts[2].parse::<u32>().ok()
                } else {
                    None
                }
            })
        } else {
            None
        }
    } else {
        None
    };

    for p in &payload.paths {
        let valid_path = validate_path_access(&state, &headers, p, true)?;
        LocalFs::chown_entry(&valid_path, uid_val, gid_val, rec)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Chown failed for {}: {}", p, e)))?;
    }

    Ok(Json(serde_json::json!({ "success": true, "uid": uid_val, "gid": gid_val })))
}

#[derive(Deserialize)]
struct TransferRequest {
    sources: Vec<String>,
    destination: String,
    paranoid: Option<bool>,
}

async fn handle_copy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TransferRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut validated_sources = Vec::new();
    for s in &payload.sources {
        validated_sources.push(validate_path_access(&state, &headers, s, false)?);
    }
    let validated_dest = validate_path_access(&state, &headers, &payload.destination, true)?;

    let paranoid = payload.paranoid.unwrap_or(state.config.paranoid.verify_after_transfer);
    let task_id = state.tasks.create_task(
        &format!("Copy {} items", validated_sources.len()),
        "copy",
        &validated_sources.join(", "),
        &validated_dest,
        0,
    ).await;

    let tasks_mgr = state.tasks.clone();
    let tid = task_id.clone();
    let sources = validated_sources;
    let destination = validated_dest;

    tokio::spawn(async move {
        crate::vfs::transfer::VfsTransfer::execute_batch_transfer(
            tasks_mgr,
            tid,
            sources,
            destination,
            false,
            paranoid,
        ).await;
    });

    Ok(Json(serde_json::json!({ "success": true, "task_id": task_id, "copied_count": payload.sources.len() })))
}

#[derive(Deserialize)]
struct DeltaCopyRequest {
    sources: Vec<String>,
    destination: String,
    options: Option<crate::tools::deltacopy::DeltaCopyOptions>,
}

async fn handle_deltacopy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DeltaCopyRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut validated_sources = Vec::new();
    for s in &payload.sources {
        validated_sources.push(validate_path_access(&state, &headers, s, false)?);
    }
    let validated_dest = validate_path_access(&state, &headers, &payload.destination, true)?;

    let opts = payload.options.unwrap_or_default();
    let tasks_mgr = state.tasks.clone();
    let cancel_token = Arc::new(std::sync::atomic::AtomicBool::new(false));

    tokio::spawn(async move {
        let _ = crate::tools::deltacopy::DeltaCopyEngine::run_deltacopy(
            tasks_mgr,
            validated_sources,
            validated_dest,
            opts,
            cancel_token,
        ).await;
    });

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "DeltaCopy transfer started in background queue",
    })))
}

async fn handle_move(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TransferRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut validated_sources = Vec::new();
    for s in &payload.sources {
        validated_sources.push(validate_path_access(&state, &headers, s, true)?);
    }
    let validated_dest = validate_path_access(&state, &headers, &payload.destination, true)?;

    let paranoid = payload.paranoid.unwrap_or(state.config.paranoid.verify_after_transfer);
    let task_id = state.tasks.create_task(
        &format!("Move {} items", validated_sources.len()),
        "move",
        &validated_sources.join(", "),
        &validated_dest,
        0,
    ).await;

    let tasks_mgr = state.tasks.clone();
    let tid = task_id.clone();
    let sources = validated_sources;
    let destination = validated_dest;

    tokio::spawn(async move {
        crate::vfs::transfer::VfsTransfer::execute_batch_transfer(
            tasks_mgr,
            tid,
            sources,
            destination,
            true,
            paranoid,
        ).await;
    });

    Ok(Json(serde_json::json!({ "success": true, "task_id": task_id, "moved_count": payload.sources.len() })))
}

#[derive(Deserialize)]
struct TestRemoteRequest {
    protocol: String,
    host: String,
    port: Option<u16>,
    user: Option<String>,
    pass: Option<String>,
    bucket: Option<String>,
    region: Option<String>,
}

async fn handle_test_remote(
    Json(payload): Json<TestRemoteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match payload.protocol.to_lowercase().as_str() {
        "sftp" => {
            let port = payload.port.unwrap_or(22);
            let user = payload.user.unwrap_or_else(|| "root".to_string());
            let params = crate::vfs::sftp::SftpParams {
                host: payload.host,
                port,
                user,
                password: payload.pass.filter(|p| !p.trim().is_empty()),
                key_path: None,
                remote_path: "/".to_string(),
            };
            match SftpClient::list_dir(&params) {
                Ok(listing) => Ok(Json(serde_json::json!({
                    "success": true,
                    "message": format!("Connected successfully to SFTP server (found {} items)", listing.entries.len()),
                }))),
                Err(e) => Err((StatusCode::BAD_REQUEST, format!("SFTP connection failed: {}", e))),
            }
        }
        "webdav" => {
            let url = payload.host;
            match WebDavClient::list_dir(&url, payload.user.as_deref(), payload.pass.as_deref()).await {
                Ok(listing) => Ok(Json(serde_json::json!({
                    "success": true,
                    "message": format!("Connected successfully to WebDAV storage (found {} items)", listing.entries.len()),
                }))),
                Err(e) => Err((StatusCode::BAD_REQUEST, format!("WebDAV connection failed: {}", e))),
            }
        }
        "s3" => {
            let s3_conf = crate::vfs::s3::S3Config {
                endpoint: payload.host,
                bucket: payload.bucket.unwrap_or_default(),
                region: payload.region.unwrap_or_else(|| "us-east-1".to_string()),
                access_key_id: payload.user.unwrap_or_default(),
                secret_access_key: payload.pass.unwrap_or_default(),
                path_style: Some(true),
            };
            let client = crate::vfs::s3::S3Client::new(s3_conf);
            match client.test_connection().await {
                Ok(_) => Ok(Json(serde_json::json!({
                    "success": true,
                    "message": "Connected successfully to S3 / Cloud Object Storage bucket!",
                }))),
                Err(e) => Err((StatusCode::BAD_REQUEST, format!("S3 connection failed: {}", e))),
            }
        }
        "proton" => {
            let status = crate::vfs::proton::ProtonDriveClient::check_status().await;
            if status.installed {
                if status.authenticated {
                    Ok(Json(serde_json::json!({
                        "success": true,
                        "message": format!("Proton Drive integration ready! Backend: {} (Version: {})", status.cli_type, status.version.unwrap_or_default()),
                    })))
                } else {
                    Ok(Json(serde_json::json!({
                        "success": false,
                        "message": format!("Proton Drive CLI detected ({}) but not yet logged in. Run '{} login' in terminal.", status.cli_type, status.cli_type),
                    })))
                }
            } else {
                Err((StatusCode::BAD_REQUEST, "Proton Drive CLI or rclone not detected on host. Install 'proton-drive' or configure an rclone proton remote.".to_string()))
            }
        }
        "smb" => {
            let port = payload.port.unwrap_or(445);
            let share = payload.bucket.unwrap_or_else(|| "share".to_string());
            let params = crate::vfs::smb::SmbParams {
                host: payload.host,
                port,
                share,
                subpath: "".to_string(),
                username: payload.user,
                password: payload.pass,
                domain: payload.region,
            };
            crate::vfs::smb::SmbClient::test_connection(&params)
                .map(|msg| Json(serde_json::json!({ "success": true, "message": msg })))
                .map_err(|e| (StatusCode::BAD_REQUEST, e))
        }
        "nfs" => {
            let port = payload.port.unwrap_or(2049);
            let export_path = payload.bucket.unwrap_or_else(|| "/".to_string());
            let params = crate::vfs::nfs::NfsParams {
                host: payload.host,
                port,
                export_path,
                subpath: "".to_string(),
                version: payload.region,
            };
            crate::vfs::nfs::NfsClient::test_connection(&params)
                .map(|msg| Json(serde_json::json!({ "success": true, "message": msg })))
                .map_err(|e| (StatusCode::BAD_REQUEST, e))
        }
        _ => Err((StatusCode::BAD_REQUEST, "Unsupported protocol".to_string())),
    }
}

async fn handle_proton_status() -> Json<crate::vfs::proton::ProtonStatus> {
    Json(crate::vfs::proton::ProtonDriveClient::check_status().await)
}

async fn handle_list_tasks(
    State(state): State<AppState>,
) -> Json<Vec<TaskInfo>> {
    Json(state.tasks.list_tasks().await)
}

async fn handle_get_task(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<TaskInfo>, (StatusCode, String)> {
    match state.tasks.get_task(&id).await {
        Some(task) => Ok(Json(task)),
        None => Err((StatusCode::NOT_FOUND, "Task not found".to_string())),
    }
}

async fn handle_cancel_task(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let cancelled = state.tasks.cancel_task(&id).await;
    Json(serde_json::json!({ "success": cancelled }))
}

async fn handle_pause_task(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let paused = state.tasks.pause_task(&id).await;
    Json(serde_json::json!({ "success": paused }))
}

async fn handle_resume_task(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let resumed = state.tasks.resume_task(&id).await;
    Json(serde_json::json!({ "success": resumed }))
}

async fn handle_clear_completed_tasks(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    state.tasks.clear_completed().await;
    Json(serde_json::json!({ "success": true }))
}

async fn handle_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let raw_dest = query.get("destination").cloned().unwrap_or_else(|| "~".to_string());
    let dest_dir = validate_path_access(&state, &headers, &raw_dest, true)?;

    let mut uploaded_files = Vec::new();
    let task_id = state.tasks.create_task("Upload Files", "upload", "Browser", &dest_dir, 0).await;

    while let Ok(Some(field)) = multipart.next_field().await {
        let file_name = field.file_name().unwrap_or("upload.bin").to_string();

        if let Ok(data) = field.bytes().await {
            let file_size = data.len() as u64;
            state.tasks.update_task_details(
                &task_id,
                Some(&file_name),
                file_size,
                file_size,
                uploaded_files.len() as u64 + 1,
                uploaded_files.len() as u64 + 1,
                file_size,
                0,
                None,
                None,
                Some(&format!("Uploaded {}", file_name)),
            ).await;

            if dest_dir.starts_with("smb://") {
                let params = match crate::vfs::smb::SmbClient::parse_uri(&dest_dir, None, None) {
                    Ok(mut p) => {
                        p.subpath = if p.subpath.is_empty() { file_name.clone() } else { format!("{}/{}", p.subpath, file_name) };
                        p
                    }
                    Err(e) => {
                        let err_msg = format!("Invalid SMB destination: {}", e);
                        state.tasks.fail_task(&task_id, &err_msg).await;
                        return Err((StatusCode::BAD_REQUEST, err_msg));
                    }
                };
                if let Err(e) = crate::vfs::smb::SmbClient::write_file(&params, &data) {
                    let err_msg = format!("Failed to write SMB upload: {}", e);
                    state.tasks.fail_task(&task_id, &err_msg).await;
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, err_msg));
                }
            } else if dest_dir.starts_with("sftp://") {
                let params = match SftpClient::parse_uri(&dest_dir, None, None) {
                    Ok(mut p) => {
                        p.remote_path = if p.remote_path.is_empty() || p.remote_path == "/" {
                            format!("/{}", file_name)
                        } else {
                            format!("{}/{}", p.remote_path.trim_end_matches('/'), file_name)
                        };
                        p
                    }
                    Err(e) => {
                        let err_msg = format!("Invalid SFTP destination: {}", e);
                        state.tasks.fail_task(&task_id, &err_msg).await;
                        return Err((StatusCode::BAD_REQUEST, err_msg));
                    }
                };
                if let Err(e) = SftpClient::write_file(&params, &data) {
                    let err_msg = format!("Failed to write SFTP upload: {}", e);
                    state.tasks.fail_task(&task_id, &err_msg).await;
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, err_msg));
                }
            } else {
                let target_path = Path::new(&dest_dir).join(&file_name);
                if let Err(e) = LocalFs::write_file(&target_path.to_string_lossy(), &data, true) {
                    let err_msg = format!("Failed to write upload: {}", e);
                    state.tasks.fail_task(&task_id, &err_msg).await;
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, err_msg));
                }
            }
            uploaded_files.push(file_name);
        }
    }

    state.tasks.complete_task(&task_id).await;
    Ok(Json(serde_json::json!({ "success": true, "uploaded": uploaded_files, "task_id": task_id })))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpRange {
    pub start: u64,
    pub end: u64, // inclusive
}

impl HttpRange {
    pub fn parse(range_header: &str, total_len: u64) -> Option<Result<HttpRange, ()>> {
        if total_len == 0 {
            return Some(Err(()));
        }

        let range_str = range_header.trim();
        if !range_str.starts_with("bytes=") {
            return None;
        }

        let spec = &range_str["bytes=".len()..].trim();
        let spec = spec.split(',').next()?.trim();

        if let Some((start_str, end_str)) = spec.split_once('-') {
            let start_str = start_str.trim();
            let end_str = end_str.trim();

            if start_str.is_empty() {
                // Suffix range: bytes=-500 (last 500 bytes)
                if let Ok(suffix_len) = end_str.parse::<u64>() {
                    if suffix_len == 0 {
                        return Some(Err(()));
                    }
                    let actual_len = suffix_len.min(total_len);
                    let start = total_len - actual_len;
                    let end = total_len - 1;
                    return Some(Ok(HttpRange { start, end }));
                }
            } else if end_str.is_empty() {
                // Prefix range: bytes=500-
                if let Ok(start) = start_str.parse::<u64>() {
                    if start >= total_len {
                        return Some(Err(())); // 416 Range Not Satisfiable
                    }
                    let end = total_len - 1;
                    return Some(Ok(HttpRange { start, end }));
                }
            } else {
                // Explicit range: bytes=500-999
                if let (Ok(start), Ok(end)) = (start_str.parse::<u64>(), end_str.parse::<u64>()) {
                    if start > end || start >= total_len {
                        return Some(Err(())); // 416
                    }
                    let end = end.min(total_len - 1);
                    return Some(Ok(HttpRange { start, end }));
                }
            }
        }

        None
    }
}

fn build_bytes_range_response(
    file_bytes: Vec<u8>,
    mime: String,
    disposition: String,
    etag: Option<String>,
    range_header: Option<&str>,
) -> Result<Response, (StatusCode, String)> {
    let total_len = file_bytes.len() as u64;

    if let Some(range_raw) = range_header {
        if let Some(range_res) = HttpRange::parse(range_raw, total_len) {
            match range_res {
                Ok(range) => {
                    let start = range.start as usize;
                    let end = (range.end as usize).min(file_bytes.len().saturating_sub(1));
                    let slice = if start <= end && start < file_bytes.len() {
                        file_bytes[start..=end].to_vec()
                    } else {
                        Vec::new()
                    };
                    let slice_len = slice.len();

                    let mut builder = Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, mime)
                        .header(header::CONTENT_DISPOSITION, disposition)
                        .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, total_len))
                        .header(header::CONTENT_LENGTH, slice_len.to_string())
                        .header(header::ACCEPT_RANGES, "bytes");

                    if let Some(et) = etag {
                        builder = builder
                            .header(header::ETAG, et)
                            .header(header::CACHE_CONTROL, "public, max-age=86400, must-revalidate");
                    }

                    return builder
                        .body(Body::from(slice))
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)));
                }
                Err(()) => {
                    return Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(header::CONTENT_RANGE, format!("bytes */{}", total_len))
                        .header(header::ACCEPT_RANGES, "bytes")
                        .body(Body::empty())
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)));
                }
            }
        }
    }

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, total_len.to_string())
        .header(header::ACCEPT_RANGES, "bytes");

    if let Some(et) = etag {
        builder = builder
            .header(header::ETAG, et)
            .header(header::CACHE_CONTROL, "public, max-age=86400, must-revalidate");
    }

    builder
        .body(Body::from(file_bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))
}

async fn build_local_file_range_response(
    path: &Path,
    total_len: u64,
    mime: String,
    disposition: String,
    etag: String,
    range_header: Option<&str>,
) -> Result<Response, (StatusCode, String)> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    if let Some(range_raw) = range_header {
        if let Some(range_res) = HttpRange::parse(range_raw, total_len) {
            match range_res {
                Ok(range) => {
                    let slice_len = range.end - range.start + 1;
                    let mut file = tokio::fs::File::open(path).await.map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open file: {}", e))
                    })?;

                    file.seek(std::io::SeekFrom::Start(range.start)).await.map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to seek file: {}", e))
                    })?;

                    let mut buffer = vec![0u8; slice_len as usize];
                    file.read_exact(&mut buffer).await.map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file slice: {}", e))
                    })?;

                    let response = Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, mime)
                        .header(header::CONTENT_DISPOSITION, disposition)
                        .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", range.start, range.end, total_len))
                        .header(header::CONTENT_LENGTH, slice_len.to_string())
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::ETAG, etag)
                        .header(header::CACHE_CONTROL, "public, max-age=86400, must-revalidate")
                        .body(Body::from(buffer))
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

                    return Ok(response);
                }
                Err(()) => {
                    let response = Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(header::CONTENT_RANGE, format!("bytes */{}", total_len))
                        .header(header::ACCEPT_RANGES, "bytes")
                        .body(Body::empty())
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

                    return Ok(response);
                }
            }
        }
    }

    let file_bytes = tokio::fs::read(path).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file: {}", e))
    })?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, total_len.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "public, max-age=86400, must-revalidate")
        .body(Body::from(file_bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

    Ok(response)
}

async fn handle_download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, (StatusCode, String)> {
    let raw_path = query.get("path").ok_or((StatusCode::BAD_REQUEST, "Missing path param".to_string()))?;

    let mut modified_headers = headers.clone();
    if !modified_headers.contains_key(header::AUTHORIZATION) {
        if let Some(tok) = query.get("token").or_else(|| query.get("auth")) {
            if let Ok(hv) = header::HeaderValue::from_str(&format!("Bearer {}", tok)) {
                modified_headers.insert(header::AUTHORIZATION, hv);
            }
        }
    }

    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

    let path_str = validate_path_access(&state, &modified_headers, raw_path, false)?;

    if path_str.starts_with("vault://") {
        let rest = path_str.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        let file_res = state.vaults.read_vault_file(vault_file, subpath)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

        use base64::Engine;
        let file_bytes = if file_res.is_binary && file_res.content.starts_with("data:application/octet-stream;base64,") {
            let b64 = file_res.content.strip_prefix("data:application/octet-stream;base64,").unwrap();
            base64::engine::general_purpose::STANDARD.decode(b64).unwrap_or_default()
        } else {
            file_res.content.into_bytes()
        };

        let file_name = file_res.name;
        let mime = file_res.mime_type;
        let is_media_type = mime.starts_with("image/")
            || mime.starts_with("video/")
            || mime.starts_with("audio/")
            || mime == "application/pdf"
            || mime.starts_with("text/");
        let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

        let disposition = if is_inline {
            format!("inline; filename=\"{}\"", file_name)
        } else {
            format!("attachment; filename=\"{}\"", file_name)
        };

        return build_bytes_range_response(file_bytes, mime, disposition, None, range_header);
    } else if path_str.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&path_str, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        let file_bytes = crate::vfs::smb::SmbClient::read_bytes(&params)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to download SMB file: {}", e)))?;
        let file_name = params.subpath.rsplit('/').next().unwrap_or(&params.subpath).to_string();
        let mime = mime_guess::from_path(&file_name).first_or_octet_stream().to_string();
        let is_media_type = mime.starts_with("image/")
            || mime.starts_with("video/")
            || mime.starts_with("audio/")
            || mime == "application/pdf"
            || mime.starts_with("text/");
        let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

        let disposition = if is_inline {
            format!("inline; filename=\"{}\"", file_name)
        } else {
            format!("attachment; filename=\"{}\"", file_name)
        };

        return build_bytes_range_response(file_bytes, mime, disposition, None, range_header);
    } else if path_str.starts_with("sftp://") {
        let params = SftpClient::parse_uri(&path_str, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?;
        let file_bytes = SftpClient::download_file(&params.host, params.port, &params.user, params.password.as_deref(), &params.remote_path, 0)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to download SFTP file: {}", e)))?;
        let file_name = params.remote_path.rsplit('/').next().unwrap_or(&params.remote_path).to_string();
        let mime = mime_guess::from_path(&file_name).first_or_octet_stream().to_string();
        let is_media_type = mime.starts_with("image/")
            || mime.starts_with("video/")
            || mime.starts_with("audio/")
            || mime == "application/pdf"
            || mime.starts_with("text/");
        let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

        let disposition = if is_inline {
            format!("inline; filename=\"{}\"", file_name)
        } else {
            format!("attachment; filename=\"{}\"", file_name)
        };

        return build_bytes_range_response(file_bytes, mime, disposition, None, range_header);
    }

    let path = Path::new(&path_str);

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "File or folder not found".to_string()));
    }

    if path.is_dir() {
        let temp_zip = tempfile::Builder::new()
            .prefix("brum_folder_")
            .suffix(".zip")
            .tempfile()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Temp file error: {}", e)))?;
        let temp_path = temp_zip.path().to_str().unwrap().to_string();

        ArchiveHandler::create_zip(&[path_str.clone()], &temp_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Zip creation failed: {}", e)))?;

        let file_bytes = tokio::fs::read(&temp_path).await.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read generated zip: {}", e))
        })?;

        let dir_name = path.file_name().unwrap_or_default().to_string_lossy();
        let zip_name = if dir_name.is_empty() { "folder.zip".to_string() } else { format!("{}.zip", dir_name) };

        let response = Response::builder()
            .header(header::CONTENT_TYPE, "application/zip")
            .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", zip_name))
            .header(header::CONTENT_LENGTH, file_bytes.len().to_string())
            .body(Body::from(file_bytes))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

        return Ok(response);
    }

    let metadata = tokio::fs::metadata(path).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file metadata: {}", e))
    })?;

    let mtime_sec = metadata.modified().ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let etag = format!("\"{:x}-{:x}\"", metadata.len(), mtime_sec);

    if let Some(inm) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if inm == etag || inm == "*" {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, etag)
                .header(header::CACHE_CONTROL, "public, max-age=86400, must-revalidate")
                .body(Body::empty())
                .unwrap());
        }
    }

    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    let is_media_type = mime.starts_with("image/")
        || mime.starts_with("video/")
        || mime.starts_with("audio/")
        || mime == "application/pdf"
        || mime.starts_with("text/");
    let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

    let disposition = if is_inline {
        format!("inline; filename=\"{}\"", file_name)
    } else {
        format!("attachment; filename=\"{}\"", file_name)
    };

    build_local_file_range_response(path, metadata.len(), mime, disposition, etag, range_header).await
}

#[derive(Deserialize)]
struct BatchDownloadRequest {
    paths: Vec<String>,
}

async fn handle_download_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchDownloadRequest>,
) -> Result<Response, (StatusCode, String)> {
    if payload.paths.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No paths specified for download".to_string()));
    }

    let mut validated_paths = Vec::new();
    for p in &payload.paths {
        validated_paths.push(validate_path_access(&state, &headers, p, false)?);
    }

    let temp_zip = tempfile::Builder::new()
        .prefix("brum_batch_")
        .suffix(".zip")
        .tempfile()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Temp file error: {}", e)))?;
    let temp_path = temp_zip.path().to_str().unwrap().to_string();

    ArchiveHandler::create_zip(&validated_paths, &temp_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Batch zip creation failed: {}", e)))?;

    let file_bytes = std::fs::read(&temp_path).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read generated zip: {}", e))
    })?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let zip_name = if validated_paths.len() == 1 {
        let p = Path::new(&validated_paths[0]);
        let base = p.file_name().unwrap_or_default().to_string_lossy();
        format!("{}.zip", base)
    } else {
        format!("brum_download_{}.zip", timestamp)
    };

    let response = Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", zip_name))
        .body(Body::from(file_bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

    Ok(response)
}

#[derive(Deserialize)]
struct ArchiveCreateRequest {
    sources: Vec<String>,
    target_path: String,
    format: String,
}

async fn handle_archive_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ArchiveCreateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut validated_sources = Vec::new();
    for s in &payload.sources {
        validated_sources.push(validate_path_access(&state, &headers, s, false)?);
    }
    let validated_target = validate_path_access(&state, &headers, &payload.target_path, true)?;

    match payload.format.to_lowercase().as_str() {
        "zip" => ArchiveHandler::create_zip(&validated_sources, &validated_target)
            .map(|_| Json(serde_json::json!({ "success": true, "archive": validated_target })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Zip failed: {}", e))),
        "targz" | "tar.gz" => ArchiveHandler::create_targz(&validated_sources, &validated_target)
            .map(|_| Json(serde_json::json!({ "success": true, "archive": validated_target })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Tar.gz failed: {}", e))),
        _ => Err((StatusCode::BAD_REQUEST, "Unsupported archive format".to_string())),
    }
}

#[derive(Deserialize)]
struct ArchiveExtractRequest {
    archive_path: String,
    target_dir: String,
}

async fn handle_archive_extract(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ArchiveExtractRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_archive = validate_path_access(&state, &headers, &payload.archive_path, false)?;
    let valid_target = validate_path_access(&state, &headers, &payload.target_dir, true)?;

    ArchiveHandler::extract_archive(&valid_archive, &valid_target)
        .map(|_| Json(serde_json::json!({ "success": true })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Extract failed: {}", e)))
}

#[derive(Deserialize)]
struct ChecksumRequest {
    path: String,
    algorithm: Option<String>,
}

async fn handle_calculate_checksum(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChecksumRequest>,
) -> Result<Json<crate::vfs::checksum::ChecksumResult>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, false)?;
    let algo = payload.algorithm.unwrap_or_else(|| "sha256".to_string());
    calculate_checksum(&valid_path, &algo)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Checksum failed: {}", e)))
}

// ---------------- TOOLS & DIFF HANDLERS ----------------

#[derive(Deserialize)]
struct DiffFilesRequest {
    file_left: String,
    file_right: String,
}

async fn handle_diff_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DiffFilesRequest>,
) -> Result<Json<crate::tools::diff::FileDiffResult>, (StatusCode, String)> {
    let left = validate_path_access(&state, &headers, &payload.file_left, false)?;
    let right = validate_path_access(&state, &headers, &payload.file_right, false)?;

    let text_l = std::fs::read_to_string(&left)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Cannot read left file {}: {}", left, e)))?;
    let text_r = std::fs::read_to_string(&right)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Cannot read right file {}: {}", right, e)))?;

    let diff = compare_files_text(&left, &text_l, &right, &text_r);
    Ok(Json(diff))
}

#[derive(Deserialize)]
struct DiffFoldersRequest {
    dir_left: String,
    dir_right: String,
    recursive: Option<bool>,
    deep_hash: Option<bool>,
    selected_items: Option<Vec<String>>,
}

async fn handle_diff_folders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DiffFoldersRequest>,
) -> Result<Json<crate::tools::diff::FolderDiffResult>, (StatusCode, String)> {
    let left = validate_path_access(&state, &headers, &payload.dir_left, false)?;
    let right = validate_path_access(&state, &headers, &payload.dir_right, false)?;

    let recursive = payload.recursive.unwrap_or(false);
    let deep = payload.deep_hash.unwrap_or(false);
    crate::tools::diff::compare_folders(&left, &right, recursive, deep, payload.selected_items)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Folder diff failed: {}", e)))
}

async fn handle_convert_file(
    Json(payload): Json<crate::tools::converter::ConvertRequest>,
) -> Result<Json<crate::tools::converter::ConvertResponse>, (StatusCode, String)> {
    crate::tools::converter::ConvertEngine::convert_file(&payload)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Conversion failed: {}", e)))
}

#[derive(Deserialize)]
struct DryRunRequest {
    action: String,
    sources: Vec<String>,
    destination: Option<String>,
}

async fn handle_paranoid_dry_run(
    Json(payload): Json<DryRunRequest>,
) -> Result<Json<crate::tools::paranoid::DryRunPreview>, (StatusCode, String)> {
    ParanoidEngine::dry_run(&payload.action, &payload.sources, payload.destination.as_deref())
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Dry run failed: {}", e)))
}

// ---------------- NOTEDOG CHEWTOY HANDLERS ----------------

#[derive(Deserialize)]
struct NoteDogInfoQuery {
    folder: Option<String>,
}

async fn handle_notedog_info(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<NoteDogInfoQuery>,
) -> Result<Json<crate::tools::notedog::NoteDogInfo>, (StatusCode, String)> {
    let cfg_folder = state.config.notedog.notes_folder.as_str();
    let info = crate::tools::notedog::scan_notedog_hierarchy(query.folder.as_deref(), Some(cfg_folder));
    Ok(Json(info))
}

async fn handle_notedog_templates() -> Json<Vec<crate::tools::notedog::NoteTemplate>> {
    Json(crate::tools::notedog::get_builtin_templates())
}

#[derive(Deserialize)]
struct NoteDogVersionsQuery {
    path: String,
}

async fn handle_notedog_versions(
    axum::extract::Query(query): axum::extract::Query<NoteDogVersionsQuery>,
) -> Json<Vec<crate::tools::notedog::NoteVersionItem>> {
    let p = Path::new(&query.path);
    Json(crate::tools::notedog::list_note_versions(p))
}

#[derive(Deserialize)]
struct NoteDogSaveVersionRequest {
    path: String,
}

async fn handle_notedog_save_version(
    Json(payload): Json<NoteDogSaveVersionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    if p.exists() {
        if let Ok(bytes) = std::fs::read(p) {
            let _ = crate::tools::notedog::create_note_snapshot(p, &bytes);
        }
    }
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
struct NoteDogDecryptRequest {
    path: String,
    passphrase: Option<String>,
}

#[derive(Serialize)]
struct NoteDogDecryptResponse {
    content: String,
    path: String,
}

async fn handle_notedog_decrypt(
    Json(payload): Json<NoteDogDecryptRequest>,
) -> Result<Json<NoteDogDecryptResponse>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    if !p.exists() {
        return Err((StatusCode::NOT_FOUND, "Note file not found".to_string()));
    }
    let pass = payload.passphrase.as_deref().unwrap_or("notedog");
    match crate::tools::notedog::decrypt_note_file(p, pass) {
        Ok(content) => Ok(Json(NoteDogDecryptResponse {
            content,
            path: payload.path,
        })),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

#[derive(Deserialize)]
struct NoteDogEncryptRequest {
    path: String,
    content: String,
    passphrase: Option<String>,
    convert_to_plain: Option<bool>,
}

#[derive(Serialize)]
struct NoteDogEncryptResponse {
    success: bool,
    path: String,
    is_encrypted: bool,
}

async fn handle_notedog_encrypt(
    Json(payload): Json<NoteDogEncryptRequest>,
) -> Result<Json<NoteDogEncryptResponse>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    let pass = payload.passphrase.as_deref().unwrap_or("notedog");

    if payload.convert_to_plain.unwrap_or(false) {
        match crate::tools::notedog::decrypt_and_save_plain_note(p, pass) {
            Ok(new_path) => Ok(Json(NoteDogEncryptResponse {
                success: true,
                path: new_path.to_string_lossy().to_string(),
                is_encrypted: false,
            })),
            Err(err) => Err((StatusCode::BAD_REQUEST, err)),
        }
    } else {
        match crate::tools::notedog::encrypt_and_save_note(p, &payload.content, pass) {
            Ok(new_path) => Ok(Json(NoteDogEncryptResponse {
                success: true,
                path: new_path.to_string_lossy().to_string(),
                is_encrypted: true,
            })),
            Err(err) => Err((StatusCode::BAD_REQUEST, err)),
        }
    }
}

#[derive(Deserialize)]
struct NoteDogCreateNoteRequest {
    section_path: String,
    title: String,
    is_encrypted: Option<bool>,
    passphrase: Option<String>,
    initial_content: Option<String>,
}

async fn handle_notedog_create_note(
    Json(payload): Json<NoteDogCreateNoteRequest>,
) -> Result<Json<crate::tools::notedog::NoteDogFile>, (StatusCode, String)> {
    let sec_p = Path::new(&payload.section_path);
    match crate::tools::notedog::create_new_note(
        sec_p,
        &payload.title,
        payload.is_encrypted.unwrap_or(false),
        payload.passphrase.as_deref(),
        payload.initial_content.as_deref(),
    ) {
        Ok(note) => Ok(Json(note)),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

#[derive(Deserialize)]
struct NoteDogSectionActionRequest {
    path: String,
    passphrase: String,
    cached_pass: Option<String>,
}

async fn handle_notedog_section_encrypt(
    Json(payload): Json<NoteDogSectionActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    match crate::tools::notedog::encrypt_section_dir(p, &payload.passphrase, payload.cached_pass.as_deref()) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "encrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

async fn handle_notedog_section_decrypt(
    Json(payload): Json<NoteDogSectionActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    match crate::tools::notedog::decrypt_section_dir(p, &payload.passphrase) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "decrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

#[derive(Deserialize)]
struct NoteDogNotebookActionRequest {
    path: String,
    passphrase: String,
    cached_pass: Option<String>,
}

async fn handle_notedog_notebook_encrypt(
    Json(payload): Json<NoteDogNotebookActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    match crate::tools::notedog::encrypt_notebook_dir(p, &payload.passphrase, payload.cached_pass.as_deref()) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "encrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

async fn handle_notedog_notebook_decrypt(
    Json(payload): Json<NoteDogNotebookActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    match crate::tools::notedog::decrypt_notebook_dir(p, &payload.passphrase) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "decrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

// ---------------- TETRADOG CLASSIC ARCADE CHEWTOY & LEADERBOARD HANDLERS ----------------

#[derive(Deserialize)]
struct TetraDogScoresQuery {
    limit: Option<usize>,
    mode: Option<String>,
    user_only: Option<bool>,
}

async fn handle_tetradog_get_scores(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TetraDogScoresQuery>,
) -> Result<Json<crate::tools::tetradog::TetraLeaderboardResponse>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;
    let limit = query.limit.unwrap_or(50);
    let mode = query.mode.as_deref();
    let user_only = query.user_only.unwrap_or(false);

    let resp = crate::tools::tetradog::get_leaderboard(&conn, &claims.sub, limit, mode, user_only)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

async fn handle_tetradog_submit_score(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::tetradog::SubmitScoreRequest>,
) -> Result<Json<crate::tools::tetradog::SubmitScoreResponse>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;

    let resp = crate::tools::tetradog::submit_score(&conn, &claims.sub, payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

async fn handle_tetradog_clear_scores(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;

    let user_filter = if claims.role == "admin" { None } else { Some(claims.sub.as_str()) };
    let cleared = crate::tools::tetradog::clear_scores(&conn, user_filter)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "success": true, "cleared_count": cleared })))
}

// ---------------- PHASE 4 HANDLERS (SYNC, SEARCH, SCRIPT ACTIONS) ----------------

#[derive(Deserialize)]
struct SyncRequest {
    source: String,
    destination: String,
    options: crate::tools::sync::SyncOptions,
}

async fn handle_sync_analyze(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SyncRequest>,
) -> Result<Json<crate::tools::sync::SyncAnalysis>, (StatusCode, String)> {
    let source = validate_path_access(&state, &headers, &payload.source, false)?;
    let destination = validate_path_access(&state, &headers, &payload.destination, false)?;

    crate::tools::sync::DirectorySyncEngine::analyze(&source, &destination, &payload.options)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Sync analysis failed: {}", e)))
}

async fn handle_sync_execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SyncRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = validate_path_access(&state, &headers, &payload.source, false)?;
    let destination = validate_path_access(&state, &headers, &payload.destination, true)?;

    crate::tools::sync::DirectorySyncEngine::execute_sync(
        state.tasks.clone(),
        &source,
        &destination,
        payload.options,
    ).await
    .map(Json)
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Sync execution failed: {}", e)))
}

async fn handle_list_backup_profiles(
    State(state): State<AppState>,
) -> Json<Vec<crate::tools::sync::BackupProfile>> {
    Json(state.backup.list_profiles())
}

async fn handle_save_backup_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(profile): Json<crate::tools::sync::BackupProfile>,
) -> Result<Json<crate::tools::sync::BackupProfile>, (StatusCode, String)> {
    validate_path_access(&state, &headers, &profile.source_dir, false)?;
    validate_path_access(&state, &headers, &profile.dest_dir, true)?;

    state.backup.save_profile(profile)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save backup profile: {}", e)))
}

async fn handle_delete_backup_profile(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state.backup.delete_profile(&id)
        .map(|_| Json(serde_json::json!({ "success": true })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete backup profile: {}", e)))
}

async fn handle_run_backup_profile(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mgr = state.backup.clone();
    let tasks = state.tasks.clone();
    
    // Run in background Tokio task
    tokio::spawn(async move {
        if let Err(e) = mgr.execute_profile_job(&id, tasks).await {
            tracing::error!("Manual backup profile execution error for '{}': {}", id, e);
        }
    });

    Ok(Json(serde_json::json!({ "success": true, "message": "Backup job initiated in background" })))
}

async fn handle_toggle_backup_profile(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state.backup.toggle_profile(&id)
        .map(|new_enabled| Json(serde_json::json!({ "success": true, "enabled": new_enabled })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to toggle backup profile: {}", e)))
}

async fn handle_get_backup_history(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Vec<crate::tools::sync::BackupHistoryItem>> {
    let profile_id = query.get("profile_id").map(|s| s.as_str());
    let limit = query.get("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(50);
    Json(state.backup.list_history(profile_id, limit))
}

async fn handle_disk_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<crate::tools::disk_usage::DiskUsageReport>, (StatusCode, String)> {
    let raw_path = query.get("path").ok_or((StatusCode::BAD_REQUEST, "Missing path param".to_string()))?;
    let path = validate_path_access(&state, &headers, raw_path, false)?;

    tokio::task::spawn_blocking(move || {
        crate::tools::disk_usage::DiskUsageEngine::analyze(&path)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Task join error: {}", e)))?
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, format!("Disk usage analysis failed: {}", e)))
}

async fn handle_get_disks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::tools::disk_usage::DiskMountInfo>>, (StatusCode, String)> {
    let _claims = extract_claims_or_local(&state, &headers)?;
    let roots = state.config.storage.roots.clone();
    let disks = tokio::task::spawn_blocking(move || {
        crate::tools::disk_usage::get_system_disks(&roots)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Disk enumeration failed: {}", e)))?;

    Ok(Json(disks))
}

async fn handle_search(
    Json(payload): Json<crate::tools::search::SearchRequest>,
) -> Result<Json<Vec<crate::tools::search::SearchResultItem>>, (StatusCode, String)> {
    crate::tools::search::SearchEngine::search(payload)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Search failed: {}", e)))
}

async fn handle_run_action(
    Json(payload): Json<crate::tools::actions::ActionExecutionRequest>,
) -> Result<Json<crate::tools::actions::ActionExecutionResult>, (StatusCode, String)> {
    crate::tools::actions::ActionRunner::execute(payload).await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Action execution error: {}", e)))
}

async fn handle_syncthing_status(
    State(state): State<AppState>,
) -> Json<crate::tools::syncthing::SyncthingStatusResponse> {
    let client = crate::tools::syncthing::SyncthingClient::new(state.config.syncthing.clone());
    Json(client.get_status().await)
}

#[derive(Deserialize)]
struct SyncthingScanRequest {
    folder_id: Option<String>,
    subpath: Option<String>,
}

async fn handle_syncthing_scan(
    State(state): State<AppState>,
    Json(payload): Json<SyncthingScanRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let client = crate::tools::syncthing::SyncthingClient::new(state.config.syncthing.clone());
    match client.trigger_scan(payload.folder_id.as_deref(), payload.subpath.as_deref()).await {
        Ok(msg) => Ok(Json(serde_json::json!({ "success": true, "message": msg }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e)),
    }
}

// ---------------- CONFIG & SYSTEM HANDLERS ----------------

async fn handle_get_config(
    State(state): State<AppState>,
) -> Json<AppConfig> {
    Json((*state.config).clone())
}

#[derive(Serialize)]
struct SystemUsersGroups {
    users: Vec<String>,
    groups: Vec<String>,
}

async fn handle_get_system_users_groups() -> Json<SystemUsersGroups> {
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

    Json(SystemUsersGroups { users, groups })
}

#[derive(Serialize)]
struct ConfigFileResponse {
    path: String,
    content: String,
    is_writable: bool,
}

#[derive(Deserialize)]
struct SaveConfigFileRequest {
    content: String,
}

fn resolve_active_config_path() -> PathBuf {
    let candidate_paths = vec![
        dirs::config_dir().map(|d| d.join("brum").join("config.toml")),
        dirs::config_dir().map(|d| d.join("commanderdog").join("config.toml")),
        Some(PathBuf::from("./brum.toml")),
        Some(PathBuf::from("./config.toml")),
        Some(PathBuf::from("/etc/brum/config.toml")),
        Some(PathBuf::from("/etc/commanderdog/config.toml")),
    ];

    for candidate in candidate_paths.into_iter().flatten() {
        if candidate.is_file() {
            return candidate;
        }
    }

    if let Some(user_config) = dirs::config_dir().map(|d| d.join("brum").join("config.toml")) {
        user_config
    } else if let Some(legacy_config) = dirs::config_dir().map(|d| d.join("commanderdog").join("config.toml")) {
        legacy_config
    } else {
        PathBuf::from("./config.toml")
    }
}

async fn handle_get_config_file(
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

async fn handle_save_config_file(
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

async fn handle_reload_config(
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

async fn handle_system_restart(
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
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
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
            if let Ok(exe) = std::env::current_exe() {
                let args: Vec<String> = std::env::args().skip(1).collect();
                let _ = std::process::Command::new(exe).args(args).spawn();
            }
        }
        std::process::exit(0);
    });

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Brum server is restarting..."
    })))
}

// ---------------- GIT & VERSION CONTROL HANDLERS ----------------

#[derive(Deserialize)]
struct GitPathQuery {
    path: String,
}

#[derive(Deserialize)]
struct GitDiffQuery {
    path: String,
    file: Option<String>,
    #[serde(default)]
    staged: Option<bool>,
}

#[derive(Deserialize)]
struct GitLogQuery {
    path: String,
    #[serde(default)]
    count: Option<usize>,
}

#[derive(Deserialize)]
struct GitFilesRequest {
    path: String,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Deserialize)]
struct GitCommitRequest {
    path: String,
    message: String,
}

#[derive(Deserialize)]
struct GitPushRequest {
    path: String,
    remote: Option<String>,
    branch: Option<String>,
}

#[derive(Deserialize)]
struct GitPullRequest {
    path: String,
}

async fn handle_git_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitPathQuery>,
) -> Result<Json<crate::tools::git::GitStatusResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::get_git_status(Path::new(&query.path)).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

async fn handle_git_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitDiffQuery>,
) -> Result<Json<crate::tools::git::GitDiffResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::get_git_diff(Path::new(&query.path), query.file.as_deref(), query.staged.unwrap_or(false)).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

async fn handle_git_stage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitFilesRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_stage(Path::new(&payload.path), &payload.files).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

async fn handle_git_unstage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitFilesRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_unstage(Path::new(&payload.path), &payload.files).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

async fn handle_git_commit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitCommitRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_commit(Path::new(&payload.path), &payload.message).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

async fn handle_git_push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitPushRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_push(Path::new(&payload.path), payload.remote.as_deref(), payload.branch.as_deref()).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

async fn handle_git_pull(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitPullRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_pull(Path::new(&payload.path)).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

async fn handle_git_log(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitLogQuery>,
) -> Result<Json<Vec<crate::tools::git::GitCommitInfo>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::get_git_log(Path::new(&query.path), query.count.unwrap_or(30)).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

// ---------------- FILE SPLITTER & COMBINER HANDLERS ----------------

async fn handle_split_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::splitter::SplitRequest>,
) -> Result<Json<crate::tools::splitter::SplitResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let source_path = Path::new(&payload.source_path);
        let dest_dir = payload.dest_dir.as_ref().map(|p| Path::new(p));
        let chunk_size = payload.chunk_size_mb.max(1) * 1024 * 1024;
        let gen_chk = payload.generate_checksum.unwrap_or(true);

        crate::tools::splitter::split_file_sync(source_path, dest_dir, chunk_size, gen_chk)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

async fn handle_combine_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::splitter::CombineRequest>,
) -> Result<Json<crate::tools::splitter::CombineResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let parts: Vec<std::path::PathBuf> = payload.parts.iter().map(std::path::PathBuf::from).collect();
        let dest_path = Path::new(&payload.dest_path);

        crate::tools::splitter::combine_files_sync(&parts, dest_path, payload.expected_sha256.as_deref())
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

// ---------------- STATIC ASSET EMBEDDED HANDLER ----------------

async fn handle_static_asset(headers: HeaderMap, uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file_path = if path.is_empty() { "index.html" } else { path };

    let is_font_or_media = file_path.starts_with("assets/fonts/")
        || file_path.ends_with(".woff2")
        || file_path.ends_with(".woff")
        || file_path.ends_with(".ttf")
        || file_path.ends_with(".svg")
        || file_path.ends_with(".png")
        || file_path.ends_with(".ico");

    let cache_control = if is_font_or_media {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
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

// ---------------- PDF TOOL HANDLERS ----------------

#[derive(Deserialize)]
struct PdfInfoQuery {
    path: String,
}

async fn handle_pdf_info(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PdfInfoQuery>,
) -> Result<Json<crate::tools::pdf::PdfInfo>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &query.path, false)?;
    let local_path = crate::vfs::local::LocalFs::resolve_local_path(&valid_path);
    crate::tools::pdf::PdfEngine::get_info(&local_path)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

async fn handle_pdf_merge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::pdf::MergeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut local_sources = Vec::new();
    for src in &payload.sources {
        let valid_src = validate_path_access(&state, &headers, src, false)?;
        local_sources.push(crate::vfs::local::LocalFs::resolve_local_path(&valid_src));
    }
    let valid_dst = validate_path_access(&state, &headers, &payload.destination, true)?;
    let local_dst = crate::vfs::local::LocalFs::resolve_local_path(&valid_dst);

    crate::tools::pdf::PdfEngine::merge(&local_sources, &local_dst, payload.add_bookmarks.unwrap_or(true))
        .map(|_| Json(serde_json::json!({ "success": true, "destination": valid_dst })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

async fn handle_pdf_split(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::pdf::SplitRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_src = validate_path_access(&state, &headers, &payload.source, false)?;
    let local_src = crate::vfs::local::LocalFs::resolve_local_path(&valid_src);

    let valid_dst_dir = validate_path_access(&state, &headers, &payload.destination_dir, true)?;
    let local_dst_dir = crate::vfs::local::LocalFs::resolve_local_path(&valid_dst_dir);

    crate::tools::pdf::PdfEngine::split(
        &local_src,
        &local_dst_dir,
        &payload.split_mode,
        payload.page_ranges.as_deref(),
        payload.chunk_size,
        payload.output_prefix.as_deref(),
    )
    .map(|files| {
        let files_str: Vec<String> = files.iter().map(|p| p.to_string_lossy().to_string()).collect();
        Json(serde_json::json!({ "success": true, "files": files_str }))
    })
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

async fn handle_pdf_reorder(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::pdf::PageReorderRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_src = validate_path_access(&state, &headers, &payload.source, false)?;
    let local_src = crate::vfs::local::LocalFs::resolve_local_path(&valid_src);

    let valid_dst = validate_path_access(&state, &headers, &payload.destination, true)?;
    let local_dst = crate::vfs::local::LocalFs::resolve_local_path(&valid_dst);

    crate::tools::pdf::PdfEngine::reorder_and_rotate(&local_src, &payload.pages, &local_dst)
        .map(|_| Json(serde_json::json!({ "success": true, "destination": valid_dst })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

// ---------------- SYSTEM AUTOSTART HANDLERS ----------------

#[derive(Serialize, Deserialize)]
struct AutostartStatus {
    enabled: bool,
    platform: String,
    target_path: Option<String>,
}

#[derive(Deserialize)]
struct SetAutostartRequest {
    enabled: bool,
    minimized: Option<bool>,
}

async fn handle_get_autostart() -> Json<AutostartStatus> {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("reg")
            .args(["query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "Brum"])
            .output();
        let enabled = output.map_or(false, |o| o.status.success());
        Json(AutostartStatus {
            enabled,
            platform: "windows".to_string(),
            target_path: std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()),
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let autostart_file = dirs::config_dir()
            .map(|c| c.join("autostart/brum.desktop"));
        let enabled = autostart_file.as_ref().map_or(false, |p| p.exists());
        Json(AutostartStatus {
            enabled,
            platform: std::env::consts::OS.to_string(),
            target_path: autostart_file.map(|p| p.to_string_lossy().to_string()),
        })
    }
}

async fn handle_set_autostart(
    Json(payload): Json<SetAutostartRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
struct OpenWithRequest {
    file_path: String,
    command: Option<String>,
}

async fn handle_open_with(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<OpenWithRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
struct RunCustomActionRequest {
    command: String,
    target_path: String,
    selection: Option<Vec<String>>,
    target_pane_path: Option<String>,
}

async fn handle_run_custom_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RunCustomActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_normalize_path_resolution() {
        assert_eq!(normalize_path(Path::new("/home/user/../user/docs")), Path::new("/home/user/docs"));
        assert_eq!(normalize_path(Path::new("/var/log/../../etc/passwd")), Path::new("/etc/passwd"));
        assert_eq!(normalize_path(Path::new("/")), Path::new("/"));
        #[cfg(windows)]
        {
            assert_eq!(
                normalize_path(Path::new(r"\\?\C:\Users\Bolt\Documents")),
                Path::new(r"C:\Users\Bolt\Documents")
            );
            assert_eq!(
                normalize_path(Path::new(r"\\?\UNC\server\share\data")),
                Path::new(r"\\server\share\data")
            );
        }
        #[cfg(not(windows))]
        {
            assert_eq!(
                normalize_path(Path::new(r"\\?\C:\Users\Bolt\Documents")),
                Path::new("C:/Users/Bolt/Documents")
            );
            assert_eq!(
                normalize_path(Path::new(r"\\?\UNC\server\share\data")),
                Path::new("//server/share/data")
            );
        }
    }

    #[test]
    fn test_path_starts_with_case_insensitive() {
        assert!(path_starts_with_case_insensitive(
            Path::new("C:/Users/Bolt/Documents/file.txt"),
            Path::new("c:/users/bolt")
        ));
        assert!(path_starts_with_case_insensitive(
            Path::new(r"\\?\C:\Users\Bolt\Documents"),
            Path::new(r"c:\users\bolt")
        ));
        assert!(path_starts_with_case_insensitive(
            Path::new("/home/bolt/projects"),
            Path::new("/home/bolt")
        ));
        assert!(!path_starts_with_case_insensitive(
            Path::new("/home/other/projects"),
            Path::new("/home/bolt")
        ));
    }

    #[tokio::test]
    async fn test_handle_static_asset_etags() {
        let headers = HeaderMap::new();
        let uri: axum::http::Uri = "/index.html".parse().unwrap();
        let res = handle_static_asset(headers, uri).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().get(header::ETAG).is_some());
        let etag = res.headers().get(header::ETAG).unwrap().to_str().unwrap().to_string();

        let mut conditional_headers = HeaderMap::new();
        conditional_headers.insert(header::IF_NONE_MATCH, etag.parse().unwrap());
        let cond_uri: axum::http::Uri = "/index.html".parse().unwrap();
        let cond_res = handle_static_asset(conditional_headers, cond_uri).await;
        assert_eq!(cond_res.status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn test_http_range_parsing() {
        let total = 10000;

        // Valid prefix range: bytes=0-499
        let r1 = HttpRange::parse("bytes=0-499", total).unwrap().unwrap();
        assert_eq!(r1, HttpRange { start: 0, end: 499 });

        // Valid open-ended range: bytes=1000-
        let r2 = HttpRange::parse("bytes=1000-", total).unwrap().unwrap();
        assert_eq!(r2, HttpRange { start: 1000, end: 9999 });

        // Valid suffix range: bytes=-500 (last 500 bytes)
        let r3 = HttpRange::parse("bytes=-500", total).unwrap().unwrap();
        assert_eq!(r3, HttpRange { start: 9500, end: 9999 });

        // Clamp end exceeding total length: bytes=9000-20000
        let r4 = HttpRange::parse("bytes=9000-20000", total).unwrap().unwrap();
        assert_eq!(r4, HttpRange { start: 9000, end: 9999 });

        // Out of bounds range: bytes=10000-
        let r5 = HttpRange::parse("bytes=10000-", total).unwrap();
        assert!(r5.is_err());

        // Inverted range: bytes=500-200
        let r6 = HttpRange::parse("bytes=500-200", total).unwrap();
        assert!(r6.is_err());

        // Non-range header
        assert!(HttpRange::parse("gzip, deflate", total).is_none());
    }
}
