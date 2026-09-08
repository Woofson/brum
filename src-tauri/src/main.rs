// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use commanderdog::config::ConfigManager;
use commanderdog::start_background_server;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

fn main() {
    commanderdog::setup_linux_desktop_env();

    let mut config = ConfigManager::load_all();
    config.server.standalone = true;
    config.server.enable_auth = false;
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0; // Ephemeral port allocation

    let window_decorations = config.ui.window_decorations;
    let minimize_to_tray = config.desktop.minimize_to_tray;
    let start_minimized = config.desktop.start_minimized;

    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();

            // Spawn embedded backend server in Tokio runtime
            tauri::async_runtime::spawn(async move {
                match start_background_server(config).await {
                    Ok(port) => {
                        let url = format!("http://127.0.0.1:{}", port);
                        println!("CommanderDog desktop backend bound to: {}", url);

                        // Create the primary standalone window pointing to the local embedded server
                        #[allow(unused_mut)]
                        let mut win_builder = WebviewWindowBuilder::new(
                            &handle,
                            "main",
                            WebviewUrl::External(url.parse().unwrap()),
                        )
                        .title("CommanderDog")
                        .inner_size(1366.0, 840.0)
                        .min_inner_size(680.0, 480.0)
                        .resizable(true)
                        .decorations(window_decorations)
                        .visible(!start_minimized)
                        .center();

                        #[cfg(target_os = "macos")]
                        {
                            win_builder = win_builder.title_bar_style(tauri::TitleBarStyle::Overlay);
                        }

                        if let Some(icon) = handle.default_window_icon() {
                            if let Ok(builder) = win_builder.icon(icon.clone()) {
                                win_builder = builder;
                            }
                        }

                        match win_builder.build() {
                            Ok(window) => {
                                let window_clone = window.clone();
                                window.on_window_event(move |event| {
                                    if let WindowEvent::CloseRequested { api, .. } = event {
                                        if minimize_to_tray {
                                            api.prevent_close();
                                            let _ = window_clone.hide();
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                eprintln!("Failed to create main window: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to start CommanderDog backend server: {}", e);
                    }
                }
            });

            // Configure System Tray
            let show_i = MenuItem::with_id(app, "show", "🐕 Show CommanderDog", true, None::<&str>)?;
            let hide_i = MenuItem::with_id(app, "hide", "➖ Hide to Tray", true, None::<&str>)?;
            let browser_i = MenuItem::with_id(app, "browser", "🌐 Open in Web Browser", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_i = MenuItem::with_id(app, "quit", "❌ Quit CommanderDog", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &hide_i, &browser_i, &sep, &quit_i])?;

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("CommanderDog - Multi-Tab File Commander")
                .show_menu_on_left_click(false);

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            let _tray = tray_builder
                .on_menu_event(|app, event| match event.id.as_ref() {
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
                        let _ = open::that("http://127.0.0.1:3140");
                    }
                    "quit" => {
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
        .expect("error while running commander dog desktop application");
}
