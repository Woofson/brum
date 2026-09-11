use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};
use walkdir::WalkDir;

const MAX_TOP_FILES: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsageItem {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub percentage: f64,
    pub file_count: usize,
    pub dir_count: usize,
    pub last_modified: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsageReport {
    pub root_path: String,
    pub total_bytes: u64,
    pub formatted_total: String,
    pub total_files: usize,
    pub total_dirs: usize,
    pub scan_duration_ms: u64,
    pub items: Vec<DiskUsageItem>,
    pub largest_files: Vec<DiskUsageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskMountInfo {
    pub name: String,
    pub mount_point: String,
    pub fs_type: String,
    pub device: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub formatted_total: String,
    pub formatted_used: String,
    pub formatted_available: String,
    pub usage_percentage: f64,
    pub is_read_only: bool,
    pub is_removable: bool,
    pub storage_root_id: Option<String>,
}

pub struct DiskUsageEngine;

#[cfg(unix)]
pub fn query_statvfs(path: &str) -> Option<(u64, u64, u64, bool)> {
    use std::ffi::CString;
    let c_path = CString::new(path).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } == 0 {
        let block_size = if stat.f_frsize > 0 {
            stat.f_frsize as u64
        } else {
            stat.f_bsize as u64
        };
        let total_bytes = stat.f_blocks as u64 * block_size;
        let available_bytes = stat.f_bavail as u64 * block_size;
        let free_bytes = stat.f_bfree as u64 * block_size;
        let used_bytes = total_bytes.saturating_sub(free_bytes);
        let is_ro = (stat.f_flag & libc::ST_RDONLY as libc::c_ulong) != 0;
        Some((total_bytes, used_bytes, available_bytes, is_ro))
    } else {
        None
    }
}

#[cfg(windows)]
fn query_windows_disk(path: &str) -> Option<(u64, u64, u64)> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
    let mut free_bytes_available: u64 = 0;
    let mut total_number_of_bytes: u64 = 0;
    let mut total_number_of_free_bytes: u64 = 0;

    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let success = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes_available,
            &mut total_number_of_bytes,
            &mut total_number_of_free_bytes,
        )
    };

    if success != 0 && total_number_of_bytes > 0 {
        let used = total_number_of_bytes.saturating_sub(total_number_of_free_bytes);
        Some((total_number_of_bytes, used, free_bytes_available))
    } else {
        None
    }
}

pub fn get_system_disks(storage_roots: &[crate::config::StorageRoot]) -> Vec<DiskMountInfo> {
    let mut disks: Vec<DiskMountInfo> = Vec::new();

    #[cfg(unix)]
    {
        // 1. Parse /proc/mounts on Linux
        if let Ok(content) = fs::read_to_string("/proc/mounts") {
            let ignored_types = [
                "proc", "sysfs", "devpts", "cgroup", "cgroup2", "pstore",
                "bpf", "tracefs", "fusectl", "securityfs", "configfs", "autofs",
                "mqueue", "hugetlbfs", "debugfs", "ramfs", "binfmt_misc", "nsfs",
                "efivarfs", "devtmpfs", "squashfs",
            ];

            let mut seen_mounts = std::collections::HashSet::new();

            for line in content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let device = parts[0];
                    let mount_point = parts[1];
                    let fs_type = parts[2];
                    let options = parts[3];

                    if ignored_types.contains(&fs_type) {
                        continue;
                    }
                    if mount_point.starts_with("/proc")
                        || mount_point.starts_with("/sys")
                        || mount_point.starts_with("/dev/")
                        || mount_point.starts_with("/run/docker")
                        || mount_point.starts_with("/var/lib/docker")
                        || mount_point.starts_with("/var/lib/containers")
                    {
                        continue;
                    }

                    if !seen_mounts.insert(mount_point.to_string()) {
                        continue;
                    }

                    if let Some((total, used, avail, is_ro_flag)) = query_statvfs(mount_point) {
                        if total == 0 {
                            continue;
                        }
                        let is_ro = is_ro_flag || options.split(',').any(|o| o == "ro");
                        let name = if mount_point == "/" {
                            "Root (/)".to_string()
                        } else {
                            Path::new(mount_point)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| mount_point.to_string())
                        };

                        let pct = if total > 0 {
                            ((used as f64 / total as f64) * 100.0).min(100.0)
                        } else {
                            0.0
                        };

                        disks.push(DiskMountInfo {
                            name,
                            mount_point: mount_point.to_string(),
                            fs_type: fs_type.to_string(),
                            device: device.to_string(),
                            total_bytes: total,
                            used_bytes: used,
                            available_bytes: avail,
                            formatted_total: format_size(total),
                            formatted_used: format_size(used),
                            formatted_available: format_size(avail),
                            usage_percentage: (pct * 10.0).round() / 10.0,
                            is_read_only: is_ro,
                            is_removable: false,
                            storage_root_id: None,
                        });
                    }
                }
            }
        }

        // 2. Fallback ensure root (/) exists if not found in /proc/mounts
        if disks.is_empty() {
            if let Some((total, used, avail, is_ro)) = query_statvfs("/") {
                let pct = if total > 0 { (used as f64 / total as f64) * 100.0 } else { 0.0 };
                disks.push(DiskMountInfo {
                    name: "Root (/)".to_string(),
                    mount_point: "/".to_string(),
                    fs_type: "rootfs".to_string(),
                    device: "/dev/root".to_string(),
                    total_bytes: total,
                    used_bytes: used,
                    available_bytes: avail,
                    formatted_total: format_size(total),
                    formatted_used: format_size(used),
                    formatted_available: format_size(avail),
                    usage_percentage: (pct * 10.0).round() / 10.0,
                    is_read_only: is_ro,
                    is_removable: false,
                    storage_root_id: None,
                });
            }
        }
    }

    #[cfg(windows)]
    {
        for b in b'A'..=b'Z' {
            let drive_root = format!("{}:\\", b as char);
            if Path::new(&drive_root).exists() {
                if let Some((total, used, avail)) = query_windows_disk(&drive_root) {
                    if total > 0 {
                        let pct = ((used as f64 / total as f64) * 100.0).min(100.0);
                        disks.push(DiskMountInfo {
                            name: format!("Local Disk ({}:)", b as char),
                            mount_point: drive_root,
                            fs_type: "NTFS".to_string(),
                            device: format!("\\\\.\\{}:", b as char),
                            total_bytes: total,
                            used_bytes: used,
                            available_bytes: avail,
                            formatted_total: format_size(total),
                            formatted_used: format_size(used),
                            formatted_available: format_size(avail),
                            usage_percentage: (pct * 10.0).round() / 10.0,
                            is_read_only: false,
                            is_removable: false,
                            storage_root_id: None,
                        });
                    }
                }
            }
        }
    }

    // Correlate with configured storage roots
    for root in storage_roots {
        if let Some(existing) = disks.iter_mut().find(|d| d.mount_point == root.path) {
            existing.storage_root_id = Some(root.id.clone());
            if existing.name.is_empty() || existing.name == "/" {
                existing.name = root.name.clone();
            }
        } else if Path::new(&root.path).exists() {
            #[cfg(unix)]
            if let Some((total, used, avail, is_ro)) = query_statvfs(&root.path) {
                let pct = if total > 0 { ((used as f64 / total as f64) * 100.0).min(100.0) } else { 0.0 };
                disks.push(DiskMountInfo {
                    name: root.name.clone(),
                    mount_point: root.path.clone(),
                    fs_type: "mount".to_string(),
                    device: root.id.clone(),
                    total_bytes: total,
                    used_bytes: used,
                    available_bytes: avail,
                    formatted_total: format_size(total),
                    formatted_used: format_size(used),
                    formatted_available: format_size(avail),
                    usage_percentage: (pct * 10.0).round() / 10.0,
                    is_read_only: root.read_only || is_ro,
                    is_removable: false,
                    storage_root_id: Some(root.id.clone()),
                });
            }
        }
    }

    // Sort by mount point length (Root first, then others alphabetically)
    disks.sort_by(|a, b| {
        if a.mount_point == "/" {
            std::cmp::Ordering::Less
        } else if b.mount_point == "/" {
            std::cmp::Ordering::Greater
        } else {
            a.mount_point.cmp(&b.mount_point)
        }
    });

    disks
}

struct SubScanResult {
    item: DiskUsageItem,
    top_files: Vec<DiskUsageItem>,
}

fn push_top_file(list: &mut Vec<DiskUsageItem>, item: DiskUsageItem) {
    if list.len() < MAX_TOP_FILES {
        list.push(item);
        list.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    } else if item.size_bytes > list.last().map(|x| x.size_bytes).unwrap_or(0) {
        list.pop();
        list.push(item);
        list.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    }
}

fn merge_top_files(a: &mut Vec<DiskUsageItem>, b: Vec<DiskUsageItem>) {
    for item in b {
        push_top_file(a, item);
    }
}

fn scan_subdir(dir_path: &Path, name: String, mtime: u64) -> SubScanResult {
    let mut size_bytes = 0u64;
    let mut file_count = 0usize;
    let mut dir_count = 1usize; // count this folder
    let mut top_files = Vec::with_capacity(MAX_TOP_FILES);

    for entry in WalkDir::new(dir_path)
        .follow_links(false)
        .max_depth(40)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                let fsize = meta.len();
                size_bytes += fsize;
                file_count += 1;

                let sub_mtime = meta
                    .modified()
                    .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
                    .unwrap_or(0);

                push_top_file(
                    &mut top_files,
                    DiskUsageItem {
                        name: entry.file_name().to_string_lossy().to_string(),
                        path: entry.path().to_string_lossy().to_string(),
                        is_dir: false,
                        size_bytes: fsize,
                        formatted_size: format_size(fsize),
                        percentage: 0.0,
                        file_count: 1,
                        dir_count: 0,
                        last_modified: sub_mtime,
                    },
                );
            } else if meta.is_dir() && entry.path() != dir_path {
                dir_count += 1;
            }
        }
    }

    SubScanResult {
        item: DiskUsageItem {
            name,
            path: dir_path.to_string_lossy().to_string(),
            is_dir: true,
            size_bytes,
            formatted_size: format_size(size_bytes),
            percentage: 0.0,
            file_count,
            dir_count,
            last_modified: mtime,
        },
        top_files,
    }
}

impl DiskUsageEngine {
    pub fn analyze(path_str: &str) -> Result<DiskUsageReport, String> {
        let start = Instant::now();
        let root = Path::new(path_str);
        if !root.exists() {
            return Err(format!("Path does not exist: {}", path_str));
        }

        // Single file target
        if root.is_file() {
            let meta = fs::metadata(root).map_err(|e| format!("Failed to read metadata: {}", e))?;
            let size = meta.len();
            let mtime = meta
                .modified()
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
                .unwrap_or(0);
            let name = root.file_name().unwrap_or_default().to_string_lossy().to_string();

            let item = DiskUsageItem {
                name: name.clone(),
                path: root.to_string_lossy().to_string(),
                is_dir: false,
                size_bytes: size,
                formatted_size: format_size(size),
                percentage: 100.0,
                file_count: 1,
                dir_count: 0,
                last_modified: mtime,
            };

            return Ok(DiskUsageReport {
                root_path: path_str.to_string(),
                total_bytes: size,
                formatted_total: format_size(size),
                total_files: 1,
                total_dirs: 0,
                scan_duration_ms: start.elapsed().as_millis() as u64,
                items: vec![item.clone()],
                largest_files: vec![item],
            });
        }

        let read_dir = fs::read_dir(root).map_err(|e| format!("Failed to read directory: {}", e))?;
        let mut dir_entries: Vec<(PathBuf, String, u64)> = Vec::new();
        let mut file_entries: Vec<(PathBuf, String, u64, u64)> = Vec::new();

        for entry in read_dir.filter_map(|e| e.ok()) {
            let p = entry.path();
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            let is_dir = p.is_dir();

            let last_modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
                .unwrap_or(0);

            if is_dir {
                dir_entries.push((p, name, last_modified));
            } else if let Ok(meta) = entry.metadata() {
                file_entries.push((p, name, last_modified, meta.len()));
            }
        }

        // Parallel scan across immediate subdirectories using Rayon
        let dir_results: Vec<SubScanResult> = dir_entries
            .into_par_iter()
            .map(|(path, name, mtime)| scan_subdir(&path, name, mtime))
            .collect();

        let mut total_bytes = 0u64;
        let mut total_files = 0usize;
        let mut total_dirs = 0usize;
        let mut items = Vec::new();
        let mut largest_files: Vec<DiskUsageItem> = Vec::with_capacity(MAX_TOP_FILES);

        // Process file entries
        for (path, name, mtime, size) in file_entries {
            total_bytes += size;
            total_files += 1;

            let item = DiskUsageItem {
                name: name.clone(),
                path: path.to_string_lossy().to_string(),
                is_dir: false,
                size_bytes: size,
                formatted_size: format_size(size),
                percentage: 0.0,
                file_count: 1,
                dir_count: 0,
                last_modified: mtime,
            };

            push_top_file(&mut largest_files, item.clone());
            items.push(item);
        }

        // Process parallel directory results
        for res in dir_results {
            total_bytes += res.item.size_bytes;
            total_files += res.item.file_count;
            total_dirs += res.item.dir_count;

            merge_top_files(&mut largest_files, res.top_files);
            items.push(res.item);
        }

        // Calculate percentages
        for item in &mut items {
            item.percentage = if total_bytes > 0 {
                (item.size_bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
        }

        for item in &mut largest_files {
            item.percentage = if total_bytes > 0 {
                (item.size_bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
        }

        // Sort items by size descending
        items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        largest_files.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

        Ok(DiskUsageReport {
            root_path: path_str.to_string(),
            total_bytes,
            formatted_total: format_size(total_bytes),
            total_files,
            total_dirs,
            scan_duration_ms: start.elapsed().as_millis() as u64,
            items,
            largest_files,
        })
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_disk_usage_scan_and_report() {
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        // Create file in root
        let file1_path = base_path.join("file1.txt");
        let mut file1 = File::create(&file1_path).unwrap();
        file1.write_all(b"Hello, 12345!").unwrap(); // 13 bytes

        // Create subdir with files
        let sub_dir = base_path.join("sub_dir");
        fs::create_dir(&sub_dir).unwrap();

        let file2_path = sub_dir.join("file2.bin");
        let mut file2 = File::create(&file2_path).unwrap();
        file2.write_all(&vec![0u8; 100]).unwrap(); // 100 bytes

        let report = DiskUsageEngine::analyze(base_path.to_str().unwrap()).unwrap();

        assert_eq!(report.total_bytes, 113);
        assert_eq!(report.total_files, 2);
        assert_eq!(report.total_dirs, 1);
        assert_eq!(report.items.len(), 2);

        // Subdir should be 100 bytes
        let dir_item = report.items.iter().find(|i| i.name == "sub_dir").unwrap();
        assert_eq!(dir_item.size_bytes, 100);
        assert!(dir_item.is_dir);

        // File1 should be 13 bytes
        let file_item = report.items.iter().find(|i| i.name == "file1.txt").unwrap();
        assert_eq!(file_item.size_bytes, 13);
        assert!(!file_item.is_dir);

        // Largest files should have both files
        assert_eq!(report.largest_files.len(), 2);
        assert_eq!(report.largest_files[0].name, "file2.bin");
        assert_eq!(report.largest_files[0].size_bytes, 100);
    }

    #[test]
    fn test_disk_usage_single_file() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("single.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"CommanderDog").unwrap(); // 12 bytes

        let report = DiskUsageEngine::analyze(file_path.to_str().unwrap()).unwrap();
        assert_eq!(report.total_bytes, 12);
        assert_eq!(report.total_files, 1);
        assert_eq!(report.total_dirs, 0);
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].percentage, 100.0);
    }

    #[test]
    fn test_disk_usage_nonexistent() {
        let res = DiskUsageEngine::analyze("/path/that/definitely/does/not/exist_12345");
        assert!(res.is_err());
    }

    #[test]
    fn test_get_system_disks_enumeration() {
        let disks = get_system_disks(&[]);
        // Should find at least one disk / mount point on any running OS
        assert!(!disks.is_empty(), "Expected at least 1 disk/mount point");
        let first = &disks[0];
        assert!(!first.mount_point.is_empty());
        assert!(!first.formatted_total.is_empty());
        assert!(!first.formatted_used.is_empty());
        assert!(!first.formatted_available.is_empty());
        assert!(first.usage_percentage >= 0.0 && first.usage_percentage <= 100.0);

        // Test with configured storage roots
        let custom_roots = vec![crate::config::StorageRoot {
            id: "test_root".to_string(),
            name: "Test Root".to_string(),
            path: "/tmp".to_string(),
            read_only: true,
            allowed_roles: Vec::new(),
        }];
        let disks_with_roots = get_system_disks(&custom_roots);
        assert!(!disks_with_roots.is_empty());
    }
}

