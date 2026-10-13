pub mod auth;
pub mod config;
pub mod instance;
pub mod logging;
pub mod plugins;
pub mod server;
pub mod tools;
pub mod vfs;

use auth::AuthManager;
use config::AppConfig;
use plugins::PluginManager;
use server::{create_router, AppState};
use std::sync::Arc;
use tracing::info;

/// Creates the shared AppState for Brum
pub fn create_app_state(
    config: &AppConfig,
    instance_info: Option<Arc<crate::instance::UserInstanceInfo>>,
) -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
    let auth_mgr = AuthManager::new(
        &config.server.database_path,
        &config.server.jwt_secret,
        config.server.session_duration_hours,
        &config.auth.mode,
        &config.auth.pam_service,
        &config.auth.default_admin_user,
        &config.auth.default_admin_pass,
    )?;

    let task_mgr = tools::tasks::TaskManager::new();
    let tag_mgr = tools::tags::TagManager::new(auth_mgr.db())?;
    let vault_mgr = vfs::vault::VaultManager::new();
    let backup_mgr = tools::sync::BackupManager::new(auth_mgr.db())?;
    let plugin_mgr = PluginManager::new(
        config.plugins.resolve_system_directory(),
        config.plugins.resolve_user_directory(),
        config.plugins.allow_user_installs,
        config.plugins.default_policy.clone(),
        config.plugins.global_whitelist.clone(),
        config.plugins.global_blacklist.clone(),
    );

    let backup_mgr_arc = Arc::new(backup_mgr);
    let task_mgr_arc = Arc::new(task_mgr);
    crate::tools::tasks::set_global_task_manager(task_mgr_arc.clone());
    backup_mgr_arc.clone().start_scheduler(task_mgr_arc.clone());

    let auth_mgr_arc = Arc::new(auth_mgr);
    let oidc_mgr = crate::auth::oidc::OidcManager::new(config.auth.oidc.clone(), auth_mgr_arc.clone());

    if instance_info.is_some() {
        let bind_host = if config.server.host.is_empty() {
            "127.0.0.1"
        } else {
            &config.server.host
        };
        crate::instance::assert_loopback_bind(bind_host)?;
    }

    if config.server.allowed_origins.iter().any(|o| o.trim() == "*") {
        tracing::warn!(
            "Security Warning: server.allowed_origins contains wildcard '*'. All origins are permitted, which disables cross-origin WebSocket and CSRF protections."
        );
    }

    Ok(AppState {
        config: Arc::new(config.clone()),
        auth: auth_mgr_arc,
        oidc: Arc::new(oidc_mgr),
        tasks: task_mgr_arc,
        tags: Arc::new(tag_mgr),
        vaults: Arc::new(vault_mgr),
        backup: backup_mgr_arc,
        plugins: Arc::new(plugin_mgr),
        rate_limiter: Arc::new(crate::server::RateLimiter::new()),
        instance_info,
    })
}

/// Probes whether a Brum server is actively running on the target host and port
pub async fn probe_running_brum_server(host: &str, port: u16) -> bool {
    let probe_host = if host.is_empty() || host == "0.0.0.0" { "127.0.0.1" } else { host };
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(350))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let health_url = format!("http://{}:{}/api/health", probe_host, port);
    if let Ok(resp) = client.get(&health_url).send().await {
        if resp.status().is_success() {
            return true;
        }
    }

    let status_url = format!("http://{}:{}/api/system/status", probe_host, port);
    if let Ok(resp) = client.get(&status_url).send().await {
        if resp.status().is_success() {
            return true;
        }
    }

    let config_url = format!("http://{}:{}/api/config", probe_host, port);
    if let Ok(resp) = client.get(&config_url).send().await {
        if resp.status().is_success() {
            return true;
        }
    }

    false
}

/// Starts the Brum HTTP server in a background task and returns the bound port and optional instance token
pub async fn start_background_server(mut config: AppConfig) -> Result<(u16, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    let bind_host = if config.server.host.is_empty() {
        "127.0.0.1"
    } else {
        &config.server.host
    };

    let instance_info = if config.server.standalone {
        crate::instance::assert_loopback_bind(bind_host)?;
        let token = crate::instance::generate_instance_token();
        let owner = crate::instance::get_current_os_username();
        Some(Arc::new(crate::instance::UserInstanceInfo {
            pid: std::process::id(),
            port: config.server.port,
            owner,
            instance_token: token,
            started_at: chrono::Utc::now().to_rfc3339(),
        }))
    } else {
        None
    };

    let state = create_app_state(&config, instance_info.clone())?;
    let app = create_router(state);

    let addr_str = format!("{}:{}", bind_host, config.server.port);
    let listener = match tokio::net::TcpListener::bind(&addr_str).await {
        Ok(l) => l,
        Err(e) if config.server.port != 0 => {
            tracing::warn!(
                "Preferred port {} is unavailable ({}), falling back to dynamic port",
                config.server.port,
                e
            );
            tokio::net::TcpListener::bind(format!("{}:0", bind_host)).await?
        }
        Err(e) => return Err(Box::new(e)),
    };
    let local_addr = listener.local_addr()?;
    let bound_port = local_addr.port();
    config.server.port = bound_port;

    let mut token_out = None;
    if let Some(ref inst) = instance_info {
        token_out = Some(inst.instance_token.clone());
        let _ = crate::instance::register_user_instance(bound_port, &inst.instance_token);
    }

    info!("Brum embedded server listening on http://{}", local_addr);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await {
            tracing::error!("Brum server error: {}", e);
        }
    });

    Ok((bound_port, token_out))
}

/// Configures Linux desktop and Wayland/Hyprland environment variables
pub fn setup_linux_desktop_env() {
    #[cfg(target_os = "linux")]
    {
        if std::env::var("__NV_DISABLE_EXPLICIT_SYNC").is_err() {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }
    }
}
