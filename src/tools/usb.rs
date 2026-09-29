use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsbDevice {
    pub name: String,
    pub device_path: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub is_removable: bool,
    pub partitions: Vec<UsbPartition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsbPartition {
    pub name: String,
    pub device_path: String,
    pub label: Option<String>,
    pub fs_type: Option<String>,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub mount_point: Option<String>,
    pub is_mounted: bool,
    pub is_read_only: bool,
    pub available_bytes: Option<u64>,
    pub formatted_available: Option<String>,
    pub used_bytes: Option<u64>,
    pub formatted_used: Option<String>,
    pub usage_percentage: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct MountUsbRequest {
    pub device_path: String,
    pub mount_point: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UnmountUsbRequest {
    pub device_path: Option<String>,
    pub mount_point: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EjectUsbRequest {
    pub device_path: String,
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn is_valid_block_device_path(path: &str) -> bool {
    let trimmed = path.trim();
    if !trimmed.starts_with("/dev/") {
        return false;
    }
    let sub = &trimmed[5..];
    if sub.is_empty()
        || sub.contains("..")
        || sub.contains('/')
        || sub.contains(' ')
        || sub.contains(';')
        || sub.contains('&')
        || sub.contains('|')
        || sub.contains('`')
        || sub.contains('$')
    {
        return false;
    }
    sub.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(unix)]
pub fn query_mounts_map() -> std::collections::HashMap<String, (String, String, bool)> {
    let mut map = std::collections::HashMap::new();
    if let Ok(content) = fs::read_to_string("/proc/mounts") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let device = parts[0].to_string();
                let mount_point = parts[1].to_string();
                let fs_type = parts[2].to_string();
                let options = parts[3];
                let is_ro = options.split(',').any(|o| o == "ro");
                map.insert(device, (mount_point, fs_type, is_ro));
            }
        }
    }
    map
}

#[cfg(unix)]
pub fn is_device_removable(device_path: &str) -> bool {
    let dev_name = Path::new(device_path).file_name().unwrap_or_default().to_string_lossy();
    // Trim partition numbers (e.g. sdb1 -> sdb, nvme0n1p1 -> nvme0n1)
    let parent_name = dev_name
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches('p');

    let sys_removable = format!("/sys/block/{}/removable", parent_name);
    if let Ok(content) = fs::read_to_string(&sys_removable) {
        if content.trim() == "1" {
            return true;
        }
    }

    // Also check direct sys path
    let direct_removable = format!("/sys/block/{}/removable", dev_name);
    if let Ok(content) = fs::read_to_string(&direct_removable) {
        if content.trim() == "1" {
            return true;
        }
    }

    false
}

#[cfg(unix)]
#[derive(Deserialize)]
struct LsblkOutput {
    #[serde(default)]
    blockdevices: Vec<LsblkDevice>,
}

#[cfg(unix)]
#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    path: Option<String>,
    label: Option<String>,
    fstype: Option<String>,
    size: Option<serde_json::Value>,
    mountpoint: Option<String>,
    mountpoints: Option<Vec<Option<String>>>,
    model: Option<String>,
    vendor: Option<String>,
    rm: Option<serde_json::Value>,
    hotplug: Option<serde_json::Value>,
    tran: Option<String>,
    ro: Option<serde_json::Value>,
    #[serde(rename = "type")]
    device_type: Option<String>,
    children: Option<Vec<LsblkDevice>>,
}

#[cfg(unix)]
fn parse_lsblk_bool(val: Option<&serde_json::Value>) -> bool {
    match val {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "1" || s.eq_ignore_ascii_case("true"),
        Some(serde_json::Value::Number(n)) => n.as_u64().map(|v| v == 1).unwrap_or(false),
        _ => false,
    }
}

#[cfg(unix)]
fn parse_lsblk_size(val: Option<&serde_json::Value>) -> u64 {
    match val {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

#[cfg(unix)]
fn list_usb_via_lsblk() -> Result<Vec<UsbDevice>, Box<dyn std::error::Error + Send + Sync>> {
    let output = Command::new("lsblk")
        .args([
            "-J",
            "-b",
            "-o",
            "NAME,PATH,LABEL,FSTYPE,SIZE,MOUNTPOINT,MOUNTPOINTS,MODEL,VENDOR,RM,HOTPLUG,TRAN,RO,TYPE",
        ])
        .output()?;

    if !output.status.success() {
        return Err("lsblk execution failed".into());
    }

    let parsed: LsblkOutput = serde_json::from_slice(&output.stdout)?;
    let mut usb_devices = Vec::new();

    for dev in parsed.blockdevices {
        let is_usb_tran = dev.tran.as_deref().map(|t| t.eq_ignore_ascii_case("usb")).unwrap_or(false);
        let is_rm = parse_lsblk_bool(dev.rm.as_ref());
        let is_hotplug = parse_lsblk_bool(dev.hotplug.as_ref());

        // Also check if any partition is mounted under /media or /run/media
        let mut has_media_mount = false;
        let mut check_mount = |m: Option<&str>| {
            if let Some(mp) = m {
                if mp.starts_with("/media") || mp.starts_with("/run/media") {
                    has_media_mount = true;
                }
            }
        };

        check_mount(dev.mountpoint.as_deref());
        if let Some(ref mps) = dev.mountpoints {
            for mp in mps {
                check_mount(mp.as_deref());
            }
        }
        if let Some(ref children) = dev.children {
            for child in children {
                check_mount(child.mountpoint.as_deref());
                if let Some(ref cmps) = child.mountpoints {
                    for cmp in cmps {
                        check_mount(cmp.as_deref());
                    }
                }
            }
        }

        let is_removable_drive = is_usb_tran || is_rm || is_hotplug || has_media_mount;
        if !is_removable_drive {
            continue;
        }

        let dev_path = dev.path.clone().unwrap_or_else(|| format!("/dev/{}", dev.name));
        let size_bytes = parse_lsblk_size(dev.size.as_ref());

        let mut partitions = Vec::new();
        if let Some(children) = dev.children {
            for child in children {
                let part_path = child.path.clone().unwrap_or_else(|| format!("/dev/{}", child.name));
                let part_size = parse_lsblk_size(child.size.as_ref());
                let mount_point = child.mountpoint.clone().or_else(|| {
                    child.mountpoints.as_ref().and_then(|mps| mps.first().and_then(|m| m.clone()))
                });

                let is_mounted = mount_point.is_some();
                let is_ro = parse_lsblk_bool(child.ro.as_ref());

                let mut avail = None;
                let mut used = None;
                let mut pct = None;

                if let Some(ref mnt) = mount_point {
                    if let Some((tot, u, a, _)) = crate::tools::disk_usage::query_statvfs(mnt) {
                        avail = Some(a);
                        used = Some(u);
                        if tot > 0 {
                            pct = Some(((u as f64 / tot as f64) * 100.0 * 10.0).round() / 10.0);
                        }
                    }
                }

                partitions.push(UsbPartition {
                    name: child.name,
                    device_path: part_path,
                    label: child.label,
                    fs_type: child.fstype,
                    size_bytes: part_size,
                    formatted_size: format_bytes(part_size),
                    mount_point,
                    is_mounted,
                    is_read_only: is_ro,
                    available_bytes: avail,
                    formatted_available: avail.map(format_bytes),
                    used_bytes: used,
                    formatted_used: used.map(format_bytes),
                    usage_percentage: pct,
                });
            }
        } else if dev.device_type.as_deref() == Some("part") || dev.fstype.is_some() || dev.mountpoint.is_some() {
            // Standalone unpartitioned drive (e.g. superfloppy FAT format)
            let mount_point = dev.mountpoint.clone();
            let is_mounted = mount_point.is_some();
            let mut avail = None;
            let mut used = None;
            let mut pct = None;

            if let Some(ref mnt) = mount_point {
                if let Some((tot, u, a, _)) = crate::tools::disk_usage::query_statvfs(mnt) {
                    avail = Some(a);
                    used = Some(u);
                    if tot > 0 {
                        pct = Some(((u as f64 / tot as f64) * 100.0 * 10.0).round() / 10.0);
                    }
                }
            }

            partitions.push(UsbPartition {
                name: dev.name.clone(),
                device_path: dev_path.clone(),
                label: dev.label.clone(),
                fs_type: dev.fstype.clone(),
                size_bytes,
                formatted_size: format_bytes(size_bytes),
                mount_point,
                is_mounted,
                is_read_only: parse_lsblk_bool(dev.ro.as_ref()),
                available_bytes: avail,
                formatted_available: avail.map(format_bytes),
                used_bytes: used,
                formatted_used: used.map(format_bytes),
                usage_percentage: pct,
            });
        }

        usb_devices.push(UsbDevice {
            name: dev.name,
            device_path: dev_path,
            vendor: dev.vendor.filter(|v| !v.trim().is_empty()),
            model: dev.model.filter(|m| !m.trim().is_empty()),
            size_bytes,
            formatted_size: format_bytes(size_bytes),
            is_removable: true,
            partitions,
        });
    }

    Ok(usb_devices)
}

#[cfg(unix)]
fn list_usb_via_sysfs() -> Vec<UsbDevice> {
    let mut usb_devices = Vec::new();
    let mounts_map = query_mounts_map();

    let block_dir = Path::new("/sys/block");
    if let Ok(entries) = fs::read_dir(block_dir) {
        for entry in entries.flatten() {
            let dev_name = entry.file_name().to_string_lossy().to_string();
            if dev_name.starts_with("loop") || dev_name.starts_with("ram") || dev_name.starts_with("zram") {
                continue;
            }

            let dev_sys_path = entry.path();
            let is_removable = fs::read_to_string(dev_sys_path.join("removable"))
                .map(|s| s.trim() == "1")
                .unwrap_or(false);

            let vendor = fs::read_to_string(dev_sys_path.join("device/vendor"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let model = fs::read_to_string(dev_sys_path.join("device/model"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let size_sectors = fs::read_to_string(dev_sys_path.join("size"))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let size_bytes = size_sectors * 512;

            let dev_node = format!("/dev/{}", dev_name);
            let mut partitions = Vec::new();

            // Find partitions under /sys/block/<dev>/
            if let Ok(sub_entries) = fs::read_dir(&dev_sys_path) {
                for sub in sub_entries.flatten() {
                    let sub_name = sub.file_name().to_string_lossy().to_string();
                    if sub.path().join("partition").exists() || (sub_name.starts_with(&dev_name) && sub_name != dev_name) {
                        let part_node = format!("/dev/{}", sub_name);
                        let part_sectors = fs::read_to_string(sub.path().join("size"))
                            .ok()
                            .and_then(|s| s.trim().parse::<u64>().ok())
                            .unwrap_or(0);
                        let part_size = part_sectors * 512;

                        let mut mount_point = None;
                        let mut fs_type = None;
                        let mut is_ro = false;

                        if let Some((mnt, fst, ro)) = mounts_map.get(&part_node) {
                            mount_point = Some(mnt.clone());
                            fs_type = Some(fst.clone());
                            is_ro = *ro;
                        }

                        let mut avail = None;
                        let mut used = None;
                        let mut pct = None;

                        if let Some(ref mnt) = mount_point {
                            if let Some((tot, u, a, _)) = crate::tools::disk_usage::query_statvfs(mnt) {
                                avail = Some(a);
                                used = Some(u);
                                if tot > 0 {
                                    pct = Some(((u as f64 / tot as f64) * 100.0 * 10.0).round() / 10.0);
                                }
                            }
                        }

                        partitions.push(UsbPartition {
                            name: sub_name,
                            device_path: part_node,
                            label: None,
                            fs_type,
                            size_bytes: part_size,
                            formatted_size: format_bytes(part_size),
                            mount_point: mount_point.clone(),
                            is_mounted: mount_point.is_some(),
                            is_read_only: is_ro,
                            available_bytes: avail,
                            formatted_available: avail.map(format_bytes),
                            used_bytes: used,
                            formatted_used: used.map(format_bytes),
                            usage_percentage: pct,
                        });
                    }
                }
            }

            let has_media_mount = partitions.iter().any(|p| {
                p.mount_point.as_deref().map(|m| m.starts_with("/media") || m.starts_with("/run/media")).unwrap_or(false)
            });

            if is_removable || has_media_mount {
                usb_devices.push(UsbDevice {
                    name: dev_name,
                    device_path: dev_node,
                    vendor,
                    model,
                    size_bytes,
                    formatted_size: format_bytes(size_bytes),
                    is_removable: true,
                    partitions,
                });
            }
        }
    }

    usb_devices
}

#[cfg(unix)]
pub fn list_usb_devices() -> Vec<UsbDevice> {
    if let Ok(devices) = list_usb_via_lsblk() {
        if !devices.is_empty() {
            return devices;
        }
    }
    list_usb_via_sysfs()
}

#[cfg(windows)]
pub fn list_usb_devices() -> Vec<UsbDevice> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    extern "system" {
        fn GetDriveTypeW(lpRootPathName: *const u16) -> u32;
        fn GetVolumeInformationW(
            lpRootPathName: *const u16,
            lpVolumeNameBuffer: *mut u16,
            nVolumeNameSize: u32,
            lpVolumeSerialNumber: *mut u32,
            lpMaximumComponentLength: *mut u32,
            lpFileSystemFlags: *mut u32,
            lpFileSystemNameBuffer: *mut u16,
            nFileSystemNameSize: u32,
        ) -> i32;
    }

    let mut usb_devices = Vec::new();

    for b in b'A'..=b'Z' {
        let drive_root = format!("{}:\\", b as char);
        let wide: Vec<u16> = OsStr::new(&drive_root).encode_wide().chain(Some(0)).collect();
        let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };

        // 2 = DRIVE_REMOVABLE, 5 = DRIVE_CDROM
        if drive_type == 2 || drive_type == 5 {
            let mut vol_name_buf = vec![0u16; 260];
            let mut fs_name_buf = vec![0u16; 260];

            let ok = unsafe {
                GetVolumeInformationW(
                    wide.as_ptr(),
                    vol_name_buf.as_mut_ptr(),
                    260,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    fs_name_buf.as_mut_ptr(),
                    260,
                )
            };

            let label = if ok != 0 {
                let len = vol_name_buf.iter().position(|&c| c == 0).unwrap_or(0);
                String::from_utf16(&vol_name_buf[..len]).ok().filter(|s| !s.is_empty())
            } else {
                None
            };

            let fs_type = if ok != 0 {
                let len = fs_name_buf.iter().position(|&c| c == 0).unwrap_or(0);
                String::from_utf16(&fs_name_buf[..len]).ok().filter(|s| !s.is_empty())
            } else {
                None
            };

            let (total, used, avail) = if let Some((tot, u, a)) = crate::tools::disk_usage::query_windows_disk(&drive_root) {
                (tot, Some(u), Some(a))
            } else {
                (0, None, None)
            };

            let pct = if let (Some(u), tot) = (used, total) {
                if tot > 0 {
                    Some(((u as f64 / tot as f64) * 100.0 * 10.0).round() / 10.0)
                } else {
                    None
                }
            } else {
                None
            };

            let part = UsbPartition {
                name: format!("{}:", b as char),
                device_path: format!("\\\\.\\{}:", b as char),
                label: label.clone(),
                fs_type,
                size_bytes: total,
                formatted_size: format_bytes(total),
                mount_point: Some(drive_root.clone()),
                is_mounted: true,
                is_read_only: drive_type == 5,
                available_bytes: avail,
                formatted_available: avail.map(format_bytes),
                used_bytes: used,
                formatted_used: used.map(format_bytes),
                usage_percentage: pct,
            };

            usb_devices.push(UsbDevice {
                name: format!("Removable Drive ({}:)", b as char),
                device_path: format!("\\\\.\\{}:", b as char),
                vendor: None,
                model: label,
                size_bytes: total,
                formatted_size: format_bytes(total),
                is_removable: true,
                partitions: vec![part],
            });
        }
    }

    usb_devices
}

#[cfg(unix)]
pub fn mount_usb_partition(device_path: &str, mount_point: Option<&str>) -> Result<String, String> {
    if !is_valid_block_device_path(device_path) {
        return Err(format!("Invalid block device path: '{}'", device_path));
    }

    // 1. Try udisksctl mount
    let udisks_out = Command::new("udisksctl")
        .args(["mount", "-b", device_path])
        .output();

    if let Ok(out) = udisks_out {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Some(pos) = stdout.find(" at ") {
                let mnt = stdout[pos + 4..].trim().trim_end_matches('.');
                return Ok(mnt.to_string());
            }
            return Ok("Mounted successfully".to_string());
        }
    }

    // 2. Fallback to mount command
    let target = if let Some(m) = mount_point.filter(|s| !s.trim().is_empty()) {
        m.to_string()
    } else {
        let dev_name = Path::new(device_path).file_name().unwrap_or_default().to_string_lossy();
        let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        format!("/media/{}/{}", user, dev_name)
    };

    let _ = fs::create_dir_all(&target);
    let mount_out = Command::new("mount")
        .args([device_path, &target])
        .output()
        .map_err(|e| format!("Failed to execute mount command: {}", e))?;

    if mount_out.status.success() {
        Ok(target)
    } else {
        let err_msg = String::from_utf8_lossy(&mount_out.stderr);
        Err(format!("Mount failed: {}", err_msg.trim()))
    }
}

#[cfg(windows)]
pub fn mount_usb_partition(_device_path: &str, _mount_point: Option<&str>) -> Result<String, String> {
    Err("Mounting raw block devices is not supported on Windows. Windows manages drive letters automatically.".to_string())
}

#[cfg(unix)]
pub fn unmount_usb_partition(device_or_mount: &str) -> Result<(), String> {
    let trimmed = device_or_mount.trim();
    if trimmed.starts_with("/dev/") && !is_valid_block_device_path(trimmed) {
        return Err(format!("Invalid block device path: '{}'", trimmed));
    }

    // 1. Try udisksctl unmount
    let udisks_res = if trimmed.starts_with("/dev/") {
        Command::new("udisksctl").args(["unmount", "-b", trimmed]).output()
    } else {
        Command::new("udisksctl").args(["unmount", "-p", trimmed]).output()
    };

    if let Ok(out) = udisks_res {
        if out.status.success() {
            return Ok(());
        }
    }

    // 2. Fallback to umount
    let umount_out = Command::new("umount")
        .arg(trimmed)
        .output()
        .map_err(|e| format!("Failed to execute umount command: {}", e))?;

    if umount_out.status.success() {
        Ok(())
    } else {
        let err_msg = String::from_utf8_lossy(&umount_out.stderr);
        Err(format!("Unmount failed: {}", err_msg.trim()))
    }
}

#[cfg(windows)]
pub fn unmount_usb_partition(_device_or_mount: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
pub fn eject_usb_device(device_path: &str) -> Result<(), String> {
    if !is_valid_block_device_path(device_path) {
        return Err(format!("Invalid block device path: '{}'", device_path));
    }

    // Flush dirty filesystem cache
    let _ = Command::new("sync").output();

    // 1. Try udisksctl power-off
    if let Ok(out) = Command::new("udisksctl").args(["power-off", "-b", device_path]).output() {
        if out.status.success() {
            return Ok(());
        }
    }

    // 2. Try eject tool
    if let Ok(out) = Command::new("eject").arg(device_path).output() {
        if out.status.success() {
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(windows)]
pub fn eject_usb_device(_device_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(format_bytes(16 * 1024 * 1024 * 1024), "16.0 GB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024 * 1024), "2.0 TB");
    }

    #[test]
    fn test_is_valid_block_device_path() {
        assert!(is_valid_block_device_path("/dev/sdb"));
        assert!(is_valid_block_device_path("/dev/sdb1"));
        assert!(is_valid_block_device_path("/dev/nvme0n1p1"));
        assert!(is_valid_block_device_path("/dev/mmcblk0p1"));
        assert!(is_valid_block_device_path("/dev/vda2"));

        // Reject dangerous/invalid inputs
        assert!(!is_valid_block_device_path("/dev/sdb1; rm -rf /"));
        assert!(!is_valid_block_device_path("/dev/../etc/passwd"));
        assert!(!is_valid_block_device_path("sdb1"));
        assert!(!is_valid_block_device_path("/dev/sdb1 && whoami"));
        assert!(!is_valid_block_device_path("/dev/sdb1 | echo"));
        assert!(!is_valid_block_device_path(""));
    }

    #[test]
    fn test_list_usb_devices_does_not_panic() {
        let devices = list_usb_devices();
        println!("Detected {} USB/removable devices", devices.len());
        for d in devices {
            println!("Device: {} ({} - {})", d.name, d.device_path, d.formatted_size);
            for p in d.partitions {
                println!("  Partition: {} -> {:?}", p.name, p.mount_point);
            }
        }
    }
}
