// ==============================================================================
// Brum - Advanced Logging & Diagnostics Engine
// Supports configurable log levels, rolling file rotation, and secure
// headless credential output.
// ==============================================================================

use crate::config::LoggingConfig;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

static LOGGING_INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Initializes the global tracing subscriber with console output and/or rolling file appenders.
pub fn init_logging(config: &LoggingConfig) -> Option<WorkerGuard> {
    if LOGGING_INITIALIZED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return None;
    }

    // 1. Build EnvFilter from RUST_LOG environment variable or config level
    let target_level = config.level.to_lowercase();
    let filter_directives = match target_level.as_str() {
        "trace" => "brum=trace,tower_http=debug",
        "debug" => "brum=debug,tower_http=info",
        "warn" => "brum=warn,tower_http=warn",
        "error" => "brum=error,tower_http=error",
        _ => "brum=info,tower_http=info",
    };

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(filter_directives));

    let (file_layer, guard) = if config.file_enabled {
        let log_dir = config.resolve_log_directory();
        if let Err(e) = fs::create_dir_all(&log_dir) {
            eprintln!("Failed to create log directory '{}': {}", log_dir.display(), e);
            (None, None)
        } else {
            // Housekeeping: clean up old rotated files if max_files is configured
            if config.max_files > 0 {
                cleanup_old_logs(&log_dir, config.max_files);
            }

            let file_appender = match config.rotation.to_lowercase().as_str() {
                "hourly" => tracing_appender::rolling::hourly(&log_dir, "brum.log"),
                "never" => tracing_appender::rolling::never(&log_dir, "brum.log"),
                _ => tracing_appender::rolling::daily(&log_dir, "brum.log"),
            };

            let (non_blocking, worker_guard) = tracing_appender::non_blocking(file_appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(true)
                .with_thread_ids(false);

            (Some(layer), Some(worker_guard))
        }
    } else {
        (None, None)
    };

    // 3. Prepare console layer if console_enabled is true
    let console_layer = if config.console_enabled {
        Some(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stdout)
                .with_ansi(true),
        )
    } else {
        None
    };

    // 4. Register layers with the subscriber registry
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .try_init();

    if config.file_enabled {
        info!(
            "Brum logging initialized. Log level: '{}', Log directory: '{}'",
            config.level,
            config.resolve_log_directory().display()
        );
    }

    guard
}

/// Rotates and retains up to `max_files` historical log files in the given directory.
pub fn cleanup_old_logs(log_dir: &Path, max_files: usize) {
    if max_files == 0 {
        return;
    }

    let entries = match fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut log_files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.starts_with("brum.log") && file_name != "brum.log" {
                    if let Ok(metadata) = entry.metadata() {
                        if let Ok(modified) = metadata.modified() {
                            log_files.push((path, modified));
                        }
                    }
                }
            }
        }
    }

    // Sort oldest first
    log_files.sort_by_key(|(_, time)| *time);

    if log_files.len() > max_files {
        let remove_count = log_files.len() - max_files;
        for (path, _) in log_files.iter().take(remove_count) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Writes initial dynamic administrative credentials to a dedicated credentials discovery file.
/// This allows headless service administrators (Windows Service, Docker, systemd) to discover the
/// one-time generated password without needing interactive terminal access.
pub fn write_initial_admin_credentials(
    log_dir: &Path,
    username: &str,
    password: &str,
) -> std::io::Result<PathBuf> {
    fs::create_dir_all(log_dir)?;
    let target = log_dir.join("initial_admin_credentials.txt");

    let now = chrono::Utc::now().to_rfc3339();
    let content = format!(
        "================================================================================\n\
         🛡️  BRUM INITIAL ADMINISTRATIVE CREDENTIALS\n\
        ================================================================================\n\
         Generated : {}\n\
         Username  : {}\n\
         Password  : {}\n\n\
         SECURITY NOTICE:\n\
         - Please log in and change this password immediately via Settings -> Security.\n\
         - Alternatively, log in with any local Windows / Linux Administrator account.\n\
         - Delete or restrict this file once credentials have been saved.\n\
        ================================================================================\n",
        now, username, password
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&target)?;
        file.write_all(content.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&target, content)?;
    }

    Ok(target)
}

/// Automatically removes the initial administrative credentials file once the admin
/// has performed their first successful login or updated their password.
pub fn cleanup_initial_admin_credentials() {
    let log_dir = crate::config::LoggingConfig::default().resolve_log_directory();
    let target = log_dir.join("initial_admin_credentials.txt");
    if target.is_file() {
        if let Err(e) = fs::remove_file(&target) {
            tracing::warn!("Failed to remove initial admin credentials file: {}", e);
        } else {
            tracing::info!("Initial administrative credentials file cleaned up successfully.");
        }
    }
}
