use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use chrono::{Local};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashItem {
    pub name: String,
    pub original_path: Option<String>,
    pub deletion_date: Option<String>,
    pub size: u64,
    pub is_dir: bool,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashSummary {
    pub total_items: usize,
    pub total_size: u64,
    pub trash_dir: String,
    pub files_dir: String,
    pub info_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct TrashRestoreRequest {
    pub items: Option<Vec<String>>,
    pub custom_trash_dir: Option<String>,
    pub windows_native_ops: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TrashDeleteRequest {
    pub items: Vec<String>,
    pub custom_trash_dir: Option<String>,
    pub windows_native_ops: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TrashEmptyRequest {
    pub custom_trash_dir: Option<String>,
    pub windows_native_ops: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TrashActionResult {
    pub success: bool,
    pub affected_count: usize,
    pub freed_bytes: Option<u64>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WinRecycleItem {
    pub r_path: PathBuf,
    pub i_path: PathBuf,
    pub original_path: String,
    pub original_name: String,
    pub file_size: u64,
    pub deletion_timestamp: Option<u64>,
    pub is_dir: bool,
}

pub struct TrashManager;

impl TrashManager {
    /// Resolves root, files, and info paths for the trash system
    pub fn get_trash_paths(
        custom_trash: Option<&str>,
        user_home: Option<&str>,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let root = if let Some(ct) = custom_trash.filter(|s| !s.trim().is_empty()) {
            PathBuf::from(ct)
        } else if let Some(home) = user_home.filter(|s| !s.trim().is_empty()) {
            PathBuf::from(home).join(".local/share/Trash")
        } else if let Some(home) = dirs::home_dir() {
            home.join(".local/share/Trash")
        } else {
            PathBuf::from("/tmp/brum_trash")
        };

        let files = root.join("files");
        let info = root.join("info");
        (root, files, info)
    }

    /// Ensures the trash directories exist
    pub fn ensure_dirs(
        custom_trash: Option<&str>,
        user_home: Option<&str>,
    ) -> io::Result<(PathBuf, PathBuf, PathBuf)> {
        let (root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if !files.exists() {
            fs::create_dir_all(&files)?;
        }
        if !info.exists() {
            fs::create_dir_all(&info)?;
        }
        Ok((root, files, info))
    }

    /// Moves a file or directory into XDG-compliant Trash and writes the .trashinfo record
    pub fn move_to_trash(
        source_path: &Path,
        custom_trash: Option<&str>,
        user_home: Option<&str>,
    ) -> io::Result<PathBuf> {
        let (_root, files, info) = Self::ensure_dirs(custom_trash, user_home)?;

        let original_abs = if source_path.is_absolute() {
            source_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(source_path)
        };
        let original_path_str = original_abs.to_string_lossy().to_string();

        let base_name = source_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if base_name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid source path",
            ));
        }

        let mut unique_name = base_name.clone();
        let path_obj = Path::new(&base_name);
        let stem = path_obj
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let ext = path_obj.extension().map(|e| e.to_string_lossy().to_string());

        let mut counter = 1;
        while files.join(&unique_name).exists()
            || info.join(format!("{}.trashinfo", unique_name)).exists()
        {
            unique_name = match &ext {
                Some(e) if !e.is_empty() => format!("{}.{}.{}", stem, counter, e),
                _ => format!("{}.{}", stem, counter),
            };
            counter += 1;
        }

        let dest_path = files.join(&unique_name);

        // Attempt fast atomic rename first
        if let Err(err) = fs::rename(source_path, &dest_path) {
            info!(
                "Direct atomic rename to trash failed ({}), performing recursive cross-device move for {}",
                err,
                source_path.display()
            );
            Self::move_entry_recursive(source_path, &dest_path)?;
        }

        // Write .trashinfo metadata per XDG specification
        let info_path = info.join(format!("{}.trashinfo", unique_name));
        let now_str = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let encoded_path = Self::encode_trash_path(&original_path_str);
        let trashinfo_content = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            encoded_path, now_str
        );

        if let Err(e) = fs::write(&info_path, trashinfo_content) {
            warn!(
                "Failed to write .trashinfo metadata at {}: {}",
                info_path.display(),
                e
            );
        }

        Ok(dest_path)
    }

    /// Parses a .trashinfo file returning (Option<OriginalPath>, Option<DeletionDate>)
    pub fn parse_trashinfo(info_file: &Path) -> (Option<String>, Option<String>) {
        let file = match File::open(info_file) {
            Ok(f) => f,
            Err(_) => return (None, None),
        };

        let reader = BufReader::new(file);
        let mut path: Option<String> = None;
        let mut deletion_date: Option<String> = None;

        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("Path=") {
                path = Some(Self::decode_trash_path(rest.trim()));
            } else if let Some(rest) = trimmed.strip_prefix("DeletionDate=") {
                deletion_date = Some(rest.trim().to_string());
            }
        }

        (path, deletion_date)
    }

    /// Percent-encodes a path for .trashinfo Path field
    pub fn encode_trash_path(path_str: &str) -> String {
        let mut out = String::new();
        for b in path_str.bytes() {
            match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                    out.push(b as char);
                }
                _ => {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
        out
    }

    /// Decodes a percent-encoded path string
    pub fn decode_trash_path(s: &str) -> String {
        let mut bytes = Vec::new();
        let mut iter = s.bytes().peekable();
        while let Some(b) = iter.next() {
            if b == b'%' {
                let h1 = iter.next();
                let h2 = iter.next();
                if let (Some(c1), Some(c2)) = (h1, h2) {
                    let hex_str = format!("{}{}", c1 as char, c2 as char);
                    if let Ok(val) = u8::from_str_radix(&hex_str, 16) {
                        bytes.push(val);
                        continue;
                    }
                    bytes.push(b'%');
                    bytes.push(c1);
                    bytes.push(c2);
                } else {
                    bytes.push(b'%');
                    if let Some(c1) = h1 {
                        bytes.push(c1);
                    }
                    if let Some(c2) = h2 {
                        bytes.push(c2);
                    }
                }
            } else {
                bytes.push(b);
            }
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    /// Parses a Windows Recycle Bin $I index file (supports format v1 and v2)
    pub fn parse_win_i_file(i_path: &Path) -> io::Result<(String, u64, Option<u64>)> {
        let bytes = fs::read(i_path)?;
        if bytes.len() < 28 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid $I file length"));
        }

        let version = u64::from_le_bytes(bytes[0..8].try_into().unwrap_or([0; 8]));
        let file_size = u64::from_le_bytes(bytes[8..16].try_into().unwrap_or([0; 8]));
        let filetime = u64::from_le_bytes(bytes[16..24].try_into().unwrap_or([0; 8]));

        let deletion_timestamp = if filetime >= 116_444_736_000_000_000 {
            Some((filetime - 116_444_736_000_000_000) / 10_000_000)
        } else {
            None
        };

        let original_path = if version == 1 {
            let raw_wchars = &bytes[24..];
            let mut u16_chars = Vec::new();
            for chunk in raw_wchars.chunks_exact(2) {
                let ch = u16::from_le_bytes([chunk[0], chunk[1]]);
                if ch == 0 {
                    break;
                }
                u16_chars.push(ch);
            }
            String::from_utf16_lossy(&u16_chars)
        } else if version == 2 {
            let char_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap_or([0; 4])) as usize;
            let path_bytes = if bytes.len() >= 28 { &bytes[28..] } else { &[] };
            let mut u16_chars = Vec::new();
            for chunk in path_bytes.chunks_exact(2).take(char_count) {
                let ch = u16::from_le_bytes([chunk[0], chunk[1]]);
                if ch == 0 {
                    break;
                }
                u16_chars.push(ch);
            }
            String::from_utf16_lossy(&u16_chars)
        } else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unknown $I version: {}", version)));
        };

        Ok((original_path, file_size, deletion_timestamp))
    }

    /// Enumerates all Windows native Recycle Bin ($Recycle.Bin) items across all mounted drives
    pub fn list_windows_recycle_bin_items() -> Vec<WinRecycleItem> {
        #[allow(unused_mut)]
        let mut items = Vec::new();

        #[cfg(windows)]
        {
            for drive_letter in b'A'..=b'Z' {
                let drive_root = format!(r"{}:\", drive_letter as char);
                let recycle_dir = PathBuf::from(&drive_root).join(r"$Recycle.Bin");
                if !recycle_dir.exists() {
                    continue;
                }

                if let Ok(sid_entries) = fs::read_dir(&recycle_dir) {
                    for sid_entry in sid_entries.flatten() {
                        let sid_path = sid_entry.path();
                        if !sid_path.is_dir() {
                            continue;
                        }

                        if let Ok(file_entries) = fs::read_dir(&sid_path) {
                            for file_entry in file_entries.flatten() {
                                let file_name = file_entry.file_name().to_string_lossy().to_string();
                                if file_name.starts_with("$I") {
                                    let i_path = file_entry.path();
                                    let r_name = format!("$R{}", &file_name[2..]);
                                    let r_path = sid_path.join(&r_name);

                                    if let Ok((orig_path, file_size, del_time)) = Self::parse_win_i_file(&i_path) {
                                        let is_dir = r_path.is_dir();
                                        let orig_name = Path::new(&orig_path)
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .to_string();
                                        let actual_name = if !orig_name.is_empty() { orig_name } else { r_name.clone() };

                                        let actual_size = if file_size > 0 {
                                            file_size
                                        } else if let Ok(m) = fs::symlink_metadata(&r_path) {
                                            m.len()
                                        } else {
                                            0
                                        };

                                        items.push(WinRecycleItem {
                                            r_path,
                                            i_path,
                                            original_path: orig_path,
                                            original_name: actual_name,
                                            file_size: actual_size,
                                            deletion_timestamp: del_time,
                                            is_dir,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        items
    }

    /// Enumerates all trashed items with metadata
    pub fn list_trash_items(
        custom_trash: Option<&str>,
        user_home: Option<&str>,
        _windows_native_ops: bool,
    ) -> io::Result<Vec<TrashItem>> {
        let mut items = Vec::new();

        #[cfg(windows)]
        if _windows_native_ops && custom_trash.is_none() {
            let win_items = Self::list_windows_recycle_bin_items();
            for it in win_items {
                let del_date_str = it.deletion_timestamp.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts as i64, 0)
                        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S").to_string())
                });

                let mime_type = if !it.is_dir {
                    mime_guess::from_path(&it.original_name)
                        .first_raw()
                        .map(|s| s.to_string())
                } else {
                    None
                };

                items.push(TrashItem {
                    name: it.original_name,
                    original_path: Some(it.original_path),
                    deletion_date: del_date_str,
                    size: it.file_size,
                    is_dir: it.is_dir,
                    mime_type,
                });
            }

            items.sort_by(|a, b| {
                match (&a.deletion_date, &b.deletion_date) {
                    (Some(d1), Some(d2)) => d2.cmp(d1),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                }
            });

            return Ok(items);
        }

        let (_root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if files.exists() {
            for entry in fs::read_dir(&files)? {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let file_name = entry.file_name().to_string_lossy().to_string();
                let entry_path = entry.path();
                let metadata = match fs::symlink_metadata(&entry_path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let is_dir = metadata.is_dir();
                let size = if is_dir {
                    Self::calculate_dir_size(&entry_path)
                } else {
                    metadata.len()
                };

                let info_file = info.join(format!("{}.trashinfo", file_name));
                let (original_path, deletion_date) = if info_file.exists() {
                    Self::parse_trashinfo(&info_file)
                } else {
                    (None, None)
                };

                let mime_type = if !is_dir {
                    mime_guess::from_path(&entry_path)
                        .first_raw()
                        .map(|s| s.to_string())
                } else {
                    None
                };

                items.push(TrashItem {
                    name: file_name,
                    original_path,
                    deletion_date,
                    size,
                    is_dir,
                    mime_type,
                });
            }
        }

        // Sort by deletion date descending (newest deletions first)
        items.sort_by(|a, b| {
            match (&a.deletion_date, &b.deletion_date) {
                (Some(d1), Some(d2)) => d2.cmp(d1),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            }
        });

        Ok(items)
    }

    /// Generates a full VFS DirectoryListing for browsing trash:// natively in panes
    pub fn list_trash_directory_entries(
        custom_trash: Option<&str>,
        user_home: Option<&str>,
        _windows_native_ops: bool,
    ) -> crate::vfs::DirectoryListing {
        let mut entries = Vec::new();
        let mut total_files = 0;
        let mut total_dirs = 0;
        let mut total_size = 0u64;

        #[cfg(windows)]
        if _windows_native_ops && custom_trash.is_none() {
            let win_items = Self::list_windows_recycle_bin_items();
            for item in win_items {
                let is_dir = item.is_dir;
                if is_dir {
                    total_dirs += 1;
                } else {
                    total_files += 1;
                }
                total_size += item.file_size;

                let mime_type = if !is_dir {
                    mime_guess::from_path(&item.original_name)
                        .first_raw()
                        .map(|s| s.to_string())
                } else {
                    None
                };
                let is_archive = !is_dir && crate::vfs::is_archive_file(&item.original_name);

                entries.push(crate::vfs::FileEntry {
                    name: item.original_name,
                    path: item.r_path.to_string_lossy().to_string(),
                    is_dir,
                    is_symlink: false,
                    is_empty: Some(item.file_size == 0),
                    size: item.file_size,
                    modified: item.deletion_timestamp,
                    permissions: if is_dir { "rwxr-xr-x".to_string() } else { "rw-r--r--".to_string() },
                    mode_octal: if is_dir { "0755".to_string() } else { "0644".to_string() },
                    owner: "user".to_string(),
                    group: "user".to_string(),
                    uid: 1000,
                    gid: 1000,
                    mime_type,
                    is_archive,
                });
            }

            entries.sort_by(|a, b| b.modified.cmp(&a.modified));

            return crate::vfs::DirectoryListing {
                current_path: "trash://".to_string(),
                parent_path: None,
                entries,
                total_files,
                total_dirs,
                total_size,
                protocol: "trash".to_string(),
                is_truncated: Some(false),
                max_limit: Some(5000),
            };
        }

        let (_root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if files.exists() {
            if let Ok(dir_entries) = fs::read_dir(&files) {
                for entry in dir_entries.flatten() {
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    let entry_path = entry.path();
                    let metadata = match fs::symlink_metadata(&entry_path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let is_dir = metadata.is_dir();
                    let size = if is_dir {
                        Self::calculate_dir_size(&entry_path)
                    } else {
                        metadata.len()
                    };
                    if is_dir {
                        total_dirs += 1;
                    } else {
                        total_files += 1;
                    }
                    total_size += size;

                    let info_file = info.join(format!("{}.trashinfo", file_name));
                    let (orig_path, del_date) = if info_file.exists() {
                        Self::parse_trashinfo(&info_file)
                    } else {
                        (None, None)
                    };

                    let modified_time = del_date.and_then(|d| {
                        chrono::NaiveDateTime::parse_from_str(&d, "%Y-%m-%dT%H:%M:%S")
                            .ok()
                            .map(|dt| dt.and_utc().timestamp() as u64)
                    });

                    let display_name = if let Some(ref p) = orig_path {
                        Path::new(p)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or(file_name.clone())
                    } else {
                        file_name.clone()
                    };

                    let mime_type = if !is_dir {
                        mime_guess::from_path(&display_name)
                            .first_raw()
                            .map(|s| s.to_string())
                    } else {
                        None
                    };
                    let is_archive = !is_dir && crate::vfs::is_archive_file(&display_name);

                    entries.push(crate::vfs::FileEntry {
                        name: display_name,
                        path: entry_path.to_string_lossy().to_string(),
                        is_dir,
                        is_symlink: metadata.file_type().is_symlink(),
                        is_empty: Some(size == 0),
                        size,
                        modified: modified_time,
                        permissions: if is_dir { "rwxr-xr-x".to_string() } else { "rw-r--r--".to_string() },
                        mode_octal: if is_dir { "0755".to_string() } else { "0644".to_string() },
                        owner: "user".to_string(),
                        group: "user".to_string(),
                        uid: 1000,
                        gid: 1000,
                        mime_type,
                        is_archive,
                    });
                }
            }
        }

        // Sort by modified / deletion date descending
        entries.sort_by(|a, b| b.modified.cmp(&a.modified));

        crate::vfs::DirectoryListing {
            current_path: "trash://".to_string(),
            parent_path: None,
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "trash".to_string(),
            is_truncated: Some(false),
            max_limit: Some(5000),
        }
    }

    /// Returns high-level summary statistics of the trash bin
    pub fn get_trash_summary(
        custom_trash: Option<&str>,
        user_home: Option<&str>,
        _windows_native_ops: bool,
    ) -> io::Result<TrashSummary> {
        let mut total_items = 0;
        let mut total_size = 0u64;

        #[cfg(windows)]
        if _windows_native_ops && custom_trash.is_none() {
            let win_items = Self::list_windows_recycle_bin_items();
            for it in &win_items {
                total_items += 1;
                total_size += it.file_size;
            }
            return Ok(TrashSummary {
                total_items,
                total_size,
                trash_dir: "shell:RecycleBinFolder".to_string(),
                files_dir: "$Recycle.Bin".to_string(),
                info_dir: "$Recycle.Bin".to_string(),
            });
        }

        let (root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if files.exists() {
            if let Ok(dir_entries) = fs::read_dir(&files) {
                for entry in dir_entries.flatten() {
                    total_items += 1;
                    let path = entry.path();
                    if let Ok(m) = fs::symlink_metadata(&path) {
                        if m.is_dir() {
                            total_size += Self::calculate_dir_size(&path);
                        } else {
                            total_size += m.len();
                        }
                    }
                }
            }
        }

        Ok(TrashSummary {
            total_items,
            total_size,
            trash_dir: root.to_string_lossy().to_string(),
            files_dir: files.to_string_lossy().to_string(),
            info_dir: info.to_string_lossy().to_string(),
        })
    }

    /// Restores specified items (or all items if empty) to their original path
    pub fn restore_items(
        items: Option<Vec<String>>,
        custom_trash: Option<&str>,
        user_home: Option<&str>,
        _windows_native_ops: bool,
    ) -> TrashActionResult {
        let mut affected_count = 0;
        let mut errors = Vec::new();

        // 1. Check for Windows Recycle Bin items if native ops enabled and no custom trash dir
        #[cfg(windows)]
        if _windows_native_ops && custom_trash.is_none() {
            let win_items = Self::list_windows_recycle_bin_items();
            let restore_all = items.as_ref().map_or(true, |l| l.is_empty());

            for it in &win_items {
                let should_restore = restore_all || items.as_ref().map_or(false, |list| {
                    list.iter().any(|name| {
                        let n_trim = name.trim();
                        let n_clean = n_trim.replace('/', "\\");
                        let orig_clean = it.original_path.replace('/', "\\");
                        let r_clean = it.r_path.to_string_lossy().replace('/', "\\");
                        let r_fn = it.r_path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        let n_basename = Path::new(n_trim).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| n_trim.to_string());

                        n_clean.eq_ignore_ascii_case(&r_clean)
                            || n_clean.eq_ignore_ascii_case(&orig_clean)
                            || n_clean.eq_ignore_ascii_case(&it.original_name)
                            || n_clean.eq_ignore_ascii_case(&r_fn)
                            || n_basename.eq_ignore_ascii_case(&it.original_name)
                            || n_basename.eq_ignore_ascii_case(&r_fn)
                            || it.original_name.eq_ignore_ascii_case(n_trim)
                            || it.original_path.eq_ignore_ascii_case(n_trim)
                    })
                });

                if should_restore {
                    let dest_path = crate::vfs::local::LocalFs::resolve_local_path(&it.original_path);
                    if let Some(parent) = dest_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    crate::vfs::local::clear_readonly_attribute(&it.r_path);
                    if dest_path.exists() {
                        crate::vfs::local::clear_readonly_attribute(&dest_path);
                    }

                    let move_res = match crate::vfs::local::windows_native_rename(&it.r_path, &dest_path) {
                        Ok(()) => Ok(()),
                        Err(_) => {
                            if let Err(err) = fs::rename(&it.r_path, &dest_path) {
                                Self::move_entry_recursive(&it.r_path, &dest_path).map_err(|e| {
                                    format!("Direct rename ({}) and cross-device move ({}) failed", err, e)
                                })
                            } else {
                                Ok(())
                            }
                        }
                    };

                    match move_res {
                        Ok(()) => {
                            let _ = fs::remove_file(&it.i_path);
                            affected_count += 1;
                            info!("Restored Windows Recycle Bin item {} -> {}", it.original_name, dest_path.display());
                        }
                        Err(e) => {
                            errors.push(format!("{}: {}", it.original_name, e));
                        }
                    }
                }
            }
            return TrashActionResult {
                success: errors.is_empty(),
                affected_count,
                freed_bytes: None,
                errors,
            };
        }

        // 2. Check POSIX / internal trash items
        let (_root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if files.exists() {
            let target_items = if let Some(list) = items.as_ref().filter(|l| !l.is_empty()) {
                list.clone()
            } else {
                match fs::read_dir(&files) {
                    Ok(read) => read
                        .filter_map(|e| e.ok().map(|ent| ent.file_name().to_string_lossy().to_string()))
                        .collect(),
                    Err(_) => Vec::new(),
                }
            };

            for name in target_items {
                let mut src_file = if Path::new(&name).is_absolute() {
                    PathBuf::from(&name)
                } else {
                    files.join(&name)
                };

                // If not found by direct filename, match against original path / name in .trashinfo
                if !src_file.exists() && !src_file.is_symlink() {
                    let n_basename = Path::new(&name).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| name.clone());
                    if let Ok(dir_entries) = fs::read_dir(&files) {
                        for ent in dir_entries.flatten() {
                            let candidate_name = ent.file_name().to_string_lossy().to_string();
                            let candidate_info = info.join(format!("{}.trashinfo", candidate_name));
                            if candidate_info.exists() {
                                let (orig_path, _) = Self::parse_trashinfo(&candidate_info);
                                if let Some(orig) = orig_path {
                                    let orig_fn = Path::new(&orig).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                                    if orig.eq_ignore_ascii_case(&name) 
                                        || orig_fn.eq_ignore_ascii_case(&name)
                                        || orig_fn.eq_ignore_ascii_case(&n_basename)
                                        || candidate_name.eq_ignore_ascii_case(&name)
                                        || candidate_name.eq_ignore_ascii_case(&n_basename) {
                                        src_file = ent.path();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                if !src_file.exists() && !src_file.is_symlink() {
                    continue;
                }

                let actual_filename = src_file.file_name().unwrap_or_default().to_string_lossy().to_string();
                let info_file = info.join(format!("{}.trashinfo", actual_filename));
                let orig_path_str = if info_file.exists() {
                    Self::parse_trashinfo(&info_file).0
                } else {
                    None
                };

                let dest_path = if let Some(p) = orig_path_str {
                    crate::vfs::local::LocalFs::resolve_local_path(&p)
                } else if let Some(home) = user_home.filter(|s| !s.trim().is_empty()) {
                    PathBuf::from(home).join(format!("restored_{}", actual_filename))
                } else if let Some(home) = dirs::home_dir() {
                    home.join(format!("restored_{}", actual_filename))
                } else {
                    PathBuf::from(format!("/tmp/restored_{}", actual_filename))
                };

                // Ensure parent directory exists
                if let Some(parent) = dest_path.parent() {
                    if !parent.exists() {
                        if let Err(e) = fs::create_dir_all(parent) {
                            errors.push(format!(
                                "{}: failed to create destination folder {}: {}",
                                actual_filename,
                                parent.display(),
                                e
                            ));
                            continue;
                        }
                    }
                }

                // Handle collision at destination
                let final_dest = if dest_path.exists() {
                    let dest_stem = dest_path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let dest_ext = dest_path
                        .extension()
                        .map(|e| e.to_string_lossy().to_string());
                    let parent = dest_path.parent().unwrap_or_else(|| Path::new("/"));

                    let mut counter = 1;
                    let mut candidate;
                    loop {
                        candidate = match &dest_ext {
                            Some(e) if !e.is_empty() => {
                                parent.join(format!("{} (restored {}).{}", dest_stem, counter, e))
                            }
                            _ => parent.join(format!("{} (restored {})", dest_stem, counter)),
                        };
                        if !candidate.exists() {
                            break;
                        }
                        counter += 1;
                    }
                    candidate
                } else {
                    dest_path
                };

                // Clear read-only attributes before moving
                crate::vfs::local::clear_readonly_attribute(&src_file);
                if final_dest.exists() {
                    crate::vfs::local::clear_readonly_attribute(&final_dest);
                }

                #[cfg(windows)]
                let move_res = match crate::vfs::local::windows_native_rename(&src_file, &final_dest) {
                    Ok(()) => Ok(()),
                    Err(_) => {
                        if let Err(err) = fs::rename(&src_file, &final_dest) {
                            Self::move_entry_recursive(&src_file, &final_dest).map_err(|e| {
                                format!("Direct rename ({}) and cross-device move ({}) failed", err, e)
                            })
                        } else {
                            Ok(())
                        }
                    }
                };

                #[cfg(not(windows))]
                let move_res = if let Err(err) = fs::rename(&src_file, &final_dest) {
                    Self::move_entry_recursive(&src_file, &final_dest).map_err(|e| {
                        format!("Direct rename ({}) and cross-device move ({}) failed", err, e)
                    })
                } else {
                    Ok(())
                };

                match move_res {
                    Ok(()) => {
                        affected_count += 1;
                        let _ = fs::remove_file(&info_file);
                        info!("Restored {} -> {}", actual_filename, final_dest.display());
                    }
                    Err(err_msg) => {
                        errors.push(format!("{}: {}", actual_filename, err_msg));
                    }
                }
            }
        }

        TrashActionResult {
            success: errors.is_empty(),
            affected_count,
            freed_bytes: None,
            errors,
        }
    }

    /// Empties all items and info files in the trash
    pub fn empty_trash(
        custom_trash: Option<&str>,
        user_home: Option<&str>,
        _windows_native_ops: bool,
    ) -> TrashActionResult {
        #[cfg(windows)]
        if _windows_native_ops && custom_trash.is_none() {
            unsafe {
                use windows_sys::Win32::UI::Shell::{
                    SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND,
                };
                let res = SHEmptyRecycleBinW(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
                );
                if res == 0 || res == (0x80004005u32 as i32) {
                    info!("Emptied Windows native Recycle Bin");
                }
            }
            return TrashActionResult {
                success: true,
                affected_count: 0,
                freed_bytes: None,
                errors: Vec::new(),
            };
        }

        let (root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if !root.exists() {
            return TrashActionResult {
                success: true,
                affected_count: 0,
                freed_bytes: Some(0),
                errors: Vec::new(),
            };
        }

        let mut affected_count = 0;
        let mut freed_bytes = 0;
        let mut errors = Vec::new();

        if files.exists() {
            if let Ok(entries) = fs::read_dir(&files) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Ok(m) = fs::symlink_metadata(&path) {
                        if m.is_dir() {
                            freed_bytes += Self::calculate_dir_size(&path);
                            if let Err(e) = fs::remove_dir_all(&path) {
                                errors.push(format!("Failed to remove directory {}: {}", path.display(), e));
                                continue;
                            }
                        } else {
                            freed_bytes += m.len();
                            if let Err(e) = fs::remove_file(&path) {
                                errors.push(format!("Failed to remove file {}: {}", path.display(), e));
                                continue;
                            }
                        }
                        affected_count += 1;
                    }
                }
            }
        }

        if info.exists() {
            if let Ok(entries) = fs::read_dir(&info) {
                for entry in entries.flatten() {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        // Re-ensure directories exist for future operations
        let _ = fs::create_dir_all(&files);
        let _ = fs::create_dir_all(&info);

        TrashActionResult {
            success: errors.is_empty(),
            affected_count,
            freed_bytes: Some(freed_bytes),
            errors,
        }
    }

    /// Permanently deletes specific items from the trash bin
    pub fn delete_items(
        items: Vec<String>,
        custom_trash: Option<&str>,
        user_home: Option<&str>,
        _windows_native_ops: bool,
    ) -> TrashActionResult {
        let mut affected_count = 0;
        let mut freed_bytes = 0;
        let mut errors = Vec::new();

        // 1. Check for Windows Recycle Bin items if native ops enabled and no custom trash dir
        #[cfg(windows)]
        if _windows_native_ops && custom_trash.is_none() {
            let win_items = Self::list_windows_recycle_bin_items();
            for it in &win_items {
                let should_delete = items.iter().any(|name| {
                    let n_trim = name.trim();
                    let n_clean = n_trim.replace('/', "\\");
                    let orig_clean = it.original_path.replace('/', "\\");
                    let r_clean = it.r_path.to_string_lossy().replace('/', "\\");
                    let r_fn = it.r_path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                    let n_basename = Path::new(n_trim).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| n_trim.to_string());

                    n_clean.eq_ignore_ascii_case(&r_clean)
                        || n_clean.eq_ignore_ascii_case(&orig_clean)
                        || n_clean.eq_ignore_ascii_case(&it.original_name)
                        || n_clean.eq_ignore_ascii_case(&r_fn)
                        || n_basename.eq_ignore_ascii_case(&it.original_name)
                        || n_basename.eq_ignore_ascii_case(&r_fn)
                        || it.original_name.eq_ignore_ascii_case(n_trim)
                        || it.original_path.eq_ignore_ascii_case(n_trim)
                });

                if should_delete {
                    crate::vfs::local::clear_readonly_attribute(&it.r_path);
                    freed_bytes += it.file_size;
                    if it.is_dir {
                        if let Err(e) = fs::remove_dir_all(&it.r_path) {
                            errors.push(format!("Failed to delete {}: {}", it.original_name, e));
                            continue;
                        }
                    } else if let Err(e) = fs::remove_file(&it.r_path) {
                        errors.push(format!("Failed to delete {}: {}", it.original_name, e));
                        continue;
                    }
                    let _ = fs::remove_file(&it.i_path);
                    affected_count += 1;
                }
            }
            return TrashActionResult {
                success: errors.is_empty(),
                affected_count,
                freed_bytes: Some(freed_bytes),
                errors,
            };
        }

        // 2. Check internal trash items
        let (_root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        for name in items {
            let mut file_path = if Path::new(&name).is_absolute() {
                PathBuf::from(&name)
            } else {
                files.join(&name)
            };

            // If not found by direct filename, match against original path / name in .trashinfo
            if !file_path.exists() && !file_path.is_symlink() {
                let n_basename = Path::new(&name).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| name.clone());
                if let Ok(dir_entries) = fs::read_dir(&files) {
                    for ent in dir_entries.flatten() {
                        let candidate_name = ent.file_name().to_string_lossy().to_string();
                        let candidate_info = info.join(format!("{}.trashinfo", candidate_name));
                        if candidate_info.exists() {
                            let (orig_path, _) = Self::parse_trashinfo(&candidate_info);
                            if let Some(orig) = orig_path {
                                let orig_fn = Path::new(&orig).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                                if orig.eq_ignore_ascii_case(&name) 
                                    || orig_fn.eq_ignore_ascii_case(&name)
                                    || orig_fn.eq_ignore_ascii_case(&n_basename)
                                    || candidate_name.eq_ignore_ascii_case(&name)
                                    || candidate_name.eq_ignore_ascii_case(&n_basename) {
                                    file_path = ent.path();
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            let actual_filename = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let info_path = info.join(format!("{}.trashinfo", actual_filename));

            if !file_path.exists() && !file_path.is_symlink() {
                let _ = fs::remove_file(&info_path);
                continue;
            }

            if let Ok(m) = fs::symlink_metadata(&file_path) {
                if m.is_dir() {
                    freed_bytes += Self::calculate_dir_size(&file_path);
                    if let Err(e) = fs::remove_dir_all(&file_path) {
                        errors.push(format!("{}: {}", actual_filename, e));
                        continue;
                    }
                } else {
                    freed_bytes += m.len();
                    if let Err(e) = fs::remove_file(&file_path) {
                        errors.push(format!("{}: {}", actual_filename, e));
                        continue;
                    }
                }
                affected_count += 1;
                let _ = fs::remove_file(&info_path);
            }
        }

        TrashActionResult {
            success: errors.is_empty(),
            affected_count,
            freed_bytes: Some(freed_bytes),
            errors,
        }
    }

    /// Recursively calculates directory size
    fn calculate_dir_size(dir: &Path) -> u64 {
        walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum()
    }

    /// Recursively moves an entry across filesystems
    fn move_entry_recursive(src: &Path, dst: &Path) -> io::Result<()> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_trash_encode_decode_path() {
        let original = "/home/bolt/My Documents/Project #1/test file (v1.0).pdf";
        let encoded = TrashManager::encode_trash_path(original);
        let decoded = TrashManager::decode_trash_path(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_trash_move_list_restore_lifecycle() {
        let temp = tempdir().unwrap();
        let trash_dir = temp.path().join("Trash");
        let work_dir = temp.path().join("workspace");
        fs::create_dir_all(&work_dir).unwrap();

        let sample_file = work_dir.join("sample.txt");
        fs::write(&sample_file, "Hello Trash").unwrap();

        let trash_str = trash_dir.to_str().unwrap();

        // 1. Move to trash
        let trashed_target = TrashManager::move_to_trash(&sample_file, Some(trash_str), None).unwrap();
        assert!(trashed_target.exists());
        assert!(!sample_file.exists());

        // 2. Summary
        let summary = TrashManager::get_trash_summary(Some(trash_str), None, false).unwrap();
        assert_eq!(summary.total_items, 1);
        assert_eq!(summary.total_size, 11);

        // 3. List items
        let items = TrashManager::list_trash_items(Some(trash_str), None, false).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "sample.txt");
        assert_eq!(items[0].original_path.as_deref(), Some(sample_file.to_str().unwrap()));
        assert!(items[0].deletion_date.is_some());

        // 4. Restore
        let restore_res = TrashManager::restore_items(Some(vec!["sample.txt".to_string()]), Some(trash_str), None, false);
        assert!(restore_res.success);
        assert_eq!(restore_res.affected_count, 1);
        assert!(sample_file.exists());
        assert_eq!(fs::read_to_string(&sample_file).unwrap(), "Hello Trash");

        // 5. Delete specific item lifecycle
        let sample2 = work_dir.join("sample2.txt");
        fs::write(&sample2, "To Delete").unwrap();
        TrashManager::move_to_trash(&sample2, Some(trash_str), None).unwrap();
        let del_res = TrashManager::delete_items(vec!["sample2.txt".to_string()], Some(trash_str), None, false);
        assert!(del_res.success);
        assert_eq!(del_res.affected_count, 1);

        // 6. Empty Trash
        let empty_res = TrashManager::empty_trash(Some(trash_str), None, false);
        assert!(empty_res.success);
        assert_eq!(empty_res.affected_count, 0);
    }

    #[test]
    fn test_trash_collision_handling() {
        let temp = tempdir().unwrap();
        let trash_dir = temp.path().join("Trash");
        let work_dir = temp.path().join("workspace");
        fs::create_dir_all(&work_dir).unwrap();

        let file1 = work_dir.join("data.csv");
        fs::write(&file1, "first").unwrap();
        let trash_str = trash_dir.to_str().unwrap();

        TrashManager::move_to_trash(&file1, Some(trash_str), None).unwrap();

        // Create second file with identical name
        let file2 = work_dir.join("data.csv");
        fs::write(&file2, "second").unwrap();

        TrashManager::move_to_trash(&file2, Some(trash_str), None).unwrap();

        let items = TrashManager::list_trash_items(Some(trash_str), None, false).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.name == "data.csv"));
        assert!(items.iter().any(|i| i.name == "data.1.csv"));
    }

    #[test]
    fn test_windows_recycle_bin_i_file_parsing_v1_and_v2() {
        let temp = tempdir().unwrap();
        
        // 1. Test Windows Vista/7/8 format (version 1)
        let i_file_v1 = temp.path().join("$Iv1.txt");
        let mut buf_v1 = Vec::new();
        buf_v1.extend_from_slice(&1u64.to_le_bytes()); // Version 1
        buf_v1.extend_from_slice(&1024u64.to_le_bytes()); // File size 1024
        buf_v1.extend_from_slice(&133400000000000000u64.to_le_bytes()); // FILETIME
        let path_v1: Vec<u16> = r"C:\Users\bolt\Documents\Report.txt".encode_utf16().chain(std::iter::once(0)).collect();
        for ch in path_v1 {
            buf_v1.extend_from_slice(&ch.to_le_bytes());
        }
        // Pad to at least 260 wchars
        while buf_v1.len() < 544 {
            buf_v1.push(0);
        }
        fs::write(&i_file_v1, buf_v1).unwrap();

        let (orig_path1, size1, time1) = TrashManager::parse_win_i_file(&i_file_v1).unwrap();
        assert_eq!(orig_path1, r"C:\Users\bolt\Documents\Report.txt");
        assert_eq!(size1, 1024);
        assert!(time1.is_some());

        // 2. Test Windows 10/11 format (version 2)
        let i_file_v2 = temp.path().join("$Iv2.docx");
        let mut buf_v2 = Vec::new();
        buf_v2.extend_from_slice(&2u64.to_le_bytes()); // Version 2
        buf_v2.extend_from_slice(&4096u64.to_le_bytes()); // File size 4096
        buf_v2.extend_from_slice(&133400000000000000u64.to_le_bytes()); // FILETIME
        let path_v2_str = r"D:\Projects\Brum\Design (2026).docx";
        let path_v2: Vec<u16> = path_v2_str.encode_utf16().chain(std::iter::once(0)).collect();
        buf_v2.extend_from_slice(&(path_v2.len() as u32).to_le_bytes()); // Character count
        for ch in path_v2 {
            buf_v2.extend_from_slice(&ch.to_le_bytes());
        }
        fs::write(&i_file_v2, buf_v2).unwrap();

        let (orig_path2, size2, time2) = TrashManager::parse_win_i_file(&i_file_v2).unwrap();
        assert_eq!(orig_path2, path_v2_str);
        assert_eq!(size2, 4096);
        assert!(time2.is_some());
    }

    #[test]
    fn test_list_trash_directory_entries_vfs() {
        let temp = tempdir().unwrap();
        let trash_dir = temp.path().join("Trash");
        let work_dir = temp.path().join("workspace");
        fs::create_dir_all(&work_dir).unwrap();

        let sample = work_dir.join("report.pdf");
        fs::write(&sample, "%PDF-1.4 dummy").unwrap();
        let trash_str = trash_dir.to_str().unwrap();

        TrashManager::move_to_trash(&sample, Some(trash_str), None).unwrap();

        let listing = TrashManager::list_trash_directory_entries(Some(trash_str), None, false);
        assert_eq!(listing.current_path, "trash://");
        assert_eq!(listing.protocol, "trash");
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "report.pdf");
        assert_eq!(listing.entries[0].mime_type.as_deref(), Some("application/pdf"));
    }
}
