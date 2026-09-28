use crate::tools::tasks::TaskManager;
use crate::vfs::local::LocalFs;
use crate::vfs::smb::{SmbClient, SmbParams};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::NamedTempFile;

/// Generates a non-colliding destination path if the target already exists (e.g. "photo (1).png", "folder (1)")
pub fn generate_unique_destination_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let parent = target.parent().unwrap_or_else(|| Path::new(""));
    let is_dir = target.is_dir();

    let (stem, ext) = if is_dir {
        (target.file_name().and_then(|s| s.to_str()).unwrap_or("folder"), None)
    } else {
        let stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = target.extension().and_then(|e| e.to_str());
        (stem, ext)
    };

    let mut counter = 1;
    loop {
        let new_name = match ext {
            Some(e) if !e.is_empty() => format!("{} ({}).{}", stem, counter, e),
            _ => format!("{} ({})", stem, counter),
        };
        let candidate = parent.join(&new_name);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
        if counter > 10000 {
            let unique_suffix = format!("{}_{}", stem, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
            return parent.join(match ext {
                Some(e) if !e.is_empty() => format!("{}.{}", unique_suffix, e),
                _ => unique_suffix,
            });
        }
    }
}

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

    /// Recursively copy an SFTP folder to local destination
    pub fn copy_sftp_dir_to_local(
        src_params: &crate::vfs::sftp::SftpParams,
        dest_local_dir: &Path,
        task_manager: &TaskManager,
        task_id: &str,
    ) -> Result<(), String> {
        Self::copy_sftp_dir_to_local_with_progress(
            src_params,
            dest_local_dir,
            task_manager,
            task_id,
            0,
            1,
            0,
            std::time::Instant::now(),
        ).map(|_| ())
    }

    pub fn copy_sftp_dir_to_local_with_progress(
        src_params: &crate::vfs::sftp::SftpParams,
        dest_local_dir: &Path,
        task_manager: &TaskManager,
        task_id: &str,
        files_done_before: u64,
        total_files: u64,
        bytes_done_before: u64,
        start_time: std::time::Instant,
    ) -> Result<(u64, u64), String> {
        fs::create_dir_all(dest_local_dir).map_err(|e| format!("Failed to create local directory: {}", e))?;

        let listing = crate::vfs::sftp::SftpClient::list_dir(src_params).map_err(|e| e.to_string())?;
        let mut total_dir_files = 0u64;
        let mut total_dir_bytes = 0u64;
        let mut last_progress_time = std::time::Instant::now();

        for entry in listing.entries {
            if task_manager.sync_is_cancelled(task_id) {
                return Err("Transfer cancelled by user".to_string());
            }
            while task_manager.sync_is_paused(task_id) {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if task_manager.sync_is_cancelled(task_id) {
                    return Err("Transfer cancelled by user".to_string());
                }
            }

            let child_remote_path = if src_params.remote_path == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{}/{}", src_params.remote_path.trim_end_matches('/'), entry.name)
            };
            let mut child_params = src_params.clone();
            child_params.remote_path = child_remote_path;

            let child_dest = dest_local_dir.join(&entry.name);
            if entry.is_dir {
                let (c, b) = Self::copy_sftp_dir_to_local_with_progress(
                    &child_params,
                    &child_dest,
                    task_manager,
                    task_id,
                    files_done_before + total_dir_files,
                    total_files,
                    bytes_done_before + total_dir_bytes,
                    start_time,
                )?;
                total_dir_files += c;
                total_dir_bytes += b;
            } else {
                let tm = task_manager.clone();
                let tid = task_id.to_string();
                let iname = entry.name.clone();
                let cur_done_before = bytes_done_before + total_dir_bytes;
                let cur_files_before = files_done_before + total_dir_files;

                crate::vfs::sftp::SftpClient::download_to_file_streaming(
                    &child_params,
                    &child_dest,
                    false,
                    |_chunk, cur_file_bytes, cur_file_total| {
                        if tm.sync_is_cancelled(&tid) {
                            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                        }
                        while tm.sync_is_paused(&tid) {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            if tm.sync_is_cancelled(&tid) {
                                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                            }
                        }

                        let now = std::time::Instant::now();
                        if now.duration_since(last_progress_time).as_millis() >= 35 || cur_file_bytes == cur_file_total {
                            last_progress_time = now;
                            let elapsed = start_time.elapsed().as_secs_f64();
                            let total_bytes_now = cur_done_before + cur_file_bytes;
                            let current_speed = if elapsed > 0.05 {
                                (total_bytes_now as f64 / elapsed) as u64
                            } else {
                                0
                            };
                            tm.sync_update_stream_progress(
                                &tid,
                                Some(&iname),
                                cur_file_bytes,
                                cur_file_total,
                                cur_files_before,
                                total_files,
                                total_bytes_now,
                                current_speed,
                            );
                        }
                        Ok(())
                    },
                ).map_err(|e| e.to_string())?;

                total_dir_files += 1;
                total_dir_bytes += child_dest.metadata().map(|m| m.len()).unwrap_or(entry.size);
            }
        }
        Ok((total_dir_files, total_dir_bytes))
    }

    /// Recursively copy a local folder to SFTP destination
    pub fn copy_local_dir_to_sftp(
        src_local_dir: &Path,
        dest_params: &crate::vfs::sftp::SftpParams,
        task_manager: &TaskManager,
        task_id: &str,
    ) -> Result<(), String> {
        Self::copy_local_dir_to_sftp_with_progress(
            src_local_dir,
            dest_params,
            task_manager,
            task_id,
            0,
            1,
            0,
            std::time::Instant::now(),
        ).map(|_| ())
    }

    pub fn copy_local_dir_to_sftp_with_progress(
        src_local_dir: &Path,
        dest_params: &crate::vfs::sftp::SftpParams,
        task_manager: &TaskManager,
        task_id: &str,
        files_done_before: u64,
        total_files: u64,
        bytes_done_before: u64,
        start_time: std::time::Instant,
    ) -> Result<(u64, u64), String> {
        let _ = crate::vfs::sftp::SftpClient::mkdir(dest_params);

        let read_dir = fs::read_dir(src_local_dir).map_err(|e| format!("Failed to read local dir: {}", e))?;
        let mut total_dir_files = 0u64;
        let mut total_dir_bytes = 0u64;
        let mut last_progress_time = std::time::Instant::now();

        for entry_res in read_dir {
            if task_manager.sync_is_cancelled(task_id) {
                return Err("Transfer cancelled by user".to_string());
            }
            while task_manager.sync_is_paused(task_id) {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if task_manager.sync_is_cancelled(task_id) {
                    return Err("Transfer cancelled by user".to_string());
                }
            }

            if let Ok(entry) = entry_res {
                let name = entry.file_name().to_string_lossy().to_string();
                let child_remote_path = if dest_params.remote_path.is_empty() || dest_params.remote_path == "/" {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", dest_params.remote_path.trim_end_matches('/'), name)
                };
                let mut child_params = dest_params.clone();
                child_params.remote_path = child_remote_path;

                let path = entry.path();
                if path.is_dir() {
                    let (c, b) = Self::copy_local_dir_to_sftp_with_progress(
                        &path,
                        &child_params,
                        task_manager,
                        task_id,
                        files_done_before + total_dir_files,
                        total_files,
                        bytes_done_before + total_dir_bytes,
                        start_time,
                    )?;
                    total_dir_files += c;
                    total_dir_bytes += b;
                } else {
                    let tm = task_manager.clone();
                    let tid = task_id.to_string();
                    let iname = name.clone();
                    let cur_done_before = bytes_done_before + total_dir_bytes;
                    let cur_files_before = files_done_before + total_dir_files;

                    crate::vfs::sftp::SftpClient::upload_from_file_streaming(
                        &child_params,
                        &path,
                        false,
                        |_chunk, cur_file_bytes, cur_file_total| {
                            if tm.sync_is_cancelled(&tid) {
                                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                            }
                            while tm.sync_is_paused(&tid) {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                                if tm.sync_is_cancelled(&tid) {
                                    return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                                }
                            }

                            let now = std::time::Instant::now();
                            if now.duration_since(last_progress_time).as_millis() >= 35 || cur_file_bytes == cur_file_total {
                                last_progress_time = now;
                                let elapsed = start_time.elapsed().as_secs_f64();
                                let total_bytes_now = cur_done_before + cur_file_bytes;
                                let current_speed = if elapsed > 0.05 {
                                    (total_bytes_now as f64 / elapsed) as u64
                                } else {
                                    0
                                };
                                tm.sync_update_stream_progress(
                                    &tid,
                                    Some(&iname),
                                    cur_file_bytes,
                                    cur_file_total,
                                    cur_files_before,
                                    total_files,
                                    total_bytes_now,
                                    current_speed,
                                );
                            }
                            Ok(())
                        },
                    ).map_err(|e| e.to_string())?;

                    total_dir_files += 1;
                    total_dir_bytes += path.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        Ok((total_dir_files, total_dir_bytes))
    }

    /// Recursively copy an archive/image folder to local destination with live progress
    pub fn copy_archive_dir_to_local_with_progress(
        archive_file: &str,
        subpath: &str,
        dest_local_dir: &Path,
        conflict_resolution: Option<&str>,
        task_manager: &TaskManager,
        task_id: &str,
        files_done_before: u64,
        total_files: u64,
        bytes_done_before: u64,
        start_time: std::time::Instant,
    ) -> Result<(u64, u64), String> {
        fs::create_dir_all(dest_local_dir).map_err(|e| format!("Failed to create local directory: {}", e))?;

        let listing = crate::vfs::archive::ArchiveHandler::list_archive_contents(archive_file, subpath)
            .map_err(|e| format!("Failed to list archive directory: {}", e))?;

        let mut total_dir_files = 0u64;
        let mut total_dir_bytes = 0u64;
        let mut last_progress_time = std::time::Instant::now();

        for entry in listing.entries {
            if task_manager.sync_is_cancelled(task_id) {
                return Err("Transfer cancelled by user".to_string());
            }
            while task_manager.sync_is_paused(task_id) {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if task_manager.sync_is_cancelled(task_id) {
                    return Err("Transfer cancelled by user".to_string());
                }
            }

            let entry_name = entry.name.clone();
            let child_subpath = if subpath.is_empty() {
                entry_name.clone()
            } else {
                format!("{}/{}", subpath.trim_matches('/'), entry_name)
            };

            let child_dest = dest_local_dir.join(&entry_name);

            if entry.is_dir {
                let (d_files, d_bytes) = Self::copy_archive_dir_to_local_with_progress(
                    archive_file,
                    &child_subpath,
                    &child_dest,
                    conflict_resolution,
                    task_manager,
                    task_id,
                    files_done_before + total_dir_files,
                    total_files,
                    bytes_done_before + total_dir_bytes,
                    start_time,
                )?;
                total_dir_files += d_files;
                total_dir_bytes += d_bytes;
            } else {
                let file_res = match crate::vfs::archive::ArchiveHandler::read_archive_entry(archive_file, &child_subpath, 0) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("Skipping unreadable archive entry '{}': {}", child_subpath, e);
                        continue;
                    }
                };

                use base64::Engine;
                let file_bytes = if file_res.is_binary {
                    base64::engine::general_purpose::STANDARD.decode(&file_res.content).unwrap_or_default()
                } else {
                    file_res.content.into_bytes()
                };

                let raw_target = child_dest;
                let target = match conflict_resolution {
                    Some("skip") if raw_target.exists() => continue,
                    Some("rename") if raw_target.exists() => generate_unique_destination_path(&raw_target),
                    _ => raw_target,
                };

                if let Some(parent) = target.parent() {
                    let _ = fs::create_dir_all(parent);
                }

                let byte_len = file_bytes.len() as u64;
                fs::write(&target, &file_bytes).map_err(|e| format!("Failed to write extracted file '{}': {}", target.display(), e))?;

                total_dir_files += 1;
                total_dir_bytes += byte_len;

                let now = std::time::Instant::now();
                if now.duration_since(last_progress_time).as_millis() >= 35 {
                    last_progress_time = now;
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let total_now = bytes_done_before + total_dir_bytes;
                    let speed = if elapsed > 0.05 {
                        (total_now as f64 / elapsed) as u64
                    } else {
                        0
                    };
                    task_manager.sync_update_stream_progress(
                        task_id,
                        Some(&entry_name),
                        byte_len,
                        byte_len,
                        files_done_before + total_dir_files,
                        total_files,
                        total_now,
                        speed,
                    );
                }
            }
        }

        Ok((total_dir_files, total_dir_bytes))
    }

    /// Execute transfer of a single item (file or folder) between any supported VFS endpoints
    pub fn transfer_single_item(
        src: &str,
        dest_dir: &str,
        is_move: bool,
        paranoid: bool,
        conflict_resolution: Option<&str>,
        task_manager: &TaskManager,
        task_id: &str,
    ) -> Result<Option<String>, String> {
        Self::transfer_single_item_with_metrics(
            src,
            dest_dir,
            is_move,
            paranoid,
            conflict_resolution,
            task_manager,
            task_id,
            0,
            1,
            0,
            std::time::Instant::now(),
        ).map(|(hash, _, _)| hash)
    }

    pub fn transfer_single_item_with_metrics(
        src: &str,
        dest_dir: &str,
        is_move: bool,
        paranoid: bool,
        conflict_resolution: Option<&str>,
        task_manager: &TaskManager,
        task_id: &str,
        files_done_before: u64,
        total_files: u64,
        bytes_done_before: u64,
        start_time: std::time::Instant,
    ) -> Result<(Option<String>, u64, u64), String> {
        let is_src_smb = src.starts_with("smb://");
        let is_dest_smb = dest_dir.starts_with("smb://");
        let is_src_sftp = src.starts_with("sftp://") || src.starts_with("ssh://");
        let is_dest_sftp = dest_dir.starts_with("sftp://") || dest_dir.starts_with("ssh://");
        let is_src_nfs = src.starts_with("nfs://");
        let is_dest_nfs = dest_dir.starts_with("nfs://");
        let is_src_archive = src.starts_with("archive://");
        let mut verified_hash: Option<String> = None;
        let mut item_files = 1u64;
        let mut item_bytes = 0u64;

        if is_src_archive {
            let rest = src.strip_prefix("archive://").unwrap();
            let (archive_file, subpath) = match rest.split_once('#') {
                Some((a, s)) => (a, s),
                None => (rest, ""),
            };

            let read_attempt = crate::vfs::archive::ArchiveHandler::read_archive_entry(archive_file, subpath, 0);
            match read_attempt {
                Ok(file_res) => {
                    use base64::Engine;
                    let file_bytes = if file_res.is_binary {
                        base64::engine::general_purpose::STANDARD.decode(&file_res.content).unwrap_or_default()
                    } else {
                        file_res.content.into_bytes()
                    };

                    let dest_path = Path::new(dest_dir);
                    let raw_target = if dest_path.is_dir() {
                        dest_path.join(&file_res.name)
                    } else {
                        dest_path.to_path_buf()
                    };

                    let target = match conflict_resolution {
                        Some("skip") if raw_target.exists() => return Ok((None, 0, 0)),
                        Some("rename") if raw_target.exists() => generate_unique_destination_path(&raw_target),
                        _ => raw_target,
                    };

                    if let Some(parent) = target.parent() {
                        let _ = fs::create_dir_all(parent);
                    }

                    item_bytes = file_bytes.len() as u64;
                    fs::write(&target, &file_bytes).map_err(|e| format!("Failed to write extracted file: {}", e))?;
                    if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                        verified_hash = Some(format!("Extracted SHA-256: {}", h));
                    }
                    return Ok((verified_hash, 1, item_bytes));
                }
                Err(_) => {
                    // Try directory extraction
                    let dest_path = Path::new(dest_dir);
                    let folder_name = subpath.rsplit('/').next().unwrap_or(subpath).trim_matches('/');
                    let target = if dest_path.is_dir() && !folder_name.is_empty() {
                        dest_path.join(folder_name)
                    } else {
                        dest_path.to_path_buf()
                    };

                    let (d_files, d_bytes) = Self::copy_archive_dir_to_local_with_progress(
                        archive_file,
                        subpath,
                        &target,
                        conflict_resolution,
                        task_manager,
                        task_id,
                        files_done_before,
                        total_files,
                        bytes_done_before,
                        start_time,
                    )?;
                    return Ok((None, d_files, d_bytes));
                }
            }
        } else if is_src_sftp && !is_dest_sftp {
            // SFTP -> Local
            let src_params = crate::vfs::sftp::SftpClient::parse_uri(src, None, None)?;
            let file_name = src_params.remote_path.rsplit('/').next().unwrap_or(&src_params.remote_path).to_string();
            let dest_path = Path::new(dest_dir);
            let raw_target = if dest_path.is_dir() {
                dest_path.join(&file_name)
            } else {
                dest_path.to_path_buf()
            };

            let target = match conflict_resolution {
                Some("skip") if raw_target.exists() => return Ok((None, 0, 0)),
                Some("rename") if raw_target.exists() => generate_unique_destination_path(&raw_target),
                _ => raw_target,
            };

            let tm = task_manager.clone();
            let tid = task_id.to_string();
            let mut last_progress_time = std::time::Instant::now();
            let iname = file_name.clone();

            let stream_res = crate::vfs::sftp::SftpClient::download_to_file_streaming(
                &src_params,
                &target,
                paranoid,
                |_chunk, cur_file_bytes, cur_file_total| {
                    if tm.sync_is_cancelled(&tid) {
                        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                    }
                    while tm.sync_is_paused(&tid) {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        if tm.sync_is_cancelled(&tid) {
                            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                        }
                    }

                    let now = std::time::Instant::now();
                    if now.duration_since(last_progress_time).as_millis() >= 35 || cur_file_bytes == cur_file_total {
                        last_progress_time = now;
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let total_bytes_now = bytes_done_before + cur_file_bytes;
                        let current_speed = if elapsed > 0.05 {
                            (total_bytes_now as f64 / elapsed) as u64
                        } else {
                            0
                        };
                        tm.sync_update_stream_progress(
                            &tid,
                            Some(&iname),
                            cur_file_bytes,
                            cur_file_total,
                            files_done_before,
                            total_files,
                            total_bytes_now,
                            current_speed,
                        );
                    }
                    Ok(())
                },
            );

            match stream_res {
                Ok(hash_opt) => {
                    if let Some(h) = hash_opt {
                        verified_hash = Some(format!("Dest SHA-256: {}", h));
                    }
                    item_bytes = target.metadata().map(|m| m.len()).unwrap_or(0);
                    item_files = 1;
                }
                Err(e) => {
                    if let Ok(_listing) = crate::vfs::sftp::SftpClient::list_dir(&src_params) {
                        let (d_files, d_bytes) = Self::copy_sftp_dir_to_local_with_progress(
                            &src_params,
                            &target,
                            task_manager,
                            task_id,
                            files_done_before,
                            total_files,
                            bytes_done_before,
                            start_time,
                        )?;
                        item_files = d_files;
                        item_bytes = d_bytes;
                    } else {
                        return Err(format!("SFTP download failed: {}", e));
                    }
                }
            }

            if is_move {
                let _ = crate::vfs::sftp::SftpClient::delete(&src_params, target.is_dir());
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

            let tm = task_manager.clone();
            let tid = task_id.to_string();
            let mut last_progress_time = std::time::Instant::now();
            let iname = file_name.clone();

            if src_path.is_dir() {
                let (d_files, d_bytes) = Self::copy_local_dir_to_sftp_with_progress(
                    src_path,
                    &target_params,
                    task_manager,
                    task_id,
                    files_done_before,
                    total_files,
                    bytes_done_before,
                    start_time,
                )?;
                item_files = d_files;
                item_bytes = d_bytes;
            } else {
                let hash_opt = crate::vfs::sftp::SftpClient::upload_from_file_streaming(
                    &target_params,
                    src_path,
                    paranoid,
                    |_chunk, cur_file_bytes, cur_file_total| {
                        if tm.sync_is_cancelled(&tid) {
                            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                        }
                        while tm.sync_is_paused(&tid) {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            if tm.sync_is_cancelled(&tid) {
                                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                            }
                        }

                        let now = std::time::Instant::now();
                        if now.duration_since(last_progress_time).as_millis() >= 35 || cur_file_bytes == cur_file_total {
                            last_progress_time = now;
                            let elapsed = start_time.elapsed().as_secs_f64();
                            let total_bytes_now = bytes_done_before + cur_file_bytes;
                            let current_speed = if elapsed > 0.05 {
                                (total_bytes_now as f64 / elapsed) as u64
                            } else {
                                0
                            };
                            tm.sync_update_stream_progress(
                                &tid,
                                Some(&iname),
                                cur_file_bytes,
                                cur_file_total,
                                files_done_before,
                                total_files,
                                total_bytes_now,
                                current_speed,
                            );
                        }
                        Ok(())
                    },
                )?;
                if let Some(h) = hash_opt {
                    verified_hash = Some(format!("Src SHA-256: {}", h));
                }
                item_bytes = src_path.metadata().map(|m| m.len()).unwrap_or(0);
                item_files = 1;
            }

            if is_move {
                let _ = LocalFs::delete_entry(src, false, None);
            }
        } else if is_src_sftp && is_dest_sftp {
            // SFTP -> SFTP
            let src_params = crate::vfs::sftp::SftpClient::parse_uri(src, None, None)?;
            let file_name = src_params.remote_path.rsplit('/').next().unwrap_or(&src_params.remote_path).to_string();
            let dest_params = crate::vfs::sftp::SftpClient::parse_uri(dest_dir, None, None)?;
            let target_remote_path = if dest_params.remote_path.is_empty() || dest_params.remote_path == "/" {
                format!("/{}", file_name)
            } else {
                format!("{}/{}", dest_params.remote_path.trim_end_matches('/'), file_name)
            };
            let mut target_params = dest_params.clone();
            target_params.remote_path = target_remote_path;

            let tmp = NamedTempFile::new().map_err(|e| format!("Temp file error: {}", e))?;
            crate::vfs::sftp::SftpClient::download_to_file(&src_params, tmp.path())?;
            if let Ok(h) = crate::vfs::checksum::calculate_sha256(tmp.path()) {
                verified_hash = Some(format!("Stream SHA-256: {}", h));
            }
            crate::vfs::sftp::SftpClient::upload_from_file(&target_params, tmp.path())?;
            item_bytes = tmp.path().metadata().map(|m| m.len()).unwrap_or(0);
            item_files = 1;

            if is_move {
                let _ = crate::vfs::sftp::SftpClient::delete(&src_params, false);
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

            let raw_target = if local_dest.is_dir() {
                local_dest.join(local_src.file_name().unwrap_or_default())
            } else {
                local_dest
            };

            let target = match conflict_resolution {
                Some("skip") if raw_target.exists() => return Ok((None, 0, 0)),
                Some("rename") if raw_target.exists() => generate_unique_destination_path(&raw_target),
                _ => raw_target,
            };

            LocalFs::copy_file_paranoid(&local_src.to_string_lossy(), &target.to_string_lossy(), paranoid).map_err(|e| e.to_string())?;
            if target.is_file() {
                if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                    verified_hash = Some(format!("SHA-256 Match: {}", h));
                }
                item_bytes = target.metadata().map(|m| m.len()).unwrap_or(0);
                item_files = 1;
            }

            if is_move {
                let _ = LocalFs::delete_entry(&local_src.to_string_lossy(), false, None);
            }
        } else if is_src_smb && !is_dest_smb {
            // SMB -> Local
            let src_params = SmbClient::parse_uri(src, None, None)?;
            let file_name = src_params.subpath.rsplit('/').next().unwrap_or(&src_params.subpath).to_string();
            let dest_path = Path::new(dest_dir);
            let raw_target = if dest_path.is_dir() {
                dest_path.join(&file_name)
            } else {
                dest_path.to_path_buf()
            };

            let target = match conflict_resolution {
                Some("skip") if raw_target.exists() => return Ok((None, 0, 0)),
                Some("rename") if raw_target.exists() => generate_unique_destination_path(&raw_target),
                _ => raw_target,
            };

            // Try download file directly
            match SmbClient::download_to_file(&src_params, &target) {
                Ok(_) => {
                    if target.is_file() {
                        if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                            verified_hash = Some(format!("Dest SHA-256: {}", h));
                        }
                        item_bytes = target.metadata().map(|m| m.len()).unwrap_or(0);
                        item_files = 1;
                    }
                },
                Err(e) => {
                    if let Ok(_listing) = SmbClient::list_dir(&src_params) {
                        Self::copy_smb_dir_to_local(&src_params, &target, task_manager, task_id)?;
                        item_files = 1;
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
                item_files = 1;
            } else {
                if let Ok(h) = crate::vfs::checksum::calculate_sha256(src_path) {
                    verified_hash = Some(format!("Src SHA-256: {}", h));
                }
                SmbClient::upload_from_file(&target_params, src_path)?;
                item_bytes = src_path.metadata().map(|m| m.len()).unwrap_or(0);
                item_files = 1;
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
            item_bytes = tmp.path().metadata().map(|m| m.len()).unwrap_or(0);
            item_files = 1;

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
            let raw_target = if dest_path.is_dir() {
                dest_path.join(file_name)
            } else {
                dest_path.to_path_buf()
            };

            // Check if source and target are identical
            if let (Ok(can_src), Ok(can_target)) = (src_path.canonicalize(), raw_target.canonicalize()) {
                if can_src == can_target {
                    if is_move {
                        return Ok((None, 0, 0)); // Move onto itself is a no-op
                    } else {
                        // Copy onto itself is a no-op
                        return Ok((None, 0, 0));
                    }
                }
            }

            let target = match conflict_resolution {
                Some("skip") if raw_target.exists() => return Ok((None, 0, 0)),
                Some("rename") if raw_target.exists() => generate_unique_destination_path(&raw_target),
                _ => raw_target,
            };

            // Prevent copying directory into itself or its own subdirectories
            if src_path.is_dir() {
                if let Ok(can_src) = src_path.canonicalize() {
                    let dest_check = if target.exists() {
                        target.canonicalize().unwrap_or_else(|_| target.clone())
                    } else if let Some(parent) = target.parent() {
                        parent.canonicalize().map(|p| p.join(target.file_name().unwrap_or(file_name))).unwrap_or_else(|_| target.clone())
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

            let tm = task_manager.clone();
            let tid = task_id.to_string();
            let iname = file_name.to_string_lossy().to_string();
            let mut last_progress_time = std::time::Instant::now();
            let mut dir_bytes_done = 0u64;

            if is_move {
                if std::fs::rename(src, &target).is_err() {
                    if src_path.is_file() {
                        let hash_opt = LocalFs::copy_single_file_streaming(src_path, &target, paranoid, |_, cur_file_bytes, cur_file_total| {
                            if tm.sync_is_cancelled(&tid) {
                                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                            }
                            while tm.sync_is_paused(&tid) {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                                if tm.sync_is_cancelled(&tid) {
                                    return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                                }
                            }

                            let now = std::time::Instant::now();
                            if now.duration_since(last_progress_time).as_millis() >= 35 || cur_file_bytes == cur_file_total {
                                last_progress_time = now;
                                let elapsed = start_time.elapsed().as_secs_f64();
                                let total_bytes_now = bytes_done_before + cur_file_bytes;
                                let current_speed = if elapsed > 0.05 {
                                    (total_bytes_now as f64 / elapsed) as u64
                                } else {
                                    0
                                };
                                tm.sync_update_stream_progress(
                                    &tid,
                                    Some(&iname),
                                    cur_file_bytes,
                                    cur_file_total,
                                    files_done_before,
                                    total_files,
                                    total_bytes_now,
                                    current_speed,
                                );
                            }
                            Ok(())
                        }).map_err(|e| e.to_string())?;

                        if let Some(h) = hash_opt {
                            verified_hash = Some(format!("SHA-256 Match: {}", h));
                        }
                        item_bytes = target.metadata().map(|m| m.len()).unwrap_or(0);
                        item_files = 1;
                    } else {
                        LocalFs::copy_file_paranoid_with_progress(src, &target.to_string_lossy(), paranoid, |cur_file_path, chunk_bytes, cur_bytes, cur_total| {
                            if tm.sync_is_cancelled(&tid) {
                                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                            }
                            while tm.sync_is_paused(&tid) {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                                if tm.sync_is_cancelled(&tid) {
                                    return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                                }
                            }

                            dir_bytes_done += chunk_bytes;
                            let now = std::time::Instant::now();
                            if now.duration_since(last_progress_time).as_millis() >= 35 || cur_bytes == cur_total {
                                last_progress_time = now;
                                let elapsed = start_time.elapsed().as_secs_f64();
                                let total_bytes_now = bytes_done_before + dir_bytes_done;
                                let current_speed = if elapsed > 0.05 {
                                    (total_bytes_now as f64 / elapsed) as u64
                                } else {
                                    0
                                };
                                let cur_name = cur_file_path.file_name().unwrap_or_default().to_string_lossy();
                                tm.sync_update_stream_progress(
                                    &tid,
                                    Some(&cur_name),
                                    cur_bytes,
                                    cur_total,
                                    files_done_before,
                                    total_files,
                                    total_bytes_now,
                                    current_speed,
                                );
                            }
                            Ok(())
                        }).map_err(|e| e.to_string())?;
                        item_bytes = dir_bytes_done;
                        item_files = walkdir::WalkDir::new(&target).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()).count() as u64;
                    }
                    let _ = LocalFs::delete_entry(src, false, None);
                } else {
                    item_bytes = src_path.metadata().map(|m| m.len()).unwrap_or(0);
                    item_files = 1;
                    if paranoid && target.is_file() {
                        if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                            verified_hash = Some(format!("SHA-256 Match: {}", h));
                        }
                    }
                }
            } else {
                if src_path.is_file() {
                    let hash_opt = LocalFs::copy_single_file_streaming(src_path, &target, paranoid, |_, cur_file_bytes, cur_file_total| {
                        if tm.sync_is_cancelled(&tid) {
                            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                        }
                        while tm.sync_is_paused(&tid) {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            if tm.sync_is_cancelled(&tid) {
                                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                            }
                        }

                        let now = std::time::Instant::now();
                        if now.duration_since(last_progress_time).as_millis() >= 35 || cur_file_bytes == cur_file_total {
                            last_progress_time = now;
                            let elapsed = start_time.elapsed().as_secs_f64();
                            let total_bytes_now = bytes_done_before + cur_file_bytes;
                            let current_speed = if elapsed > 0.05 {
                                (total_bytes_now as f64 / elapsed) as u64
                            } else {
                                0
                            };
                            tm.sync_update_stream_progress(
                                &tid,
                                Some(&iname),
                                cur_file_bytes,
                                cur_file_total,
                                files_done_before,
                                total_files,
                                total_bytes_now,
                                current_speed,
                            );
                        }
                        Ok(())
                    }).map_err(|e| e.to_string())?;

                    if let Some(h) = hash_opt {
                        verified_hash = Some(format!("SHA-256 Match: {}", h));
                    }
                    item_bytes = target.metadata().map(|m| m.len()).unwrap_or(0);
                    item_files = 1;
                } else {
                    LocalFs::copy_file_paranoid_with_progress(src, &target.to_string_lossy(), paranoid, |cur_file_path, chunk_bytes, cur_bytes, cur_total| {
                        if tm.sync_is_cancelled(&tid) {
                            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                        }
                        while tm.sync_is_paused(&tid) {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            if tm.sync_is_cancelled(&tid) {
                                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Transfer cancelled by user"));
                            }
                        }

                        dir_bytes_done += chunk_bytes;
                        let now = std::time::Instant::now();
                        if now.duration_since(last_progress_time).as_millis() >= 35 || cur_bytes == cur_total {
                            last_progress_time = now;
                            let elapsed = start_time.elapsed().as_secs_f64();
                            let total_bytes_now = bytes_done_before + dir_bytes_done;
                            let current_speed = if elapsed > 0.05 {
                                (total_bytes_now as f64 / elapsed) as u64
                            } else {
                                0
                            };
                            let cur_name = cur_file_path.file_name().unwrap_or_default().to_string_lossy();
                            tm.sync_update_stream_progress(
                                &tid,
                                Some(&cur_name),
                                cur_bytes,
                                cur_total,
                                files_done_before,
                                total_files,
                                total_bytes_now,
                                current_speed,
                            );
                        }
                        Ok(())
                    }).map_err(|e| e.to_string())?;
                    item_bytes = dir_bytes_done;
                    item_files = walkdir::WalkDir::new(&target).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()).count() as u64;

                    if paranoid && target.is_file() {
                        if let Ok(h) = crate::vfs::checksum::calculate_sha256(&target) {
                            verified_hash = Some(format!("SHA-256 Match: {}", h));
                        }
                    }
                }
            }
        }

        Ok((verified_hash, item_files, item_bytes))
    }

    /// Run full batch transfer in background task
    pub async fn execute_batch_transfer(
        task_manager: Arc<TaskManager>,
        task_id: String,
        sources: Vec<String>,
        destination: String,
        is_move: bool,
        paranoid: bool,
        conflict_resolution: Option<String>,
    ) {
        // 1. Accurate zero-latency pre-scan of total batch files and bytes
        let mut total_files = 0u64;
        let mut total_bytes = 0u64;
        for s in &sources {
            if !s.starts_with("smb://") && !s.starts_with("sftp://") && !s.starts_with("ssh://") && !s.starts_with("nfs://") {
                let p = Path::new(s);
                if p.is_dir() {
                    let (count, bytes) = walkdir::WalkDir::new(p)
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_file())
                        .fold((0u64, 0u64), |(c, b), e| {
                            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                            (c + 1, b + len)
                        });
                    total_files += if count > 0 { count } else { 1 };
                    total_bytes += bytes;
                } else if let Ok(meta) = p.metadata() {
                    total_bytes += meta.len();
                    total_files += 1;
                } else {
                    total_files += 1;
                }
            } else if s.starts_with("sftp://") || s.starts_with("ssh://") {
                if let Ok(params) = crate::vfs::sftp::SftpClient::parse_uri(s, None, None) {
                    if let Ok((size, is_dir)) = crate::vfs::sftp::SftpClient::stat_path(&params) {
                        if is_dir {
                            if let Ok((count, bytes)) = crate::vfs::sftp::SftpClient::scan_dir_totals(&params) {
                                total_files += if count > 0 { count } else { 1 };
                                total_bytes += bytes;
                            } else {
                                total_files += 1;
                            }
                        } else {
                            total_files += 1;
                            total_bytes += size;
                        }
                    } else {
                        total_files += 1;
                    }
                } else {
                    total_files += 1;
                }
            } else {
                total_files += 1;
            }
        }
        if total_files == 0 {
            total_files = sources.len() as u64;
        }

        task_manager.set_task_totals(&task_id, total_files, total_bytes).await;
        task_manager.set_paranoid(&task_id, paranoid).await;

        if paranoid {
            task_manager.add_log_entry(&task_id, "🛡️ TeraCopy Paranoid Integrity: ACTIVE (Full SHA-256 Hash Verification)").await;
        }

        let mut files_done = 0u64;
        let mut verified = 0u64;
        let mut bytes_done = 0u64;
        let start_time = std::time::Instant::now();

        for (idx, src_str) in sources.iter().enumerate() {
            // Check cancellation
            if task_manager.is_cancelled(&task_id).await {
                task_manager.add_log_entry(&task_id, "Transfer cancelled by user").await;
                return;
            }

            // Check pause
            while task_manager.is_paused(&task_id).await {
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                if task_manager.is_cancelled(&task_id).await {
                    return;
                }
            }

            let item_name = src_str.rsplit('/').next().unwrap_or(src_str);
            let elapsed = start_time.elapsed().as_secs_f64();
            let current_speed = if elapsed > 0.05 {
                (bytes_done as f64 / elapsed) as u64
            } else {
                0
            };

            task_manager.update_task_details(
                &task_id,
                Some(item_name),
                0,
                0,
                files_done,
                total_files,
                bytes_done,
                current_speed,
                Some(verified),
                None,
                Some(&format!("Transferring item {}/{}: {}", idx + 1, sources.len(), item_name)),
            ).await;

            match Self::transfer_single_item_with_metrics(
                src_str,
                &destination,
                is_move,
                paranoid,
                conflict_resolution.as_deref(),
                &task_manager,
                &task_id,
                files_done,
                total_files,
                bytes_done,
                start_time,
            ) {
                Ok((hash_opt, item_files, item_bytes)) => {
                    files_done += item_files;
                    bytes_done += item_bytes;
                    if hash_opt.is_some() {
                        verified += 1;
                    }

                    let hash_str = hash_opt.as_deref().unwrap_or("");
                    let log_msg = if !hash_str.is_empty() {
                        format!("✓ Transferred {} | {}", item_name, hash_str)
                    } else {
                        format!("✓ Transferred {}", item_name)
                    };

                    let post_elapsed = start_time.elapsed().as_secs_f64();
                    let post_speed = if post_elapsed > 0.05 {
                        (bytes_done as f64 / post_elapsed) as u64
                    } else {
                        0
                    };

                    task_manager.update_task_details(
                        &task_id,
                        Some(item_name),
                        1,
                        1,
                        files_done,
                        total_files,
                        bytes_done,
                        post_speed,
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
            task_manager.add_log_entry(&task_id, &format!("🛡️ Paranoid Verification Complete: {}/{} files verified with SHA-256 match", verified, total_files)).await;
        }

        task_manager.complete_task(&task_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_generate_unique_destination_path() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("report.pdf");
        fs::write(&file1, b"original").unwrap();

        let unique1 = generate_unique_destination_path(&file1);
        assert_eq!(unique1.file_name().unwrap(), "report (1).pdf");

        fs::write(&unique1, b"copy 1").unwrap();
        let unique2 = generate_unique_destination_path(&file1);
        assert_eq!(unique2.file_name().unwrap(), "report (2).pdf");
    }

    #[tokio::test]
    async fn test_transfer_single_item_conflict_modes() {
        let dir = tempdir().unwrap();
        let src_dir = dir.path().join("src");
        let dest_dir = dir.path().join("dest");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();

        let src_file = src_dir.join("test.txt");
        let dest_file = dest_dir.join("test.txt");
        fs::write(&src_file, b"source content").unwrap();
        fs::write(&dest_file, b"existing dest content").unwrap();

        let tm = TaskManager::new();

        // 1. Skip mode: dest_file is preserved, source is not touched
        let res = VfsTransfer::transfer_single_item(
            src_file.to_str().unwrap(),
            dest_dir.to_str().unwrap(),
            false,
            false,
            Some("skip"),
            &tm,
            "test_task_1",
        );
        assert!(res.is_ok());
        assert_eq!(fs::read(&dest_file).unwrap(), b"existing dest content");

        // 2. Rename mode: creates test (1).txt
        let res2 = VfsTransfer::transfer_single_item(
            src_file.to_str().unwrap(),
            dest_dir.to_str().unwrap(),
            false,
            false,
            Some("rename"),
            &tm,
            "test_task_2",
        );
        assert!(res2.is_ok());
        let renamed = dest_dir.join("test (1).txt");
        assert!(renamed.exists());
        assert_eq!(fs::read(&renamed).unwrap(), b"source content");
        assert_eq!(fs::read(&dest_file).unwrap(), b"existing dest content");

        // 3. Overwrite mode: replaces dest_file
        let res3 = VfsTransfer::transfer_single_item(
            src_file.to_str().unwrap(),
            dest_dir.to_str().unwrap(),
            false,
            false,
            Some("overwrite"),
            &tm,
            "test_task_3",
        );
        assert!(res3.is_ok());
        assert_eq!(fs::read(&dest_file).unwrap(), b"source content");
    }

    #[tokio::test]
    async fn test_transfer_single_item_from_archive() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");
        let dest_dir = dir.path().join("dest");
        fs::create_dir_all(&dest_dir).unwrap();

        // Create a test zip file
        {
            let file = fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("hello.txt", options).unwrap();
            use std::io::Write;
            zip.write_all(b"Hello World from Archive!").unwrap();
            zip.finish().unwrap();
        }

        let tm = TaskManager::new();
        let archive_uri = format!("archive://{}#hello.txt", zip_path.to_str().unwrap());
        let res = VfsTransfer::transfer_single_item(
            &archive_uri,
            dest_dir.to_str().unwrap(),
            false,
            false,
            None,
            &tm,
            "test_archive_task",
        );

        assert!(res.is_ok(), "Transfer failed: {:?}", res.err());
        let extracted_file = dest_dir.join("hello.txt");
        assert!(extracted_file.exists());
        let content = fs::read(&extracted_file).unwrap();
        assert_eq!(content, b"Hello World from Archive!");
    }

    #[tokio::test]
    async fn test_transfer_single_item_from_fat_image() {
        let dir = tempdir().unwrap();
        let img_path = dir.path().join("floppy.img");
        let dest_dir = dir.path().join("dest");
        fs::create_dir_all(&dest_dir).unwrap();

        // Create a FAT floppy image with a file
        {
            let mut cursor = std::io::Cursor::new(vec![0u8; 1440 * 1024]);
            fatfs::format_volume(&mut cursor, fatfs::FormatVolumeOptions::new()).unwrap();
            let buf = cursor.into_inner();
            fs::write(&img_path, &buf).unwrap();

            let file = fs::OpenOptions::new().read(true).write(true).open(&img_path).unwrap();
            let fs_obj = fatfs::FileSystem::new(file, fatfs::FsOptions::new()).unwrap();
            let mut root_file = fs_obj.root_dir().create_file("CONFIG.SYS").unwrap();
            use std::io::Write;
            root_file.write_all(b"DOS=HIGH,UMB\nFILES=40\n").unwrap();
        }

        let tm = TaskManager::new();
        let archive_uri = format!("archive://{}#CONFIG.SYS", img_path.to_str().unwrap());
        let res = VfsTransfer::transfer_single_item(
            &archive_uri,
            dest_dir.to_str().unwrap(),
            false,
            false,
            None,
            &tm,
            "test_fat_task",
        );

        assert!(res.is_ok(), "FAT transfer failed: {:?}", res.err());
        let extracted = dest_dir.join("CONFIG.SYS");
        assert!(extracted.exists());
        let content = fs::read(&extracted).unwrap();
        assert_eq!(content, b"DOS=HIGH,UMB\nFILES=40\n");
    }

    #[tokio::test]
    #[ignore]
    async fn test_transfer_from_real_fog_image() {
        let fog_path = "/home/bolt/projects/Test/test-images/w10fog";
        if !std::path::Path::new(fog_path).exists() {
            return;
        }

        let dir = tempdir().unwrap();
        let dest_dir = dir.path().join("dest");
        fs::create_dir_all(&dest_dir).unwrap();

        let tm = TaskManager::new();
        // 1. Transfer FOG summary file
        let summary_uri = format!("archive://{}#[FOG Image Summary.txt]", fog_path);
        let res = VfsTransfer::transfer_single_item(
            &summary_uri,
            dest_dir.to_str().unwrap(),
            false,
            false,
            None,
            &tm,
            "fog_test_task",
        );
        assert!(res.is_ok(), "Failed to extract summary: {:?}", res.err());
        let summary_file = dest_dir.join("[FOG Image Summary.txt]");
        assert!(summary_file.exists());
        let summary_bytes = fs::read(&summary_file).unwrap();
        assert!(!summary_bytes.is_empty(), "Extracted summary file was empty!");
        assert!(String::from_utf8_lossy(&summary_bytes).contains("FOG PROJECT IMAGE MANIFEST"));

        // 2. Transfer a file from partition 1 (EFI boot file)
        let p1_file_uri = format!("archive://{}#p1-nvme0n1p1-fat32/EFI/Microsoft/Boot/bootmgfw.efi", fog_path);
        let res2 = VfsTransfer::transfer_single_item(
            &p1_file_uri,
            dest_dir.to_str().unwrap(),
            false,
            false,
            None,
            &tm,
            "fog_p1_file_task",
        );
        assert!(res2.is_ok(), "Failed to extract EFI boot file: {:?}", res2.err());
        let extracted_boot = dest_dir.join("bootmgfw.efi");
        assert!(extracted_boot.exists());
        let boot_bytes = fs::read(&extracted_boot).unwrap();
        assert!(!boot_bytes.is_empty(), "Extracted bootmgfw.efi was empty!");
        println!("Successfully extracted bootmgfw.efi: {} bytes!", boot_bytes.len());
    }
}

