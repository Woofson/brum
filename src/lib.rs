pub mod auth;
pub mod config;
pub mod server;
pub mod tools;
pub mod vfs;

use auth::AuthManager;
use config::AppConfig;
use server::{create_router, AppState};
use std::sync::Arc;
use tracing::info;

/// Creates the shared AppState for CommanderDog
pub fn create_app_state(config: &AppConfig) -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
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

    Ok(AppState {
        config: Arc::new(config.clone()),
        auth: Arc::new(auth_mgr),
        tasks: task_mgr_arc,
        tags: Arc::new(tag_mgr),
        vaults: Arc::new(vault_mgr),
        backup: backup_mgr_arc,
    })
}

/// Starts the CommanderDog HTTP server in a background task and returns the bound port
pub async fn start_background_server(mut config: AppConfig) -> Result<u16, Box<dyn std::error::Error + Send + Sync>> {
    let state = create_app_state(&config)?;
    let app = create_router(state);

    let bind_host = if config.server.host.is_empty() {
        "127.0.0.1"
    } else {
        &config.server.host
    };

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

    info!("CommanderDog embedded server listening on http://{}", local_addr);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("CommanderDog server error: {}", e);
        }
    });

    Ok(bound_port)
}

/// Configures Linux desktop and Wayland/Hyprland environment variables before GTK/WebKit initializes
pub fn setup_linux_desktop_env() {
    #[cfg(target_os = "linux")]
    {
        // 1. Fix WebKitGTK Error 71 (Protocol error) on Wayland compositors (Hyprland, Sway, KDE, GNOME)
        // Disabling DMA-BUF renderer avoids Wayland wl_surface protocol errors
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        if std::env::var("__NV_DISABLE_EXPLICIT_SYNC").is_err() {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }

        // 2. Fix GDK "Unable to load pointer from the cursor theme" spam on Hyprland / Wayland
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/bolt".to_string());
        if std::env::var("XCURSOR_PATH").is_err() {
            let paths = format!("{}/.local/share/icons:{}/.icons:/usr/share/icons:/usr/local/share/icons:/usr/share/pixmaps", home, home);
            std::env::set_var("XCURSOR_PATH", paths);
        }

        // Verify if XCURSOR_THEME has a valid cursors directory, otherwise fallback to Adwaita or default
        let current_theme = std::env::var("XCURSOR_THEME").unwrap_or_default();
        let is_valid = !current_theme.is_empty() && (
            std::path::Path::new(&format!("/usr/share/icons/{}/cursors", current_theme)).exists()
            || std::path::Path::new(&format!("{}/.local/share/icons/{}/cursors", home, current_theme)).exists()
            || std::path::Path::new(&format!("{}/.icons/{}/cursors", home, current_theme)).exists()
        );

        if !is_valid {
            let candidate_themes = ["Adwaita", "default", "breeze_cursors", "mate", "HighContrast"];
            let fallback = candidate_themes.iter().find(|t| {
                std::path::Path::new(&format!("/usr/share/icons/{}/cursors", t)).exists()
            }).unwrap_or(&"Adwaita");
            std::env::set_var("XCURSOR_THEME", fallback);
        }

        if std::env::var("XCURSOR_SIZE").is_err() {
            std::env::set_var("XCURSOR_SIZE", "24");
        }

        // Disable verbose GLib/GDK/WebKit diagnostic noise in child processes
        std::env::set_var("G_ENABLE_DIAGNOSTIC", "0");
        std::env::set_var("G_MESSAGES_DEBUG", "");

        // 3. Directly suppress GDK "Unable to load pointer..." log messages via GLib C-FFI (Structured + Classic)
        silence_gdk_pointer_logs();
    }
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct GLogField {
    key: *const std::ffi::c_char,
    value: *const std::ffi::c_void,
    length: isize,
}

#[cfg(target_os = "linux")]
type GLogWriterFunc = unsafe extern "C" fn(
    log_level: i32,
    fields: *const GLogField,
    n_fields: usize,
    user_data: *mut std::ffi::c_void,
) -> i32;

#[cfg(target_os = "linux")]
type GLogFunc = unsafe extern "C" fn(
    log_domain: *const std::ffi::c_char,
    log_level: i32,
    message: *const std::ffi::c_char,
    user_data: *mut std::ffi::c_void,
);

#[cfg(target_os = "linux")]
static mut DEFAULT_LOG_WRITER: Option<GLogWriterFunc> = None;

#[cfg(target_os = "linux")]
unsafe extern "C" fn structured_log_filter(
    log_level: i32,
    fields: *const GLogField,
    n_fields: usize,
    user_data: *mut std::ffi::c_void,
) -> i32 {
    if !fields.is_null() && n_fields > 0 {
        let slice = std::slice::from_raw_parts(fields, n_fields);
        for field in slice {
            if !field.key.is_null() && !field.value.is_null() {
                let key = std::ffi::CStr::from_ptr(field.key).to_bytes();
                if key == b"MESSAGE" {
                    let msg = if field.length < 0 {
                        std::ffi::CStr::from_ptr(field.value as *const std::ffi::c_char).to_string_lossy()
                    } else {
                        let bytes = std::slice::from_raw_parts(field.value as *const u8, field.length as usize);
                        String::from_utf8_lossy(bytes)
                    };
                    if msg.contains("cursor") || msg.contains("pointer") || msg.contains("Unable to load") {
                        return 1; // 1 = G_LOG_WRITER_HANDLED (Silently drop)
                    }
                }
            }
        }
    }

    if let Some(def_writer) = DEFAULT_LOG_WRITER {
        def_writer(log_level, fields, n_fields, user_data)
    } else {
        1
    }
}

#[cfg(target_os = "linux")]
unsafe extern "C" fn classic_log_filter(
    _log_domain: *const std::ffi::c_char,
    _log_level: i32,
    message: *const std::ffi::c_char,
    _user_data: *mut std::ffi::c_void,
) {
    if !message.is_null() {
        let msg = std::ffi::CStr::from_ptr(message).to_string_lossy();
        if msg.contains("cursor") || msg.contains("pointer") || msg.contains("Unable to load") {
            return; // Silently drop
        }
        eprintln!("{}", msg);
    }
}

#[cfg(target_os = "linux")]
fn silence_gdk_pointer_logs() {
    unsafe {
        // Dynamically load libglib-2.0 to register log handlers
        let glib_libs = ["libglib-2.0.so.0\0", "libglib-2.0.so\0"];
        for lib_name in &glib_libs {
            let handle = libc::dlopen(lib_name.as_ptr() as *const _, libc::RTLD_LAZY | libc::RTLD_GLOBAL);
            if !handle.is_null() {
                // 1. Hook Structured Logging (GLib 2.50+ default used by WebKit & GDK)
                let def_writer_sym = b"g_log_writer_default\0";
                let def_writer_ptr = libc::dlsym(handle, def_writer_sym.as_ptr() as *const _);
                if !def_writer_ptr.is_null() {
                    DEFAULT_LOG_WRITER = Some(std::mem::transmute(def_writer_ptr));
                }

                let set_writer_sym = b"g_log_set_writer_func\0";
                let set_writer_ptr = libc::dlsym(handle, set_writer_sym.as_ptr() as *const _);
                if !set_writer_ptr.is_null() {
                    let g_log_set_writer_func: unsafe extern "C" fn(
                        GLogWriterFunc,
                        *mut std::ffi::c_void,
                        Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
                    ) = std::mem::transmute(set_writer_ptr);
                    g_log_set_writer_func(structured_log_filter, std::ptr::null_mut(), None);
                }

                // 2. Hook Classic Logging
                let set_handler_sym = b"g_log_set_handler\0";
                let func_ptr = libc::dlsym(handle, set_handler_sym.as_ptr() as *const _);
                if !func_ptr.is_null() {
                    let g_log_set_handler: unsafe extern "C" fn(
                        *const std::ffi::c_char,
                        i32,
                        GLogFunc,
                        *mut std::ffi::c_void,
                    ) -> u32 = std::mem::transmute(func_ptr);

                    for dom in [b"Gdk\0".as_ptr(), b"Gdk-Wayland\0".as_ptr(), b"Gtk\0".as_ptr(), b"WebKit\0".as_ptr()] {
                        g_log_set_handler(dom as *const _, 0xFF, classic_log_filter, std::ptr::null_mut());
                    }
                }

                let default_sym = b"g_log_set_default_handler\0";
                let def_ptr = libc::dlsym(handle, default_sym.as_ptr() as *const _);
                if !def_ptr.is_null() {
                    let g_log_set_default_handler: unsafe extern "C" fn(
                        GLogFunc,
                        *mut std::ffi::c_void,
                    ) -> GLogFunc = std::mem::transmute(def_ptr);
                    g_log_set_default_handler(classic_log_filter, std::ptr::null_mut());
                }
                break;
            }
        }
    }
}
