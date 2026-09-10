pub mod auth;
pub mod config;
pub mod server;
pub mod tools;
pub mod vfs;

use auth::AuthManager;
use config::ConfigManager;
use server::{create_router, AppState};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    brum::setup_linux_desktop_env();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "brum=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    println!(r#"
▄▄▄▄· ▄▄▄  ▄• ▄▌• ▌ ▄ ·. 
▐█ ▀█▪▀▄ █·█▪██▌·██ ▐███▪
▐█▀▀█▄▐▀▀▄ █▌▐█▌▐█ ▌▐▌▐█·
██▄▪▐█▐█•█▌▐█▄█▌██ ██▌▐█▌
·▀▀▀▀ .▀  ▀ ▀▀▀ ▀▀  █▪▀▀▀
         by Woofson
"#);

    let mut config = ConfigManager::load_all();

    // Parse CLI Arguments
    let args: Vec<String> = std::env::args().collect();
    let mut is_server_mode = false;
    let mut auto_open = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--server" | "--headless" => {
                is_server_mode = true;
                config.server.standalone = false;
            }
            "--standalone" | "-s" => {
                config.server.standalone = true;
                config.server.enable_auth = false;
                if config.server.host == "0.0.0.0" {
                    config.server.host = "127.0.0.1".to_string();
                }
                auto_open = true;
            }
            "--no-auth" => {
                config.server.enable_auth = false;
            }
            "--open" | "-o" => {
                auto_open = true;
            }
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    if let Ok(p) = args[i + 1].parse::<u16>() {
                        config.server.port = p;
                    }
                    i += 1;
                }
            }
            "--host" => {
                if i + 1 < args.len() {
                    config.server.host = args[i + 1].clone();
                    i += 1;
                }
            }
            "service" => {
                if i + 1 < args.len() {
                    let subcmd = args[i + 1].clone();
                    handle_service_command(&subcmd)?;
                    return Ok(());
                } else {
                    eprintln!("Usage: brum service [install|uninstall|start|stop|status]");
                    return Ok(());
                }
            }
            "--minimized" | "--tray-only" => {
                auto_open = false;
                config.server.standalone = true;
            }
            "--no-decorations" | "--frameless" => {
                config.ui.window_decorations = false;
            }
            "--decorations" => {
                config.ui.window_decorations = true;
            }
            "--version" | "-v" => {
                println!("Brum v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!("Brum v{} - Multi-Pane Web Environment (File Commander/Manager)", env!("CARGO_PKG_VERSION"));
                println!();
                println!("USAGE:");
                println!("    brum [OPTIONS]");
                println!("    brum service [COMMAND]");
                println!();
                println!("OPTIONS:");
                println!("    -s, --standalone    Run in standalone desktop mode (auto-authenticates as local user, opens browser/window)");
                println!("    -o, --open          Automatically open Brum in default web browser / webview");
                println!("    -p, --port <PORT>   Override web server port (default: 3140 or config.toml setting)");
                println!("        --host <HOST>   Override web server bind host (default: 0.0.0.0)");
                println!("        --no-auth       Disable login authentication and run with local permissions");
                println!("        --minimized     Launch minimized in system tray / background without opening window");
                println!("        --no-decorations Launch without window titlebar/frame (ideal for Hyprland/tiling WMs)");
                println!("        --frameless     Alias for --no-decorations");
                println!("        --decorations   Force enable window titlebar and borders");
                println!("    -v, --version       Print version information");
                println!("    -h, --help          Print this help message");
                println!();
                println!("SERVICE COMMANDS:");
                println!("    install             Register Brum as Windows NT Service or systemd user service");
                println!("    uninstall           Remove registered background service");
                println!("    start               Start background service");
                println!("    stop                Stop running service");
                println!("    status              Query service running status");
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    info!("Starting Brum v{} with active configuration...", env!("CARGO_PKG_VERSION"));

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

    let backup_mgr_arc = Arc::new(backup_mgr);
    let task_mgr_arc = Arc::new(task_mgr);
    backup_mgr_arc.clone().start_scheduler(task_mgr_arc.clone());

    let state = AppState {
        config: Arc::new(config.clone()),
        auth: Arc::new(auth_mgr),
        tasks: task_mgr_arc,
        tags: Arc::new(tag_mgr),
        vaults: Arc::new(vault_mgr),
        backup: backup_mgr_arc,
    };

    let app = create_router(state);

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    info!("Brum Web Server listening on http://{}", addr);
    if config.server.standalone || !config.server.enable_auth {
        let current_user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        info!("MODE: Standalone Desktop Mode (Running with local credentials for '{}')", current_user);
    } else {
        info!("Default Admin User: '{}' (Change password in settings)", config.auth.default_admin_user);
    }
    info!("Default Theme: '{}'", config.themes.default_theme);
    info!("Paranoid File Verification: {}", if config.paranoid.enabled { "ENABLED" } else { "DISABLED" });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    let open_url = format!("http://127.0.0.1:{}", bound_addr.port());
    let is_standalone = config.server.standalone;

    #[cfg(feature = "gui")]
    let has_display = std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
    #[cfg(not(feature = "gui"))]
    let has_display = false;

    let should_launch_gui = !is_server_mode && (is_standalone || has_display);

    if should_launch_gui && has_display {
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("Brum server error: {}", e);
            }
        });

        #[cfg(feature = "gui")]
        {
            let dec_status = if config.ui.window_decorations { "enabled" } else { "disabled (borderless/tiling mode)" };
            info!("Launching Brum native desktop window (decorations: {}): {}", dec_status, open_url);
            run_native_gui(&open_url, "Brum", config.ui.window_decorations)?;
            return Ok(());
        }
        #[cfg(not(feature = "gui"))]
        {
            info!("Native GUI feature not compiled in. Opening via browser launcher: {}", open_url);
            let u = open_url.clone();
            tokio::spawn(async move {
                let _ = open::that(&u);
            });
        }
    } else {
        if auto_open {
            let u = open_url.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                info!("Opening Brum in browser: {}", u);
                let _ = open::that(&u);
            });
        }

        axum::serve(listener, app).await?;
    }

    Ok(())
}

#[cfg(feature = "gui")]
fn run_native_gui(url: &str, title: &str, decorations: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;

    #[cfg(target_os = "linux")]
    use tao::platform::unix::WindowExtUnix;
    #[cfg(target_os = "linux")]
    use wry::WebViewBuilderExtUnix;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(1366.0, 840.0))
        .with_min_inner_size(LogicalSize::new(680.0, 480.0))
        .with_resizable(true)
        .with_decorations(decorations)
        .build(&event_loop)?;

    let builder = WebViewBuilder::new().with_url(url);

    #[cfg(target_os = "linux")]
    let _webview = {
        let vbox = window.default_vbox().ok_or("Failed to obtain GTK default vbox for Wayland/X11")?;
        builder.build_gtk(vbox)?
    };

    #[cfg(not(target_os = "linux"))]
    let _webview = builder.build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}

fn handle_service_command(cmd: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "windows")]
    {
        match cmd {
            "install" => {
                println!("Installing Brum Windows Service via sc.exe...");
                let status = std::process::Command::new("sc.exe")
                    .args(["create", "Brum", "binPath=", &format!("\"{}\" --server", exe.display()), "start=", "auto", "DisplayName=", "Brum Web Service"])
                    .status()?;
                if status.success() {
                    println!("Successfully registered Brum Windows Service.");
                } else {
                    eprintln!("Failed to register service. Ensure you are running Command Prompt / PowerShell as Administrator.");
                }
            }
            "uninstall" => {
                println!("Removing Brum Windows Service...");
                let _ = std::process::Command::new("sc.exe").args(["stop", "Brum"]).status();
                let status = std::process::Command::new("sc.exe").args(["delete", "Brum"]).status()?;
                if status.success() {
                    println!("Successfully removed Brum Windows Service.");
                }
            }
            "start" => {
                println!("Starting Brum Windows Service...");
                let status = std::process::Command::new("sc.exe").args(["start", "Brum"]).status()?;
                if status.success() {
                    println!("Service started.");
                }
            }
            "stop" => {
                println!("Stopping Brum Windows Service...");
                let status = std::process::Command::new("sc.exe").args(["stop", "Brum"]).status()?;
                if status.success() {
                    println!("Service stopped.");
                }
            }
            "status" => {
                let _ = std::process::Command::new("sc.exe").args(["query", "Brum"]).status();
            }
            _ => eprintln!("Unknown service command: {}. Available: install, uninstall, start, stop, status", cmd),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        match cmd {
            "install" => {
                println!("Creating user systemd service ~/.config/systemd/user/brum.service...");
                if let Some(config_dir) = dirs::config_dir() {
                    let systemd_dir = config_dir.join("systemd/user");
                    std::fs::create_dir_all(&systemd_dir)?;
                    let unit_path = systemd_dir.join("brum.service");
                    let content = format!(
                        "[Unit]\nDescription=Brum Web Commander Server\nAfter=network.target\n\n[Service]\nExecStart=\"{}\" --server\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
                        exe.display()
                    );
                    std::fs::write(&unit_path, content)?;
                    println!("Created service unit at {}", unit_path.display());
                    println!("To enable: systemctl --user daemon-reload && systemctl --user enable --now brum");
                }
            }
            "uninstall" => {
                if let Some(config_dir) = dirs::config_dir() {
                    let unit_path = config_dir.join("systemd/user/brum.service");
                    let _ = std::process::Command::new("systemctl").args(["--user", "disable", "--now", "brum"]).status();
                    if unit_path.exists() {
                        let _ = std::fs::remove_file(unit_path);
                    }
                    println!("Brum systemd service uninstalled.");
                }
            }
            "start" => {
                let _ = std::process::Command::new("systemctl").args(["--user", "start", "brum"]).status();
            }
            "stop" => {
                let _ = std::process::Command::new("systemctl").args(["--user", "stop", "brum"]).status();
            }
            "status" => {
                let _ = std::process::Command::new("systemctl").args(["--user", "status", "brum"]).status();
            }
            _ => eprintln!("Unknown service command: {}. Available: install, uninstall, start, stop, status", cmd),
        }
    }
    Ok(())
}
