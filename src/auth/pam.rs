#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString};
#[cfg(target_os = "linux")]
use std::os::raw::{c_char, c_int, c_void};
#[cfg(target_os = "linux")]
use std::ptr;

#[cfg(target_os = "linux")]
const PAM_SUCCESS: c_int = 0;
#[cfg(target_os = "linux")]
const PAM_PROMPT_ECHO_OFF: c_int = 1;
#[cfg(target_os = "linux")]
const PAM_PROMPT_ECHO_ON: c_int = 2;
#[cfg(target_os = "linux")]
const PAM_CONV_ERR: c_int = 19;
#[cfg(target_os = "linux")]
const PAM_BUF_ERR: c_int = 5;
#[cfg(target_os = "linux")]
const PAM_AUTH_ERR: c_int = 7;

#[cfg(target_os = "linux")]
#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct PamConv {
    conv: Option<
        unsafe extern "C" fn(
            c_int,
            *mut *const PamMessage,
            *mut *mut PamResponse,
            *mut c_void,
        ) -> c_int,
    >,
    appdata_ptr: *mut c_void,
}

#[cfg(target_os = "linux")]
type PamHandle = c_void;

#[cfg(target_os = "linux")]
type FnPamStart = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const PamConv,
    *mut *mut PamHandle,
) -> c_int;

#[cfg(target_os = "linux")]
type FnPamAuthenticate = unsafe extern "C" fn(*mut PamHandle, c_int) -> c_int;
#[cfg(target_os = "linux")]
type FnPamAcctMgmt = unsafe extern "C" fn(*mut PamHandle, c_int) -> c_int;
#[cfg(target_os = "linux")]
type FnPamEnd = unsafe extern "C" fn(*mut PamHandle, c_int) -> c_int;
#[cfg(target_os = "linux")]
type FnPamStrerror = unsafe extern "C" fn(*mut PamHandle, c_int) -> *const c_char;

#[cfg(target_os = "linux")]
unsafe extern "C" fn pam_conversation_cb(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int {
    if num_msg <= 0 || num_msg > 32 || msg.is_null() || resp.is_null() || appdata_ptr.is_null() {
        return PAM_CONV_ERR;
    }

    let password_cstr = &*(appdata_ptr as *const CString);
    let resp_arr = libc::calloc(num_msg as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
    if resp_arr.is_null() {
        return PAM_BUF_ERR;
    }

    for i in 0..num_msg as isize {
        let msg_ptr = *msg.offset(i);
        if msg_ptr.is_null() {
            continue;
        }
        let style = (*msg_ptr).msg_style;
        if style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON {
            let resp_str = libc::strdup(password_cstr.as_ptr());
            (*resp_arr.offset(i)).resp = resp_str;
            (*resp_arr.offset(i)).resp_retcode = 0;
        } else {
            (*resp_arr.offset(i)).resp = ptr::null_mut();
            (*resp_arr.offset(i)).resp_retcode = 0;
        }
    }

    *resp = resp_arr;
    PAM_SUCCESS
}

/// Why a PAM sign-in did not succeed. `code` is PAM's own result number, when PAM got that far.
#[derive(Debug, Clone)]
pub struct PamFailure {
    pub code: Option<i32>,
    pub message: String,
}

impl PamFailure {
    fn setup(message: impl Into<String>) -> Self {
        Self { code: None, message: message.into() }
    }

    /// True when this login service did its job and said no (for example a wrong password,
    /// a locked or expired account). Trying more services then only adds failed logins on the
    /// machine, which can lock the account (pam_faillock). False when the service itself could
    /// not be used (missing, broken, does not know the account), so another one may be tried.
    pub fn is_final(&self) -> bool {
        // 1 open, 2 symbol, 3 service, 4 system, 5 buffer, 9 auth info unavailable,
        // 10 user unknown, 19 conversation, 26 abort
        !matches!(self.code, None | Some(1 | 2 | 3 | 4 | 5 | 9 | 10 | 19 | 26))
    }
}

impl std::fmt::Display for PamFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(target_os = "linux")]
pub fn authenticate(service: &str, username: &str, password: &str) -> Result<(), PamFailure> {
    unsafe {
        let lib_names = [
            "libpam.so.0",
            "libpam.so",
            "/usr/lib/libpam.so.0",
            "/usr/lib64/libpam.so.0",
            "/lib/x86_64-linux-gnu/libpam.so.0",
            "/usr/lib/x86_64-linux-gnu/libpam.so.0",
            "/lib64/libpam.so.0",
            "/lib/libpam.so.0",
        ];

        let mut handle = ptr::null_mut();
        for name in &lib_names {
            let c_name = match CString::new(*name) {
                Ok(c) => c,
                Err(_) => continue,
            };
            handle = libc::dlopen(c_name.as_ptr(), libc::RTLD_NOW);
            if !handle.is_null() {
                break;
            }
        }

        if handle.is_null() {
            return Err(PamFailure::setup("PAM shared library (libpam.so.0) not found on system"));
        }

        let pam_start_sym = libc::dlsym(handle, b"pam_start\0".as_ptr() as *const c_char);
        let pam_authenticate_sym = libc::dlsym(handle, b"pam_authenticate\0".as_ptr() as *const c_char);
        let pam_acct_mgmt_sym = libc::dlsym(handle, b"pam_acct_mgmt\0".as_ptr() as *const c_char);
        let pam_end_sym = libc::dlsym(handle, b"pam_end\0".as_ptr() as *const c_char);
        let pam_strerror_sym = libc::dlsym(handle, b"pam_strerror\0".as_ptr() as *const c_char);

        if pam_start_sym.is_null() || pam_authenticate_sym.is_null() || pam_end_sym.is_null() {
            libc::dlclose(handle);
            return Err(PamFailure::setup("Failed to resolve PAM symbols from libpam.so.0"));
        }

        let pam_start: FnPamStart = std::mem::transmute(pam_start_sym);
        let pam_authenticate: FnPamAuthenticate = std::mem::transmute(pam_authenticate_sym);
        let pam_acct_mgmt: Option<FnPamAcctMgmt> = if !pam_acct_mgmt_sym.is_null() {
            Some(std::mem::transmute(pam_acct_mgmt_sym))
        } else {
            None
        };
        let pam_end: FnPamEnd = std::mem::transmute(pam_end_sym);
        let pam_strerror: Option<FnPamStrerror> = if !pam_strerror_sym.is_null() {
            Some(std::mem::transmute(pam_strerror_sym))
        } else {
            None
        };

        let to_c = |v: &str| CString::new(v).map_err(|e| {
            libc::dlclose(handle);
            PamFailure { code: Some(PAM_AUTH_ERR), message: e.to_string() }
        });
        let c_service = to_c(service)?;
        let c_user = to_c(username)?;
        let c_pass = to_c(password)?;

        let conv = PamConv {
            conv: Some(pam_conversation_cb),
            appdata_ptr: &c_pass as *const CString as *mut c_void,
        };

        let mut pamh: *mut PamHandle = ptr::null_mut();
        let status = pam_start(c_service.as_ptr(), c_user.as_ptr(), &conv, &mut pamh);
        if status != PAM_SUCCESS || pamh.is_null() {
            libc::dlclose(handle);
            return Err(PamFailure::setup(format!("pam_start failed with status {}", status)));
        }

        let auth_status = pam_authenticate(pamh, 0);
        let mut final_status = auth_status;
        if auth_status == PAM_SUCCESS {
            if let Some(acct_mgmt) = pam_acct_mgmt {
                let acct_status = acct_mgmt(pamh, 0);
                if acct_status != PAM_SUCCESS {
                    final_status = acct_status;
                }
            }
        }

        let err_msg = if final_status != PAM_SUCCESS {
            if let Some(strerror_fn) = pam_strerror {
                let err_ptr = strerror_fn(pamh, final_status);
                if !err_ptr.is_null() {
                    CStr::from_ptr(err_ptr).to_string_lossy().to_string()
                } else {
                    format!("PAM authentication error code {}", final_status)
                }
            } else {
                format!("PAM authentication error code {}", final_status)
            }
        } else {
            String::new()
        };

        pam_end(pamh, final_status);
        libc::dlclose(handle);

        if final_status == PAM_SUCCESS {
            Ok(())
        } else {
            Err(PamFailure { code: Some(final_status), message: err_msg })
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn authenticate(_service: &str, _username: &str, _password: &str) -> Result<(), PamFailure> {
    Err(PamFailure::setup("PAM authentication is only supported on Linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_password_ends_the_search_for_a_login_service() {
        let f = |code: Option<i32>| PamFailure { code, message: String::new() };
        // wrong password, too many tries, expired, new password needed, permission denied
        for code in [7, 11, 13, 12, 6, 8] {
            assert!(f(Some(code)).is_final(), "code {code}");
        }
        // the service itself could not be used: another may be tried
        for code in [1, 2, 3, 4, 5, 9, 10, 19, 26] {
            assert!(!f(Some(code)).is_final(), "code {code}");
        }
        assert!(!f(None).is_final(), "PAM not loaded or not started");
    }
}
