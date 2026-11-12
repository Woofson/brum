// ==============================================================================
// Brum - Windows Native System Authentication Module
// Authenticates Windows OS users (Local SAM & Active Directory) via LogonUserW
// ==============================================================================

use crate::auth::User;
use std::error::Error;
#[cfg(windows)]
use tracing::{info, warn};

/// The ways a typed username is handed to Windows, tried in order (domain, name).
/// - `DOMAIN\\username` -> that domain and name
/// - `MicrosoftAccount\\me@mail.com` -> as typed, then the bare address
/// - `me@mail.com` -> the bare address (Windows wants no domain with it), then as a Microsoft account
/// - `username` -> the name alone (this computer first, then trusted domains)
pub fn windows_logon_candidates(input: &str) -> Vec<(Option<String>, String)> {
    let trimmed = input.trim();
    let mut out = Vec::new();
    if let Some((domain, user)) = trimmed.split_once('\\') {
        let dom = domain.trim();
        let u = user.trim().to_string();
        if dom.is_empty() {
            out.push((None, u));
        } else {
            out.push((Some(dom.to_string()), u.clone()));
            if dom.eq_ignore_ascii_case("MicrosoftAccount") && u.contains('@') {
                out.push((None, u));
            }
        }
    } else if trimmed.contains('@') {
        out.push((None, trimmed.to_string()));
        out.push((Some("MicrosoftAccount".to_string()), trimmed.to_string()));
    } else {
        out.push((None, trimmed.to_string()));
    }
    out
}

/// The name Brum files a Windows login under, so both ways of typing
/// a Microsoft account land on the same Brum profile.
pub fn windows_profile_username(input: &str) -> String {
    let trimmed = input.trim();
    match trimmed.split_once('\\') {
        Some((dom, user)) if dom.trim().eq_ignore_ascii_case("MicrosoftAccount") => user.trim().to_string(),
        _ => trimmed.to_string(),
    }
}

/// When Windows turns down a Microsoft account address with "wrong username or password" in
/// every form, the password Windows checks is often not the one typed: Windows only knows a
/// Microsoft account password after someone has signed in to this PC with it (not a PIN).
pub fn microsoft_account_hint(input: &str, refusals: &[u32]) -> Option<&'static str> {
    let is_address = windows_profile_username(input).contains('@');
    if is_address && !refusals.is_empty() && refusals.iter().all(|&c| c == 1326) {
        Some(
            "Hint: Windows only knows a Microsoft account password after it has been used to sign in \
             to this PC (a PIN, fingerprint or face does not count), and keeps the one from that sign-in. \
             If the account was set up with a code or the Authenticator app, the password changed online, \
             or Windows Hello-only sign-in is on (Settings > Accounts > Sign-in options), sign in to \
             Windows once with the password, then try again.",
        )
    } else {
        None
    }
}

/// Plain-words reason for a Windows login refusal code, for the log.
pub fn describe_logon_error(code: u32) -> &'static str {
    match code {
        1326 => "wrong username or password",
        1327 => "account restriction (for example a blank password, or a policy that blocks password sign-in)",
        1328 => "outside the account's allowed logon hours",
        1329 => "not allowed to sign in from this computer",
        1330 => "password has expired",
        1331 => "account is disabled",
        1385 | 1387 => "this kind of sign-in is not granted to the account on this computer",
        1355 => "domain not found",
        1311 => "no sign-in server available (offline, and no saved sign-in for this account)",
        1909 => "account is locked out",
        1907 => "password must be changed before signing in",
        _ => "other Windows error",
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
    use windows_sys::Win32::UI::Shell::GetUserProfileDirectoryW;

    let mut token_handle: HANDLE = 0 as HANDLE;
    let mut logged_on = false;
    let mut clean_username = String::new();
    let mut refusals: Vec<u32> = Vec::new();

    // Try each way of handing the name to Windows; each with a network sign-in
    // first, then an interactive one. Every refusal is logged with its reason.
    'outer: for (domain_opt, name) in windows_logon_candidates(username) {
        let user_wide = to_wide_null(&name);
        let pass_wide = to_wide_null(password);
        let domain_wide: Option<Vec<u16>> = domain_opt.as_deref().map(to_wide_null);
        let domain_ptr = match domain_wide.as_ref() {
            Some(w) => w.as_ptr(),
            None => std::ptr::null(),
        };
        let shown = match &domain_opt {
            Some(d) => format!("{}\\{}", d, name),
            None => name.clone(),
        };

        for (kind, logon_type) in [("network", LOGON32_LOGON_NETWORK), ("interactive", LOGON32_LOGON_INTERACTIVE)] {
            let ok = unsafe {
                LogonUserW(
                    user_wide.as_ptr(),
                    domain_ptr,
                    pass_wide.as_ptr(),
                    logon_type,
                    LOGON32_PROVIDER_DEFAULT,
                    &mut token_handle,
                )
            };
            if ok != 0 && token_handle != (0 as HANDLE) {
                info!(tried_as = %shown, sign_in = kind, "Windows accepted the sign-in");
                clean_username = name.split('@').next().unwrap_or(&name).to_string();
                logged_on = true;
                break 'outer;
            }
            let err_code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            refusals.push(err_code);
            warn!(
                tried_as = %shown,
                sign_in = kind,
                win32_error = err_code,
                reason = describe_logon_error(err_code),
                "Windows refused the sign-in"
            );
        }
    }

    if !logged_on {
        if let Some(hint) = microsoft_account_hint(username, &refusals) {
            warn!(tried_as = %username, "{}", hint);
        }
        return Err("Invalid Windows username or password".into());
    }

    // The real profile folder (Microsoft accounts often get a shortened folder name)
    let profile_dir = {
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = unsafe { GetUserProfileDirectoryW(token_handle, buf.as_mut_ptr(), &mut len) };
        if ok != 0 {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end])).filter(|p| !p.trim().is_empty())
        } else {
            None
        }
    };

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
    let home_dir = profile_dir.unwrap_or_else(|| resolve_windows_home_dir(username));
    let profile_name = windows_profile_username(username);

    info!(
        username = %profile_name,
        role = %role,
        home = %home_dir,
        "Windows native user successfully authenticated via LogonUserW"
    );

    let mut user = User {
        id: 0,
        username: profile_name,
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
        auth_source: Some("windows".to_string()),
        shell: None,
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
    fn test_windows_logon_candidates() {
        let some = |d: &str, u: &str| (Some(d.to_string()), u.to_string());
        let none = |u: &str| (None, u.to_string());

        assert_eq!(windows_logon_candidates(r"CORP\alice"), vec![some("CORP", "alice")]);
        assert_eq!(windows_logon_candidates(r".\bob"), vec![some(".", "bob")]);
        assert_eq!(windows_logon_candidates("administrator"), vec![none("administrator")]);

        // A full address goes to Windows without a split-off domain, then as a Microsoft account
        assert_eq!(
            windows_logon_candidates(" me@mail.com "),
            vec![none("me@mail.com"), some("MicrosoftAccount", "me@mail.com")]
        );
        assert_eq!(
            windows_logon_candidates(r"MicrosoftAccount\me@mail.com"),
            vec![some("MicrosoftAccount", "me@mail.com"), none("me@mail.com")]
        );
    }

    #[test]
    fn test_windows_profile_username() {
        assert_eq!(windows_profile_username(r"MicrosoftAccount\me@mail.com"), "me@mail.com");
        assert_eq!(windows_profile_username(r"microsoftaccount\me@mail.com"), "me@mail.com");
        assert_eq!(windows_profile_username("me@mail.com"), "me@mail.com");
        assert_eq!(windows_profile_username(r"CORP\alice"), r"CORP\alice");
    }

    #[test]
    fn test_microsoft_account_hint() {
        // Every form refused as wrong name/password: hint for addresses only
        assert!(microsoft_account_hint("me@mail.com", &[1326, 1326, 1326, 1326]).is_some());
        assert!(microsoft_account_hint(r"MicrosoftAccount\me@mail.com", &[1326, 1326]).is_some());
        assert!(microsoft_account_hint("alice", &[1326, 1326]).is_none());
        // Another reason in the mix, or nothing tried: no hint
        assert!(microsoft_account_hint("me@mail.com", &[1326, 1331]).is_none());
        assert!(microsoft_account_hint("me@mail.com", &[]).is_none());
    }

    #[test]
    fn test_describe_logon_error() {
        assert_eq!(describe_logon_error(1326), "wrong username or password");
        assert_eq!(describe_logon_error(4242), "other Windows error");
    }

    #[test]
    fn test_resolve_windows_home_dir() {
        let home = resolve_windows_home_dir(r"WORKGROUP\charlie");
        assert!(home.contains("charlie"));
    }
}
