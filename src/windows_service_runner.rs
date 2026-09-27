use std::ffi::OsString;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

const SERVICE_NAME: &str = "Brum";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

define_windows_service!(ffi_service_main, brum_service_main);

/// Entry point for running Brum as a managed Windows NT Service via SCM.
pub fn run_as_service() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Err(e) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        eprintln!("Failed to start Windows Service dispatcher: {}", e);
        eprintln!("Note: This argument is intended to be invoked by the Windows Service Control Manager (SCM).");
        eprintln!("To launch Brum interactively in console server mode, run 'brum --server' instead.");
        return Err(Box::new(e));
    }
    Ok(())
}

/// ServiceMain callback invoked by Windows Service Control Manager.
pub fn brum_service_main(arguments: Vec<OsString>) {
    if let Err(e) = run_service(arguments) {
        error!("Windows Service encounter error: {:?}", e);
    }
}

fn run_service(_arguments: Vec<OsString>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Change working directory to the application install folder so relative paths resolve cleanly
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let _ = std::env::set_current_dir(exe_dir);
        }
    }

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let shutdown_tx_clone = shutdown_tx.clone();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = shutdown_tx_clone.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    // Register service control handler
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    // Inform SCM that the service is running
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // Start Tokio runtime for the Axum web server
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let mut config = crate::config::ConfigManager::load_all();
        config.server.standalone = false;

        info!("Starting Brum Windows Service (v{})...", env!("CARGO_PKG_VERSION"));

        let auth_mgr = match crate::auth::AuthManager::new(
            &config.server.database_path,
            &config.server.jwt_secret,
            config.server.session_duration_hours,
            &config.auth.mode,
            &config.auth.pam_service,
            &config.auth.default_admin_user,
            &config.auth.default_admin_pass,
        ) {
            Ok(m) => m,
            Err(e) => {
                error!("Windows Service failed to init AuthManager: {}", e);
                return;
            }
        };

        let task_mgr = crate::tools::tasks::TaskManager::new();
        let tag_mgr = match crate::tools::tags::TagManager::new(auth_mgr.db()) {
            Ok(m) => m,
            Err(e) => {
                error!("Windows Service failed to init TagManager: {}", e);
                return;
            }
        };
        let vault_mgr = crate::vfs::vault::VaultManager::new();
        let backup_mgr = match crate::tools::sync::BackupManager::new(auth_mgr.db()) {
            Ok(m) => m,
            Err(e) => {
                error!("Windows Service failed to init BackupManager: {}", e);
                return;
            }
        };
        let plugin_mgr = crate::plugins::PluginManager::new(
            std::path::PathBuf::from(&config.plugins.directory),
            std::path::PathBuf::from(&config.plugins.user_directory),
            config.plugins.allow_user_installs,
            config.plugins.default_policy.clone(),
            config.plugins.global_whitelist.clone(),
            config.plugins.global_blacklist.clone(),
        );

        let backup_mgr_arc = Arc::new(backup_mgr);
        let task_mgr_arc = Arc::new(task_mgr);
        backup_mgr_arc.clone().start_scheduler(task_mgr_arc.clone());

        let auth_mgr_arc = Arc::new(auth_mgr);
        let oidc_mgr = crate::auth::oidc::OidcManager::new(config.auth.oidc.clone(), auth_mgr_arc.clone());

        let state = crate::server::AppState {
            config: Arc::new(config.clone()),
            auth: auth_mgr_arc,
            oidc: Arc::new(oidc_mgr),
            tasks: task_mgr_arc,
            tags: Arc::new(tag_mgr),
            vaults: Arc::new(vault_mgr),
            backup: backup_mgr_arc,
            plugins: Arc::new(plugin_mgr),
        };

        let app = crate::server::create_router(state);
        let addr_str = format!("{}:{}", config.server.host, config.server.port);
        let addr: std::net::SocketAddr = match addr_str.parse() {
            Ok(a) => a,
            Err(e) => {
                error!("Failed to parse server socket address '{}': {}", addr_str, e);
                return;
            }
        };

        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Windows Service failed to bind TCP listener on {}: {}", addr, e);
                return;
            }
        };

        info!("Brum Windows Service web server listening on http://{}", addr);

        let (async_stop_tx, async_stop_rx) = tokio::sync::oneshot::channel::<()>();

        std::thread::spawn(move || {
            let _ = shutdown_rx.recv();
            let _ = async_stop_tx.send(());
        });

        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = async_stop_rx.await;
                info!("Brum Windows Service stop requested. Shutting down HTTP server.");
            })
            .await;
    });

    // Inform SCM that the service is stopped
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    Ok(())
}
