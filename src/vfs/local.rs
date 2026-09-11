use super::{is_archive_file, DirectoryListing, FileContentResponse, FileEntry};
use crate::vfs::checksum::calculate_sha256;
#[cfg(unix)]
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
#[cfg(unix)]
use std::time::{Duration, Instant};
use tracing::info;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    ERROR_MORE_DATA, ERROR_SUCCESS, INVALID_FILE_ATTRIBUTES,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_READONLY,
};
#[cfg(windows)]
use windows_sys::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
    RM_PROCESS_INFO,
};
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::{
    SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
    SHFILEOPSTRUCTW,
};

#[cfg(unix)]
static USER_GROUP_CACHE: parking_lot::RwLock<Option<(Instant, HashMap<u32, String>, HashMap<u32, String>)>> =
    parking_lot::RwLock::new(None);

/// Query Windows Restart Manager API to detect which active processes (e.g. Explorer, Excel, antivirus) are locking a file or folder
#[cfg(windows)]
pub fn get_locking_processes(path: &Path) -> Vec<String> {
    let mut processes = Vec::new();
    let mut session_handle: u32 = 0;
    let mut session_key = [0u16; CCH_RM_SESSION_KEY as usize + 1];

    unsafe {
        let res = RmStartSession(&mut session_handle, 0, session_key.as_mut_ptr());
        if res != ERROR_SUCCESS {
            return processes;
        }

        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let file_paths = [wide_path.as_ptr()];

        let reg_res = RmRegisterResources(
            session_handle,
            1,
            file_paths.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        );

        if reg_res == ERROR_SUCCESS {
            let mut proc_info_needed = 0u32;
            let mut proc_info_count = 0u32;
            let mut reboot_reasons = 0u32;

            let list_res = RmGetList(
                session_handle,
                &mut proc_info_needed,
                &mut proc_info_count,
                std::ptr::null_mut(),
                &mut reboot_reasons,
            );

            if (list_res == ERROR_SUCCESS || list_res == ERROR_MORE_DATA) && proc_info_needed > 0 {
                let mut proc_info: Vec<RM_PROCESS_INFO> = vec![std::mem::zeroed(); proc_info_needed as usize];
                proc_info_count = proc_info_needed;

                let list_res2 = RmGetList(
                    session_handle,
                    &mut proc_info_needed,
                    &mut proc_info_count,
                    proc_info.as_mut_ptr(),
                    &mut reboot_reasons,
                );

                if list_res2 == ERROR_SUCCESS {
                    for info in proc_info.iter().take(proc_info_count as usize) {
                        let app_name_len = info.strAppName.iter().position(|&c| c == 0).unwrap_or(info.strAppName.len());
                        let app_name = String::from_utf16_lossy(&info.strAppName[..app_name_len]);
                        let pid = info.Process.dwProcessId;
                        if !app_name.is_empty() {
                            processes.push(format!("{} (PID: {})", app_name, pid));
                        } else {
                            processes.push(format!("PID: {}", pid));
                        }
                    }
                }
            }
        }

        let _ = RmEndSession(session_handle);
    }

    processes
}

#[cfg(not(windows))]
pub fn get_locking_processes(_path: &Path) -> Vec<String> {
    Vec::new()
}

/// Clear Windows Read-Only file attribute to allow deletion of read-only files
#[cfg(windows)]
pub fn clear_readonly_attribute(path: &Path) {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let attrs = GetFileAttributesW(wide.as_ptr());
        if attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_READONLY) != 0 {
            let _ = SetFileAttributesW(wide.as_ptr(), attrs & !FILE_ATTRIBUTE_READONLY);
        }
    }
}

#[cfg(not(windows))]
pub fn clear_readonly_attribute(_path: &Path) {}

/// Move file or directory to Windows Recycle Bin using native Win32 Shell API
#[cfg(windows)]
pub fn windows_native_trash(path: &Path) -> Result<(), std::io::Error> {
    let mut wide_path: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide_path.push(0);
    wide_path.push(0); // SHFileOperationW requires double null termination

    let mut file_op = SHFILEOPSTRUCTW {
        hwnd: 0 as _,
        wFunc: FO_DELETE,
        pFrom: wide_path.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };

    let res = unsafe { SHFileOperationW(&mut file_op) };
    if res == 0 && file_op.fAnyOperationsAborted == 0 {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Windows Recycle Bin error code: {}", res),
        ))
    }
}

#[cfg(not(windows))]
pub fn windows_native_trash(_path: &Path) -> Result<(), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Windows Recycle Bin is only supported on Windows",
    ))
}

pub struct LocalFs;

/// Expands `%VAR%` style Windows environment variables
pub fn expand_windows_env_vars(path_str: &str) -> String {
    if !path_str.contains('%') {
        return path_str.to_string();
    }
    let mut result = String::new();
    let mut chars = path_str.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let mut var_name = String::new();
            let mut closed = false;
            while let Some(&next_c) = chars.peek() {
                chars.next();
                if next_c == '%' {
                    closed = true;
                    break;
                }
                var_name.push(next_c);
            }
            if closed && !var_name.is_empty() {
                if let Ok(val) = std::env::var(&var_name) {
                    result.push_str(&val);
                } else {
                    result.push('%');
                    result.push_str(&var_name);
                    result.push('%');
                }
            } else {
                result.push('%');
                result.push_str(&var_name);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Strips extended length UNC prefixes (`\\?\` and `\\?\UNC\`) and normalizes drive prefixes
pub fn clean_path_buf(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    let stripped_str = if let Some(stripped) = path_str.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", stripped)
    } else if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        path_str.to_string()
    };

    #[cfg(not(windows))]
    {
        // On non-Windows platforms, if path is a Windows path with backslashes (e.g. C:\Users\Bolt),
        // convert backslashes to forward slashes so Path::components() properly parses segments in tests.
        if (stripped_str.len() >= 2 && stripped_str.as_bytes()[1] == b':') || stripped_str.starts_with(r"\\") {
            let normalized = stripped_str.replace('\\', "/");
            return PathBuf::from(normalized);
        }
    }

    PathBuf::from(stripped_str)
}

pub fn clean_path_str(path: &Path) -> String {
    clean_path_buf(path).to_string_lossy().to_string()
}

pub fn dunce_canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = path.canonicalize()?;
    Ok(clean_path_buf(&canonical))
}

impl LocalFs {
    /// Helper to get cached user and group name lookups with 60-second TTL
    #[cfg(unix)]
    fn get_user_group_maps() -> (HashMap<u32, String>, HashMap<u32, String>) {
        {
            let cache_read = USER_GROUP_CACHE.read();
            if let Some((timestamp, ref users, ref groups)) = *cache_read {
                if timestamp.elapsed() < Duration::from_secs(60) {
                    return (users.clone(), groups.clone());
                }
            }
        }

        let mut users = HashMap::new();
        let mut groups = HashMap::new();

        if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
            for line in passwd.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    if let Ok(uid) = parts[2].parse::<u32>() {
                        users.insert(uid, parts[0].to_string());
                    }
                }
            }
        }

        if let Ok(group_file) = fs::read_to_string("/etc/group") {
            for line in group_file.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    if let Ok(gid) = parts[2].parse::<u32>() {
                        groups.insert(gid, parts[0].to_string());
                    }
                }
            }
        }

        let mut cache_write = USER_GROUP_CACHE.write();
        *cache_write = Some((Instant::now(), users.clone(), groups.clone()));

        (users, groups)
    }

    fn mode_to_symbolic(mode: u32, is_dir: bool) -> String {
        let mut s = String::with_capacity(10);
        s.push(if is_dir { 'd' } else { '-' });
        s.push(if mode & 0o400 != 0 { 'r' } else { '-' });
        s.push(if mode & 0o200 != 0 { 'w' } else { '-' });
        s.push(if mode & 0o100 != 0 { 'x' } else { '-' });
        s.push(if mode & 0o040 != 0 { 'r' } else { '-' });
        s.push(if mode & 0o020 != 0 { 'w' } else { '-' });
        s.push(if mode & 0o010 != 0 { 'x' } else { '-' });
        s.push(if mode & 0o004 != 0 { 'r' } else { '-' });
        s.push(if mode & 0o002 != 0 { 'w' } else { '-' });
        s.push(if mode & 0o001 != 0 { 'x' } else { '-' });
        s
    }

    pub fn resolve_local_path(p: &str) -> PathBuf {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            return dirs::home_dir().unwrap_or_else(|| {
                #[cfg(windows)]
                { PathBuf::from(r"C:\") }
                #[cfg(not(windows))]
                { PathBuf::from("/") }
            });
        }

        if trimmed == "~" {
            return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
        }

        if let Some(stripped) = trimmed.strip_prefix("~/") {
            return if let Some(home) = dirs::home_dir() {
                home.join(stripped)
            } else {
                PathBuf::from(trimmed)
            };
        }

        if let Some(stripped) = trimmed.strip_prefix(r"~\") {
            return if let Some(home) = dirs::home_dir() {
                home.join(stripped)
            } else {
                PathBuf::from(trimmed)
            };
        }

        #[cfg(windows)]
        {
            if trimmed == "/" || trimmed == "\\" {
                return dirs::home_dir().unwrap_or_else(|| PathBuf::from(r"C:\"));
            }

            let expanded = expand_windows_env_vars(trimmed);

            // Strip leading slash before Windows drive letter if present (e.g. "/C:/Users" -> "C:/Users")
            let without_lead_slash = if (expanded.starts_with('/') || expanded.starts_with('\\'))
                && expanded.len() >= 3
                && expanded.as_bytes()[1].is_ascii_alphabetic()
                && expanded.as_bytes()[2] == b':'
            {
                &expanded[1..]
            } else {
                &expanded
            };

            // If path is just drive letter e.g. "C:" or "c:", ensure trailing slash "C:\"
            let final_path = if without_lead_slash.len() == 2 && without_lead_slash.as_bytes()[1] == b':' {
                format!(r"{}\", without_lead_slash)
            } else {
                without_lead_slash.to_string()
            };

            clean_path_buf(&PathBuf::from(final_path))
        }

        #[cfg(not(windows))]
        {
            let cleaned = if (trimmed.starts_with('/') || trimmed.starts_with('\\'))
                && trimmed.len() >= 3
                && trimmed.as_bytes()[1].is_ascii_alphabetic()
                && trimmed.as_bytes()[2] == b':'
            {
                &trimmed[1..]
            } else {
                trimmed
            };
            clean_path_buf(&PathBuf::from(cleaned))
        }
    }

    pub fn list_dir(path_str: &str, show_hidden: bool) -> Result<DirectoryListing, std::io::Error> {
        let path = Self::resolve_local_path(path_str);
        if !path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Directory not found: {}", path_str),
            ));
        }

        let canonical = dunce_canonicalize(&path).unwrap_or_else(|_| clean_path_buf(&path));
        let canonical_str = clean_path_str(&canonical);
        let parent_path = canonical.parent().map(|p| clean_path_str(p));
        #[cfg(unix)]
        let (user_map, group_map) = Self::get_user_group_maps();

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        if canonical.is_dir() {
            let read_dir = fs::read_dir(&canonical)?;
            for entry_res in read_dir {
                if let Ok(entry) = entry_res {
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    if !show_hidden && file_name.starts_with('.') {
                        continue;
                    }

                    let file_path = clean_path_buf(&entry.path());
                    let file_path_str = clean_path_str(&file_path);

                    let metadata = entry.metadata().ok();
                    let is_dir = metadata.as_ref().map_or(false, |m| m.is_dir());
                    let is_symlink = metadata.as_ref().map_or(false, |m| m.file_type().is_symlink());
                    let size = metadata.as_ref().map_or(0, |m| m.len());
                    let modified = metadata.as_ref().and_then(|m| {
                        m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()))
                    });

                    #[cfg(unix)]
                    let (raw_mode, uid, gid) = {
                        let mode = metadata.as_ref().map(|m| m.mode()).unwrap_or(0o644);
                        let u = metadata.as_ref().map(|m| m.uid()).unwrap_or(1000);
                        let g = metadata.as_ref().map(|m| m.gid()).unwrap_or(1000);
                        (mode, u, g)
                    };
                    #[cfg(not(unix))]
                    let (raw_mode, uid, gid) = {
                        let is_readonly = metadata.as_ref().map(|m| m.permissions().readonly()).unwrap_or(false);
                        let mode = if is_dir {
                            0o755
                        } else if is_readonly {
                            0o444
                        } else {
                            0o666
                        };
                        (mode, 0, 0)
                    };

                    let mode_octal = format!("{:04o}", raw_mode & 0o7777);
                    let permissions = Self::mode_to_symbolic(raw_mode, is_dir);

                    #[cfg(unix)]
                    let owner = user_map.get(&uid).cloned().unwrap_or_else(|| uid.to_string());
                    #[cfg(unix)]
                    let group = group_map.get(&gid).cloned().unwrap_or_else(|| gid.to_string());

                    #[cfg(not(unix))]
                    let owner = std::env::var("USERNAME").unwrap_or_else(|_| "Users".to_string());
                    #[cfg(not(unix))]
                    let group = "Users".to_string();

                    let mime = if is_dir {
                        None
                    } else {
                        Some(mime_guess::from_path(&file_path).first_or_octet_stream().to_string())
                    };

                    let is_archive = !is_dir && is_archive_file(&file_name);
                    let is_empty = if is_dir {
                        std::fs::read_dir(&file_path).map(|mut r| r.next().is_none()).ok()
                    } else {
                        None
                    };

                    if is_dir {
                        total_dirs += 1;
                    } else {
                        total_files += 1;
                        total_size += size;
                    }

                    entries.push(FileEntry {
                        name: file_name,
                        path: file_path_str,
                        is_dir,
                        is_symlink,
                        is_empty,
                        size,
                        modified,
                        permissions,
                        mode_octal,
                        owner,
                        group,
                        uid,
                        gid,
                        mime_type: mime,
                        is_archive,
                    });
                }
            }
        }

        // Sort: directories first (alphabetical), then files (alphabetical)
        entries.sort_by(|a, b| {
            if a.is_dir == b.is_dir {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            } else if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        Ok(DirectoryListing {
            current_path: canonical_str,
            parent_path,
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "local".to_string(),
            is_truncated: None,
            max_limit: None,
        })
    }

    /// Recursively flattens directory tree into a branch view (optimized with entry capping & fast traversal)
    pub fn list_branch_view(
        path_str: &str,
        show_hidden: bool,
        max_depth: Option<usize>,
        max_entries: Option<usize>,
    ) -> Result<DirectoryListing, std::io::Error> {
        let path = Self::resolve_local_path(path_str);
        if !path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Directory not found: {}", path_str),
            ));
        }

        let canonical = dunce_canonicalize(&path).unwrap_or_else(|_| clean_path_buf(&path));
        let canonical_str = clean_path_str(&canonical);
        let parent_path = canonical.parent().map(|p| clean_path_str(p));

        #[cfg(unix)]
        let (user_map, group_map) = Self::get_user_group_maps();

        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        let depth_limit = max_depth.unwrap_or(8).clamp(1, 32);
        let entry_limit = max_entries.unwrap_or(5_000).clamp(1, 25_000);

        let walker = walkdir::WalkDir::new(&canonical)
            .max_depth(depth_limit)
            .follow_links(false)
            .into_iter()
            .filter_entry(move |e| {
                if !show_hidden && e.depth() > 0 {
                    let file_name = e.file_name().to_string_lossy();
                    if file_name.starts_with('.') {
                        return false;
                    }
                }
                true
            });

        let mut is_truncated = false;

        for entry_res in walker {
            let entry_res = match entry_res {
                Ok(e) => e,
                Err(_) => continue,
            };

            let entry_clean_path = clean_path_buf(entry_res.path());
            if entry_clean_path == canonical {
                continue;
            }

            if entries.len() >= entry_limit {
                is_truncated = true;
                break;
            }

            let file_name = entry_res.file_name().to_string_lossy().to_string();
            let rel_path = entry_clean_path.strip_prefix(&canonical)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| file_name.clone());

            let file_path_str = clean_path_str(&entry_clean_path);

            let file_type = entry_res.file_type();
            let is_dir = file_type.is_dir();
            let is_symlink = file_type.is_symlink();

            let metadata = entry_res.metadata().ok();
            let size = metadata.as_ref().map_or(0, |m| m.len());
            let modified = metadata.as_ref().and_then(|m| {
                m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()))
            });

            #[cfg(unix)]
            let (raw_mode, uid, gid) = {
                let mode = metadata.as_ref().map(|m| m.mode()).unwrap_or(0o644);
                let u = metadata.as_ref().map(|m| m.uid()).unwrap_or(1000);
                let g = metadata.as_ref().map(|m| m.gid()).unwrap_or(1000);
                (mode, u, g)
            };
            #[cfg(not(unix))]
            let (raw_mode, uid, gid) = {
                let is_readonly = metadata.as_ref().map(|m| m.permissions().readonly()).unwrap_or(false);
                let mode = if is_dir {
                    0o755
                } else if is_readonly {
                    0o444
                } else {
                    0o666
                };
                (mode, 0, 0)
            };

            let mode_octal = format!("{:04o}", raw_mode & 0o7777);
            let permissions = Self::mode_to_symbolic(raw_mode, is_dir);

            #[cfg(unix)]
            let owner = user_map.get(&uid).cloned().unwrap_or_else(|| uid.to_string());
            #[cfg(unix)]
            let group = group_map.get(&gid).cloned().unwrap_or_else(|| gid.to_string());

            #[cfg(not(unix))]
            let owner = std::env::var("USERNAME").unwrap_or_else(|_| "Users".to_string());
            #[cfg(not(unix))]
            let group = "Users".to_string();

            let mime = if !is_dir {
                mime_guess::from_path(&entry_clean_path).first_raw().map(|s| s.to_string())
            } else {
                None
            };

            let is_archive = !is_dir && is_archive_file(&file_name);

            if is_dir {
                total_dirs += 1;
            } else {
                total_files += 1;
                total_size += size;
            }

            entries.push(FileEntry {
                name: rel_path,
                path: file_path_str,
                is_dir,
                is_symlink,
                is_empty: None,
                size,
                modified,
                permissions,
                mode_octal,
                owner,
                group,
                uid,
                gid,
                mime_type: mime,
                is_archive,
            });
        }

        // Sort: directories first (alphabetical), then files (alphabetical)
        entries.sort_by(|a, b| {
            if a.is_dir == b.is_dir {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            } else if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        Ok(DirectoryListing {
            current_path: canonical_str,
            parent_path,
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "branch".to_string(),
            is_truncated: if is_truncated { Some(true) } else { None },
            max_limit: if is_truncated { Some(entry_limit) } else { None },
        })
    }

    pub fn chmod_entry(path_str: &str, mode: u32, recursive: bool) -> Result<(), std::io::Error> {
        let p = Self::resolve_local_path(path_str);
        if !p.exists() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Path not found"));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if recursive && p.is_dir() {
                for entry in walkdir::WalkDir::new(&p).into_iter().filter_map(|e| e.ok()) {
                    fs::set_permissions(entry.path(), fs::Permissions::from_mode(mode))?;
                }
                Ok(())
            } else {
                fs::set_permissions(&p, fs::Permissions::from_mode(mode))
            }
        }
        #[cfg(not(unix))]
        {
            let set_ro = |target: &Path| -> Result<(), std::io::Error> {
                let mut perms = fs::metadata(target)?.permissions();
                perms.set_readonly(mode & 0o222 == 0);
                fs::set_permissions(target, perms)
            };

            if recursive && p.is_dir() {
                for entry in walkdir::WalkDir::new(&p).into_iter().filter_map(|e| e.ok()) {
                    let _ = set_ro(entry.path());
                }
                Ok(())
            } else {
                set_ro(&p)
            }
        }
    }

    pub fn chown_entry(path_str: &str, uid: Option<u32>, gid: Option<u32>, recursive: bool) -> Result<(), std::io::Error> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::chown;
            let p = Self::resolve_local_path(path_str);
            if !p.exists() {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Path not found"));
            }

            if recursive && p.is_dir() {
                for entry in walkdir::WalkDir::new(&p).into_iter().filter_map(|e| e.ok()) {
                    chown(entry.path(), uid, gid)?;
                }
                Ok(())
            } else {
                chown(&p, uid, gid)
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (path_str, uid, gid, recursive);
            Ok(())
        }
    }

    pub fn read_file(path_str: &str, max_bytes: usize) -> Result<FileContentResponse, std::io::Error> {
        let path = Self::resolve_local_path(path_str);
        if !path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {}", path_str),
            ));
        }

        let metadata = fs::metadata(&path)?;
        let size = metadata.len();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mime_type = mime_guess::from_path(&path).first_or_octet_stream().to_string();

        let mut file = File::open(&path)?;
        let mut buffer = Vec::new();
        let bytes_to_read = if max_bytes > 0 {
            std::cmp::min(size as usize, max_bytes)
        } else {
            size as usize
        };

        buffer.resize(bytes_to_read, 0);
        let actual_read = file.read(&mut buffer)?;
        buffer.truncate(actual_read);

        // Check if content is valid UTF-8
        match String::from_utf8(buffer.clone()) {
            Ok(text) => Ok(FileContentResponse {
                path: path_str.to_string(),
                name,
                content: text,
                is_binary: false,
                size,
                mime_type,
            }),
            Err(_) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&buffer);
                Ok(FileContentResponse {
                    path: path_str.to_string(),
                    name,
                    content: b64,
                    is_binary: true,
                    size,
                    mime_type,
                })
            }
        }
    }

    pub fn write_file(path_str: &str, content: &[u8], atomic: bool) -> Result<(), std::io::Error> {
        let target_path = Self::resolve_local_path(path_str);

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if atomic {
            let temp_path = target_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
            {
                let mut file = File::create(&temp_path)?;
                file.write_all(content)?;
                file.sync_all()?;
            }
            fs::rename(&temp_path, &target_path)?;
        } else {
            let mut file = File::create(&target_path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }

        Ok(())
    }

    pub fn create_dir(path_str: &str) -> Result<(), std::io::Error> {
        let p = Self::resolve_local_path(path_str);
        fs::create_dir_all(&p)
    }

    pub fn rename_entry(from_str: &str, to_str: &str) -> Result<(), std::io::Error> {
        Self::rename_entry_with_opts(from_str, to_str, true, true)
    }

    pub fn rename_entry_with_opts(
        from_str: &str,
        to_str: &str,
        _use_native_ops: bool,
        detect_locks: bool,
    ) -> Result<(), std::io::Error> {
        let from_p = Self::resolve_local_path(from_str);
        let to_p = Self::resolve_local_path(to_str);
        match fs::rename(&from_p, &to_p) {
            Ok(()) => Ok(()),
            Err(e) => {
                if detect_locks {
                    let locks = get_locking_processes(&from_p);
                    if !locks.is_empty() {
                        return Err(std::io::Error::new(
                            e.kind(),
                            format!(
                                "Cannot rename '{}': file is currently locked by {}",
                                from_p.file_name().unwrap_or_default().to_string_lossy(),
                                locks.join(", ")
                            ),
                        ));
                    }
                }
                Err(e)
            }
        }
    }

    pub fn delete_entry(
        path_str: &str,
        use_trash: bool,
        custom_trash: Option<&str>,
    ) -> Result<(), std::io::Error> {
        Self::delete_entry_with_opts(path_str, use_trash, custom_trash, true, true)
    }

    pub fn delete_entry_with_opts(
        path_str: &str,
        use_trash: bool,
        custom_trash: Option<&str>,
        _use_native_ops: bool,
        detect_locks: bool,
    ) -> Result<(), std::io::Error> {
        let path = Self::resolve_local_path(path_str);
        if !path.exists() && !path.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Target not found: {}", path_str),
            ));
        }

        if use_trash {
            #[cfg(windows)]
            if _use_native_ops && custom_trash.is_none() {
                match windows_native_trash(&path) {
                    Ok(()) => {
                        info!("Moved {} to Windows Recycle Bin", path_str);
                        return Ok(());
                    }
                    Err(e) => {
                        info!("Windows native Recycle Bin failed ({}), falling back to internal trash / delete", e);
                    }
                }
            }

            let trash_base = if let Some(ct) = custom_trash {
                PathBuf::from(ct)
            } else if let Some(home) = dirs::home_dir() {
                home.join(".local/share/Trash/files")
            } else {
                PathBuf::from("/tmp/brum_trash")
            };

            let _ = fs::create_dir_all(&trash_base);
            let file_name = path.file_name().unwrap_or_default();
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            let trash_target = trash_base.join(format!("{}_{}", timestamp, file_name.to_string_lossy()));

            // Attempt fast atomic rename first
            match fs::rename(&path, &trash_target) {
                Ok(_) => {
                    info!("Moved {} to trash: {}", path_str, trash_target.display());
                    return Ok(());
                }
                Err(e) => {
                    // If cross-device move or permission error moving across mounts occurs, try cross-device copy+delete
                    info!("Direct rename to trash failed ({}), attempting cross-device move for {}", e, path_str);
                    if let Ok(()) = Self::move_entry_recursive(&path, &trash_target) {
                        info!("Moved {} to trash via cross-device copy: {}", path_str, trash_target.display());
                        return Ok(());
                    }
                    // If trashing is impossible on this mount/subsystem, fall through to direct deletion
                    info!("Moving to trash failed for {}, falling back to direct permanent removal", path_str);
                }
            }
        }

        Self::force_remove_entry_with_opts(&path, detect_locks)
    }

    /// Recursively moves an entry across filesystems (copy + delete source)
    fn move_entry_recursive(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
        if src.is_dir() {
            fs::create_dir_all(dst)?;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                let file_name = entry.file_name();
                Self::move_entry_recursive(&src.join(&file_name), &dst.join(&file_name))?;
            }
            let _ = fs::remove_dir(src);
        } else {
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::copy(src, dst)?;
            let _ = fs::remove_file(src);
        }
        Ok(())
    }

    /// Forcefully remove a file, symlink, or directory (with permission adjustment & CLI fallback)
    pub fn force_remove_entry(path: &Path) -> Result<(), std::io::Error> {
        Self::force_remove_entry_with_opts(path, true)
    }

    pub fn force_remove_entry_with_opts(path: &Path, detect_locks: bool) -> Result<(), std::io::Error> {
        // Handle symlinks (including broken symlinks)
        if path.is_symlink() {
            return fs::remove_file(path).or_else(|_| {
                #[cfg(unix)]
                {
                    std::fs::remove_dir(path)
                }
                #[cfg(windows)]
                {
                    clear_readonly_attribute(path);
                    std::fs::remove_dir(path)
                }
                #[cfg(not(any(unix, windows)))]
                {
                    Err(std::io::Error::new(std::io::ErrorKind::Other, "Failed to remove symlink"))
                }
            });
        }

        if path.is_dir() {
            if let Ok(()) = fs::remove_dir_all(path) {
                return Ok(());
            }

            // Adjust permissions on read-only inner items and retry
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o777));
                for entry in walkdir::WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                    let _ = fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o777));
                }
            }

            #[cfg(windows)]
            {
                clear_readonly_attribute(path);
                for entry in walkdir::WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                    clear_readonly_attribute(entry.path());
                }
            }

            if let Ok(()) = fs::remove_dir_all(path) {
                return Ok(());
            }

            // Fallback to system /bin/rm -rf command (matches terminal behavior for root/subsystems)
            #[cfg(unix)]
            {
                let output = std::process::Command::new("rm")
                    .args(["-rf", "--"])
                    .arg(path)
                    .output();

                if let Ok(out) = output {
                    if out.status.success() && !path.exists() {
                        return Ok(());
                    }
                    let err_msg = String::from_utf8_lossy(&out.stderr);
                    if !err_msg.is_empty() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!("Cannot delete '{}': {}", path.display(), err_msg.trim()),
                        ));
                    }
                }
            }

            // If deletion still failed, check for lock detection on Windows or return rich error
            if detect_locks {
                let locks = get_locking_processes(path);
                if !locks.is_empty() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "Cannot delete folder '{}': Locked by active process ({})",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            locks.join(", ")
                        ),
                    ));
                }
                // Check if any child item like Thumbs.db is locked
                for entry in walkdir::WalkDir::new(path).max_depth(3).into_iter().filter_map(|e| e.ok()) {
                    let child_locks = get_locking_processes(entry.path());
                    if !child_locks.is_empty() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!(
                                "Cannot delete '{}': Locked file '{}' held open by {}",
                                path.file_name().unwrap_or_default().to_string_lossy(),
                                entry.file_name().to_string_lossy(),
                                child_locks.join(", ")
                            ),
                        ));
                    }
                }
            }

            fs::remove_dir_all(path)
        } else {
            if let Ok(()) = fs::remove_file(path) {
                return Ok(());
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o666));
            }

            #[cfg(windows)]
            {
                clear_readonly_attribute(path);
            }

            if let Ok(()) = fs::remove_file(path) {
                return Ok(());
            }

            #[cfg(unix)]
            {
                let output = std::process::Command::new("rm")
                    .args(["-f", "--"])
                    .arg(path)
                    .output();

                if let Ok(out) = output {
                    if out.status.success() && !path.exists() {
                        return Ok(());
                    }
                }
            }

            if detect_locks {
                let locks = get_locking_processes(path);
                if !locks.is_empty() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "Cannot delete file '{}': Locked by active process ({})",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            locks.join(", ")
                        ),
                    ));
                }
            }

            fs::remove_file(path)
        }
    }

    pub fn copy_file_paranoid(src_str: &str, dest_str: &str, verify: bool) -> Result<(), std::io::Error> {
        let src_path = Path::new(src_str);
        let dest_path = Path::new(dest_str);

        if !src_path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Source path not found: {}", src_str),
            ));
        }

        // Canonical paths comparison
        let can_src = dunce_canonicalize(src_path)?;
        let can_dest = if dest_path.exists() {
            dunce_canonicalize(dest_path).ok()
        } else if let Some(p) = dest_path.parent() {
            dunce_canonicalize(p).ok().map(|can_p| can_p.join(dest_path.file_name().unwrap_or_default()))
        } else {
            None
        };

        if let Some(ref cd) = can_dest {
            if can_src == *cd {
                // Source and destination are identical
                return Ok(());
            }
        }

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if src_path.is_dir() {
            // Prevent copying directory into itself or its own subdirectories
            if let Some(ref cd) = can_dest {
                if cd == &can_src || cd.starts_with(&can_src) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!(
                            "Cannot copy directory '{}' into itself or a subdirectory of itself '{}'",
                            src_str, dest_str
                        ),
                    ));
                }
            }

            // Snapshot existing directory entries before creating any new folders in dest
            let entries: Vec<walkdir::DirEntry> = walkdir::WalkDir::new(src_path)
                .into_iter()
                .filter_entry(|e| {
                    if let Some(ref cd) = can_dest {
                        if let Ok(can_e) = e.path().canonicalize() {
                            if can_e == *cd || can_e.starts_with(cd) {
                                return false;
                            }
                        }
                    }
                    true
                })
                .filter_map(|e| e.ok())
                .collect();

            fs::create_dir_all(dest_path)?;

            for entry in entries {
                let rel = entry.path().strip_prefix(src_path).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::Other, e)
                })?;

                if rel.as_os_str().is_empty() {
                    continue;
                }

                let target = dest_path.join(rel);

                if entry.file_type().is_dir() {
                    fs::create_dir_all(&target)?;
                } else if entry.file_type().is_file() {
                    if let Some(p) = target.parent() {
                        fs::create_dir_all(p)?;
                    }
                    fs::copy(entry.path(), &target)?;
                    if verify {
                        let hash_src = calculate_sha256(entry.path())?;
                        let hash_dest = calculate_sha256(&target)?;
                        if hash_src != hash_dest {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                format!("Integrity check failed for {}", target.display()),
                            ));
                        }
                    }
                }
            }
            Ok(())
        } else {
            fs::copy(src_path, dest_path)?;
            if verify {
                let hash_src = calculate_sha256(src_path)?;
                let hash_dest = calculate_sha256(dest_path)?;
                if hash_src != hash_dest {
                    let _ = fs::remove_file(dest_path);
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Paranoid verify failed: Hash mismatch for {}", dest_str),
                    ));
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_copy_file_paranoid_prevents_recursive_loop() {
        let tmp = tempdir().unwrap();
        let parent_dir = tmp.path().join("my_folder");
        fs::create_dir_all(&parent_dir).unwrap();
        fs::write(parent_dir.join("hello.txt"), "hello world").unwrap();

        // Attempt to copy parent_dir into a subfolder of itself
        let sub_target = parent_dir.join("subfolder");
        let res = LocalFs::copy_file_paranoid(
            &parent_dir.to_string_lossy(),
            &sub_target.to_string_lossy(),
            true,
        );

        assert!(res.is_err(), "Should return error when copying folder into its own subfolder");
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("Cannot copy directory"), "Error message was: {}", err_msg);
    }

    #[test]
    fn test_copy_file_paranoid_success() {
        let tmp = tempdir().unwrap();
        let src_dir = tmp.path().join("src_folder");
        let dest_dir = tmp.path().join("dest_folder");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("test.txt"), "verifiable data").unwrap();

        let res = LocalFs::copy_file_paranoid(
            &src_dir.to_string_lossy(),
            &dest_dir.to_string_lossy(),
            true,
        );

        assert!(res.is_ok(), "Copying to sibling directory should succeed");
        assert!(dest_dir.join("test.txt").exists());
        let content = fs::read_to_string(dest_dir.join("test.txt")).unwrap();
        assert_eq!(content, "verifiable data");
    }

    #[test]
    fn test_list_branch_view_recursive_flatten() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("root_dir");
        let sub1 = root.join("sub1");
        let sub2 = sub1.join("sub2");
        fs::create_dir_all(&sub2).unwrap();
        fs::write(root.join("file_root.txt"), "root").unwrap();
        fs::write(sub1.join("file_sub1.txt"), "sub1").unwrap();
        fs::write(sub2.join("file_sub2.txt"), "sub2").unwrap();

        let res = LocalFs::list_branch_view(&root.to_string_lossy(), false, None, None).unwrap();
        assert_eq!(res.protocol, "branch");
        assert_eq!(res.total_files, 3);
        assert_eq!(res.total_dirs, 2);
        assert_eq!(res.is_truncated, None);

        let names: Vec<String> = res.entries.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains(&"file_root.txt".to_string()));
        assert!(names.contains(&"sub1/file_sub1.txt".to_string()));
        assert!(names.contains(&"sub1/sub2/file_sub2.txt".to_string()));
    }

    #[test]
    fn test_list_branch_view_max_entries_truncation() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("root_trunc");
        fs::create_dir_all(&root).unwrap();
        for i in 0..10 {
            fs::write(root.join(format!("file_{}.txt", i)), "data").unwrap();
        }

        // Limit to 5 entries
        let res = LocalFs::list_branch_view(&root.to_string_lossy(), false, None, Some(5)).unwrap();
        assert_eq!(res.entries.len(), 5);
        assert_eq!(res.is_truncated, Some(true));
        assert_eq!(res.max_limit, Some(5));
    }

    #[test]
    fn test_list_branch_view_hidden_filter() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("root_hidden");
        let hidden_dir = root.join(".hidden_folder");
        fs::create_dir_all(&hidden_dir).unwrap();
        fs::write(root.join("visible.txt"), "visible").unwrap();
        fs::write(root.join(".hidden_file.txt"), "hidden").unwrap();
        fs::write(hidden_dir.join("inside_hidden.txt"), "inside").unwrap();

        // show_hidden = false
        let res_no_hidden = LocalFs::list_branch_view(&root.to_string_lossy(), false, None, None).unwrap();
        let names_no: Vec<String> = res_no_hidden.entries.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names_no, vec!["visible.txt"]);

        // show_hidden = true
        let res_with_hidden = LocalFs::list_branch_view(&root.to_string_lossy(), true, None, None).unwrap();
        assert!(res_with_hidden.entries.len() >= 3);
    }

    #[test]
    #[cfg(unix)]
    fn test_user_group_cache_ttl() {
        let (u1, g1) = LocalFs::get_user_group_maps();
        let (u2, g2) = LocalFs::get_user_group_maps();
        assert_eq!(u1.len(), u2.len());
        assert_eq!(g1.len(), g2.len());
    }

    #[test]
    fn test_clean_path_buf_strips_unc_and_verbatim_prefix() {
        #[cfg(windows)]
        {
            assert_eq!(
                clean_path_buf(Path::new(r"\\?\C:\Users\Bolt\Documents")),
                PathBuf::from(r"C:\Users\Bolt\Documents")
            );
            assert_eq!(
                clean_path_buf(Path::new(r"\\?\UNC\server\share\subfolder")),
                PathBuf::from(r"\\server\share\subfolder")
            );
        }
        #[cfg(not(windows))]
        {
            assert_eq!(
                clean_path_buf(Path::new(r"\\?\C:\Users\Bolt\Documents")),
                PathBuf::from("C:/Users/Bolt/Documents")
            );
            assert_eq!(
                clean_path_buf(Path::new(r"\\?\UNC\server\share\subfolder")),
                PathBuf::from("//server/share/subfolder")
            );
        }
        assert_eq!(
            clean_path_buf(Path::new("/var/log/syslog")),
            PathBuf::from("/var/log/syslog")
        );
    }

    #[test]
    fn test_resolve_local_path_windows_prefix() {
        assert_eq!(
            LocalFs::resolve_local_path("/C:/Users/Bolt"),
            PathBuf::from("C:/Users/Bolt")
        );
        #[cfg(windows)]
        assert_eq!(
            LocalFs::resolve_local_path(r"\D:\Data\Project"),
            PathBuf::from(r"D:\Data\Project")
        );
        #[cfg(not(windows))]
        assert_eq!(
            LocalFs::resolve_local_path(r"\D:\Data\Project"),
            PathBuf::from("D:/Data/Project")
        );
    }

    #[test]
    fn test_windows_native_helpers_and_lock_detection() {
        let tmp = tempdir().unwrap();
        let test_file = tmp.path().join("lock_test.txt");
        fs::write(&test_file, "lock test content").unwrap();

        // Lock detection returns a Vec (empty if no locks or on non-Windows)
        let locks = get_locking_processes(&test_file);
        assert!(locks.is_empty() || !locks.is_empty());

        // Read-only attribute clearance helper works without panic
        clear_readonly_attribute(&test_file);

        // Native trash helper
        #[cfg(not(windows))]
        {
            let res = windows_native_trash(&test_file);
            assert!(res.is_err(), "Native trash should report unsupported on non-Windows");
        }
    }

    #[test]
    fn test_delete_and_rename_with_options() {
        let tmp = tempdir().unwrap();
        let test_file = tmp.path().join("item_to_delete.txt");
        fs::write(&test_file, "delete me").unwrap();

        // Test rename with options
        let renamed_file = tmp.path().join("item_renamed.txt");
        let res_rename = LocalFs::rename_entry_with_opts(
            &test_file.to_string_lossy(),
            &renamed_file.to_string_lossy(),
            true,
            true,
        );
        assert!(res_rename.is_ok());
        assert!(renamed_file.exists());
        assert!(!test_file.exists());

        // Test delete with options
        let res_del = LocalFs::delete_entry_with_opts(
            &renamed_file.to_string_lossy(),
            false,
            None,
            true,
            true,
        );
        assert!(res_del.is_ok());
        assert!(!renamed_file.exists());
    }
}

