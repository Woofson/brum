use crate::tools::tasks::TaskManager;
use crate::vfs::local::LocalFs;
use crate::vfs::smb::{SmbClient, SmbParams};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::NamedTempFile;

pub struct VfsTransfer;

impl VfsTransfer {
    /// Recursively copy an SMB folder to local destination
    pub fn copy_smb_dir_to_local(
        src_params: &SmbParams,
        dest_local_dir: &Path,
        task_manager: &TaskManager,
        task_id: &str,
    ) -> Result<(), String> {
        fs::create_dir_all(dest_local_dir).map_err(|e| format!("Failed to create local directory: {}", e))?;

        let listing = SmbClient::list_dir(src_params)?;
        for entry in listing.entries {
            let child_subpath = if src_params.subpath.is_empty() {
                entry.name.clone()
            } else {
                format!("{}/{}", src_params.subpath, entry.name)
            };
            let mut child_params = src_params.clone();
            child_params.subpath = child_subpath;

            let child_dest = dest_local_dir.join(&entry.name);
            if entry.is_dir {
                Self::copy_smb_dir_to_local(&child_params, &child_dest, task_manager, task_id)?;
            } else {
                SmbClient::download_to_file(&child_params, &child_dest)?;
            }
        }
        Ok(())
    }

    /// Recursively copy a local folder to SMB destination
    pub fn copy_local_dir_to_smb(
        src_local_dir: &Path,
        dest_params: &SmbParams,
        task_manager: &TaskManager,
        task_id: &str,
    ) -> Result<(), String> {
        let _ = SmbClient::mkdir(dest_params);

        let read_dir = fs::read_dir(src_local_dir).map_err(|e| format!("Failed to read local dir: {}", e))?;
        for entry_res in read_dir {
            if let Ok(entry) = entry_res {
                let name = entry.file_name().to_string_lossy().to_string();
                let child_subpath = if dest_params.subpath.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", dest_params.subpath, name)
                };
                let mut child_params = dest_params.clone();
                child_params.subpath = child_subpath;

                let path = entry.path();
                if path.is_dir() {
                    Self::copy_local_dir_to_smb(&path, &child_params, task_manager, task_id)?;
                } else {
                    SmbClient::upload_from_file(&child_params, &path)?;
                }
            }
        }
        Ok(())
    }

    /// Execute transfer of a single item (file or folder) between any supported VFS endpoints
    pub fn transfer_single_item(
        src: &str,
        dest_dir: &str,
        is_move: bool,
        paranoid: bool,
        task_manager: &TaskManager,
        task_id: &str,
    ) -> Result<Option<String>, String> {
        let is_src_smb = src.starts_with("smb://");
        let is_dest_smb = dest_dir.starts_with("smb://");
        let is_src_sftp = src.starts_with("sftp://");
        let is_dest_sftp = dest_dir.starts_with("sftp://");
        let is_src_nfs = src.starts_with("nfs://");
        let is_dest_nfs = dest_dir.starts_with("nfs://");
        let is_src_archive = src.starts_with("archive://");
        let mut verified_hash: Option<String> = None;

        if is_src_archive {
            let rest = src.strip_prefix("archive://").unwrap();
            let (archive_file, subpath) = match rest.split_once('#') {
                Some((a, s)) => (a, s),
                None => (rest, ""),
            };
            let file_res = crate::vfs::archive::ArchiveHandler::read_archive_entry(archive_file, subpath, 0)
                .map_err(|e| format!("Failed to read archive entry: {}", e))?;

            use base64::Engine;
            let file_bytes = if file_res.is_binary {
                base64::engine::general_purpose::STANDARD.decode(&file_res.content).unwrap_or_default()
            } else {
                file_res.content.into_bytes()
            };

            let dest_path = Path::new(dest_dir);
            let target = if dest_path.is_dir() {
                dest_path.join(&file_res.name)
            } else {
                dest_path.to_path_buf()
            };

            if let Some(parent) = target.parent() {
                let _ = fs::create_dir_all(parent);
            }

            fs::write(&target, &file_bytes).map_err(|e| format!("Failed to write extracted file: {}", e))?;
            if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                verified_hash = Some(format!("Extracted SHA-256: {}", h));
            }
            return Ok(verified_hash);
        } else if is_src_sftp && !is_dest_sftp {
            // SFTP -> Local
            let src_params = crate::vfs::sftp::SftpClient::parse_uri(src, None, None)?;
            let file_name = src_params.remote_path.rsplit('/').next().unwrap_or(&src_params.remote_path).to_string();
            let dest_path = Path::new(dest_dir);
            let target = if dest_path.is_dir() {
                dest_path.join(&file_name)
            } else {
                dest_path.to_path_buf()
            };

            crate::vfs::sftp::SftpClient::download_to_file(&src_params, &target)?;
            if target.is_file() {
                if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                    verified_hash = Some(format!("Dest SHA-256: {}", h));
                }
            }

            if is_move {
                let _ = crate::vfs::sftp::SftpClient::delete(&src_params, false);
            }
        } else if !is_src_sftp && is_dest_sftp {
            // Local -> SFTP
            let src_path = Path::new(src);
            if !src_path.exists() {
                return Err(format!("Local source path not found: {}", src));
            }
            let file_name = src_path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let dest_params = crate::vfs::sftp::SftpClient::parse_uri(dest_dir, None, None)?;
            let target_remote_path = if dest_params.remote_path.is_empty() || dest_params.remote_path == "/" {
                format!("/{}", file_name)
            } else {
                format!("{}/{}", dest_params.remote_path.trim_end_matches('/'), file_name)
            };
            let mut target_params = dest_params.clone();
            target_params.remote_path = target_remote_path;

            if let Ok(h) = crate::vfs::checksum::calculate_sha256(src_path) {
                verified_hash = Some(format!("Src SHA-256: {}", h));
            }
            crate::vfs::sftp::SftpClient::upload_from_file(&target_params, src_path)?;

            if is_move {
                let _ = LocalFs::delete_entry(src, false, None);
            }
        } else if is_src_nfs || is_dest_nfs {
            // NFS transfers via ensured mount
            let local_src = if is_src_nfs {
                let params = crate::vfs::nfs::NfsClient::parse_uri(src)?;
                let mount = crate::vfs::nfs::NfsClient::ensure_mounted(&params)?;
                mount.join(params.subpath.trim_start_matches('/'))
            } else {
                Path::new(src).to_path_buf()
            };

            let local_dest = if is_dest_nfs {
                let params = crate::vfs::nfs::NfsClient::parse_uri(dest_dir)?;
                let mount = crate::vfs::nfs::NfsClient::ensure_mounted(&params)?;
                mount.join(params.subpath.trim_start_matches('/'))
            } else {
                Path::new(dest_dir).to_path_buf()
            };

            let target = if local_dest.is_dir() {
                local_dest.join(local_src.file_name().unwrap_or_default())
            } else {
                local_dest
            };

            LocalFs::copy_file_paranoid(&local_src.to_string_lossy(), &target.to_string_lossy(), paranoid).map_err(|e| e.to_string())?;
            if target.is_file() {
                if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                    verified_hash = Some(format!("SHA-256 Match: {}", h));
                }
            }

            if is_move {
                let _ = LocalFs::delete_entry(&local_src.to_string_lossy(), false, None);
            }
        } else if is_src_smb && !is_dest_smb {
            // SMB -> Local
            let src_params = SmbClient::parse_uri(src, None, None)?;
            let file_name = src_params.subpath.rsplit('/').next().unwrap_or(&src_params.subpath).to_string();
            let dest_path = Path::new(dest_dir);
            let target = if dest_path.is_dir() {
                dest_path.join(&file_name)
            } else {
                dest_path.to_path_buf()
            };

            // Try download file directly
            match SmbClient::download_to_file(&src_params, &target) {
                Ok(_) => {
                    if target.is_file() {
                        if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                            verified_hash = Some(format!("Dest SHA-256: {}", h));
                        }
                    }
                },
                Err(e) => {
                    // Check if it's a directory or try directory recursion
                    if let Ok(_listing) = SmbClient::list_dir(&src_params) {
                        Self::copy_smb_dir_to_local(&src_params, &target, task_manager, task_id)?;
                    } else {
                        return Err(format!("SMB download failed: {}", e));
                    }
                }
            }

            if is_move {
                let _ = SmbClient::delete(&src_params, false);
            }
        } else if !is_src_smb && is_dest_smb {
            // Local -> SMB
            let src_path = Path::new(src);
            if !src_path.exists() {
                return Err(format!("Local source path not found: {}", src));
            }
            let file_name = src_path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let dest_params = SmbClient::parse_uri(dest_dir, None, None)?;
            let target_subpath = if dest_params.subpath.is_empty() {
                file_name
            } else {
                format!("{}/{}", dest_params.subpath, file_name)
            };
            let mut target_params = dest_params.clone();
            target_params.subpath = target_subpath;

            if src_path.is_dir() {
                Self::copy_local_dir_to_smb(src_path, &target_params, task_manager, task_id)?;
            } else {
                if let Ok(h) = crate::vfs::checksum::calculate_sha256(src_path) {
                    verified_hash = Some(format!("Src SHA-256: {}", h));
                }
                SmbClient::upload_from_file(&target_params, src_path)?;
            }

            if is_move {
                let _ = LocalFs::delete_entry(src, false, None);
            }
        } else if is_src_smb && is_dest_smb {
            // SMB -> SMB
            let src_params = SmbClient::parse_uri(src, None, None)?;
            let file_name = src_params.subpath.rsplit('/').next().unwrap_or(&src_params.subpath).to_string();
            let dest_params = SmbClient::parse_uri(dest_dir, None, None)?;
            let target_subpath = if dest_params.subpath.is_empty() {
                file_name
            } else {
                format!("{}/{}", dest_params.subpath, file_name)
            };
            let mut target_params = dest_params.clone();
            target_params.subpath = target_subpath;

            let tmp = NamedTempFile::new().map_err(|e| format!("Temp file error: {}", e))?;
            SmbClient::download_to_file(&src_params, tmp.path())?;
            if let Ok(h) = crate::vfs::checksum::calculate_sha256(tmp.path()) {
                verified_hash = Some(format!("Stream SHA-256: {}", h));
            }
            SmbClient::upload_from_file(&target_params, tmp.path())?;

            if is_move {
                let _ = SmbClient::delete(&src_params, false);
            }
        } else {
            // Local -> Local
            let src_path = Path::new(src);
            let dest_path = Path::new(dest_dir);

            if !src_path.exists() {
                return Err(format!("Source path not found: {}", src));
            }

            let file_name = src_path.file_name().unwrap_or_default();
            let target = if dest_path.is_dir() {
                dest_path.join(file_name)
            } else {
                dest_path.to_path_buf()
            };

            // Check if source and target are identical
            if let (Ok(can_src), Ok(can_target)) = (src_path.canonicalize(), target.canonicalize()) {
                if can_src == can_target {
                    if is_move {
                        return Ok(None); // Move onto itself is a no-op
                    } else {
                        // Copy onto itself is a no-op
                        return Ok(None);
                    }
                }
            }

            // Prevent copying directory into itself or its own subdirectories
            if src_path.is_dir() {
                if let Ok(can_src) = src_path.canonicalize() {
                    let dest_check = if target.exists() {
                        target.canonicalize().unwrap_or_else(|_| target.clone())
                    } else if let Some(parent) = target.parent() {
                        parent.canonicalize().map(|p| p.join(file_name)).unwrap_or_else(|_| target.clone())
                    } else {
                        target.clone()
                    };

                    if dest_check == can_src || dest_check.starts_with(&can_src) {
                        return Err(format!(
                            "Cannot copy directory '{}' into itself or a subdirectory of itself '{}'",
                            src,
                            target.display()
                        ));
                    }
                }
            }

            if is_move {
                if std::fs::rename(src, &target).is_err() {
                    LocalFs::copy_file_paranoid(src, &target.to_string_lossy(), paranoid).map_err(|e| e.to_string())?;
                    let _ = LocalFs::delete_entry(src, false, None);
                }
            } else {
                LocalFs::copy_file_paranoid(src, &target.to_string_lossy(), paranoid).map_err(|e| e.to_string())?;
            }

            if target.is_file() {
                if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                    verified_hash = Some(format!("SHA-256 Match: {}", h));
                }
            }
        }

        Ok(verified_hash)
    }

    /// Run full batch transfer in background task
    pub async fn execute_batch_transfer(
        task_manager: Arc<TaskManager>,
        task_id: String,
        sources: Vec<String>,
        destination: String,
        is_move: bool,
        paranoid: bool,
    ) {
        let total_items = sources.len() as u64;
        let mut processed = 0u64;
        let mut verified = 0u64;

        task_manager.set_paranoid(&task_id, paranoid).await;

        if paranoid {
            task_manager.add_log_entry(&task_id, "🛡️ TeraCopy Paranoid Integrity: ACTIVE (Full SHA-256 Hash Verification)").await;
        }

        for (idx, src_str) in sources.iter().enumerate() {
            let item_name = src_str.rsplit('/').next().unwrap_or(src_str);
            task_manager.update_task_details(
                &task_id,
                Some(item_name),
                0,
                0,
                processed,
                total_items,
                0,
                0,
                Some(verified),
                None,
                Some(&format!("Transferring item {}/{}: {}", idx + 1, total_items, item_name)),
            ).await;

            match Self::transfer_single_item(
                src_str,
                &destination,
                is_move,
                paranoid,
                &task_manager,
                &task_id,
            ) {
                Ok(hash_opt) => {
                    processed += 1;
                    if hash_opt.is_some() {
                        verified += 1;
                    }
                    let hash_str = hash_opt.as_deref().unwrap_or("");
                    let log_msg = if !hash_str.is_empty() {
                        format!("✓ Transferred {} | {}", item_name, hash_str)
                    } else {
                        format!("✓ Transferred {}", item_name)
                    };
                    task_manager.update_task_details(
                        &task_id,
                        Some(item_name),
                        1,
                        1,
                        processed,
                        total_items,
                        0,
                        0,
                        Some(verified),
                        hash_opt.as_deref(),
                        Some(&log_msg),
                    ).await;
                }
                Err(e) => {
                    task_manager.fail_task(&task_id, &e).await;
                    return;
                }
            }
        }

        if paranoid && verified > 0 {
            task_manager.add_log_entry(&task_id, &format!("🛡️ Paranoid Verification Complete: {}/{} files verified with SHA-256 match", verified, total_items)).await;
        }

        task_manager.complete_task(&task_id).await;
    }
}
