use crate::vfs::checksum::calculate_sha256;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::fs;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub line_num_left: Option<usize>,
    pub line_num_right: Option<usize>,
    pub tag: String, // "equal", "insert", "delete"
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffResult {
    pub file_left: String,
    pub file_right: String,
    pub lines: Vec<DiffLine>,
    pub additions: usize,
    pub deletions: usize,
    pub is_identical: bool,
}

pub fn compare_files_text(
    file_left_name: &str,
    text_left: &str,
    file_right_name: &str,
    text_right: &str,
) -> FileDiffResult {
    let diff = TextDiff::from_lines(text_left, text_right);
    let mut lines = Vec::new();
    let mut additions = 0;
    let mut deletions = 0;

    let mut left_idx = 1;
    let mut right_idx = 1;

    for change in diff.iter_all_changes() {
        let (tag_str, num_l, num_r) = match change.tag() {
            ChangeTag::Equal => {
                let l = left_idx;
                let r = right_idx;
                left_idx += 1;
                right_idx += 1;
                ("equal", Some(l), Some(r))
            }
            ChangeTag::Delete => {
                let l = left_idx;
                left_idx += 1;
                deletions += 1;
                ("delete", Some(l), None)
            }
            ChangeTag::Insert => {
                let r = right_idx;
                right_idx += 1;
                additions += 1;
                ("insert", None, Some(r))
            }
        };

        lines.push(DiffLine {
            line_num_left: num_l,
            line_num_right: num_r,
            tag: tag_str.to_string(),
            content: change.value().to_string(),
        });
    }

    FileDiffResult {
        file_left: file_left_name.to_string(),
        file_right: file_right_name.to_string(),
        lines,
        additions,
        deletions,
        is_identical: additions == 0 && deletions == 0,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderEntryDiff {
    pub relative_path: String,
    pub status: String, // "identical", "modified", "left_only", "right_only"
    pub is_dir: bool,
    pub size_left: Option<u64>,
    pub size_right: Option<u64>,
    pub size_delta: Option<i64>,
    pub mtime_left: Option<u64>,
    pub mtime_right: Option<u64>,
    pub time_status: Option<String>, // "left_newer", "right_newer", "equal"
    pub hash_left: Option<String>,
    pub hash_right: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderDiffResult {
    pub dir_left: String,
    pub dir_right: String,
    pub total_left_entries: usize,
    pub total_right_entries: usize,
    pub identical_count: usize,
    pub modified_count: usize,
    pub left_only_count: usize,
    pub right_only_count: usize,
    pub entries: Vec<FolderEntryDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksumEntryResult {
    pub relative_path: String,
    pub hash_left: Option<String>,
    pub hash_right: Option<String>,
    pub is_identical: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchChecksumResult {
    pub dir_left: String,
    pub dir_right: String,
    pub results: Vec<ChecksumEntryResult>,
}

fn mtime_from_meta(meta: &fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

pub fn compare_folders(
    dir_left_str: &str,
    dir_right_str: &str,
    recursive: bool,
    deep_hash_compare: bool,
    selected_items: Option<Vec<String>>,
) -> Result<FolderDiffResult, std::io::Error> {
    let p_left = Path::new(dir_left_str);
    let p_right = Path::new(dir_right_str);

    // Map: path -> (is_dir, size, mtime, hash)
    let mut map_left: HashMap<String, (bool, u64, Option<u64>, Option<String>)> = HashMap::new();
    let mut map_right: HashMap<String, (bool, u64, Option<u64>, Option<String>)> = HashMap::new();

    // If specific selected items were requested:
    if let Some(items) = selected_items {
        for item in items {
            let path_l = p_left.join(&item);
            let path_r = p_right.join(&item);

            if path_l.exists() {
                let is_dir = path_l.is_dir();
                let meta = fs::metadata(&path_l).ok();
                let size = if is_dir { 0 } else { meta.as_ref().map_or(0, |m| m.len()) };
                let mtime = meta.as_ref().and_then(mtime_from_meta);
                let hash = if !is_dir && deep_hash_compare { calculate_sha256(&path_l).ok() } else { None };
                map_left.insert(item.clone(), (is_dir, size, mtime, hash));
            }
            if path_r.exists() {
                let is_dir = path_r.is_dir();
                let meta = fs::metadata(&path_r).ok();
                let size = if is_dir { 0 } else { meta.as_ref().map_or(0, |m| m.len()) };
                let mtime = meta.as_ref().and_then(mtime_from_meta);
                let hash = if !is_dir && deep_hash_compare { calculate_sha256(&path_r).ok() } else { None };
                map_right.insert(item, (is_dir, size, mtime, hash));
            }
        }
    } else if !recursive {
        // Fast Immediate Level Compare: only read immediate children (O(N) ~ 5ms)
        if p_left.is_dir() {
            if let Ok(entries) = fs::read_dir(p_left) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let meta = entry.metadata().ok();
                    let size = if is_dir { 0 } else { meta.as_ref().map_or(0, |m| m.len()) };
                    let mtime = meta.as_ref().and_then(mtime_from_meta);
                    let hash = if !is_dir && deep_hash_compare { calculate_sha256(&entry.path()).ok() } else { None };
                    map_left.insert(name, (is_dir, size, mtime, hash));
                }
            }
        }

        if p_right.is_dir() {
            if let Ok(entries) = fs::read_dir(p_right) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let meta = entry.metadata().ok();
                    let size = if is_dir { 0 } else { meta.as_ref().map_or(0, |m| m.len()) };
                    let mtime = meta.as_ref().and_then(mtime_from_meta);
                    let hash = if !is_dir && deep_hash_compare { calculate_sha256(&entry.path()).ok() } else { None };
                    map_right.insert(name, (is_dir, size, mtime, hash));
                }
            }
        }
    } else {
        // Recursive WalkDir
        if p_left.is_dir() {
            for entry in WalkDir::new(p_left).max_depth(10).into_iter().filter_map(|e| e.ok()) {
                if let Ok(rel) = entry.path().strip_prefix(p_left) {
                    let rel_str = rel.to_string_lossy().to_string();
                    if rel_str.is_empty() {
                        continue;
                    }
                    let is_dir = entry.file_type().is_dir();
                    let meta = entry.metadata().ok();
                    let size = if is_dir { 0 } else { meta.as_ref().map_or(0, |m| m.len()) };
                    let mtime = meta.as_ref().and_then(mtime_from_meta);
                    let hash = if !is_dir && deep_hash_compare {
                        calculate_sha256(entry.path()).ok()
                    } else {
                        None
                    };
                    map_left.insert(rel_str, (is_dir, size, mtime, hash));
                }
            }
        }

        if p_right.is_dir() {
            for entry in WalkDir::new(p_right).max_depth(10).into_iter().filter_map(|e| e.ok()) {
                if let Ok(rel) = entry.path().strip_prefix(p_right) {
                    let rel_str = rel.to_string_lossy().to_string();
                    if rel_str.is_empty() {
                        continue;
                    }
                    let is_dir = entry.file_type().is_dir();
                    let meta = entry.metadata().ok();
                    let size = if is_dir { 0 } else { meta.as_ref().map_or(0, |m| m.len()) };
                    let mtime = meta.as_ref().and_then(mtime_from_meta);
                    let hash = if !is_dir && deep_hash_compare {
                        calculate_sha256(entry.path()).ok()
                    } else {
                        None
                    };
                    map_right.insert(rel_str, (is_dir, size, mtime, hash));
                }
            }
        }
    }

    let mut all_keys: HashSet<String> = HashSet::new();
    for k in map_left.keys() {
        all_keys.insert(k.clone());
    }
    for k in map_right.keys() {
        all_keys.insert(k.clone());
    }

    let mut sorted_keys: Vec<String> = all_keys.into_iter().collect();
    sorted_keys.sort();

    let mut entries = Vec::new();
    let mut identical_count = 0;
    let mut modified_count = 0;
    let mut left_only_count = 0;
    let mut right_only_count = 0;

    for key in sorted_keys {
        let in_left = map_left.get(&key);
        let in_right = map_right.get(&key);

        match (in_left, in_right) {
            (Some(l), Some(r)) => {
                let is_dir = l.0;
                let size_l = l.1;
                let size_r = r.1;
                let mtime_l = l.2;
                let mtime_r = r.2;
                let hash_l = l.3.clone();
                let hash_r = r.3.clone();

                let size_delta = Some(size_l as i64 - size_r as i64);
                let time_status = match (mtime_l, mtime_r) {
                    (Some(tl), Some(tr)) => {
                        if tl.abs_diff(tr) <= 1 {
                            Some("equal".to_string())
                        } else if tl > tr {
                            Some("left_newer".to_string())
                        } else {
                            Some("right_newer".to_string())
                        }
                    }
                    _ => None,
                };

                let is_same = if is_dir {
                    true
                } else if deep_hash_compare && hash_l.is_some() && hash_r.is_some() {
                    hash_l == hash_r
                } else if size_l != size_r {
                    false
                } else if let (Some(tl), Some(tr)) = (mtime_l, mtime_r) {
                    tl.abs_diff(tr) <= 1
                } else {
                    true
                };

                let status = if is_same {
                    identical_count += 1;
                    "identical"
                } else {
                    modified_count += 1;
                    "modified"
                };

                entries.push(FolderEntryDiff {
                    relative_path: key,
                    status: status.to_string(),
                    is_dir,
                    size_left: Some(size_l),
                    size_right: Some(size_r),
                    size_delta,
                    mtime_left: mtime_l,
                    mtime_right: mtime_r,
                    time_status,
                    hash_left: hash_l,
                    hash_right: hash_r,
                });
            }
            (Some(l), None) => {
                left_only_count += 1;
                entries.push(FolderEntryDiff {
                    relative_path: key,
                    status: "left_only".to_string(),
                    is_dir: l.0,
                    size_left: Some(l.1),
                    size_right: None,
                    size_delta: None,
                    mtime_left: l.2,
                    mtime_right: None,
                    time_status: None,
                    hash_left: l.3.clone(),
                    hash_right: None,
                });
            }
            (None, Some(r)) => {
                right_only_count += 1;
                entries.push(FolderEntryDiff {
                    relative_path: key,
                    status: "right_only".to_string(),
                    is_dir: r.0,
                    size_left: None,
                    size_right: Some(r.1),
                    size_delta: None,
                    mtime_left: None,
                    mtime_right: r.2,
                    time_status: None,
                    hash_left: None,
                    hash_right: r.3.clone(),
                });
            }
            (None, None) => {}
        }
    }

    Ok(FolderDiffResult {
        dir_left: dir_left_str.to_string(),
        dir_right: dir_right_str.to_string(),
        total_left_entries: map_left.len(),
        total_right_entries: map_right.len(),
        identical_count,
        modified_count,
        left_only_count,
        right_only_count,
        entries,
    })
}

pub fn calculate_checksums_for_entries(
    dir_left: &str,
    dir_right: &str,
    relative_paths: &[String],
) -> BatchChecksumResult {
    let p_left = Path::new(dir_left);
    let p_right = Path::new(dir_right);
    let mut results = Vec::new();

    for rel in relative_paths {
        let path_l = p_left.join(rel);
        let path_r = p_right.join(rel);

        let hash_l = if path_l.is_file() {
            calculate_sha256(&path_l).ok()
        } else {
            None
        };

        let hash_r = if path_r.is_file() {
            calculate_sha256(&path_r).ok()
        } else {
            None
        };

        let is_identical = match (&hash_l, &hash_r) {
            (Some(hl), Some(hr)) => hl == hr,
            _ => false,
        };

        results.push(ChecksumEntryResult {
            relative_path: rel.clone(),
            hash_left: hash_l,
            hash_right: hash_r,
            is_identical,
        });
    }

    BatchChecksumResult {
        dir_left: dir_left.to_string(),
        dir_right: dir_right.to_string(),
        results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_compare_files_text() {
        let text_a = "line1\nline2\nline3\n";
        let text_b = "line1\nline2_modified\nline3\n";

        let diff = compare_files_text("a.txt", text_a, "b.txt", text_b);
        assert_eq!(diff.file_left, "a.txt");
        assert_eq!(diff.file_right, "b.txt");
        assert!(!diff.is_identical);
        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.additions, 1);

        let del_line = diff.lines.iter().find(|l| l.tag == "delete").unwrap();
        assert_eq!(del_line.content.trim(), "line2");

        let ins_line = diff.lines.iter().find(|l| l.tag == "insert").unwrap();
        assert_eq!(ins_line.content.trim(), "line2_modified");
    }

    #[test]
    fn test_compare_folders_with_deltas_and_mtimes() {
        let dir = tempdir().unwrap();
        let left = dir.path().join("left");
        let right = dir.path().join("right");
        fs::create_dir(&left).unwrap();
        fs::create_dir(&right).unwrap();

        // 1. Identical file
        fs::write(left.join("same.txt"), "hello world").unwrap();
        fs::write(right.join("same.txt"), "hello world").unwrap();

        // 2. Left only
        fs::write(left.join("left_only.txt"), "left content").unwrap();

        // 3. Right only
        fs::write(right.join("right_only.txt"), "right content").unwrap();

        // 4. Modified file with size delta
        fs::write(left.join("delta.txt"), "abc").unwrap();
        fs::write(right.join("delta.txt"), "abcdefgh").unwrap();

        let result = compare_folders(left.to_str().unwrap(), right.to_str().unwrap(), false, false, None).unwrap();
        assert_eq!(result.total_left_entries, 3);
        assert_eq!(result.total_right_entries, 3);

        // Find each entry
        let same = result.entries.iter().find(|e| e.relative_path == "same.txt").unwrap();
        assert_eq!(same.status, "identical");
        assert_eq!(same.size_delta, Some(0));

        let l_only = result.entries.iter().find(|e| e.relative_path == "left_only.txt").unwrap();
        assert_eq!(l_only.status, "left_only");

        let r_only = result.entries.iter().find(|e| e.relative_path == "right_only.txt").unwrap();
        assert_eq!(r_only.status, "right_only");

        let delta = result.entries.iter().find(|e| e.relative_path == "delta.txt").unwrap();
        assert_eq!(delta.status, "modified");
        // left is 3 bytes, right is 8 bytes => delta is 3 - 8 = -5
        assert_eq!(delta.size_delta, Some(-5));
        assert!(delta.time_status.is_some());
    }

    #[test]
    fn test_calculate_checksums_for_entries() {
        let dir = tempdir().unwrap();
        let left = dir.path().join("left");
        let right = dir.path().join("right");
        fs::create_dir(&left).unwrap();
        fs::create_dir(&right).unwrap();

        fs::write(left.join("file1.txt"), "match").unwrap();
        fs::write(right.join("file1.txt"), "match").unwrap();

        fs::write(left.join("file2.txt"), "different left").unwrap();
        fs::write(right.join("file2.txt"), "different right").unwrap();

        let rels = vec!["file1.txt".to_string(), "file2.txt".to_string()];
        let batch = calculate_checksums_for_entries(left.to_str().unwrap(), right.to_str().unwrap(), &rels);

        assert_eq!(batch.results.len(), 2);
        let r1 = batch.results.iter().find(|r| r.relative_path == "file1.txt").unwrap();
        assert!(r1.is_identical);
        assert!(r1.hash_left.is_some());
        assert_eq!(r1.hash_left, r1.hash_right);

        let r2 = batch.results.iter().find(|r| r.relative_path == "file2.txt").unwrap();
        assert!(!r2.is_identical);
        assert_ne!(r2.hash_left, r2.hash_right);
    }
}
