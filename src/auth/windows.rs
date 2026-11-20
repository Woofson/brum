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
        1793 => "account has expired",
        1907 => "password must be changed before signing in",
        _ => "other Windows error",
    }
}

/// What to do after Windows refused one sign-in try.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRefusal {
    /// Ask again for the same name as an interactive sign-in (only when the network
    /// kind is not allowed, or Windows could not check it at all, for example offline).
    TryInteractive,
    /// Windows checked the password and said no: skip to the next way of writing the
    /// name, if any. Asking the same name again would be one more failed login.
    NextName,
    /// The account is locked, disabled, expired or barred: stop asking.
    Stop,
}

/// Every try Windows turns down counts toward its account lockout (10 by default on
/// Windows 11), so Brum only asks again when the refusal was not about the password.
pub fn after_refusal(code: u32, interactive: bool) -> AfterRefusal {
    match code {
        // locked out, disabled, expired, must change password, restriction, logon hours, workstation
        1909 | 1331 | 1330 | 1907 | 1327 | 1328 | 1329 | 1793 => AfterRefusal::Stop,
        1326 => AfterRefusal::NextName,
        _ if interactive => AfterRefusal::NextName,
        // 1385/1387 (network sign-in not granted), 1311 (offline: only interactive
        // sign-in knows saved logins), and anything unexpected
        _ => AfterRefusal::TryInteractive,
    }
}

/// The way of asking Windows that worked last time for a typed name, so later sign-ins
/// (and the lock screen) ask once instead of going through every way again.
#[cfg(windows)]
fn remembered_logons() -> &'static std::sync::Mutex<std::collections::HashMap<String, (Option<String>, String, bool)>> {
    static CELL: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, (Option<String>, String, bool)>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(Default::default)
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

/// Looks up a user account of this Windows machine (local or domain) by name, without its
/// password. Returns its profile folder as Windows records it, or the usual `C:\\Users\\<name>`
/// when it has never signed in here. `None` when no such user exists; groups and built-in names
/// such as SYSTEM are not user accounts and count as missing.
#[cfg(windows)]
pub fn lookup_windows_account(username: &str) -> Option<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{LookupAccountNameW, SidTypeUser, SID_NAME_USE};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    let name = username.trim();
    if name.is_empty() {
        return None;
    }
    let wide_name = to_wide_null(name);
    let mut sid = vec![0u8; 256];
    let mut sid_len = sid.len() as u32;
    let mut domain = vec![0u16; 256];
    let mut domain_len = domain.len() as u32;
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: every buffer is passed with its real size; Windows writes no more than that
    let found = unsafe {
        LookupAccountNameW(std::ptr::null(), wide_name.as_ptr(), sid.as_mut_ptr() as _, &mut sid_len, domain.as_mut_ptr(), &mut domain_len, &mut kind)
    };
    if found == 0 || kind != SidTypeUser {
        return None;
    }

    // The profile folder: HKLM\...\ProfileList\<SID>\ProfileImagePath
    let mut sid_text_ptr: *mut u16 = std::ptr::null_mut();
    // SAFETY: sid holds the SID written above; Windows allocates the text, freed with LocalFree
    let sid_text = unsafe {
        if ConvertSidToStringSidW(sid.as_mut_ptr() as _, &mut sid_text_ptr) == 0 || sid_text_ptr.is_null() {
            None
        } else {
            let mut len = 0;
            while *sid_text_ptr.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(sid_text_ptr, len));
            LocalFree(sid_text_ptr as _);
            Some(text)
        }
    };
    if let Some(sid_text) = sid_text {
        let key = to_wide_null(&format!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList\\{sid_text}"));
        let value = to_wide_null("ProfileImagePath");
        let mut buf = vec![0u16; 1024];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: buf is passed with its size in bytes; REG_EXPAND_SZ values come back expanded
        let rc = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as _, &mut bytes)
        };
        if rc == 0 {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let path = String::from_utf16_lossy(&buf[..len]);
            if !path.trim().is_empty() {
                return Some(path);
            }
        }
    }
    Some(resolve_windows_home_dir(name))
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

    // The ways to try: the one that worked last time for this name alone, else each way of
    // handing the name to Windows, with a network sign-in first. Windows counts every refusal
    // toward locking the account, so after "wrong password" a name is never asked again.
    let memory_key = username.trim().to_lowercase();
    let remembered = remembered_logons().lock().ok().and_then(|m| m.get(&memory_key).cloned());
    let from_memory = remembered.is_some();
    let plan: Vec<(Option<String>, String, bool)> = match remembered {
        Some(way) => vec![way],
        None => windows_logon_candidates(username).into_iter().map(|(d, n)| (d, n, false)).collect(),
    };

    let pass_wide = to_wide_null(password);
    'outer: for (domain_opt, name, start_interactive) in plan {
        let user_wide = to_wide_null(&name);
        let domain_wide: Option<Vec<u16>> = domain_opt.as_deref().map(to_wide_null);
        let domain_ptr = match domain_wide.as_ref() {
            Some(w) => w.as_ptr(),
            None => std::ptr::null(),
        };
        let shown = match &domain_opt {
            Some(d) => format!("{}\\{}", d, name),
            None => name.clone(),
        };

        let mut interactive = start_interactive;
        loop {
            let (kind, logon_type) = if interactive {
                ("interactive", LOGON32_LOGON_INTERACTIVE)
            } else {
                ("network", LOGON32_LOGON_NETWORK)
            };
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
                if let Ok(mut m) = remembered_logons().lock() {
                    m.insert(memory_key.clone(), (domain_opt.clone(), name.clone(), interactive));
                }
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
            match after_refusal(err_code, interactive) {
                AfterRefusal::TryInteractive => interactive = true,
                AfterRefusal::NextName => break,
                AfterRefusal::Stop => break 'outer,
            }
        }
    }

    // The remembered way stopped working for a reason other than the password (for example
    // the account's sign-in rights changed): forget it, so the next sign-in tries every way.
    if !logged_on && from_memory && !refusals.iter().any(|&c| matches!(after_refusal(c, true), AfterRefusal::Stop) || c == 1326) {
        if let Ok(mut m) = remembered_logons().lock() {
            m.remove(&memory_key);
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
    fn wrong_password_is_never_asked_again_for_the_same_name() {
        // Wrong password: never retried as an interactive sign-in (each try counts toward lockout)
        assert_eq!(after_refusal(1326, false), AfterRefusal::NextName);
        assert_eq!(after_refusal(1326, true), AfterRefusal::NextName);
        // Locked, disabled, expired, must change password: stop at once
        for code in [1909, 1331, 1330, 1793, 1907, 1327, 1328, 1329] {
            assert_eq!(after_refusal(code, false), AfterRefusal::Stop, "code {code}");
            assert_eq!(after_refusal(code, true), AfterRefusal::Stop, "code {code}");
        }
        // Network sign-in not granted, or offline: the interactive kind may work
        for code in [1385, 1387, 1311] {
            assert_eq!(after_refusal(code, false), AfterRefusal::TryInteractive, "code {code}");
            assert_eq!(after_refusal(code, true), AfterRefusal::NextName, "code {code}");
        }
    }

    #[test]
    fn plain_name_wrong_password_costs_one_windows_failure() {
        // Walk the plan as the sign-in does, with Windows always saying "wrong password"
        let count = |input: &str| {
            let mut tries = 0;
            for _ in windows_logon_candidates(input) {
                let mut interactive = false;
                loop {
                    tries += 1;
                    match after_refusal(1326, interactive) {
                        AfterRefusal::TryInteractive => interactive = true,
                        AfterRefusal::NextName => break,
                        AfterRefusal::Stop => return tries,
                    }
                }
            }
            tries
        };
        assert_eq!(count("bolt"), 1);
        assert_eq!(count(r"HAWK\bolt"), 1);
        assert_eq!(count("me@mail.com"), 2, "both ways of writing a Microsoft account, once each");
    }

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
