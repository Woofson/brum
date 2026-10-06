pub mod auth;
pub mod config;
pub mod plugins;
pub mod server;
pub mod tools;
pub mod vfs;

#[cfg(target_os = "windows")]
mod windows_service_runner;

use auth::AuthManager;
use config::ConfigManager;
use server::{create_router, AppState};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

async fn probe_running_brum_server(host: &str, port: u16) -> bool {
    let probe_host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(250))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

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
    let mut connect_url: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--connect" | "-c" => {
                if i + 1 < args.len() {
                    let mut target = args[i + 1].clone();
                    if !target.starts_with("http://") && !target.starts_with("https://") {
                        target = format!("http://{}", target);
                    }
                    connect_url = Some(target);
                    i += 1;
                }
            }
            "--windows-service" | "--service" => {
                #[cfg(target_os = "windows")]
                {
                    windows_service_runner::run_as_service()?;
                    return Ok(());
                }
                #[cfg(not(target_os = "windows"))]
                {
                    eprintln!("Windows Service mode is only supported on Windows.");
                    return Ok(());
                }
            }
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
                    if subcmd == "run" {
                        #[cfg(target_os = "windows")]
                        {
                            windows_service_runner::run_as_service()?;
                            return Ok(());
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            is_server_mode = true;
                            config.server.standalone = false;
                        }
                    } else {
                        handle_service_command(&subcmd)?;
                        return Ok(());
                    }
                } else {
                    eprintln!("Usage: brum service [install|uninstall|start|stop|restart|status|run]");
                    return Ok(());
                }
            }
            "shortcut" => {
                if i + 1 < args.len() {
                    let subcmd = args[i + 1].clone();
                    let with_desktop = args.iter().any(|a| a == "--desktop" || a == "-d" || a == "-Desktop");
                    handle_shortcut_command(&subcmd, with_desktop)?;
                    return Ok(());
                } else {
                    eprintln!("Usage: brum shortcut [install [--desktop] | uninstall]");
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
                println!(
                    "Brum v{} (build #{} · commit {} · {} · {})",
                    env!("CARGO_PKG_VERSION"),
                    env!("BRUM_BUILD_NUMBER"),
                    env!("BRUM_BUILD_COMMIT"),
                    env!("BRUM_BUILD_TIMESTAMP"),
                    env!("BRUM_BUILD_TARGET")
                );
                return Ok(());
            }
            "--help" | "-h" => {
                println!("Brum v{} (build #{}) - Multi-Pane Web Environment (File Commander/Manager)", env!("CARGO_PKG_VERSION"), env!("BRUM_BUILD_NUMBER"));
                println!();
                println!("USAGE:");
                println!("    brum [OPTIONS]");
                println!("    brum service [COMMAND]");
                println!("    brum shortcut [COMMAND] [--desktop]");
                println!();
                println!("OPTIONS:");
                println!("    -c, --connect <URL>    Connect desktop GUI/browser directly to an existing or remote Brum server");
                println!("    -s, --standalone       Run in standalone desktop mode (auto-authenticates as local user, opens browser/window)");
                println!("    -o, --open             Automatically open Brum in default web browser / webview");
                println!("    -p, --port <PORT>      Override web server port (default: 3140 or config.toml setting)");
                println!("        --host <HOST>      Override web server bind host (default: 0.0.0.0)");
                println!("        --no-auth          Disable login authentication and run with local permissions");
                println!("        --windows-service  Internal entry point for Windows Service Control Manager");
                println!("        --minimized        Launch minimized in system tray / background without opening window");
                println!("        --no-decorations   Launch without window titlebar/frame (ideal for Hyprland/tiling WMs)");
                println!("        --frameless        Alias for --no-decorations");
                println!("        --decorations      Force enable window titlebar and borders");
                println!("    -v, --version          Print version information");
                println!("    -h, --help             Print this help message");
                println!();
                println!("SERVICE COMMANDS:");
                println!("    install                Register Brum as Windows NT Service or systemd user service");
                println!("    uninstall              Remove registered background service");
                println!("    start                  Start background service");
                println!("    stop                   Stop running service");
                println!("    restart                Restart background service");
                println!("    status                 Query service running status");
                println!("    run                    Execute service dispatcher directly");
                println!();
                println!("SHORTCUT COMMANDS:");
                println!("    install [--desktop]    Create Start Menu shortcut (and optional Desktop shortcut)");
                println!("    uninstall              Remove created shortcuts");
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    if config.server.standalone {
        config.server.enable_auth = false;
        if config.server.host == "0.0.0.0" {
            config.server.host = "127.0.0.1".to_string();
        }
        if !is_server_mode {
            auto_open = true;
        }
    }

    if let Some(target_url) = connect_url {
        info!("Connecting Brum client directly to server instance: {}", target_url);
        #[allow(unused_variables)]
        let has_display = is_graphical_display_available();

        #[cfg(feature = "gui")]
        if has_display {
            let dec_status = if config.ui.window_decorations { "enabled" } else { "disabled (borderless/tiling mode)" };
            info!("Launching Brum native desktop window (decorations: {}): {}", dec_status, target_url);
            run_native_gui(&target_url, "Brum", config.ui.window_decorations)?;
            return Ok(());
        }

        info!("Opening in browser: {}", target_url);
        let _ = open::that(&target_url);
        return Ok(());
    }

    if !is_server_mode && (config.server.standalone || auto_open) {
        if probe_running_brum_server(&config.server.host, config.server.port).await {
            let existing_url = format!("http://127.0.0.1:{}", config.server.port);
            info!("Detected running Brum server on {}. Attaching desktop interface...", existing_url);
            #[allow(unused_variables)]
            let has_display = is_graphical_display_available();

            #[cfg(feature = "gui")]
            if has_display {
                let dec_status = if config.ui.window_decorations { "enabled" } else { "disabled (borderless/tiling mode)" };
                info!("Launching Brum native desktop window (decorations: {}): {}", dec_status, existing_url);
                run_native_gui(&existing_url, "Brum", config.ui.window_decorations)?;
                return Ok(());
            }

            info!("Opening in browser: {}", existing_url);
            let _ = open::that(&existing_url);
            return Ok(());
        }
    }

    info!(
        "Starting Brum v{} (build #{}, commit: {}, target: {})...",
        env!("CARGO_PKG_VERSION"),
        env!("BRUM_BUILD_NUMBER"),
        env!("BRUM_BUILD_COMMIT"),
        env!("BRUM_BUILD_TARGET")
    );

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
    let plugin_mgr = plugins::PluginManager::new(
        std::path::PathBuf::from(&config.plugins.directory),
        std::path::PathBuf::from(&config.plugins.user_directory),
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

    let state = AppState {
        config: Arc::new(config.clone()),
        auth: auth_mgr_arc,
        oidc: Arc::new(oidc_mgr),
        tasks: task_mgr_arc,
        tags: Arc::new(tag_mgr),
        vaults: Arc::new(vault_mgr),
        backup: backup_mgr_arc,
        plugins: Arc::new(plugin_mgr),
    };

    let app = create_router(state);

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    info!("Brum Web Server listening on http://{}", addr);
    if config.server.standalone || !config.server.enable_auth {
        let current_user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "user".to_string());
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
    #[allow(unused_variables)]
    let has_display = is_graphical_display_available();
    let should_launch_gui = !is_server_mode && (is_standalone || auto_open);

    if should_launch_gui {
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("Brum server error: {}", e);
            }
        });

        #[cfg(feature = "gui")]
        if has_display {
            let dec_status = if config.ui.window_decorations { "enabled" } else { "disabled (borderless/tiling mode)" };
            info!("Launching Brum native desktop window (decorations: {}): {}", dec_status, open_url);
            run_native_gui(&open_url, "Brum", config.ui.window_decorations)?;
            return Ok(());
        }

        info!("Opening Brum in browser: {}", open_url);
        let u = open_url.clone();
        tokio::spawn(async move {
            let _ = open::that(&u);
        });

        tokio::signal::ctrl_c().await?;
        return Ok(());
    } else {
        axum::serve(listener, app).await?;
    }

    Ok(())
}

fn is_graphical_display_available() -> bool {
    if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
        true
    } else {
        std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
    }
}

#[cfg(feature = "gui")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct GuiWindowState {
    x: Option<i32>,
    y: Option<i32>,
    width: u32,
    height: u32,
    maximized: bool,
}

#[cfg(feature = "gui")]
impl Default for GuiWindowState {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: 1366,
            height: 840,
            maximized: false,
        }
    }
}

#[cfg(feature = "gui")]
fn load_gui_window_state() -> GuiWindowState {
    if let Some(config_dir) = dirs::config_dir() {
        let state_file = config_dir.join("brum").join("window_state.json");
        if let Ok(content) = std::fs::read_to_string(&state_file) {
            if let Ok(state) = serde_json::from_str::<GuiWindowState>(&content) {
                return state;
            }
        }
    }
    GuiWindowState::default()
}

#[cfg(feature = "gui")]
fn save_gui_window_state(state: &GuiWindowState) {
    if let Some(config_dir) = dirs::config_dir() {
        let dir = config_dir.join("brum");
        let _ = std::fs::create_dir_all(&dir);
        let state_file = dir.join("window_state.json");
        if let Ok(json) = serde_json::to_string_pretty(state) {
            let _ = std::fs::write(state_file, json);
        }
    }
}

#[cfg(feature = "gui")]
fn run_native_gui(url: &str, title: &str, decorations: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tao::dpi::{LogicalPosition, LogicalSize};
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;

    #[cfg(target_os = "linux")]
    use tao::platform::unix::WindowExtUnix;
    #[cfg(target_os = "linux")]
    use wry::WebViewBuilderExtUnix;

    let saved_state = load_gui_window_state();
    let mut current_state = saved_state.clone();

    let event_loop = EventLoop::new();
    let mut win_builder = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(
            (current_state.width.max(680)) as f64,
            (current_state.height.max(480)) as f64,
        ))
        .with_min_inner_size(LogicalSize::new(680.0, 480.0))
        .with_resizable(true)
        .with_decorations(decorations)
        .with_maximized(current_state.maximized);

    if let (Some(x), Some(y)) = (current_state.x, current_state.y) {
        win_builder = win_builder.with_position(LogicalPosition::new(x as f64, y as f64));
    }

    let window = win_builder.build(&event_loop)?;

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
        match event {
            Event::WindowEvent {
                event: WindowEvent::Moved(pos),
                ..
            } => {
                if !window.is_maximized() {
                    current_state.x = Some(pos.x);
                    current_state.y = Some(pos.y);
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                let is_max = window.is_maximized();
                current_state.maximized = is_max;
                if !is_max && size.width > 0 && size.height > 0 {
                    current_state.width = size.width;
                    current_state.height = size.height;
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                current_state.maximized = window.is_maximized();
                if let Ok(pos) = window.outer_position() {
                    if !current_state.maximized {
                        current_state.x = Some(pos.x);
                        current_state.y = Some(pos.y);
                    }
                }
                save_gui_window_state(&current_state);
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
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
                let bin_path = format!("\"{}\" --windows-service", exe.display());
                let status = std::process::Command::new("sc.exe")
                    .args(["create", "Brum", "binPath=", &bin_path, "start=", "auto", "DisplayName=", "Brum Web Service"])
                    .status()?;
                if status.success() {
                    let _ = std::process::Command::new("sc.exe")
                        .args(["description", "Brum", "Multi-Pane Web Environment and Fleet Commander Daemon"])
                        .status();
                    let _ = std::process::Command::new("sc.exe")
                        .args(["failure", "Brum", "reset=", "86400", "actions=", "restart/2000/restart/2000/restart/5000"])
                        .status();
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
                    println!("Service start requested.");
                }
            }
            "stop" => {
                println!("Stopping Brum Windows Service...");
                let status = std::process::Command::new("sc.exe").args(["stop", "Brum"]).status()?;
                if status.success() {
                    println!("Service stop requested.");
                }
            }
            "restart" => {
                println!("Restarting Brum Windows Service...");
                let _ = std::process::Command::new("sc.exe").args(["stop", "Brum"]).status();
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let status = std::process::Command::new("sc.exe").args(["start", "Brum"]).status()?;
                if status.success() {
                    println!("Service restarted.");
                }
            }
            "status" => {
                let _ = std::process::Command::new("sc.exe").args(["query", "Brum"]).status();
            }
            _ => eprintln!("Unknown service command: {}. Available: install, uninstall, start, stop, restart, status, run", cmd),
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
            "restart" => {
                let _ = std::process::Command::new("systemctl").args(["--user", "restart", "brum"]).status();
            }
            "status" => {
                let _ = std::process::Command::new("systemctl").args(["--user", "status", "brum"]).status();
            }
            _ => eprintln!("Unknown service command: {}. Available: install, uninstall, start, stop, restart, status, run", cmd),
        }
    }
    Ok(())
}

fn handle_shortcut_command(action: &str, with_desktop: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let exe = std::env::current_exe()?;

    #[cfg(target_os = "windows")]
    {
        let exe_dir = exe.parent().unwrap_or(std::path::Path::new("."));
        match action {
            "install" | "create" => {
                let ico_path = exe_dir.join("brum.ico");
                let ico_arg = if ico_path.exists() {
                    format!("$s.IconLocation = '{}'; ", ico_path.display())
                } else {
                    String::new()
                };

                let ps_script = format!(
                    r#"$w = New-Object -ComObject WScript.Shell;
$p = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::Programs);
$d = Join-Path $p 'Brum';
if (-not (Test-Path $d)) {{ New-Item -ItemType Directory -Path $d -Force | Out-Null }};
$s = $w.CreateShortcut((Join-Path $d 'Brum.lnk'));
$s.TargetPath = '{}';
$s.Arguments = '-s';
$s.WorkingDirectory = '{}';
$s.Description = 'Brum - File Commander & Manager (Standalone)';
{}$s.Save();
Write-Host 'Created Start Menu shortcut: ' (Join-Path $d 'Brum.lnk');
if ('{}' -eq 'true') {{
    $desk = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::Desktop);
    $ds = $w.CreateShortcut((Join-Path $desk 'Brum.lnk'));
    $ds.TargetPath = '{}';
    $ds.Arguments = '-s';
    $ds.WorkingDirectory = '{}';
    $ds.Description = 'Brum - File Commander & Manager (Standalone)';
    {}$ds.Save();
    Write-Host 'Created Desktop shortcut: ' (Join-Path $desk 'Brum.lnk');
}};"#,
                    exe.display(),
                    exe_dir.display(),
                    ico_arg,
                    if with_desktop { "true" } else { "false" },
                    exe.display(),
                    exe_dir.display(),
                    ico_arg
                );

                let status = std::process::Command::new("powershell.exe")
                    .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
                    .status()?;
                if status.success() {
                    println!("Shortcuts created successfully.");
                }
            }
            "uninstall" | "remove" => {
                let ps_script = r#"$p = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::Programs);
$d = Join-Path $p 'Brum';
if (Test-Path $d) { Remove-Item -Recurse -Force $d };
$desk = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::Desktop);
$deskLink = Join-Path $desk 'Brum.lnk';
if (Test-Path $deskLink) { Remove-Item -Force $deskLink };
Write-Host 'Removed shortcuts.';"#;

                let _ = std::process::Command::new("powershell.exe")
                    .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
                    .status();
                println!("Shortcuts removed.");
            }
            _ => eprintln!("Unknown shortcut command: {}. Available: install [--desktop], uninstall", action),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        match action {
            "install" | "create" => {
                if let Some(data_dir) = dirs::data_dir() {
                    let apps_dir = data_dir.join("applications");
                    let _ = std::fs::create_dir_all(&apps_dir)?;
                    let desktop_file = apps_dir.join("brum.desktop");
                    let content = format!(
                        "[Desktop Entry]\nName=Brum\nComment=Multi-Pane Web Environment (File Commander/Manager)\nExec=\"{}\" -s\nTerminal=false\nType=Application\nIcon=brum\nCategories=Utility;FileManager;System;FileTools;\nStartupNotify=true\n",
                        exe.display()
                    );
                    std::fs::write(&desktop_file, &content)?;
                    println!("Created Start Menu / Application shortcut at {}", desktop_file.display());

                    if with_desktop {
                        if let Some(desk_dir) = dirs::desktop_dir() {
                            let desk_file = desk_dir.join("brum.desktop");
                            std::fs::write(&desk_file, &content)?;
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(&desk_file, std::fs::Permissions::from_mode(0o755));
                            }
                            println!("Created Desktop shortcut at {}", desk_file.display());
                        }
                    }
                }
            }
            "uninstall" | "remove" => {
                if let Some(data_dir) = dirs::data_dir() {
                    let desktop_file = data_dir.join("applications/brum.desktop");
                    if desktop_file.exists() {
                        let _ = std::fs::remove_file(desktop_file);
                    }
                }
                if let Some(desk_dir) = dirs::desktop_dir() {
                    let desk_file = desk_dir.join("brum.desktop");
                    if desk_file.exists() {
                        let _ = std::fs::remove_file(desk_file);
                    }
                }
                println!("Desktop shortcuts removed.");
            }
            _ => eprintln!("Unknown shortcut command: {}. Available: install [--desktop], uninstall", action),
        }
    }

    Ok(())
}
