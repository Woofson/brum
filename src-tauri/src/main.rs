// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use brum::config::ConfigManager;
use brum::start_background_server;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedWindowState {
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub is_maximized: Option<bool>,
}

impl Default for SavedWindowState {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: Some(1366.0),
            height: Some(840.0),
            is_maximized: Some(false),
        }
    }
}

impl SavedWindowState {
    fn file_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("brum").join("window_state.json"))
    }

    fn load() -> Self {
        if let Some(path) = Self::file_path() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(state) = serde_json::from_str::<Self>(&data) {
                    return state;
                }
            }
        }
        Self::default()
    }

    fn save(&self) {
        if let Some(path) = Self::file_path() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = fs::write(path, json);
            }
        }
    }
}

fn main() {
    brum::setup_linux_desktop_env();

    let mut config = ConfigManager::load_all();
    config.server.standalone = true;
    config.server.enable_auth = false;
    config.storage.allow_entire_system = true;
    config.paranoid.windows_native_file_ops = true;
    config.server.host = "127.0.0.1".to_string();
    if config.server.port == 0 {
        config.server.port = 3140; // Default preferred port
    }

    let active_port = Arc::new(AtomicU16::new(config.server.port));
    let active_port_server = active_port.clone();
    let active_port_tray = active_port.clone();

    let window_decorations = config.ui.window_decorations;
    let minimize_to_tray = config.desktop.minimize_to_tray;
    let start_minimized = config.desktop.start_minimized;

    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();

            // Spawn embedded backend server in Tokio runtime or attach to running service
            tauri::async_runtime::spawn(async move {
                let host = config.server.host.clone();
                let configured_port = config.server.port;

                let bound_port = if brum::probe_running_brum_server(&host, configured_port).await {
                    println!("Detected running Brum server on http://127.0.0.1:{}. Attaching desktop interface...", configured_port);
                    configured_port
                } else {
                    match start_background_server(config).await {
                        Ok(port) => {
                            println!("Brum desktop backend bound to: http://127.0.0.1:{}", port);
                            port
                        }
                        Err(e) => {
                            eprintln!("Failed to start Brum backend server: {}", e);
                            return;
                        }
                    }
                };

                active_port_server.store(bound_port, Ordering::Relaxed);
                let url = format!("http://127.0.0.1:{}", bound_port);
                println!("Brum desktop interface connected to: {}", url);

                let saved_state = SavedWindowState::load();
                let win_width = saved_state.width.unwrap_or(1366.0).max(680.0);
                let win_height = saved_state.height.unwrap_or(840.0).max(480.0);
                let is_max = saved_state.is_maximized.unwrap_or(false);

                // Create the primary standalone window pointing to the local embedded server
                #[allow(unused_mut)]
                let mut win_builder = WebviewWindowBuilder::new(
                    &handle,
                    "main",
                    WebviewUrl::External(url.parse().unwrap()),
                )
                        .title("Brum")
                        .inner_size(win_width, win_height)
                        .min_inner_size(680.0, 480.0)
                        .resizable(true)
                        .disable_drag_drop_handler()
                        .decorations(window_decorations)
                        .visible(!start_minimized);

                        if let (Some(x), Some(y)) = (saved_state.x, saved_state.y) {
                            win_builder = win_builder.position(x, y);
                        } else {
                            win_builder = win_builder.center();
                        }

                        if is_max {
                            win_builder = win_builder.maximized(true);
                        }

                        #[cfg(target_os = "macos")]
                        {
                            win_builder = win_builder.title_bar_style(tauri::TitleBarStyle::Overlay);
                        }

                        match win_builder.build() {
                            Ok(window) => {
                                if let Some(icon) = handle.default_window_icon() {
                                    let _ = window.set_icon(icon.clone());
                                }
                                let window_clone = window.clone();
                                let state_lock = Arc::new(std::sync::Mutex::new(saved_state));
                                let state_clone = state_lock.clone();

                                window.on_window_event(move |event| match event {
                                    WindowEvent::CloseRequested { api, .. } => {
                                        if let Ok(mut state) = state_clone.lock() {
                                            if let Ok(is_max) = window_clone.is_maximized() {
                                                state.is_maximized = Some(is_max);
                                            }
                                            if !window_clone.is_maximized().unwrap_or(false) {
                                                let scale = window_clone.scale_factor().unwrap_or(1.0);
                                                if let Ok(pos) = window_clone.outer_position() {
                                                    state.x = Some(pos.x as f64 / scale);
                                                    state.y = Some(pos.y as f64 / scale);
                                                }
                                                if let Ok(size) = window_clone.inner_size() {
                                                    state.width = Some(size.width as f64 / scale);
                                                    state.height = Some(size.height as f64 / scale);
                                                }
                                            }
                                            state.save();
                                        }
                                        if minimize_to_tray {
                                            api.prevent_close();
                                            let _ = window_clone.hide();
                                        }
                                    }
                                    WindowEvent::Moved(pos) => {
                                        if !window_clone.is_maximized().unwrap_or(false) {
                                            let scale = window_clone.scale_factor().unwrap_or(1.0);
                                            if let Ok(mut state) = state_clone.lock() {
                                                state.x = Some(pos.x as f64 / scale);
                                                state.y = Some(pos.y as f64 / scale);
                                                state.save();
                                            }
                                        }
                                    }
                                    WindowEvent::Resized(size) => {
                                        if !window_clone.is_maximized().unwrap_or(false) {
                                            let scale = window_clone.scale_factor().unwrap_or(1.0);
                                            if let Ok(mut state) = state_clone.lock() {
                                                state.width = Some(size.width as f64 / scale);
                                                state.height = Some(size.height as f64 / scale);
                                                state.save();
                                            }
                                        }
                                    }
                                    _ => {}
                                });
                            }
                            Err(e) => {
                                eprintln!("Failed to create main window: {}", e);
                            }
                        }
            });

            // Configure System Tray
            let show_i = MenuItem::with_id(app, "show", "🐻 Show Brum", true, None::<&str>)?;
            let hide_i = MenuItem::with_id(app, "hide", "➖ Hide to Tray", true, None::<&str>)?;
            let browser_i = MenuItem::with_id(app, "browser", "🌐 Open in Web Browser", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_i = MenuItem::with_id(app, "quit", "❌ Quit Brum", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &hide_i, &browser_i, &sep, &quit_i])?;

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("Brum - Multi-Pane Web Environment")
                .show_menu_on_left_click(false);

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            let _tray = tray_builder
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "browser" => {
                        let port = active_port_tray.load(Ordering::Relaxed);
                        let _ = open::that(format!("http://127.0.0.1:{}", port));
                    }
                    "quit" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let is_max = window.is_maximized().unwrap_or(false);
                            let scale = window.scale_factor().unwrap_or(1.0);
                            let mut state = SavedWindowState::load();
                            state.is_maximized = Some(is_max);
                            if !is_max {
                                if let Ok(pos) = window.outer_position() {
                                    state.x = Some(pos.x as f64 / scale);
                                    state.y = Some(pos.y as f64 / scale);
                                }
                                if let Ok(size) = window.inner_size() {
                                    state.width = Some(size.width as f64 / scale);
                                    state.height = Some(size.height as f64 / scale);
                                }
                            }
                            state.save();
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running brum desktop application");
}
