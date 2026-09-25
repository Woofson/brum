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
}

#[derive(Debug, Deserialize)]
pub struct TrashDeleteRequest {
    pub items: Vec<String>,
    pub custom_trash_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrashEmptyRequest {
    pub custom_trash_dir: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TrashActionResult {
    pub success: bool,
    pub affected_count: usize,
    pub freed_bytes: Option<u64>,
    pub errors: Vec<String>,
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

    /// Enumerates all trashed items with metadata
    pub fn list_trash_items(
        custom_trash: Option<&str>,
        user_home: Option<&str>,
    ) -> io::Result<Vec<TrashItem>> {
        let (_root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if !files.exists() {
            return Ok(Vec::new());
        }

        let mut items = Vec::new();
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

    /// Returns high-level summary statistics of the trash bin
    pub fn get_trash_summary(
        custom_trash: Option<&str>,
        user_home: Option<&str>,
    ) -> io::Result<TrashSummary> {
        let (root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if !files.exists() {
            return Ok(TrashSummary {
                total_items: 0,
                total_size: 0,
                trash_dir: root.to_string_lossy().to_string(),
                files_dir: files.to_string_lossy().to_string(),
                info_dir: info.to_string_lossy().to_string(),
            });
        }

        let mut total_items = 0;
        let mut total_size = 0;

        for entry in fs::read_dir(&files)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
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
    ) -> TrashActionResult {
        let (_root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        if !files.exists() {
            return TrashActionResult {
                success: true,
                affected_count: 0,
                freed_bytes: Some(0),
                errors: Vec::new(),
            };
        }

        let target_items = if let Some(list) = items.filter(|l| !l.is_empty()) {
            list
        } else {
            // Restore everything
            match fs::read_dir(&files) {
                Ok(read) => read
                    .filter_map(|e| e.ok().map(|ent| ent.file_name().to_string_lossy().to_string()))
                    .collect(),
                Err(e) => {
                    return TrashActionResult {
                        success: false,
                        affected_count: 0,
                        freed_bytes: None,
                        errors: vec![format!("Failed to read trash directory: {}", e)],
                    };
                }
            }
        };

        let mut affected_count = 0;
        let mut errors = Vec::new();

        for name in target_items {
            let src_file = files.join(&name);
            if !src_file.exists() && !src_file.is_symlink() {
                errors.push(format!("{}: file not found in trash", name));
                continue;
            }

            let info_file = info.join(format!("{}.trashinfo", name));
            let orig_path_str = if info_file.exists() {
                Self::parse_trashinfo(&info_file).0
            } else {
                None
            };

            let dest_path = if let Some(p) = orig_path_str {
                PathBuf::from(p)
            } else if let Some(home) = user_home.filter(|s| !s.trim().is_empty()) {
                PathBuf::from(home).join(format!("restored_{}", name))
            } else if let Some(home) = dirs::home_dir() {
                home.join(format!("restored_{}", name))
            } else {
                PathBuf::from(format!("/tmp/restored_{}", name))
            };

            // Ensure parent directory exists
            if let Some(parent) = dest_path.parent() {
                if !parent.exists() {
                    if let Err(e) = fs::create_dir_all(parent) {
                        errors.push(format!(
                            "{}: failed to create destination folder {}: {}",
                            name,
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

            // Move back to destination
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
                    info!("Restored {} -> {}", name, final_dest.display());
                }
                Err(err_msg) => {
                    errors.push(format!("{}: {}", name, err_msg));
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
    ) -> TrashActionResult {
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
    ) -> TrashActionResult {
        let (_root, files, info) = Self::get_trash_paths(custom_trash, user_home);
        let mut affected_count = 0;
        let mut freed_bytes = 0;
        let mut errors = Vec::new();

        for name in items {
            let file_path = files.join(&name);
            let info_path = info.join(format!("{}.trashinfo", name));

            if !file_path.exists() && !file_path.is_symlink() {
                // If info file exists, clean it up
                let _ = fs::remove_file(&info_path);
                continue;
            }

            if let Ok(m) = fs::symlink_metadata(&file_path) {
                if m.is_dir() {
                    freed_bytes += Self::calculate_dir_size(&file_path);
                    if let Err(e) = fs::remove_dir_all(&file_path) {
                        errors.push(format!("{}: {}", name, e));
                        continue;
                    }
                } else {
                    freed_bytes += m.len();
                    if let Err(e) = fs::remove_file(&file_path) {
                        errors.push(format!("{}: {}", name, e));
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
        let summary = TrashManager::get_trash_summary(Some(trash_str), None).unwrap();
        assert_eq!(summary.total_items, 1);
        assert_eq!(summary.total_size, 11);

        // 3. List items
        let items = TrashManager::list_trash_items(Some(trash_str), None).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "sample.txt");
        assert_eq!(items[0].original_path.as_deref(), Some(sample_file.to_str().unwrap()));
        assert!(items[0].deletion_date.is_some());

        // 4. Restore
        let restore_res = TrashManager::restore_items(Some(vec!["sample.txt".to_string()]), Some(trash_str), None);
        assert!(restore_res.success);
        assert_eq!(restore_res.affected_count, 1);
        assert!(sample_file.exists());
        assert_eq!(fs::read_to_string(&sample_file).unwrap(), "Hello Trash");

        // 5. Empty Trash
        let empty_res = TrashManager::empty_trash(Some(trash_str), None);
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

        let items = TrashManager::list_trash_items(Some(trash_str), None).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.name == "data.csv"));
        assert!(items.iter().any(|i| i.name == "data.1.csv"));
    }
}
