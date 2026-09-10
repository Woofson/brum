use super::{DirectoryListing, FileEntry, VfsResult};
use crate::vfs::local::LocalFs;
use std::process::Command;
use std::path::PathBuf;
use std::fs;

pub struct NfsClient;

#[derive(Debug, Clone)]
pub struct NfsParams {
    pub host: String,
    pub port: u16,
    pub export_path: String,
    pub subpath: String,
    pub version: Option<String>,
}

impl NfsClient {
    /// Parse an nfs:// URI
    /// Formats:
    /// - nfs://host/export_path
    /// - nfs://host:2049/export_path/subpath
    pub fn parse_uri(uri: &str) -> VfsResult<NfsParams> {
        let clean = uri.strip_prefix("nfs://").ok_or_else(|| "Invalid NFS URI (must start with nfs://)".to_string())?;
        
        let parts: Vec<&str> = clean.splitn(2, '/').collect();
        let host_port = parts[0];
        let path_part = if parts.len() > 1 { format!("/{}", parts[1]) } else { "/".to_string() };

        let (host, port) = if let Some(colon) = host_port.find(':') {
            let h = &host_port[..colon];
            let p = host_port[colon + 1..].parse::<u16>().unwrap_or(2049);
            (h.to_string(), p)
        } else {
            (host_port.to_string(), 2049)
        };

        // Split export path vs subpath
        let (export_path, subpath) = if path_part == "/" {
            ("/".to_string(), "".to_string())
        } else {
            (path_part.clone(), "".to_string())
        };

        Ok(NfsParams {
            host,
            port,
            export_path,
            subpath,
            version: None,
        })
    }

    /// Check if showmount or mount is available
    pub fn is_available() -> bool {
        Command::new("showmount")
            .arg("-v")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Get local mount point for an NFS share
    pub fn get_mount_dir(params: &NfsParams) -> PathBuf {
        let sanitized = format!("{}_{}", params.host, params.export_path.replace('/', "_").trim_matches('_'));
        PathBuf::from("/run/brum/mounts/nfs").join(sanitized)
    }

    /// Ensure the NFS share is mounted locally
    pub fn ensure_mounted(params: &NfsParams) -> VfsResult<PathBuf> {
        let mount_dir = Self::get_mount_dir(params);
        if mount_dir.exists() {
            // Check if already mounted
            if let Ok(mounts) = fs::read_to_string("/proc/mounts") {
                let target_str = mount_dir.to_string_lossy();
                if mounts.lines().any(|l| l.contains(&*target_str)) {
                    return Ok(mount_dir);
                }
            }
        }

        fs::create_dir_all(&mount_dir).map_err(|e| format!("Failed to create mount directory: {}", e))?;

        let nfs_source = format!("{}:{}", params.host, params.export_path);
        let mut cmd = Command::new("mount");
        cmd.arg("-t").arg("nfs");
        cmd.arg("-o").arg("ro,soft,timeo=50");
        cmd.arg(&nfs_source);
        cmd.arg(&mount_dir);

        let output = cmd.output().map_err(|e| format!("Failed to mount NFS export: {}", e))?;
        if output.status.success() {
            Ok(mount_dir)
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(format!("Failed to mount NFS '{}': {}", nfs_source, if err.trim().is_empty() { "Check server export permissions or install nfs-common (sudo apt install nfs-common)".to_string() } else { err.to_string() }))
        }
    }

    /// List directory contents over NFS
    pub fn list_dir(params: &NfsParams, show_hidden: bool) -> VfsResult<DirectoryListing> {
        // If export path is root "/", try to discover exports via showmount -e host
        if params.export_path == "/" || params.export_path.is_empty() {
            let mut cmd = Command::new("showmount");
            cmd.arg("-e").arg(&params.host);
            if let Ok(output) = cmd.output() {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let mut entries = Vec::new();
                    for line in stdout.lines().skip(1) {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if let Some(&export) = parts.first() {
                            let clean_name = export.trim_start_matches('/');
                            entries.push(FileEntry {
                                name: clean_name.to_string(),
                                path: format!("nfs://{}{}", params.host, export),
                                is_dir: true,
                                is_symlink: false,
                                is_empty: None,
                                size: 0,
                                modified: None,
                                permissions: "drwxr-xr-x".to_string(),
                                mode_octal: "0755".to_string(),
                                owner: "nfs".to_string(),
                                group: "nfs".to_string(),
                                uid: 1000,
                                gid: 1000,
                                mime_type: None,
                                is_archive: false,
                            });
                        }
                    }

                    return Ok(DirectoryListing {
                        current_path: format!("nfs://{}/", params.host),
                        parent_path: None,
                        total_files: 0,
                        total_dirs: entries.len(),
                        total_size: 0,
                        entries,
                        protocol: "nfs".to_string(),
                        is_truncated: None,
                        max_limit: None,
                    });
                }
            }
        }

        // Try mounting and listing
        let mount_dir = Self::ensure_mounted(params)?;
        let target_local_dir = if params.subpath.is_empty() {
            mount_dir.clone()
        } else {
            mount_dir.join(&params.subpath)
        };

        let mut listing = LocalFs::list_dir(&target_local_dir.to_string_lossy(), show_hidden)
            .map_err(|e| format!("Failed to list NFS directory: {}", e))?;

        // Map local paths back to nfs:// URLs
        let mount_str = mount_dir.to_string_lossy();
        for entry in &mut listing.entries {
            let rel = entry.path.strip_prefix(&*mount_str).unwrap_or(&entry.path).trim_start_matches('/');
            entry.path = if rel.is_empty() {
                format!("nfs://{}{}", params.host, params.export_path)
            } else {
                format!("nfs://{}{}/{}", params.host, params.export_path.trim_end_matches('/'), rel)
            };
        }

        let sub_suffix = if params.subpath.is_empty() {
            "".to_string()
        } else {
            format!("/{}", params.subpath.trim_start_matches('/'))
        };
        listing.current_path = format!("nfs://{}{}{}", params.host, params.export_path, sub_suffix);
        listing.protocol = "nfs".to_string();

        Ok(listing)
    }

    /// Test NFS connection
    pub fn test_connection(params: &NfsParams) -> VfsResult<String> {
        let mut cmd = Command::new("showmount");
        cmd.arg("-e").arg(&params.host);
        match cmd.output() {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let exports: Vec<&str> = stdout.lines().skip(1).filter_map(|l| l.split_whitespace().next()).collect();
                Ok(format!("✓ Successfully discovered {} NFS export(s) on '{}':\n{}", exports.len(), params.host, exports.join(", ")))
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("NFS showmount failed: {}", if err.trim().is_empty() { "Check NFS server RPC status (port 111 / 2049)".to_string() } else { err.to_string() }))
            }
            Err(_) => {
                Err("NFS client utility ('showmount' / 'nfs-common') not found.\nInstall with: sudo apt install nfs-common (Debian/Ubuntu)".to_string())
            }
        }
    }
}
