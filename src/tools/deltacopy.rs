use crate::tools::tasks::TaskManager;
use crc32fast::Hasher as Crc32Hasher;
use filetime::{set_file_times, FileTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaCopyOptions {
    pub overwrite_mode: String, // "delta_mtime_size" (default), "always", "if_newer", "never"
    #[serde(default = "default_true")]
    pub block_delta: bool,       // In-place binary block-level delta patching (Bvckup 2 speed)
    pub verify_checksum: bool,   // TeraCopy signature verification
    pub verify_algo: String,     // "crc32" or "sha256"
    pub retry_count: u32,        // Robocopy /R:3
    pub retry_delay_ms: u64,     // Robocopy /W:1000
    pub resume_partial: bool,    // Resume interrupted transfers
    pub preserve_timestamps: bool,
    pub preserve_permissions: bool,
    pub delete_orphans: bool,    // Robocopy /MIR mirror mode
}

fn default_true() -> bool {
    true
}

impl Default for DeltaCopyOptions {
    fn default() -> Self {
        Self {
            overwrite_mode: "delta_mtime_size".to_string(),
            block_delta: true,
            verify_checksum: true,
            verify_algo: "crc32".to_string(),
            retry_count: 3,
            retry_delay_ms: 500,
            resume_partial: true,
            preserve_timestamps: true,
            preserve_permissions: true,
            delete_orphans: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaCopyStats {
    pub task_id: String,
    pub total_files: usize,
    pub files_copied: usize,
    pub files_skipped: usize,
    pub files_failed: usize,
    pub total_bytes: u64,
    pub bytes_copied: u64,
    pub current_file: String,
    pub speed_bytes_per_sec: u64,
    pub eta_seconds: u64,
    pub verified_count: usize,
    pub is_completed: bool,
    pub blocks_transferred: usize,
    pub blocks_matched: usize,
    pub bytes_saved: u64,
    pub error_log: Vec<String>,
}

pub struct DeltaCopyEngine;

impl DeltaCopyEngine {
    pub async fn run_deltacopy_pairs(
        task_manager: Arc<TaskManager>,
        file_pairs: Vec<(PathBuf, PathBuf)>,
        job_label: &str,
        options: DeltaCopyOptions,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<DeltaCopyStats, String> {
        let mut total_bytes = 0u64;
        for (src, _) in &file_pairs {
            if let Ok(meta) = src.metadata() {
                total_bytes += meta.len();
            }
        }

        let total_files = file_pairs.len();
        let task_id = task_manager.create_task(
            &format!("⚡ DeltaCopy ({} items)", total_files),
            "deltacopy",
            job_label,
            "Replication Destination",
            total_bytes,
        ).await;

        let mut stats = DeltaCopyStats {
            task_id: task_id.clone(),
            total_files,
            files_copied: 0,
            files_skipped: 0,
            files_failed: 0,
            total_bytes,
            bytes_copied: 0,
            current_file: String::new(),
            speed_bytes_per_sec: 0,
            eta_seconds: 0,
            verified_count: 0,
            is_completed: false,
            blocks_transferred: 0,
            blocks_matched: 0,
            bytes_saved: 0,
            error_log: Vec::new(),
        };

        let start_time = Instant::now();
        let mut last_speed_calc = Instant::now();
        let mut last_bytes_marker = 0u64;

        for (src, dest) in file_pairs {
            if cancel_token.load(Ordering::Relaxed) {
                task_manager.cancel_task(&task_id).await;
                stats.error_log.push("Job cancelled by user".to_string());
                break;
            }

            stats.current_file = src.file_name().unwrap_or_default().to_string_lossy().to_string();

            // Check if we should skip via Delta logic
            if should_skip_file(&src, &dest, &options) {
                stats.files_skipped += 1;
                if let Ok(m) = src.metadata() {
                    stats.bytes_copied += m.len();
                }
                task_manager.add_log_entry(&task_id, &format!("⏩ Skipped unchanged: {}", stats.current_file)).await;
                continue;
            }

            // Ensure destination directory exists
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).ok();
            }

            task_manager.add_log_entry(&task_id, &format!("📥 Copying: {}", stats.current_file)).await;

            // Perform robust copy with retry logic
            let copy_res = copy_file_robust(&src, &dest, &options, &mut stats, &task_manager, &task_id, &start_time, &mut last_speed_calc, &mut last_bytes_marker).await;

            match copy_res {
                Ok(_bytes) => {
                    stats.files_copied += 1;
                    if options.verify_checksum {
                        if verify_file_checksum(&src, &dest, &options.verify_algo) {
                            stats.verified_count += 1;
                            task_manager.add_log_entry(&task_id, &format!("🔒 Verified {}: MATCH ({})", options.verify_algo.to_uppercase(), stats.current_file)).await;
                        } else {
                            stats.files_failed += 1;
                            let err_m = format!("Verification checksum mismatch: {}", src.display());
                            stats.error_log.push(err_m.clone());
                            task_manager.add_log_entry(&task_id, &format!("❌ CRC Mismatch: {}", stats.current_file)).await;
                        }
                    } else {
                        task_manager.add_log_entry(&task_id, &format!("✓ Finished: {}", stats.current_file)).await;
                    }
                }
                Err(e) => {
                    stats.files_failed += 1;
                    stats.error_log.push(format!("Failed {}: {}", src.display(), e));
                    task_manager.add_log_entry(&task_id, &format!("❌ Error {}: {}", stats.current_file, e)).await;
                }
            }
        }

        stats.is_completed = true;
        if stats.files_failed == 0 {
            task_manager.complete_task(&task_id).await;
        } else {
            task_manager.fail_task(&task_id, &stats.error_log.join("; ")).await;
        }

        Ok(stats)
    }

    pub async fn run_deltacopy(
        task_manager: Arc<TaskManager>,
        sources: Vec<String>,
        destination: String,
        options: DeltaCopyOptions,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<DeltaCopyStats, String> {
        let dest_path = Path::new(&destination);
        if !dest_path.exists() {
            fs::create_dir_all(dest_path)
                .map_err(|e| format!("Failed to create destination directory: {}", e))?;
        }

        // 1. Pre-scan total files and build pairs
        let mut file_pairs: Vec<(PathBuf, PathBuf)> = Vec::new();

        for src_str in &sources {
            let src_p = Path::new(src_str);
            if !src_p.exists() {
                continue;
            }

            if src_p.is_file() {
                let file_name = src_p.file_name().unwrap();
                let target = if dest_path.is_dir() {
                    dest_path.join(file_name)
                } else {
                    dest_path.to_path_buf()
                };
                file_pairs.push((src_p.to_path_buf(), target));
            } else if src_p.is_dir() {
                let base_name = src_p.file_name().unwrap();
                let target_root = dest_path.join(base_name);

                // Prevent copying folder into itself or its own subfolder
                if let (Ok(can_src), Ok(can_dest)) = (src_p.canonicalize(), dest_path.canonicalize()) {
                    let can_target = can_dest.join(base_name);
                    if can_target == can_src || can_target.starts_with(&can_src) {
                        continue;
                    }
                }

                for entry in WalkDir::new(src_p).into_iter().filter_map(|e| e.ok()) {
                    if entry.path() == src_p {
                        continue;
                    }
                    if entry.file_type().is_file() {
                        if let Ok(rel) = entry.path().strip_prefix(src_p) {
                            let target = target_root.join(rel);
                            file_pairs.push((entry.path().to_path_buf(), target));
                        }
                    }
                }
            }
        }

        let label = format!("{} ➔ {}", sources.join(", "), destination);
        Self::run_deltacopy_pairs(task_manager, file_pairs, &label, options, cancel_token).await
    }
}

fn should_skip_file(src: &Path, dest: &Path, options: &DeltaCopyOptions) -> bool {
    if !dest.exists() {
        return false;
    }

    let src_meta = match src.metadata() {
        Ok(m) => m,
        Err(_) => return false,
    };
    let dest_meta = match dest.metadata() {
        Ok(m) => m,
        Err(_) => return false,
    };

    match options.overwrite_mode.as_str() {
        "never" => true,
        "always" => false,
        "if_newer" => {
            if let (Ok(s_mtime), Ok(d_mtime)) = (src_meta.modified(), dest_meta.modified()) {
                d_mtime >= s_mtime
            } else {
                false
            }
        }
        "delta_mtime_size" | _ => {
            let same_size = src_meta.len() == dest_meta.len();
            let same_mtime = if let (Ok(s_mtime), Ok(d_mtime)) = (src_meta.modified(), dest_meta.modified()) {
                let diff = if s_mtime > d_mtime {
                    s_mtime.duration_since(d_mtime).unwrap_or_default()
                } else {
                    d_mtime.duration_since(s_mtime).unwrap_or_default()
                };
                diff.as_secs() < 2 // 2-second FAT/Samba window
            } else {
                false
            };

            same_size && same_mtime
        }
    }
}

async fn copy_file_robust(
    src: &Path,
    dest: &Path,
    options: &DeltaCopyOptions,
    stats: &mut DeltaCopyStats,
    task_manager: &Arc<TaskManager>,
    task_id: &str,
    start_time: &Instant,
    last_speed_calc: &mut Instant,
    last_bytes_marker: &mut u64,
) -> Result<u64, String> {
    let mut attempts = 0;
    let max_attempts = options.retry_count.max(1);

    loop {
        attempts += 1;
        let copy_res = if options.block_delta && dest.exists() {
            copy_stream_block_delta(src, dest, options, stats, task_manager, task_id, start_time, last_speed_calc, last_bytes_marker).await
        } else {
            copy_stream_chunked(src, dest, options, stats, task_manager, task_id, start_time, last_speed_calc, last_bytes_marker).await
        };

        match copy_res {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                if attempts >= max_attempts {
                    return Err(format!("Exceeded retry limit ({} attempts): {}", max_attempts, e));
                }
                warn!("Copy failed on attempt {}/{}, retrying in {}ms: {}", attempts, max_attempts, options.retry_delay_ms, e);
                tokio::time::sleep(Duration::from_millis(options.retry_delay_ms)).await;
            }
        }
    }
}

async fn copy_stream_block_delta(
    src: &Path,
    dest: &Path,
    options: &DeltaCopyOptions,
    stats: &mut DeltaCopyStats,
    task_manager: &Arc<TaskManager>,
    task_id: &str,
    _start_time: &Instant,
    last_speed_calc: &mut Instant,
    last_bytes_marker: &mut u64,
) -> Result<u64, std::io::Error> {
    let mut src_file = File::open(src)?;
    let src_meta = src_file.metadata()?;
    let src_len = src_meta.len();

    let mut dest_file = match OpenOptions::new().read(true).write(true).open(dest) {
        Ok(f) => f,
        Err(_) => return copy_stream_chunked(src, dest, options, stats, task_manager, task_id, _start_time, last_speed_calc, last_bytes_marker).await,
    };
    let dest_len = dest_file.metadata()?.len();

    const BLOCK_SIZE: usize = 256 * 1024; // 256 KB block size
    let mut src_buf = vec![0u8; BLOCK_SIZE];
    let mut dest_buf = vec![0u8; BLOCK_SIZE];

    let mut offset = 0u64;
    let mut actual_bytes_written = 0u64;

    while offset < src_len {
        let to_read = ((src_len - offset).min(BLOCK_SIZE as u64)) as usize;
        src_file.seek(SeekFrom::Start(offset))?;
        src_file.read_exact(&mut src_buf[..to_read])?;

        let is_block_match = if offset + (to_read as u64) <= dest_len {
            dest_file.seek(SeekFrom::Start(offset))?;
            match dest_file.read_exact(&mut dest_buf[..to_read]) {
                Ok(_) => {
                    let mut src_hasher = Crc32Hasher::new();
                    src_hasher.update(&src_buf[..to_read]);
                    let mut dest_hasher = Crc32Hasher::new();
                    dest_hasher.update(&dest_buf[..to_read]);
                    src_hasher.finalize() == dest_hasher.finalize()
                }
                Err(_) => false,
            }
        } else {
            false
        };

        if is_block_match {
            stats.blocks_matched += 1;
            stats.bytes_saved += to_read as u64;
        } else {
            dest_file.seek(SeekFrom::Start(offset))?;
            dest_file.write_all(&src_buf[..to_read])?;
            actual_bytes_written += to_read as u64;
            stats.blocks_transferred += 1;
        }

        offset += to_read as u64;
        stats.bytes_copied += to_read as u64;

        let now = Instant::now();
        if now.duration_since(*last_speed_calc).as_millis() >= 500 {
            let elapsed_sec = now.duration_since(*last_speed_calc).as_secs_f64();
            let delta_bytes = stats.bytes_copied.saturating_sub(*last_bytes_marker);
            let speed = if elapsed_sec > 0.0 { (delta_bytes as f64 / elapsed_sec) as u64 } else { 0 };

            stats.speed_bytes_per_sec = speed;
            if speed > 0 && stats.total_bytes > stats.bytes_copied {
                stats.eta_seconds = (stats.total_bytes - stats.bytes_copied) / speed;
            }

            task_manager.update_task_details(
                task_id,
                Some(&stats.current_file),
                offset,
                src_len,
                stats.files_copied as u64,
                stats.total_files as u64,
                stats.bytes_copied,
                speed,
                Some(stats.verified_count as u64),
                None,
                None,
            ).await;

            *last_speed_calc = now;
            *last_bytes_marker = stats.bytes_copied;
        }
    }

    // Truncate destination if source was smaller than existing destination
    if dest_len > src_len {
        dest_file.set_len(src_len)?;
    }

    dest_file.flush()?;

    // Preserve permissions
    if options.preserve_permissions {
        #[cfg(unix)]
        {
            let perm = src_meta.permissions().mode();
            let _ = fs::set_permissions(dest, fs::Permissions::from_mode(perm));
        }
        #[cfg(not(unix))]
        {
            let _ = fs::set_permissions(dest, src_meta.permissions());
        }
    }

    // Preserve timestamps
    if options.preserve_timestamps {
        if let Ok(mtime) = src_meta.modified() {
            let ft = FileTime::from_system_time(mtime);
            let _ = set_file_times(dest, ft, ft);
        }
    }

    Ok(actual_bytes_written)
}

async fn copy_stream_chunked(
    src: &Path,
    dest: &Path,
    options: &DeltaCopyOptions,
    stats: &mut DeltaCopyStats,
    task_manager: &Arc<TaskManager>,
    task_id: &str,
    _start_time: &Instant,
    last_speed_calc: &mut Instant,
    last_bytes_marker: &mut u64,
) -> Result<u64, std::io::Error> {
    let mut src_file = File::open(src)?;
    let src_meta = src_file.metadata()?;
    let src_len = src_meta.len();

    let mut resume_offset = 0u64;

    if options.resume_partial && dest.exists() {
        if let Ok(dest_meta) = dest.metadata() {
            if dest_meta.len() < src_len {
                resume_offset = dest_meta.len();
                src_file.seek(SeekFrom::Start(resume_offset))?;
            }
        }
    }

    let mut dest_file = if resume_offset > 0 {
        OpenOptions::new().append(true).open(dest)?
    } else {
        File::create(dest)?
    };

    let mut buffer = vec![0u8; 256 * 1024]; // 256 KB heap buffer
    let mut written = resume_offset;

    loop {
        let read_bytes = src_file.read(&mut buffer)?;
        if read_bytes == 0 {
            break;
        }

        dest_file.write_all(&buffer[..read_bytes])?;
        written += read_bytes as u64;
        stats.bytes_copied += read_bytes as u64;

        let now = Instant::now();
        if now.duration_since(*last_speed_calc).as_millis() >= 500 {
            let elapsed_sec = now.duration_since(*last_speed_calc).as_secs_f64();
            let delta_bytes = stats.bytes_copied.saturating_sub(*last_bytes_marker);
            let speed = if elapsed_sec > 0.0 { (delta_bytes as f64 / elapsed_sec) as u64 } else { 0 };

            stats.speed_bytes_per_sec = speed;
            if speed > 0 && stats.total_bytes > stats.bytes_copied {
                stats.eta_seconds = (stats.total_bytes - stats.bytes_copied) / speed;
            }

            task_manager.update_task_details(
                task_id,
                Some(&stats.current_file),
                written,
                src_len,
                stats.files_copied as u64,
                stats.total_files as u64,
                stats.bytes_copied,
                speed,
                Some(stats.verified_count as u64),
                None,
                None,
            ).await;

            *last_speed_calc = now;
            *last_bytes_marker = stats.bytes_copied;
        }
    }

    dest_file.flush()?;

    // Preserve permissions
    if options.preserve_permissions {
        #[cfg(unix)]
        {
            let perm = src_meta.permissions().mode();
            let _ = fs::set_permissions(dest, fs::Permissions::from_mode(perm));
        }
        #[cfg(not(unix))]
        {
            let _ = fs::set_permissions(dest, src_meta.permissions());
        }
    }

    // Preserve timestamps
    if options.preserve_timestamps {
        if let Ok(mtime) = src_meta.modified() {
            let ft = FileTime::from_system_time(mtime);
            let _ = set_file_times(dest, ft, ft);
        }
    }

    Ok(written)
}

fn verify_file_checksum(src: &Path, dest: &Path, algo: &str) -> bool {
    if algo.to_lowercase() == "crc32" {
        let mut h1 = Crc32Hasher::new();
        let mut h2 = Crc32Hasher::new();

        if let (Ok(mut f1), Ok(mut f2)) = (File::open(src), File::open(dest)) {
            let mut b1 = vec![0u8; 64 * 1024];
            let mut b2 = vec![0u8; 64 * 1024];

            while let Ok(n) = f1.read(&mut b1) {
                if n == 0 { break; }
                h1.update(&b1[..n]);
            }
            while let Ok(n) = f2.read(&mut b2) {
                if n == 0 { break; }
                h2.update(&b2[..n]);
            }
            return h1.finalize() == h2.finalize();
        }
    } else {
        let mut h1 = Sha256::new();
        let mut h2 = Sha256::new();

        if let (Ok(mut f1), Ok(mut f2)) = (File::open(src), File::open(dest)) {
            let mut b1 = vec![0u8; 64 * 1024];
            let mut b2 = vec![0u8; 64 * 1024];

            while let Ok(n) = f1.read(&mut b1) {
                if n == 0 { break; }
                h1.update(&b1[..n]);
            }
            while let Ok(n) = f2.read(&mut b2) {
                if n == 0 { break; }
                h2.update(&b2[..n]);
            }
            return h1.finalize() == h2.finalize();
        }
    }
    false
}
