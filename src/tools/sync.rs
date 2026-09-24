use crate::tools::deltacopy::{DeltaCopyEngine, DeltaCopyOptions};
use crate::tools::tasks::TaskManager;
use chrono::{Local, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tracing::{error, info};
use uuid::Uuid;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOptions {
    pub mode: String, // "synchronize", "echo", "contribute", "subscribe", "mirror_left_to_right", "mirror_right_to_left", "bidirectional_newer"
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub verify_checksum: bool,
    #[serde(default = "default_true")]
    pub block_delta: bool,
    #[serde(default)]
    pub delete_orphans: bool,
    #[serde(default)]
    pub archive_dir: Option<String>,
    #[serde(default)]
    pub retention_days: Option<u32>,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            mode: "synchronize".to_string(),
            dry_run: false,
            verify_checksum: true,
            block_delta: true,
            delete_orphans: false,
            archive_dir: Some("_archive".to_string()),
            retention_days: Some(30),
            exclusions: Vec::new(),
        }
    }
}

/// Checks if a relative path or file matches any configured exclusion patterns
pub fn matches_exclusion(rel_path: &Path, filename: &str, exclusions: &[String]) -> bool {
    if exclusions.is_empty() {
        return false;
    }
    let rel_str = rel_path.to_string_lossy().to_string();
    let rel_lower = rel_str.to_lowercase();
    let file_lower = filename.to_lowercase();

    for pattern in exclusions {
        let pat = pattern.trim();
        if pat.is_empty() {
            continue;
        }
        let pat_lower = pat.to_lowercase();
        let pat_clean = pat_lower.trim_start_matches('/').trim_end_matches('/');

        if file_lower == pat_clean || rel_lower == pat_clean {
            return true;
        }

        // Check path components (e.g. "node_modules", ".git", "target", ".cache")
        for comp in rel_path.components() {
            let comp_str = comp.as_os_str().to_string_lossy().to_lowercase();
            if comp_str == pat_clean {
                return true;
            }
        }

        // Wildcard extension matching (e.g. "*.tmp", "*.log", "*.bak", "*.iso")
        if pat_clean.starts_with("*.") {
            let ext = &pat_clean[1..];
            if file_lower.ends_with(ext) || rel_lower.ends_with(ext) {
                return true;
            }
        }

        // Wildcard prefix matching (e.g. "~*")
        if pat_clean.ends_with('*') && pat_clean.len() > 1 {
            let prefix = &pat_clean[..pat_clean.len() - 1];
            if file_lower.starts_with(prefix) {
                return true;
            }
        }

        // Substring / folder contains match
        if rel_lower.contains(pat_clean) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFileItem {
    pub rel_path: String,
    pub filename: String,
    pub status: String, // "left_only", "right_only", "identical", "modified_newer_left", "modified_newer_right", "size_conflict"
    pub src_size: u64,
    pub dest_size: u64,
    pub src_size_formatted: String,
    pub dest_size_formatted: String,
    pub src_mtime: u64,
    pub dest_mtime: u64,
    pub suggested_action: String, // "copy_right", "copy_left", "delta_update_right", "delta_update_left", "delete_right", "archive_and_update", "archive_and_delete", "identical", "skip"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncAnalysis {
    pub source_dir: String,
    pub dest_dir: String,
    pub mode: String,
    pub to_copy: Vec<String>,
    pub to_update: Vec<String>,
    pub to_delete: Vec<String>,
    pub to_archive: Vec<String>,
    pub identical_count: usize,
    pub left_only_count: usize,
    pub right_only_count: usize,
    pub modified_count: usize,
    pub archive_count: usize,
    pub total_transfer_bytes: u64,
    pub bytes_saved_estimate: u64,
    pub files: Vec<SyncFileItem>,
    #[serde(default)]
    pub is_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupProfile {
    pub id: String,
    pub name: String,
    pub source_dir: String,
    pub dest_dir: String,
    pub profile_mode: String, // "synchronize", "echo", "contribute", "subscribe"
    pub block_delta: bool,
    pub verify_checksum: bool,
    pub archive_dir: Option<String>,
    pub retention_days: u32,
    pub schedule_type: String, // "manual", "realtime", "interval", "daily"
    pub schedule_interval_mins: u32,
    pub schedule_time: Option<String>, // "02:00"
    pub webhook_url: Option<String>,
    pub enabled: bool,
    pub last_run: Option<i64>,
    pub last_status: Option<String>,
    pub last_result: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub exclusions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupHistoryItem {
    pub id: String,
    pub profile_id: String,
    pub profile_name: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String, // "success", "failed", "running"
    pub files_copied: u64,
    pub files_updated: u64,
    pub files_deleted: u64,
    pub files_archived: u64,
    pub bytes_transferred: u64,
    pub duration_ms: u64,
    pub error_message: Option<String>,
}

pub struct DirectorySyncEngine;

impl DirectorySyncEngine {
    pub fn analyze(source: &str, destination: &str, options: &SyncOptions) -> Result<SyncAnalysis, String> {
        let src_path = Path::new(source);
        let dest_path = Path::new(destination);

        if !src_path.exists() || !src_path.is_dir() {
            return Err(format!("Source directory does not exist: {}", source));
        }

        let mode_norm = normalize_mode(&options.mode);

        let mut to_copy = Vec::new();
        let mut to_update = Vec::new();
        let mut to_delete = Vec::new();
        let mut to_archive = Vec::new();
        let mut identical_count = 0;
        let mut left_only_count = 0;
        let mut right_only_count = 0;
        let mut modified_count = 0;
        let mut archive_count = 0;
        let mut total_transfer_bytes = 0u64;
        let mut bytes_saved_estimate = 0u64;
        let mut files = Vec::new();
        let mut is_truncated = false;
        const MAX_ANALYSIS_FILES: usize = 10_000;

        let mut src_rel_set: HashSet<PathBuf> = HashSet::new();

        // 1. Scan Source Files
        for entry in WalkDir::new(src_path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if files.len() >= MAX_ANALYSIS_FILES {
                    is_truncated = true;
                    break;
                }

                let rel = entry.path().strip_prefix(src_path).unwrap().to_path_buf();
                
                // Skip internal _archive directory if located in source
                if rel.starts_with("_archive") || rel.starts_with(".archive") {
                    continue;
                }

                let filename = entry.file_name().to_string_lossy().to_string();
                if matches_exclusion(&rel, &filename, &options.exclusions) {
                    continue;
                }

                src_rel_set.insert(rel.clone());

                let target_file = dest_path.join(&rel);
                let src_meta = entry.metadata().ok();
                let src_size = src_meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let src_mtime = src_meta.as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let rel_str = rel.to_string_lossy().to_string();

                if !target_file.exists() {
                    to_copy.push(rel_str.clone());
                    left_only_count += 1;
                    total_transfer_bytes += src_size;

                    files.push(SyncFileItem {
                        rel_path: rel_str,
                        filename,
                        status: "left_only".to_string(),
                        src_size,
                        dest_size: 0,
                        src_size_formatted: format_size(src_size),
                        dest_size_formatted: "-".to_string(),
                        src_mtime,
                        dest_mtime: 0,
                        suggested_action: "copy_right".to_string(),
                    });
                } else {
                    let dest_meta = target_file.metadata().ok();
                    let dest_size = dest_meta.as_ref().map(|m| m.len()).unwrap_or(0);
                    let dest_mtime = dest_meta.as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    let is_identical = src_size == dest_size && src_mtime.abs_diff(dest_mtime) < 2;

                    if is_identical {
                        identical_count += 1;
                        files.push(SyncFileItem {
                            rel_path: rel_str,
                            filename,
                            status: "identical".to_string(),
                            src_size,
                            dest_size,
                            src_size_formatted: format_size(src_size),
                            dest_size_formatted: format_size(dest_size),
                            src_mtime,
                            dest_mtime,
                            suggested_action: "identical".to_string(),
                        });
                    } else {
                        modified_count += 1;
                        to_update.push(rel_str.clone());
                        total_transfer_bytes += src_size;

                        if options.block_delta && src_size > 64 * 1024 {
                            // Rough savings estimate for block-delta transfers
                            bytes_saved_estimate += (src_size / 2).max(1024);
                        }

                        let status = if src_mtime > dest_mtime {
                            "modified_newer_left"
                        } else if dest_mtime > src_mtime {
                            "modified_newer_right"
                        } else {
                            "size_conflict"
                        };

                        let suggested_action = match mode_norm.as_str() {
                            "subscribe" => {
                                to_archive.push(rel_str.clone());
                                archive_count += 1;
                                "archive_and_update"
                            }
                            "synchronize" => {
                                if status == "modified_newer_right" {
                                    "delta_update_left"
                                } else {
                                    "delta_update_right"
                                }
                            }
                            _ => "delta_update_right",
                        };

                        files.push(SyncFileItem {
                            rel_path: rel_str,
                            filename,
                            status: status.to_string(),
                            src_size,
                            dest_size,
                            src_size_formatted: format_size(src_size),
                            dest_size_formatted: format_size(dest_size),
                            src_mtime,
                            dest_mtime,
                            suggested_action: suggested_action.to_string(),
                        });
                    }
                }
            }
        }

        // 2. Scan Destination for Right-Only Files
        if dest_path.exists() && dest_path.is_dir() && !is_truncated {
            for entry in WalkDir::new(dest_path).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    if files.len() >= MAX_ANALYSIS_FILES {
                        is_truncated = true;
                        break;
                    }

                    let rel = entry.path().strip_prefix(dest_path).unwrap().to_path_buf();
                    
                    // Skip internal _archive snapshot folder
                    if rel.starts_with("_archive") || rel.starts_with(".archive") {
                        continue;
                    }

                    let filename = entry.file_name().to_string_lossy().to_string();
                    if matches_exclusion(&rel, &filename, &options.exclusions) {
                        continue;
                    }

                    if !src_rel_set.contains(&rel) {
                        let rel_str = rel.to_string_lossy().to_string();
                        let dest_meta = entry.metadata().ok();
                        let dest_size = dest_meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let dest_mtime = dest_meta.as_ref()
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);

                        right_only_count += 1;

                        let suggested_action = match mode_norm.as_str() {
                            "subscribe" => {
                                to_archive.push(rel_str.clone());
                                archive_count += 1;
                                "archive_and_delete"
                            }
                            "echo" => {
                                to_delete.push(rel_str.clone());
                                "delete_right"
                            }
                            "synchronize" => "copy_left",
                            "mirror_right_to_left" => "copy_left",
                            "contribute" => "skip", // Keep destination orphan untouched
                            _ => {
                                if options.delete_orphans {
                                    to_delete.push(rel_str.clone());
                                    "delete_right"
                                } else {
                                    "skip"
                                }
                            }
                        };

                        files.push(SyncFileItem {
                            rel_path: rel_str,
                            filename,
                            status: "right_only".to_string(),
                            src_size: 0,
                            dest_size,
                            src_size_formatted: "-".to_string(),
                            dest_size_formatted: format_size(dest_size),
                            src_mtime: 0,
                            dest_mtime,
                            suggested_action: suggested_action.to_string(),
                        });
                    }
                }
            }
        }

        files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

        Ok(SyncAnalysis {
            source_dir: source.to_string(),
            dest_dir: destination.to_string(),
            mode: options.mode.clone(),
            to_copy,
            to_update,
            to_delete,
            to_archive,
            identical_count,
            left_only_count,
            right_only_count,
            modified_count,
            archive_count,
            total_transfer_bytes,
            bytes_saved_estimate,
            files,
            is_truncated,
        })
    }

    pub async fn execute_sync(
        task_manager: Arc<TaskManager>,
        source: &str,
        destination: &str,
        options: SyncOptions,
    ) -> Result<serde_json::Value, String> {
        let analysis = Self::analyze(source, destination, &options)?;

        if options.dry_run {
            return Ok(serde_json::to_value(&analysis).map_err(|e| e.to_string())?);
        }

        let start = Instant::now();
        let mode_norm = normalize_mode(&options.mode);
        let dest_p = Path::new(destination);
        let src_p = Path::new(source);

        let mut archived_count = 0u64;
        let mut deleted_count = 0u64;

        // 1. If Subscribe (Historical Versioning) mode, archive files into timestamped directory
        let mut snapshot_rel_dir = String::new();
        if mode_norm == "subscribe" && !analysis.to_archive.is_empty() {
            let timestamp_str = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
            let archive_sub = options.archive_dir.as_deref().unwrap_or("_archive");
            let archive_root = dest_p.join(archive_sub).join(&timestamp_str);
            snapshot_rel_dir = format!("{}/{}", archive_sub, timestamp_str);

            for rel in &analysis.to_archive {
                let target_f = dest_p.join(rel);
                if target_f.exists() {
                    let archive_dest = archive_root.join(rel);
                    if let Some(parent) = archive_dest.parent() {
                        fs::create_dir_all(parent).ok();
                    }
                    if fs::rename(&target_f, &archive_dest).is_ok() || fs::copy(&target_f, &archive_dest).is_ok() {
                        archived_count += 1;
                        if analysis.to_delete.contains(rel) {
                            let _ = fs::remove_file(&target_f);
                        }
                    }
                }
            }

            // Prune old archive directories if retention_days configured
            if let Some(days) = options.retention_days {
                if days > 0 {
                    let prune_limit = Utc::now().timestamp() - (days as i64 * 86400);
                    let archive_base = dest_p.join(archive_sub);
                    if let Ok(entries) = fs::read_dir(&archive_base) {
                        for ent in entries.filter_map(|e| e.ok()) {
                            if let Ok(meta) = ent.metadata() {
                                if meta.is_dir() {
                                    if let Ok(mtime) = meta.modified() {
                                        if let Ok(dur) = mtime.duration_since(UNIX_EPOCH) {
                                            if (dur.as_secs() as i64) < prune_limit {
                                                let _ = fs::remove_dir_all(ent.path());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Delete orphan files if Echo / Mirror mode
        if (mode_norm == "echo" || mode_norm.contains("mirror") || options.delete_orphans) && mode_norm != "subscribe" {
            for rel in &analysis.to_delete {
                let p = dest_p.join(rel);
                if fs::remove_file(&p).is_ok() {
                    deleted_count += 1;
                }
            }
        }

        // 3. Perform DeltaCopy from Source to Destination for to_copy and to_update items
        let delta_opts = DeltaCopyOptions {
            overwrite_mode: if mode_norm.contains("mirror") || mode_norm == "echo" {
                "delta_mtime_size".to_string()
            } else {
                "if_newer".to_string()
            },
            block_delta: options.block_delta,
            verify_checksum: options.verify_checksum,
            verify_algo: "crc32".to_string(),
            retry_count: 3,
            retry_delay_ms: 500,
            resume_partial: true,
            preserve_timestamps: true,
            preserve_permissions: true,
            delete_orphans: false,
        };

        let forward_pairs: Vec<(PathBuf, PathBuf)> = analysis.files.iter()
            .filter(|f| f.suggested_action == "copy_right" || f.suggested_action == "delta_update_right" || f.suggested_action == "archive_and_update")
            .map(|f| (src_p.join(&f.rel_path), dest_p.join(&f.rel_path)))
            .collect();

        let cancel_token = Arc::new(AtomicBool::new(false));
        let job_label = format!("{} ➔ {}", source, destination);

        let copy_stats = DeltaCopyEngine::run_deltacopy_pairs(
            task_manager.clone(),
            forward_pairs,
            &job_label,
            delta_opts.clone(),
            cancel_token.clone(),
        ).await?;

        // 4. If Two-Way Sync, replicate files from Destination back to Source that are Right-Only or newer on Right
        let mut reverse_copied = 0usize;
        if mode_norm == "synchronize" || mode_norm == "bidirectional_newer" {
            let reverse_pairs: Vec<(PathBuf, PathBuf)> = analysis.files.iter()
                .filter(|f| f.suggested_action == "copy_left" || f.suggested_action == "delta_update_left")
                .map(|f| (dest_p.join(&f.rel_path), src_p.join(&f.rel_path)))
                .collect();

            if !reverse_pairs.is_empty() {
                let rev_label = format!("{} ➔ {}", destination, source);
                let rev_stats = DeltaCopyEngine::run_deltacopy_pairs(
                    task_manager,
                    reverse_pairs,
                    &rev_label,
                    delta_opts,
                    cancel_token,
                ).await?;
                reverse_copied = rev_stats.files_copied;
            }
        }

        let duration_ms = start.elapsed().as_millis();

        Ok(serde_json::json!({
            "success": true,
            "mode": mode_norm,
            "copied": copy_stats.files_copied + reverse_copied,
            "skipped": copy_stats.files_skipped,
            "deleted": deleted_count,
            "archived": archived_count,
            "verified": copy_stats.verified_count,
            "blocks_matched": copy_stats.blocks_matched,
            "bytes_saved": copy_stats.bytes_saved,
            "snapshot_dir": snapshot_rel_dir,
            "duration_ms": duration_ms,
        }))
    }
}

fn normalize_mode(mode: &str) -> String {
    match mode.to_lowercase().as_str() {
        "synchronize" | "sync" | "bidirectional_newer" | "2way" => "synchronize".to_string(),
        "echo" | "mirror" | "mirror_left_to_right" | "1way" => "echo".to_string(),
        "contribute" | "additive" => "contribute".to_string(),
        "subscribe" | "archive" | "versioning" => "subscribe".to_string(),
        "mirror_right_to_left" => "mirror_right_to_left".to_string(),
        other => other.to_string(),
    }
}

fn format_size(bytes: u64) -> String {
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

// ------------------------------------------------------------------------------------------------
// Backup Profiles & Background Automation Manager
// ------------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct BackupManager {
    db: Arc<Mutex<Connection>>,
}

impl BackupManager {
    pub fn new(db: Arc<Mutex<Connection>>) -> Result<Self, String> {
        let conn = db.lock().map_err(|e| e.to_string())?;
        
        conn.execute(
            "CREATE TABLE IF NOT EXISTS backup_profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source_dir TEXT NOT NULL,
                dest_dir TEXT NOT NULL,
                profile_mode TEXT NOT NULL,
                block_delta INTEGER DEFAULT 1,
                verify_checksum INTEGER DEFAULT 1,
                archive_dir TEXT,
                retention_days INTEGER DEFAULT 30,
                schedule_type TEXT DEFAULT 'manual',
                schedule_interval_mins INTEGER DEFAULT 60,
                schedule_time TEXT,
                webhook_url TEXT,
                enabled INTEGER DEFAULT 1,
                last_run INTEGER,
                last_status TEXT,
                last_result TEXT,
                created_at INTEGER NOT NULL,
                exclusions TEXT
            )",
            [],
        ).map_err(|e| format!("Failed to create backup_profiles table: {}", e))?;

        // Migration: add exclusions column if it didn't exist in older databases
        let _ = conn.execute("ALTER TABLE backup_profiles ADD COLUMN exclusions TEXT", []);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS backup_history (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                profile_name TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                status TEXT NOT NULL,
                files_copied INTEGER DEFAULT 0,
                files_updated INTEGER DEFAULT 0,
                files_deleted INTEGER DEFAULT 0,
                files_archived INTEGER DEFAULT 0,
                bytes_transferred INTEGER DEFAULT 0,
                duration_ms INTEGER DEFAULT 0,
                error_message TEXT
            )",
            [],
        ).map_err(|e| format!("Failed to create backup_history table: {}", e))?;

        drop(conn);
        Ok(Self { db })
    }

    pub fn list_profiles(&self) -> Vec<BackupProfile> {
        let conn = match self.db.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let mut stmt = match conn.prepare(
            "SELECT id, name, source_dir, dest_dir, profile_mode, block_delta, verify_checksum, archive_dir, retention_days, schedule_type, schedule_interval_mins, schedule_time, webhook_url, enabled, last_run, last_status, last_result, created_at, exclusions FROM backup_profiles ORDER BY created_at DESC"
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = stmt.query_map([], |row| {
            Ok(BackupProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                source_dir: row.get(2)?,
                dest_dir: row.get(3)?,
                profile_mode: row.get(4)?,
                block_delta: row.get::<_, i32>(5)? == 1,
                verify_checksum: row.get::<_, i32>(6)? == 1,
                archive_dir: row.get(7)?,
                retention_days: row.get::<_, i32>(8)? as u32,
                schedule_type: row.get(9)?,
                schedule_interval_mins: row.get::<_, i32>(10)? as u32,
                schedule_time: row.get(11)?,
                webhook_url: row.get(12)?,
                enabled: row.get::<_, i32>(13)? == 1,
                last_run: row.get(14)?,
                last_status: row.get(15)?,
                last_result: row.get(16)?,
                created_at: row.get(17)?,
                exclusions: row.get(18).ok().flatten(),
            })
        });

        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn get_profile(&self, id: &str) -> Option<BackupProfile> {
        let conn = self.db.lock().ok()?;
        conn.query_row(
            "SELECT id, name, source_dir, dest_dir, profile_mode, block_delta, verify_checksum, archive_dir, retention_days, schedule_type, schedule_interval_mins, schedule_time, webhook_url, enabled, last_run, last_status, last_result, created_at, exclusions FROM backup_profiles WHERE id = ?1",
            params![id],
            |row| {
                Ok(BackupProfile {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    source_dir: row.get(2)?,
                    dest_dir: row.get(3)?,
                    profile_mode: row.get(4)?,
                    block_delta: row.get::<_, i32>(5)? == 1,
                    verify_checksum: row.get::<_, i32>(6)? == 1,
                    archive_dir: row.get(7)?,
                    retention_days: row.get::<_, i32>(8)? as u32,
                    schedule_type: row.get(9)?,
                    schedule_interval_mins: row.get::<_, i32>(10)? as u32,
                    schedule_time: row.get(11)?,
                    webhook_url: row.get(12)?,
                    enabled: row.get::<_, i32>(13)? == 1,
                    last_run: row.get(14)?,
                    last_status: row.get(15)?,
                    last_result: row.get(16)?,
                    created_at: row.get(17)?,
                    exclusions: row.get(18).ok().flatten(),
                })
            }
        ).optional().ok().flatten()
    }

    pub fn save_profile(&self, mut profile: BackupProfile) -> Result<BackupProfile, String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        if profile.id.trim().is_empty() {
            profile.id = Uuid::new_v4().to_string();
        }
        if profile.created_at == 0 {
            profile.created_at = Utc::now().timestamp();
        }

        conn.execute(
            "INSERT INTO backup_profiles (id, name, source_dir, dest_dir, profile_mode, block_delta, verify_checksum, archive_dir, retention_days, schedule_type, schedule_interval_mins, schedule_time, webhook_url, enabled, last_run, last_status, last_result, created_at, exclusions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                source_dir = excluded.source_dir,
                dest_dir = excluded.dest_dir,
                profile_mode = excluded.profile_mode,
                block_delta = excluded.block_delta,
                verify_checksum = excluded.verify_checksum,
                archive_dir = excluded.archive_dir,
                retention_days = excluded.retention_days,
                schedule_type = excluded.schedule_type,
                schedule_interval_mins = excluded.schedule_interval_mins,
                schedule_time = excluded.schedule_time,
                webhook_url = excluded.webhook_url,
                enabled = excluded.enabled,
                exclusions = excluded.exclusions",
            params![
                profile.id,
                profile.name,
                profile.source_dir,
                profile.dest_dir,
                profile.profile_mode,
                if profile.block_delta { 1 } else { 0 },
                if profile.verify_checksum { 1 } else { 0 },
                profile.archive_dir,
                profile.retention_days as i32,
                profile.schedule_type,
                profile.schedule_interval_mins as i32,
                profile.schedule_time,
                profile.webhook_url,
                if profile.enabled { 1 } else { 0 },
                profile.last_run,
                profile.last_status,
                profile.last_result,
                profile.created_at,
                profile.exclusions,
            ],
        ).map_err(|e| format!("Failed to save backup profile: {}", e))?;

        Ok(profile)
    }

    pub fn delete_profile(&self, id: &str) -> Result<(), String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM backup_profiles WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete backup profile: {}", e))?;
        Ok(())
    }

    pub fn toggle_profile(&self, id: &str) -> Result<bool, String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        let current_state: Option<i32> = conn.query_row(
            "SELECT enabled FROM backup_profiles WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ).optional().map_err(|e| e.to_string())?;

        let new_state = match current_state {
            Some(1) => 0,
            _ => 1,
        };

        conn.execute(
            "UPDATE backup_profiles SET enabled = ?1 WHERE id = ?2",
            params![new_state, id],
        ).map_err(|e| e.to_string())?;

        Ok(new_state == 1)
    }

    pub fn list_history(&self, profile_id: Option<&str>, limit: usize) -> Vec<BackupHistoryItem> {
        let conn = match self.db.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let lim = limit.min(100).max(1);

        if let Some(pid) = profile_id {
            let mut stmt = match conn.prepare(
                "SELECT id, profile_id, profile_name, started_at, finished_at, status, files_copied, files_updated, files_deleted, files_archived, bytes_transferred, duration_ms, error_message FROM backup_history WHERE profile_id = ?1 ORDER BY started_at DESC LIMIT ?2"
            ) {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };

            let rows = stmt.query_map(params![pid, lim as i64], |row| {
                Ok(BackupHistoryItem {
                    id: row.get(0)?,
                    profile_id: row.get(1)?,
                    profile_name: row.get(2)?,
                    started_at: row.get(3)?,
                    finished_at: row.get(4)?,
                    status: row.get(5)?,
                    files_copied: row.get::<_, i64>(6)? as u64,
                    files_updated: row.get::<_, i64>(7)? as u64,
                    files_deleted: row.get::<_, i64>(8)? as u64,
                    files_archived: row.get::<_, i64>(9)? as u64,
                    bytes_transferred: row.get::<_, i64>(10)? as u64,
                    duration_ms: row.get::<_, i64>(11)? as u64,
                    error_message: row.get(12)?,
                })
            });

            match rows {
                Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
                Err(_) => Vec::new(),
            }
        } else {
            let mut stmt = match conn.prepare(
                "SELECT id, profile_id, profile_name, started_at, finished_at, status, files_copied, files_updated, files_deleted, files_archived, bytes_transferred, duration_ms, error_message FROM backup_history ORDER BY started_at DESC LIMIT ?1"
            ) {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };

            let rows = stmt.query_map(params![lim as i64], |row| {
                Ok(BackupHistoryItem {
                    id: row.get(0)?,
                    profile_id: row.get(1)?,
                    profile_name: row.get(2)?,
                    started_at: row.get(3)?,
                    finished_at: row.get(4)?,
                    status: row.get(5)?,
                    files_copied: row.get::<_, i64>(6)? as u64,
                    files_updated: row.get::<_, i64>(7)? as u64,
                    files_deleted: row.get::<_, i64>(8)? as u64,
                    files_archived: row.get::<_, i64>(9)? as u64,
                    bytes_transferred: row.get::<_, i64>(10)? as u64,
                    duration_ms: row.get::<_, i64>(11)? as u64,
                    error_message: row.get(12)?,
                })
            });

            match rows {
                Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
                Err(_) => Vec::new(),
            }
        }
    }

    pub fn record_history(&self, item: &BackupHistoryItem) -> Result<(), String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO backup_history (id, profile_id, profile_name, started_at, finished_at, status, files_copied, files_updated, files_deleted, files_archived, bytes_transferred, duration_ms, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                item.id,
                item.profile_id,
                item.profile_name,
                item.started_at,
                item.finished_at,
                item.status,
                item.files_copied as i64,
                item.files_updated as i64,
                item.files_deleted as i64,
                item.files_archived as i64,
                item.bytes_transferred as i64,
                item.duration_ms as i64,
                item.error_message,
            ],
        ).map_err(|e| format!("Failed to record backup history: {}", e))?;

        // Also update last_run on profile
        let result_summary = format!("Status: {}, Copied: {}, Archived: {}", item.status, item.files_copied, item.files_archived);
        conn.execute(
            "UPDATE backup_profiles SET last_run = ?1, last_status = ?2, last_result = ?3 WHERE id = ?4",
            params![item.started_at, item.status, result_summary, item.profile_id],
        ).ok();

        Ok(())
    }

    pub async fn execute_profile_job(
        &self,
        profile_id: &str,
        task_manager: Arc<TaskManager>,
    ) -> Result<serde_json::Value, String> {
        let profile = self.get_profile(profile_id)
            .ok_or_else(|| format!("Backup profile not found: {}", profile_id))?;

        let run_id = Uuid::new_v4().to_string();
        let started_at = Utc::now().timestamp();

        let exclusions: Vec<String> = profile.exclusions
            .as_deref()
            .unwrap_or("")
            .split([',', '\n', ';'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let sync_options = SyncOptions {
            mode: profile.profile_mode.clone(),
            dry_run: false,
            verify_checksum: profile.verify_checksum,
            block_delta: profile.block_delta,
            delete_orphans: profile.profile_mode == "echo",
            archive_dir: profile.archive_dir.clone().or_else(|| Some("_archive".to_string())),
            retention_days: Some(profile.retention_days),
            exclusions,
        };

        info!("Starting backup profile execution: '{}' ({})", profile.name, profile.profile_mode);

        let exec_res = DirectorySyncEngine::execute_sync(
            task_manager,
            &profile.source_dir,
            &profile.dest_dir,
            sync_options,
        ).await;

        let finished_at = Utc::now().timestamp();

        match exec_res {
            Ok(val) => {
                let copied = val.get("copied").and_then(|v| v.as_u64()).unwrap_or(0);
                let deleted = val.get("deleted").and_then(|v| v.as_u64()).unwrap_or(0);
                let archived = val.get("archived").and_then(|v| v.as_u64()).unwrap_or(0);
                let duration_ms = val.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);

                let history = BackupHistoryItem {
                    id: run_id,
                    profile_id: profile.id.clone(),
                    profile_name: profile.name.clone(),
                    started_at,
                    finished_at: Some(finished_at),
                    status: "success".to_string(),
                    files_copied: copied,
                    files_updated: 0,
                    files_deleted: deleted,
                    files_archived: archived,
                    bytes_transferred: 0,
                    duration_ms,
                    error_message: None,
                };

                let _ = self.record_history(&history);

                // Dispatch webhook if configured
                if let Some(webhook) = &profile.webhook_url {
                    if !webhook.trim().is_empty() {
                        let client = reqwest::Client::new();
                        let payload = serde_json::json!({
                            "event": "backup_completed",
                            "profile_id": profile.id,
                            "profile_name": profile.name,
                            "profile_mode": profile.profile_mode,
                            "status": "success",
                            "files_copied": copied,
                            "files_deleted": deleted,
                            "files_archived": archived,
                            "duration_ms": duration_ms,
                            "timestamp": finished_at
                        });
                        let _ = client.post(webhook).json(&payload).timeout(Duration::from_secs(5)).send().await;
                    }
                }

                Ok(val)
            }
            Err(e) => {
                let history = BackupHistoryItem {
                    id: run_id,
                    profile_id: profile.id.clone(),
                    profile_name: profile.name.clone(),
                    started_at,
                    finished_at: Some(finished_at),
                    status: "failed".to_string(),
                    files_copied: 0,
                    files_updated: 0,
                    files_deleted: 0,
                    files_archived: 0,
                    bytes_transferred: 0,
                    duration_ms: (finished_at - started_at) as u64 * 1000,
                    error_message: Some(e.clone()),
                };

                let _ = self.record_history(&history);

                if let Some(webhook) = &profile.webhook_url {
                    if !webhook.trim().is_empty() {
                        let client = reqwest::Client::new();
                        let payload = serde_json::json!({
                            "event": "backup_failed",
                            "profile_id": profile.id,
                            "profile_name": profile.name,
                            "status": "failed",
                            "error": e,
                            "timestamp": finished_at
                        });
                        let _ = client.post(webhook).json(&payload).timeout(Duration::from_secs(5)).send().await;
                    }
                }

                Err(e)
            }
        }
    }

    /// Spawns background scheduler checking interval & daily triggers every 15 seconds
    pub fn start_scheduler(self: Arc<Self>, task_manager: Arc<TaskManager>) {
        tokio::spawn(async move {
            info!("⚡ Backup & Sync scheduler daemon started");
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            let mut last_minute_marker = String::new();

            loop {
                interval.tick().await;

                let now_ts = Utc::now().timestamp();
                let now_local = Local::now();
                let current_time_str = now_local.format("%H:%M").to_string();

                let profiles = self.list_profiles();
                for profile in profiles {
                    if !profile.enabled {
                        continue;
                    }

                    let should_run = match profile.schedule_type.as_str() {
                        "interval" => {
                            let interval_sec = (profile.schedule_interval_mins.max(1) as i64) * 60;
                            let last = profile.last_run.unwrap_or(0);
                            now_ts >= (last + interval_sec)
                        }
                        "daily" => {
                            if let Some(sched_time) = &profile.schedule_time {
                                sched_time == &current_time_str && current_time_str != last_minute_marker
                            } else {
                                false
                            }
                        }
                        "realtime" => {
                            // Run periodically (every 2 minutes) if realtime mode is selected
                            let last = profile.last_run.unwrap_or(0);
                            now_ts >= (last + 120)
                        }
                        _ => false,
                    };

                    if should_run {
                        info!("Triggering automated backup profile: '{}'", profile.name);
                        let mgr = self.clone();
                        let tm = task_manager.clone();
                        let pid = profile.id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = mgr.execute_profile_job(&pid, tm).await {
                                error!("Scheduled backup job '{}' error: {}", pid, e);
                            }
                        });
                    }
                }

                last_minute_marker = current_time_str;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_sync_replication_profiles() {
        let temp = tempdir().unwrap();
        let src_dir = temp.path().join("source");
        let dest_dir = temp.path().join("dest");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();

        // 1. Setup initial files
        fs::write(src_dir.join("file_a.txt"), "Content A").unwrap();
        fs::write(src_dir.join("file_b.txt"), "Content B v1").unwrap();
        fs::write(dest_dir.join("orphan.txt"), "Orphan Dest File").unwrap();

        let task_manager = Arc::new(TaskManager::new());

        // Test Echo / Mirror (should delete orphan)
        let echo_opts = SyncOptions {
            mode: "echo".to_string(),
            dry_run: false,
            verify_checksum: true,
            block_delta: true,
            delete_orphans: true,
            archive_dir: None,
            retention_days: None,
            exclusions: vec!["*.tmp".to_string(), "node_modules".to_string()],
        };

        let res = DirectorySyncEngine::execute_sync(
            task_manager.clone(),
            src_dir.to_str().unwrap(),
            dest_dir.to_str().unwrap(),
            echo_opts,
        ).await.unwrap();

        assert_eq!(res.get("success").unwrap(), true);
        assert!(dest_dir.join("file_a.txt").exists());
        assert!(dest_dir.join("file_b.txt").exists());
        assert!(!dest_dir.join("orphan.txt").exists()); // Echo mode deletes orphans

        // Test Subscribe / Archive (should archive modified files before overwriting)
        fs::write(src_dir.join("file_b.txt"), "Content B v2 (Updated)").unwrap();
        fs::write(dest_dir.join("orphan2.txt"), "Orphan Dest 2").unwrap();

        let sub_opts = SyncOptions {
            mode: "subscribe".to_string(),
            dry_run: false,
            verify_checksum: true,
            block_delta: true,
            delete_orphans: false,
            archive_dir: Some("_archive".to_string()),
            retention_days: Some(30),
            exclusions: Vec::new(),
        };

        let sub_res = DirectorySyncEngine::execute_sync(
            task_manager.clone(),
            src_dir.to_str().unwrap(),
            dest_dir.to_str().unwrap(),
            sub_opts,
        ).await.unwrap();

        assert_eq!(sub_res.get("success").unwrap(), true);
        assert_eq!(fs::read_to_string(dest_dir.join("file_b.txt")).unwrap(), "Content B v2 (Updated)");
        
        // Verify archive directory was created with snapshot
        let archive_base = dest_dir.join("_archive");
        assert!(archive_base.exists());
        let entries: Vec<_> = fs::read_dir(&archive_base).unwrap().filter_map(|e| e.ok()).collect();
        assert!(!entries.is_empty(), "Snapshot folder must exist in _archive");
    }

    #[tokio::test]
    async fn test_backup_manager_crud() {
        let conn = Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(conn));
        let manager = BackupManager::new(db).unwrap();

        let profile = BackupProfile {
            id: "test-profile-1".to_string(),
            name: "Daily Work Backup".to_string(),
            source_dir: "/home/user/work".to_string(),
            dest_dir: "/mnt/backup/work".to_string(),
            profile_mode: "subscribe".to_string(),
            block_delta: true,
            verify_checksum: true,
            archive_dir: Some("_archive".to_string()),
            retention_days: 14,
            schedule_type: "daily".to_string(),
            schedule_interval_mins: 60,
            schedule_time: Some("02:00".to_string()),
            webhook_url: Some("https://discord.com/api/webhooks/test".to_string()),
            enabled: true,
            last_run: None,
            last_status: None,
            last_result: None,
            created_at: 1725130000,
            exclusions: Some("node_modules, .git, target".to_string()),
        };

        // Save
        let saved = manager.save_profile(profile).unwrap();
        assert_eq!(saved.name, "Daily Work Backup");

        // List
        let list = manager.list_profiles();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "test-profile-1");

        // Toggle
        let new_state = manager.toggle_profile("test-profile-1").unwrap();
        assert_eq!(new_state, false);

        // Record history
        let hist = BackupHistoryItem {
            id: "hist-1".to_string(),
            profile_id: "test-profile-1".to_string(),
            profile_name: "Daily Work Backup".to_string(),
            started_at: 1725134000,
            finished_at: Some(1725134010),
            status: "success".to_string(),
            files_copied: 25,
            files_updated: 5,
            files_deleted: 0,
            files_archived: 5,
            bytes_transferred: 1048576,
            duration_ms: 10000,
            error_message: None,
        };
        manager.record_history(&hist).unwrap();

        let history_list = manager.list_history(Some("test-profile-1"), 10);
        assert_eq!(history_list.len(), 1);
        assert_eq!(history_list[0].files_copied, 25);
        assert_eq!(history_list[0].files_archived, 5);

        // Delete
        manager.delete_profile("test-profile-1").unwrap();
        assert_eq!(manager.list_profiles().len(), 0);
    }
}
