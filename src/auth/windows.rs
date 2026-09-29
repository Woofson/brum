// ==============================================================================
// Brum - Windows Native System Authentication Module
// Authenticates Windows OS users (Local SAM & Active Directory) via LogonUserW
// ==============================================================================

use crate::auth::User;
use std::error::Error;
#[cfg(windows)]
use tracing::{info, warn};

/// Parses a Windows credential username string into (Optional Domain, Clean Username).
/// Supports:
/// - `DOMAIN\username` -> (Some("DOMAIN"), "username")
/// - `username@domain.com` -> (Some("domain.com"), "username@domain.com")
/// - `.\username` -> (Some("."), "username")
/// - `username` -> (None, "username")
pub fn parse_windows_username(input: &str) -> (Option<String>, String) {
    let trimmed = input.trim();
    if let Some((domain, user)) = trimmed.split_once('\\') {
        let dom = domain.trim();
        let u = user.trim();
        (if dom.is_empty() { None } else { Some(dom.to_string()) }, u.to_string())
    } else if let Some((_user, domain)) = trimmed.split_once('@') {
        let dom = domain.trim();
        (if dom.is_empty() { None } else { Some(dom.to_string()) }, trimmed.to_string())
    } else {
        (None, trimmed.to_string())
    }
}

/// Resolves the home directory for a Windows user.
pub fn resolve_windows_home_dir(username: &str) -> String {
    let clean_user = username
        .split('\\')
        .last()
        .unwrap_or(username)
        .split('@')
        .next()
        .unwrap_or(username)
        .trim();

    // Check if the authenticated user is the current process user
    let cur_user = std::env::var("USERNAME").unwrap_or_default();
    if !cur_user.is_empty() && cur_user.eq_ignore_ascii_case(clean_user) {
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            if !userprofile.is_empty() {
                return userprofile;
            }
        }
    }

    // Default to standard Windows Users folder path
    let sys_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    format!("{}\\Users\\{}", sys_drive, clean_user)
}

#[cfg(windows)]
fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Authenticates a Windows local or domain account using the Win32 LogonUserW API.
#[cfg(windows)]
pub fn authenticate_windows_user(
    username: &str,
    password: &str,
) -> Result<User, Box<dyn Error + Send + Sync>> {
    use windows_sys::Win32::Foundation::{CloseHandle, BOOL, FALSE, HANDLE, TRUE};
    use windows_sys::Win32::Security::{
        CheckTokenMembership, CreateWellKnownSid, LogonUserW,
        LOGON32_LOGON_INTERACTIVE, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT,
        WinBuiltinAdministratorsSid,
    };

    let (domain_opt, clean_username) = parse_windows_username(username);
    let user_wide = to_wide_null(&clean_username);
    let pass_wide = to_wide_null(password);
    let domain_wide: Option<Vec<u16>> = domain_opt.as_deref().map(to_wide_null);

    let domain_ptr = match domain_wide.as_ref() {
        Some(w) => w.as_ptr(),
        None => std::ptr::null(),
    };

    let mut token_handle: HANDLE = 0 as HANDLE;

    // 1. Try LOGON32_LOGON_NETWORK first (standard network authentication, works without SeTcbPrivilege)
    let mut logon_success = unsafe {
        LogonUserW(
            user_wide.as_ptr(),
            domain_ptr,
            pass_wide.as_ptr(),
            LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT,
            &mut token_handle,
        )
    };

    // 2. If NETWORK logon fails or returns invalid parameter, fall back to LOGON32_LOGON_INTERACTIVE
    if logon_success == 0 {
        logon_success = unsafe {
            LogonUserW(
                user_wide.as_ptr(),
                domain_ptr,
                pass_wide.as_ptr(),
                LOGON32_LOGON_INTERACTIVE,
                LOGON32_PROVIDER_DEFAULT,
                &mut token_handle,
            )
        };
    }

    if logon_success == 0 || token_handle == (0 as HANDLE) {
        let err_code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        warn!(
            username = %username,
            win32_error = err_code,
            "Windows native authentication (LogonUserW) failed"
        );
        return Err("Invalid Windows username or password".into());
    }

    // 3. Inspect administrator group membership
    let mut is_admin = false;
    let mut sid_buffer = [0u8; 68]; // SECURITY_MAX_SID_SIZE = 68
    let mut sid_size = sid_buffer.len() as u32;
    let sid_ptr = sid_buffer.as_mut_ptr() as *mut std::ffi::c_void;

    let create_sid_ok = unsafe {
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            std::ptr::null_mut(),
            sid_ptr,
            &mut sid_size,
        )
    };

    if create_sid_ok != 0 {
        let mut is_member: BOOL = FALSE;
        let check_ok = unsafe { CheckTokenMembership(token_handle, sid_ptr, &mut is_member) };
        if check_ok != 0 && is_member == TRUE {
            is_admin = true;
        }
    }

    // Fallback admin check for well-known Administrator username
    if !is_admin && clean_username.eq_ignore_ascii_case("Administrator") {
        is_admin = true;
    }

    // Cleanly close the logon token handle
    unsafe {
        CloseHandle(token_handle);
    }

    let role = if is_admin { "admin" } else { "user" }.to_string();
    let home_dir = resolve_windows_home_dir(username);

    info!(
        username = %username,
        role = %role,
        home = %home_dir,
        "Windows native user successfully authenticated via LogonUserW"
    );

    let mut user = User {
        id: 0,
        username: username.to_string(),
        nickname: Some(clean_username),
        full_name: None,
        bio: None,
        email: None,
        avatar_url: None,
        role,
        home_dir,
        is_pam: true, // Marked as OS-managed system user
        is_disabled: false,
        allowed_services: "[\"*\"]".to_string(),
        allowed_roots: "[\"*\"]".to_string(),
        can_install_plugins: is_admin,
        allowed_plugins: "[\"*\"]".to_string(),
        blocked_plugins: "[]".to_string(),
    };
    user.resolve_avatar();

    Ok(user)
}

/// Fallback for non-Windows platforms (e.g. Linux development / cross-testing)
#[cfg(not(windows))]
pub fn authenticate_windows_user(
    _username: &str,
    _password: &str,
) -> Result<User, Box<dyn Error + Send + Sync>> {
    Err("Windows native authentication (LogonUserW) is only available on Windows builds".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_windows_username_formats() {
        // Domain\User
        let (dom, u) = parse_windows_username(r"CORP\alice");
        assert_eq!(dom, Some("CORP".to_string()));
        assert_eq!(u, "alice");

        // Local dot prefix .\bob
        let (dom, u) = parse_windows_username(r".\bob");
        assert_eq!(dom, Some(".".to_string()));
        assert_eq!(u, "bob");

        // User@domain.com (UPN)
        let (dom, u) = parse_windows_username("alice@corp.example.com");
        assert_eq!(dom, Some("corp.example.com".to_string()));
        assert_eq!(u, "alice@corp.example.com");

        // Simple local user
        let (dom, u) = parse_windows_username("administrator");
        assert_eq!(dom, None);
        assert_eq!(u, "administrator");
    }

    #[test]
    fn test_resolve_windows_home_dir() {
        let home = resolve_windows_home_dir(r"WORKGROUP\charlie");
        assert!(home.contains("charlie"));
    }
}
