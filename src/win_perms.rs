// ==============================================================================
// Brum - Windows file and folder permissions
// Keeps Brum's private files (key file, first-login note, shared data folder)
// out of reach of other Windows accounts on the same computer.
// ==============================================================================

use std::path::Path;

/// True when Brum runs as an administrator (elevated) or as the Windows system account.
/// Always false on other platforms.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The shared data folder of an all-users install (`C:\ProgramData\Brum`).
pub fn shared_data_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("ProgramData").ok().map(|p| Path::new(&p).join("Brum"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Only the file's owner, the system account and administrators may open the file.
#[cfg(windows)]
const OWNER_ONLY: &str = "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)";
/// Folder owned by administrators; only the system account and administrators get in.
#[cfg(windows)]
const ADMIN_FOLDER: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
/// Owner only: administrators.
#[cfg(windows)]
const ADMIN_OWNER: &str = "O:BA";
/// Extra: every user may read and run what is inside (system Chewtoys folder).
#[cfg(windows)]
const USERS_READ: &str = "D:(A;OICI;0x1200a9;;;BU)";

#[cfg(windows)]
fn apply_sddl(path: &Path, sddl: &str, info: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{LocalFree, BOOL};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, ACL, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let sddl_w = wide(sddl);
    let path_w = wide(&path.to_string_lossy());

    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl_w.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut())
    };
    if ok == 0 {
        return Err(format!("permission text not understood (Windows error {})", unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }

    let mut present: BOOL = 0;
    let mut defaulted: BOOL = 0;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut owner: windows_sys::Win32::Security::PSID = std::ptr::null_mut();
    unsafe {
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
        if info & OWNER_SECURITY_INFORMATION != 0 {
            GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted);
        }
    }

    let err = unsafe {
        SetNamedSecurityInfoW(path_w.as_ptr(), SE_FILE_OBJECT, info, owner, std::ptr::null_mut(), dacl, std::ptr::null())
    };
    unsafe {
        LocalFree(sd as _);
    }
    if err == 0 {
        Ok(())
    } else {
        Err(format!("Windows error {}", err))
    }
}

/// Lets only the file's owner, the system account and administrators open this file.
/// Does nothing on other platforms (those use file modes instead).
pub fn restrict_file_to_owner(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION};
        apply_sddl(path, OWNER_ONLY, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

/// Locks the shared data folder of an all-users install: owned by administrators, open only to
/// the system account and administrators. Everything inside follows, except the system Chewtoys
/// folder, which every user may read. Only runs when Brum is elevated and the folder is
/// `C:\ProgramData\Brum`; anything else is left alone.
pub fn secure_shared_data_dir(dir: &Path) {
    #[cfg(windows)]
    {
        use tracing::{info, warn};
        use windows_sys::Win32::Security::{
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
            UNPROTECTED_DACL_SECURITY_INFORMATION,
        };

        let Some(shared) = shared_data_dir() else { return };
        let same = dir.to_string_lossy().trim_end_matches('\\').eq_ignore_ascii_case(shared.to_string_lossy().trim_end_matches('\\'));
        if !same || !dir.is_dir() || !is_elevated() {
            return;
        }
        match apply_sddl(
            dir,
            ADMIN_FOLDER,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
        ) {
            Ok(()) => info!("Shared data folder '{}' is limited to the system account and administrators", dir.display()),
            Err(e) => warn!("Could not limit access to '{}': {}", dir.display(), e),
        }
        // Older installs may hold files owned by an ordinary account, and an owner can always
        // give itself access again: hand everything inside to administrators
        let mut failed = 0usize;
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    stack.push(entry.path());
                }
                if apply_sddl(&entry.path(), ADMIN_OWNER, OWNER_SECURITY_INFORMATION).is_err() {
                    failed += 1;
                }
            }
        }
        if failed > 0 {
            warn!("Could not hand {} item(s) in '{}' to administrators", failed, dir.display());
        }

        let plugins = dir.join("plugins");
        if std::fs::create_dir_all(&plugins).is_ok() {
            if let Err(e) = apply_sddl(&plugins, USERS_READ, DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION) {
                warn!("Could not let users read '{}': {}", plugins.display(), e);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
    }
}

/// Files kept next to the database that move along with it.
#[cfg_attr(not(windows), allow(dead_code))]
fn companion_names(db_file: &str) -> Vec<String> {
    vec![
        db_file.to_string(),
        format!("{db_file}-wal"),
        format!("{db_file}-shm"),
        "brum-secrets.key".to_string(),
        "initial_admin_credentials.txt".to_string(),
        "proton".to_string(),
    ]
}

/// Earlier versions with the shipped `database_path = "brum.db"` kept the service's data in the
/// install folder (the service starts there). When Brum now runs as the service or as
/// administrator with its data in `C:\ProgramData\Brum`, it moves that data over once:
/// the database and its side files, the key file, the first-login note and Proton files.
pub fn move_install_folder_data(db_path: &Path) {
    #[cfg(windows)]
    {
        use tracing::{info, warn};
        let (Some(shared), Some(db_dir), Some(db_file)) = (
            shared_data_dir(),
            db_path.parent(),
            db_path.file_name().map(|n| n.to_string_lossy().to_string()),
        ) else {
            return;
        };
        let same = |a: &Path, b: &Path| {
            a.to_string_lossy().trim_end_matches('\\').eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches('\\'))
        };
        if !same(db_dir, &shared) || !is_elevated() {
            return;
        }
        let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return };
        let in_program_files = crate::config::windows_program_files_dirs().iter().any(|pf| {
            let pf = pf.to_string_lossy().trim_end_matches('\\').to_lowercase();
            exe_dir.to_string_lossy().to_lowercase().starts_with(&format!("{pf}\\"))
        });
        let old_db = exe_dir.join(&db_file);
        if !in_program_files || !old_db.is_file() {
            return;
        }
        if db_path.exists() {
            warn!(
                "An older database is still in the install folder ('{}'), but '{}' already exists and is used. Move the older one by hand if it holds the accounts you need.",
                old_db.display(),
                db_path.display()
            );
            return;
        }
        info!("Moving Brum's data from the install folder '{}' to '{}'", exe_dir.display(), db_dir.display());
        let mut problems = 0usize;
        for name in companion_names(&db_file) {
            let from = exe_dir.join(&name);
            if !from.exists() {
                continue;
            }
            let to = db_dir.join(&name);
            let moved = std::fs::rename(&from, &to).is_ok()
                || (from.is_file() && std::fs::copy(&from, &to).is_ok() && std::fs::remove_file(&from).is_ok());
            if moved {
                info!("Moved '{}'", name);
            } else {
                problems += 1;
                warn!("Could not move '{}' to '{}'; move it by hand while Brum is stopped", from.display(), to.display());
            }
        }
        if problems == 0 {
            info!("Brum's data now lives in '{}'", db_dir.display());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = db_path;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_companion_names() {
        let names = super::companion_names("brum.db");
        for n in ["brum.db", "brum.db-wal", "brum.db-shm", "brum-secrets.key", "initial_admin_credentials.txt", "proton"] {
            assert!(names.iter().any(|x| x == n), "{n} must move with the database");
        }
    }
}
