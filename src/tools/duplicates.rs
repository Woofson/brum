use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DuplicateFileItem {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub is_image: bool,
    pub hash: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DuplicateGroup {
    pub hash: String,
    pub total_size: u64,
    pub files: Vec<DuplicateFileItem>,
}

#[derive(Debug, Deserialize)]
pub struct DuplicateScanRequest {
    pub paths: Vec<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub check_images_similarity: Option<bool>,
    pub include_hidden: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DuplicateScanResponse {
    pub total_files_scanned: usize,
    pub total_duplicate_groups: usize,
    pub total_duplicate_files: usize,
    pub reclaimable_bytes: u64,
    pub groups: Vec<DuplicateGroup>,
}

#[derive(Debug, Deserialize)]
pub struct DuplicateCleanRequest {
    pub files: Vec<String>,
    pub action: String, // "trash" or "delete"
    pub custom_trash_dir: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DuplicateCleanResponse {
    pub cleaned_count: usize,
    pub reclaimed_bytes: u64,
    pub failed_files: Vec<String>,
}

/// Computes a fast partial hash (first 4KB + last 4KB)
fn compute_partial_hash(path: &Path, size: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 4096];

    let n = file.read(&mut buf).ok()?;
    hasher.update(&buf[..n]);

    if size > 8192 {
        if file.seek(SeekFrom::End(-4096)).is_ok() {
            let n = file.read(&mut buf).ok()?;
            hasher.update(&buf[..n]);
        }
    }

    Some(hex::encode(hasher.finalize()))
}

/// Computes full SHA-256 hash of a file
fn compute_full_hash(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];

    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Some(hex::encode(hasher.finalize()))
}

/// Inspects basic image dimensions from JPEG/PNG headers if available
fn read_image_dimensions(path: &Path) -> (Option<u32>, Option<u32>, bool) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if !["jpg", "jpeg", "png", "webp", "bmp", "gif"].contains(&ext.as_str()) {
        return (None, None, false);
    }

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, None, true),
    };

    let mut header = [0u8; 32];
    let n = file.read(&mut header).unwrap_or(0);
    if n < 24 {
        return (None, None, true);
    }

    // PNG dimension check
    if header.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
        let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
        return (Some(width), Some(height), true);
    }

    (None, None, true)
}

/// Executes duplicate file scan across requested directories
pub fn scan_duplicates(req: DuplicateScanRequest) -> DuplicateScanResponse {
    let min_size = req.min_size.unwrap_or(1);
    let max_size = req.max_size.unwrap_or(u64::MAX);
    let include_hidden = req.include_hidden.unwrap_or(false);

    let mut all_files: Vec<(PathBuf, u64, u64)> = Vec::new();
    let mut scanned_count = 0;

    for root_str in req.paths {
        let root = Path::new(&root_str);
        if !root.exists() {
            continue;
        }

        let walker = WalkDir::new(root).follow_links(false).into_iter();
        for entry in walker.filter_entry(|e| {
            if e.depth() > 0 && !include_hidden {
                let file_name = e.file_name().to_string_lossy();
                if file_name.starts_with('.') {
                    return false;
                }
            }
            true
        }) {
            if let Ok(e) = entry {
                if e.file_type().is_file() {
                    scanned_count += 1;
                    if let Ok(meta) = e.metadata() {
                        let size = meta.len();
                        if size >= min_size && size <= max_size {
                            let modified = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            all_files.push((e.path().to_path_buf(), size, modified));
                        }
                    }
                }
            }
        }
    }

    // Step 1: Group by file size
    let mut by_size: HashMap<u64, Vec<(PathBuf, u64)>> = HashMap::new();
    for (p, size, modified) in all_files {
        by_size.entry(size).or_default().push((p, modified));
    }

    // Step 2: For same size, filter groups with > 1 file
    let mut size_candidates: Vec<(u64, Vec<(PathBuf, u64)>)> = Vec::new();
    for (size, paths) in by_size {
        if paths.len() > 1 {
            size_candidates.push((size, paths));
        }
    }

    // Step 3: Fast partial hash
    let mut by_partial: HashMap<(u64, String), Vec<(PathBuf, u64)>> = HashMap::new();
    for (size, paths) in size_candidates {
        for (p, modified) in paths {
            if let Some(ph) = compute_partial_hash(&p, size) {
                by_partial.entry((size, ph)).or_default().push((p, modified));
            }
        }
    }

    // Step 4: Full SHA-256 hash for partial matches
    let mut by_full: HashMap<String, Vec<DuplicateFileItem>> = HashMap::new();
    for ((size, _), paths) in by_partial {
        if paths.len() > 1 {
            for (p, modified) in paths {
                if let Some(fh) = compute_full_hash(&p) {
                    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let (width, height, is_image) = read_image_dimensions(&p);
                    by_full.entry(fh.clone()).or_default().push(DuplicateFileItem {
                        path: p.to_string_lossy().to_string(),
                        name,
                        size,
                        modified,
                        width,
                        height,
                        is_image,
                        hash: fh,
                    });
                }
            }
        }
    }

    // Step 5: Format duplicate groups
    let mut groups: Vec<DuplicateGroup> = Vec::new();
    let mut total_dup_files = 0;
    let mut reclaimable_bytes = 0;

    for (hash, mut files) in by_full {
        if files.len() > 1 {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            let total_size = files[0].size;
            let group_reclaimable = total_size * (files.len() as u64 - 1);
            reclaimable_bytes += group_reclaimable;
            total_dup_files += files.len();
            groups.push(DuplicateGroup {
                hash,
                total_size,
                files,
            });
        }
    }

    groups.sort_by(|a, b| (b.total_size * b.files.len() as u64).cmp(&(a.total_size * a.files.len() as u64)));

    DuplicateScanResponse {
        total_files_scanned: scanned_count,
        total_duplicate_groups: groups.len(),
        total_duplicate_files: total_dup_files,
        reclaimable_bytes,
        groups,
    }
}

/// Cleans or moves duplicate files to trash / quarantine
pub fn clean_duplicates(req: DuplicateCleanRequest) -> DuplicateCleanResponse {
    let mut cleaned_count = 0;
    let mut reclaimed_bytes = 0;
    let mut failed_files = Vec::new();

    let trash_dir = req
        .custom_trash_dir
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/share/Trash/files")
        });

    if req.action == "trash" && !trash_dir.exists() {
        let _ = fs::create_dir_all(&trash_dir);
    }

    for file_str in req.files {
        let path = Path::new(&file_str);
        if !path.exists() || !path.is_file() {
            failed_files.push(file_str);
            continue;
        }

        let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        if req.action == "trash" {
            let file_name = path.file_name().unwrap_or_default();
            let mut target = trash_dir.join(file_name);
            let mut counter = 1;
            while target.exists() {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                target = trash_dir.join(format!("{}_{}{}", stem, counter, ext));
                counter += 1;
            }

            match fs::rename(path, &target) {
                Ok(_) => {
                    cleaned_count += 1;
                    reclaimed_bytes += file_size;
                }
                Err(_) => {
                    // Fallback to copy + delete
                    if fs::copy(path, &target).is_ok() && fs::remove_file(path).is_ok() {
                        cleaned_count += 1;
                        reclaimed_bytes += file_size;
                    } else {
                        failed_files.push(file_str);
                    }
                }
            }
        } else {
            match fs::remove_file(path) {
                Ok(_) => {
                    cleaned_count += 1;
                    reclaimed_bytes += file_size;
                }
                Err(_) => {
                    failed_files.push(file_str);
                }
            }
        }
    }

    DuplicateCleanResponse {
        cleaned_count,
        reclaimed_bytes,
        failed_files,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_duplicate_scan_and_clean() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("a.txt");
        let file2 = dir.path().join("b.txt");
        let file3 = dir.path().join("unique.txt");

        fs::write(&file1, b"identical content 1234567890").unwrap();
        fs::write(&file2, b"identical content 1234567890").unwrap();
        fs::write(&file3, b"unique content").unwrap();

        let req = DuplicateScanRequest {
            paths: vec![dir.path().to_string_lossy().to_string()],
            min_size: Some(1),
            max_size: None,
            include_hidden: Some(false),
            check_images_similarity: None,
        };

        let res = scan_duplicates(req);
        assert_eq!(res.groups.len(), 1);
        assert_eq!(res.groups[0].files.len(), 2);
        assert!(res.reclaimable_bytes > 0);

        let clean_req = DuplicateCleanRequest {
            files: vec![file2.to_string_lossy().to_string()],
            action: "delete".to_string(),
            custom_trash_dir: None,
        };

        let clean_res = clean_duplicates(clean_req);
        assert_eq!(clean_res.cleaned_count, 1);
        assert_eq!(clean_res.failed_files.len(), 0);
        assert!(!file2.exists());
        assert!(file1.exists());
    }
}

