// ==============================================================================
// Brum - User Instance Registry & Loopback Token Handshake
// Provides strict isolation for per-user desktop instances on multi-user systems.
// Prevents cross-user port collisions and unauthorized local loopback access.
// ==============================================================================

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInstanceInfo {
    pub pid: u32,
    pub port: u16,
    pub owner: String,
    pub instance_token: String,
    pub started_at: String,
}

/// Returns the current OS user identity.
pub fn get_current_os_username() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".to_string())
}

/// Generates a cryptographically secure 256-bit instance token.
pub fn generate_instance_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Resolves the user-private runtime instance file path.
/// Protected by operating system file ACLs (only the current user and SYSTEM can access).
pub fn get_instance_file_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // %APPDATA%\Brum\run\instance.json
        dirs::config_dir().map(|d| d.join("Brum").join("run").join("instance.json"))
    }
    #[cfg(not(windows))]
    {
        // Prefer $XDG_RUNTIME_DIR/brum/instance.json on Linux (in-memory tmpfs, 0700 per user)
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            let p = PathBuf::from(runtime_dir).join("brum").join("instance.json");
            return Some(p);
        }
        dirs::data_local_dir()
            .or_else(dirs::config_dir)
            .map(|d| d.join("brum").join("run").join("instance.json"))
    }
}

/// Registers the current running instance in the user's private runtime registry.
pub fn register_user_instance(port: u16, instance_token: &str) -> std::io::Result<Option<PathBuf>> {
    if let Some(path) = get_instance_file_path() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let info = UserInstanceInfo {
            pid: std::process::id(),
            port,
            owner: get_current_os_username(),
            instance_token: instance_token.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };

        let json = serde_json::to_string_pretty(&info)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        fs::write(&path, json)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }

        return Ok(Some(path));
    }
    Ok(None)
}

/// Removes the user's instance descriptor on clean process shutdown.
pub fn unregister_user_instance() {
    if let Some(path) = get_instance_file_path() {
        if path.is_file() {
            // Verify this file belongs to the current PID before deleting
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(info) = serde_json::from_str::<UserInstanceInfo>(&content) {
                    if info.pid == std::process::id() {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
    }
}

/// Checks if a running user instance exists and is still responsive.
pub fn get_active_user_instance() -> Option<UserInstanceInfo> {
    let path = get_instance_file_path()?;
    if !path.is_file() {
        return None;
    }

    let data = fs::read_to_string(&path).ok()?;
    let info: UserInstanceInfo = serde_json::from_str(&data).ok()?;

    // Check if the registered PID is still alive
    if !is_process_alive(info.pid) {
        let _ = fs::remove_file(&path);
        return None;
    }

    Some(info)
}

/// Cross-platform check if a process ID is currently active.
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, FALSE, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
            if handle == 0 as _ {
                return false;
            }
            let mut exit_code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut exit_code);
            CloseHandle(handle);
            ok != 0 && exit_code == STILL_ACTIVE as u32
        }
    }
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
}
