pub mod middleware;
pub mod auth_handlers;
pub mod fs_handlers;
pub mod share_handlers;
pub mod tool_handlers;
pub mod system_handlers;
pub mod terminal;
pub mod fleet;

pub use middleware::*;
pub use auth_handlers::*;
pub use fs_handlers::*;
pub use share_handlers::*;
pub use tool_handlers::*;
pub use system_handlers::*;
pub use terminal::*;
pub use fleet::*;

use crate::auth::AuthManager;
use crate::config::AppConfig;
use crate::tools::tasks::TaskManager;

use axum::{
    extract::DefaultBodyLimit,
    http::header,
    routing::{any, delete, get, post, put},
    Router,
};
use rust_embed::RustEmbed;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(RustEmbed)]
#[folder = "frontend/"]
pub struct Asset;

#[derive(RustEmbed)]
#[folder = "manuals/"]
pub struct ManualsAsset;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub auth: Arc<AuthManager>,
    pub oidc: Arc<crate::auth::oidc::OidcManager>,
    pub tasks: Arc<TaskManager>,
    pub tags: Arc<crate::tools::tags::TagManager>,
    pub vaults: Arc<crate::vfs::vault::VaultManager>,
    pub backup: Arc<crate::tools::sync::BackupManager>,
    pub plugins: Arc<crate::plugins::PluginManager>,
    pub rate_limiter: Arc<RateLimiter>,
}

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
            axum::http::Method::HEAD,
            axum::http::Method::PATCH,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::COOKIE,
            header::ORIGIN,
            header::RANGE,
            header::HeaderName::from_static("x-csrf-token"),
            header::HeaderName::from_static("x-requested-with"),
        ])
        .expose_headers([
            header::HeaderName::from_static("x-is-directory"),
            header::HeaderName::from_static("content-disposition"),
            header::HeaderName::from_static("content-range"),
            header::HeaderName::from_static("accept-ranges"),
            header::HeaderName::from_static("etag"),
            header::SET_COOKIE,
        ]);

    Router::new()
        // System & Platform Status & Removable Storage
        .route("/api/health", get(handle_health))
        .route("/api/system/status", get(handle_system_status))
        .route("/api/system/exit", post(handle_system_exit))
        .route("/api/system/restart", post(handle_system_restart))
        .route("/api/system/usb", get(handle_list_usb))
        .route("/api/system/usb/mount", post(handle_mount_usb))
        .route("/api/system/usb/unmount", post(handle_unmount_usb))
        .route("/api/system/usb/eject", post(handle_eject_usb))
        // Auth & Security API (OIDC / SSO & Local)
        .route("/api/auth/oidc/config", get(handle_oidc_config))
        .route("/api/auth/oidc/login", get(handle_oidc_login))
        .route("/api/auth/oidc/callback", get(handle_oidc_callback))
        .route("/api/auth/login", post(handle_login))
        .route("/api/auth/logout", post(handle_logout))
        .route("/api/auth/unlock", post(handle_unlock_session))
        .route("/api/auth/security-settings", get(handle_get_security_settings).post(handle_update_security_settings))
        .route("/api/auth/me", get(handle_get_me))
        .route("/api/auth/profile", post(handle_update_profile))
        .route("/api/auth/avatar/:username", get(handle_get_user_avatar))
        .route("/api/auth/users", get(handle_list_users).post(handle_create_user))
        .route("/api/auth/users/:username", delete(handle_delete_user).post(handle_update_user_rbac).put(handle_update_user_rbac))
        .route("/api/auth/tokens", get(handle_list_api_tokens).post(handle_create_api_token))
        .route("/api/auth/tokens/:id", delete(handle_revoke_api_token))
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
        // Public Link Sharing & Advanced Sharing Center
        .route("/share/:token", get(handle_public_share_page))
        .route("/api/shares", get(handle_list_shares).post(handle_create_share))
        .route("/api/shares/:id", put(handle_update_share).delete(handle_delete_share))
        .route("/api/shares/:id/revoke", post(handle_revoke_share))
        .route("/api/shares/:id/logs", get(handle_get_share_logs))
        .route("/api/public/shares/:token", get(handle_public_get_share_meta))
        .route("/api/public/shares/:token/verify", post(handle_public_verify_share))
        .route("/api/public/shares/:token/verify-email", post(handle_public_verify_email))
        .route("/api/public/shares/:token/preview", get(handle_public_preview_share))
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
        // Fleet Manager & Gateway Reverse Proxy
        .route("/api/fleet/nodes", get(fleet::handle_list_fleet_nodes))
        .route("/api/fleet/ping/:node_id", get(fleet::handle_fleet_ping))
        .route("/api/fleet/proxy/:node_id/api/terminal/ws", get(fleet::handle_fleet_ws_proxy))
        .route("/api/fleet/proxy/:node_id/ws", get(fleet::handle_fleet_ws_proxy))
        .route("/api/fleet/proxy/:node_id/*path", any(fleet::handle_fleet_http_proxy))
        .route("/api/fleet/proxy/:node_id", any(fleet::handle_fleet_http_proxy_root))
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
        .route("/api/tools/duplicates/scan", post(handle_duplicates_scan))
        .route("/api/tools/duplicates/clean", post(handle_duplicates_clean))
        .route("/api/tools/metadata/read", get(handle_metadata_read))
        .route("/api/tools/metadata/update", post(handle_metadata_update))
        .route("/api/tools/metadata/batch", post(handle_metadata_batch))
        .route("/api/tools/logviewer/tail", get(handle_logviewer_tail))
        .route("/api/tools/trash/summary", get(handle_trash_summary))
        .route("/api/tools/trash/items", get(handle_trash_items))
        .route("/api/tools/trash/restore", post(handle_trash_restore))
        .route("/api/tools/trash/empty", post(handle_trash_empty))
        .route("/api/tools/trash/delete", post(handle_trash_delete))
        .route("/api/tools/trash/open-native", post(handle_trash_open_native))
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
        // Persistent Database Notes & Attachments Engine
        .route("/api/notes", get(handle_list_db_notes).post(handle_create_db_note))
        .route("/api/notes/:id", get(handle_get_db_note).put(handle_update_db_note).delete(handle_delete_db_note))
        .route("/api/notes/:id/attachments", get(handle_list_db_note_attachments).post(handle_upload_db_note_attachment))
        .route("/api/notes/attachments/upload", post(handle_upload_db_note_attachment_standalone))
        .route("/api/notes/attachments/:attachment_id", get(handle_get_db_note_attachment_binary).delete(handle_delete_db_note_attachment))
        .route("/api/notes/attachments/:attachment_id/:filename", get(handle_get_db_note_attachment_binary_with_name))
        .route("/api/notes/migrate/export", post(handle_notes_migrate_export))
        .route("/api/notes/migrate/import", post(handle_notes_migrate_import))
        // Repository Manuals & Documentation API
        .route("/api/manuals", get(handle_list_manuals))
        .route("/api/manuals/:name", get(handle_get_manual))
        // TetraDog Classic Arcade ChewToy & Leaderboard API
        .route("/api/tools/tetradog/scores", get(handle_tetradog_get_scores).post(handle_tetradog_submit_score).delete(handle_tetradog_clear_scores))
        .route("/api/chewtoys/tetradog/scores", get(handle_tetradog_get_scores).post(handle_tetradog_submit_score))
        // ChewToy Plugins Architecture & Add-ons API
        .route("/api/plugins", get(handle_list_plugins))
        .route("/api/plugins/install", post(handle_install_plugin))
        .route("/api/plugins/:id/toggle", post(handle_toggle_plugin))
        .route("/api/plugins/:id", delete(handle_delete_plugin))
        .route("/api/plugins/:id/assets/*subpath", get(handle_plugin_asset))
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
        .layer(axum::middleware::from_fn(security_headers_middleware))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};

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
            assert_eq!(
                normalize_path(Path::new(r"\\meteorite")),
                Path::new(r"\\meteorite")
            );
            assert_eq!(
                normalize_path(Path::new(r"\\meteorite\")),
                Path::new(r"\\meteorite")
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
            assert_eq!(
                normalize_path(Path::new(r"\\meteorite")),
                Path::new("//meteorite")
            );
            assert_eq!(
                normalize_path(Path::new("//meteorite/")),
                Path::new("//meteorite")
            );
        }
    }

    #[test]
    fn test_is_root_path() {
        assert!(is_root_path(Path::new("/")));
        assert!(is_root_path(Path::new("\\")));
        assert!(is_root_path(Path::new("")));
        assert!(is_root_path(Path::new("C:")));
        assert!(is_root_path(Path::new("C:\\")));
        assert!(is_root_path(Path::new("c:/")));
        assert!(is_root_path(Path::new("D:\\")));
        assert!(!is_root_path(Path::new("/home/user")));
        assert!(!is_root_path(Path::new("C:\\Users\\admin")));
        assert!(!is_root_path(Path::new("C:/Users/admin")));
        assert!(!is_root_path(Path::new("/mnt/storage")));
    }

    #[test]
    fn test_resolve_effective_home_schemes() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("home_schemes_test.db");
        let mut config = crate::config::AppConfig::default();
        config.server.database_path = db_path.to_string_lossy().to_string();
        config.storage.allow_entire_system = false;
        config.storage.roots = vec![crate::config::StorageRoot {
            id: "vault".to_string(),
            name: "Vault".to_string(),
            path: temp.path().to_string_lossy().to_string(),
            read_only: false,
            allowed_roles: vec!["admin".to_string()],
        }];

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
        let auth_arc = std::sync::Arc::new(auth);
        let task_mgr = std::sync::Arc::new(crate::tools::tasks::TaskManager::new());
        let tag_mgr = std::sync::Arc::new(crate::tools::tags::TagManager::new(db.clone()).unwrap());
        let vault_mgr = std::sync::Arc::new(crate::vfs::vault::VaultManager::new());
        let backup_mgr = std::sync::Arc::new(crate::tools::sync::BackupManager::new(db).unwrap());
        let plugin_mgr = std::sync::Arc::new(crate::plugins::PluginManager::new(
            temp.path().join("sys_plugins"),
            temp.path().join("usr_plugins"),
            false,
            "allow_all".to_string(),
            vec!["*".to_string()],
            vec![],
        ));
        let oidc_mgr = std::sync::Arc::new(crate::auth::oidc::OidcManager::new(config.auth.oidc.clone(), auth_arc.clone()));

        let mut state = AppState {
            config: std::sync::Arc::new(config.clone()),
            auth: auth_arc,
            tasks: task_mgr,
            tags: tag_mgr,
            vaults: vault_mgr,
            backup: backup_mgr,
            plugins: plugin_mgr,
            oidc: oidc_mgr,
            rate_limiter: std::sync::Arc::new(RateLimiter::new()),
        };

        let claims = crate::auth::Claims {
            sub: "admin".to_string(),
            role: "admin".to_string(),
            home_dir: "/".to_string(),
            is_pam: false,
            allowed_roots: Some("[\"*\"]".to_string()),
            token_id: None,
            exp: 9999999999,
        };

        // 1. "roots_only" scheme returns None for personal home
        let mut roots_only_config = config.clone();
        roots_only_config.storage.home_fallback_scheme = "roots_only".to_string();
        state.config = std::sync::Arc::new(roots_only_config);
        assert_eq!(resolve_effective_home(&claims, &state), None);

        // 2. "auto" scheme falls back to first accessible storage root if home is root
        let mut auto_config = config.clone();
        auto_config.storage.home_fallback_scheme = "auto".to_string();
        state.config = std::sync::Arc::new(auto_config);
        let effective = resolve_effective_home(&claims, &state);
        assert!(effective.is_some());
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

    #[tokio::test]
    async fn test_handle_health_endpoint() {
        use crate::config::AppConfig;
        use std::sync::Arc;

        let mut config = AppConfig::default();
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("health_test.db");
        config.server.database_path = db_path.to_string_lossy().to_string();
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

        let state = AppState {
            config: Arc::new(config),
            auth: auth_arc,
            oidc: oidc_mgr,
            tasks: task_mgr,
            tags: tag_mgr,
            vaults: vault_mgr,
            backup: backup_mgr,
            plugins: plugin_mgr,
            rate_limiter: Arc::new(RateLimiter::new()),
        };

        let res = handle_health(State(state)).await;
        let val = res.0;
        assert_eq!(val["status"], "ok");
        assert_eq!(val["version"], env!("CARGO_PKG_VERSION"));
        assert!(val["hostname"].is_string());
        assert!(val["os"].is_string());
        assert!(val["arch"].is_string());
        assert!(val["time"].is_string());
    }

    #[tokio::test]
    async fn test_handle_install_and_list_plugins() {
        use crate::config::AppConfig;
        use std::sync::Arc;

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("test_auth.db");
        let mut config = AppConfig::default();
        config.server.database_path = db_path.to_str().unwrap().to_string();
        config.server.standalone = true;

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
            temp.path().join("system_plugins"),
            temp.path().join("user_plugins"),
            true,
            "allow_all".to_string(),
            vec!["*".to_string()],
            vec![],
        ));

        let oidc_mgr = Arc::new(crate::auth::oidc::OidcManager::new(config.auth.oidc.clone(), auth_arc.clone()));

        let state = AppState {
            config: Arc::new(config),
            auth: auth_arc,
            oidc: oidc_mgr,
            tasks: task_mgr,
            tags: tag_mgr,
            vaults: vault_mgr,
            backup: backup_mgr,
            plugins: plugin_mgr,
            rate_limiter: Arc::new(RateLimiter::new()),
        };

        // Create a test .grr package
        let src_dir = temp.path().join("test_src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("plugin.toml"), r#"
[plugin]
id = "test-chewtoy"
name = "Test ChewToy"
version = "1.0.0"
description = "Test chewtoy package"
"#).unwrap();
        std::fs::write(src_dir.join("index.html"), "<h1>Test ChewToy</h1>").unwrap();

        let grr_path = temp.path().join("test-chewtoy.grr");
        crate::plugins::PluginManager::pack_grr(&src_dir, &grr_path).unwrap();
        let grr_bytes = std::fs::read(&grr_path).unwrap();

        // 1. Test POST /api/plugins/install with raw binary bytes in unauthenticated standalone/local mode
        let headers = HeaderMap::new();
        let install_res = handle_install_plugin(
            State(state.clone()),
            headers.clone(),
            axum::body::Bytes::from(grr_bytes),
        ).await.unwrap();

        assert_eq!(install_res.0.id, "test-chewtoy");
        assert_eq!(install_res.0.name, "Test ChewToy");

        // 2. Test GET /api/plugins
        let list_res = handle_list_plugins(
            State(state.clone()),
            headers.clone(),
        ).await.unwrap();

        assert_eq!(list_res.0.plugins.len(), 1);
        assert_eq!(list_res.0.plugins[0].id, "test-chewtoy");
        assert!(list_res.0.can_install);

        // 3. Test GET /api/plugins/test-chewtoy/assets/index.html
        let asset_res = handle_plugin_asset(
            State(state.clone()),
            axum::extract::Path(("test-chewtoy".to_string(), "index.html".to_string())),
        ).await.unwrap();

        assert_eq!(asset_res.status(), StatusCode::OK);
    }

    #[test]
    fn test_sanitize_uploaded_file_name() {
        assert_eq!(sanitize_uploaded_file_name(r"C:\Users\bolt\Documents\report.docx"), "report.docx");
        assert_eq!(sanitize_uploaded_file_name("C:/Users/bolt/Documents/report.docx"), "report.docx");
        assert_eq!(sanitize_uploaded_file_name("/home/bolt/file.txt"), "file.txt");
        assert_eq!(sanitize_uploaded_file_name("relative/path/sub/test.rs"), "test.rs");
        assert_eq!(sanitize_uploaded_file_name("simple.png"), "simple.png");
        assert_eq!(sanitize_uploaded_file_name(r"..\..\etc\passwd"), "passwd");
        assert_eq!(sanitize_uploaded_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_uploaded_file_name(r"C:\"), "upload.bin");
        assert_eq!(sanitize_uploaded_file_name(""), "upload.bin");
        assert_eq!(sanitize_uploaded_file_name("   "), "upload.bin");
        assert_eq!(sanitize_uploaded_file_name(r"D:\Data\Archive.tar.gz"), "Archive.tar.gz");
    }

    #[test]
    fn test_sanitize_uploaded_relative_path() {
        assert_eq!(sanitize_uploaded_relative_path(r"C:\Users\bolt\Documents\report.docx"), "Users/bolt/Documents/report.docx");
        assert_eq!(sanitize_uploaded_relative_path("C:/Users/bolt/Documents/report.docx"), "Users/bolt/Documents/report.docx");
        assert_eq!(sanitize_uploaded_relative_path("/home/bolt/file.txt"), "home/bolt/file.txt");
        assert_eq!(sanitize_uploaded_relative_path("relative/path/sub/test.rs"), "relative/path/sub/test.rs");
        assert_eq!(sanitize_uploaded_relative_path(r"relative\path\sub\test.rs"), "relative/path/sub/test.rs");
        assert_eq!(sanitize_uploaded_relative_path("simple.png"), "simple.png");
        assert_eq!(sanitize_uploaded_relative_path(r"..\..\etc\passwd"), "etc/passwd");
        assert_eq!(sanitize_uploaded_relative_path("../../etc/passwd"), "etc/passwd");
        assert_eq!(sanitize_uploaded_relative_path("folder/../sub/file.txt"), "folder/sub/file.txt");
        assert_eq!(sanitize_uploaded_relative_path(r"C:\"), "upload.bin");
        assert_eq!(sanitize_uploaded_relative_path(""), "upload.bin");
        assert_eq!(sanitize_uploaded_relative_path("   "), "upload.bin");
    }
}
