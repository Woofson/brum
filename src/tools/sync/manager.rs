//! Saved backups, schedules, run history with per-file logs, and restore.

use super::engine::{self, JobSpec, LogEntry, Mode, Progress, RunCounts, STAMP_FORMAT, VERSIONS_DIR};
use super::exclude::Exclusions;
use super::store::{join_rel, safe_rel, DirItem, FileMeta, Listing, LocalStore, Store, PART_SUFFIX};
use crate::tools::tasks::TaskManager;
use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use uuid::Uuid;

fn default_true() -> bool {
    true
}
fn default_mode() -> String {
    "versions".into()
}
fn default_retention() -> u32 {
    30
}
fn default_log_days() -> u32 {
    90
}
fn default_schedule() -> String {
    "manual".into()
}
fn default_interval() -> u32 {
    5
}
fn default_transfer() -> String {
    "builtin".into()
}

/// A saved backup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupProfile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// The (first) folder to back up
    pub source_dir: String,
    /// All folders to back up; each gets its own subfolder in the backup when there are several
    #[serde(default)]
    pub sources: Vec<String>,
    pub dest_dir: String,
    /// "copy", "versions" or "two_way"
    #[serde(default = "default_mode")]
    pub profile_mode: String,
    /// Never remove files from the backup, even when they are gone from the source
    #[serde(default)]
    pub never_remove: bool,
    /// Rewrite only the changed parts of large files
    #[serde(default = "default_true")]
    pub block_delta: bool,
    /// Read each copy back and compare it with the original
    #[serde(default = "default_true")]
    pub verify_checksum: bool,
    /// Folder at the destination that keeps old versions; empty means `.brum-versions`
    #[serde(default)]
    pub archive_dir: Option<String>,
    /// Days to keep old versions; 0 keeps them forever
    #[serde(default = "default_retention")]
    pub retention_days: u32,
    /// "manual", "interval" or "daily"
    #[serde(default = "default_schedule")]
    pub schedule_type: String,
    #[serde(default = "default_interval")]
    pub schedule_interval_mins: u32,
    /// "HH:MM" for daily backups
    #[serde(default)]
    pub schedule_time: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: Option<i64>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub last_result: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub exclusions: Option<String>,
    /// Brum user who made it; backups from before owners existed are admin-only
    #[serde(default)]
    pub owner: Option<String>,
    /// Days to keep run logs
    #[serde(default = "default_log_days")]
    pub log_retention_days: u32,
    /// When the last run started (used by the schedule, so a running backup is never started twice)
    #[serde(default)]
    pub last_started: Option<i64>,
    /// Address of the Fleet machine for fleet:// destinations; set by the server when saving
    #[serde(default, skip_deserializing)]
    pub dest_endpoint: Option<String>,
    /// Who does the copying: "builtin" (Brum) or "rsync"
    #[serde(default = "default_transfer")]
    pub transfer: String,
    /// Password for a remote source (SFTP). Accepted from the browser, never sent back.
    #[serde(default, skip_serializing)]
    pub source_password: Option<String>,
    /// Password for a remote destination (SFTP). Accepted from the browser, never sent back.
    #[serde(default, skip_serializing)]
    pub dest_password: Option<String>,
}

impl BackupProfile {
    /// Every folder this backup copies from
    pub fn all_sources(&self) -> Vec<String> {
        let list: Vec<String> = self.sources.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if list.is_empty() { vec![self.source_dir.clone()] } else { list }
    }
    pub fn is_multi_source(&self) -> bool {
        self.all_sources().len() > 1
    }
    pub fn mode(&self) -> Mode {
        Mode::parse(&self.profile_mode).0
    }
    pub fn versions_dir(&self) -> String {
        self.archive_dir
            .as_deref()
            .map(|d| d.trim().trim_matches('/'))
            .filter(|d| !d.is_empty() && !d.contains(".."))
            .unwrap_or(VERSIONS_DIR)
            .to_string()
    }
}

/// One backup or restore run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub profile_id: String,
    pub profile_name: String,
    /// "backup" or "restore"
    pub kind: String,
    /// "manual" or "schedule"
    pub trigger: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// "running", "ok", "warning", "failed", "cancelled", "nothing" (already up to date)
    pub status: String,
    #[serde(flatten)]
    pub counts: RunCounts,
    pub warnings: u64,
    pub duration_ms: u64,
    /// Plain one-line summary, or why it failed
    pub message: Option<String>,
}

/// One line of a run's per-file log
#[derive(Debug, Clone, Serialize)]
pub struct RunFile {
    pub seq: i64,
    pub path: String,
    pub action: String,
    pub size: u64,
    pub ok: bool,
    pub detail: Option<String>,
}

/// What is running right now for a backup
#[derive(Debug, Clone, Serialize)]
pub struct RunningInfo {
    pub run_id: String,
    pub kind: String,
    pub started_at: i64,
    #[serde(flatten)]
    pub progress: Progress,
}

struct Running {
    run_id: String,
    kind: String,
    started_at: i64,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
}

/// A backup as the window shows it
#[derive(Debug, Clone, Serialize)]
pub struct ProfileView {
    #[serde(flatten)]
    pub profile: BackupProfile,
    /// A password is saved for the remote source / destination (the password itself is never sent)
    pub has_source_password: bool,
    pub has_dest_password: bool,
    pub running: Option<RunningInfo>,
    pub next_run: Option<i64>,
    pub last: Option<RunRecord>,
    pub last_success_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewResult {
    pub summary: engine::PlanSummary,
    pub items: Vec<engine::PlanItem>,
    pub total_items: usize,
    pub warnings: Vec<String>,
    pub total_warnings: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestorePoint {
    /// "latest" or the dated folder name
    pub id: String,
    pub label: String,
    pub when: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RestoreRequest {
    #[serde(default = "latest")]
    pub point: String,
    /// Files and folders inside the backup; empty restores everything
    #[serde(default)]
    pub items: Vec<String>,
    /// "separate" (default), "original" or "folder"
    #[serde(default = "separate")]
    pub target: String,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub confirm_overwrite: bool,
    #[serde(default)]
    pub dry_run: bool,
}
fn latest() -> String {
    "latest".into()
}
fn separate() -> String {
    "separate".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct RestorePreview {
    pub files: u64,
    pub bytes: u64,
    /// Existing files at the target that would be replaced
    pub overwrites: u64,
    pub target_path: String,
    pub needs_confirmation: bool,
    pub run_id: Option<String>,
}

const PROFILE_COLUMNS: &str = "id, name, source_dir, dest_dir, profile_mode, block_delta, verify_checksum, archive_dir, retention_days, schedule_type, schedule_interval_mins, schedule_time, webhook_url, enabled, last_run, last_status, last_result, created_at, exclusions, owner, never_remove, log_retention_days, last_started, source_password, dest_password, transfer, dest_endpoint, sources";
const RUN_COLUMNS: &str = "id, profile_id, profile_name, kind, trigger, started_at, finished_at, status, copied, updated, removed, versions_kept, failed, bytes, warnings, duration_ms, message";

fn profile_from_row(row: &rusqlite::Row) -> rusqlite::Result<BackupProfile> {
    Ok(BackupProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        source_dir: row.get(2)?,
        dest_dir: row.get(3)?,
        profile_mode: row.get(4)?,
        block_delta: row.get::<_, i64>(5)? == 1,
        verify_checksum: row.get::<_, i64>(6)? == 1,
        archive_dir: row.get(7)?,
        retention_days: row.get::<_, i64>(8)?.max(0) as u32,
        schedule_type: row.get(9)?,
        schedule_interval_mins: row.get::<_, i64>(10)?.max(1) as u32,
        schedule_time: row.get(11)?,
        webhook_url: row.get(12)?,
        enabled: row.get::<_, i64>(13)? == 1,
        last_run: row.get(14)?,
        last_status: row.get(15)?,
        last_result: row.get(16)?,
        created_at: row.get(17)?,
        exclusions: row.get(18)?,
        owner: row.get(19)?,
        never_remove: row.get::<_, Option<i64>>(20)?.unwrap_or(0) == 1,
        log_retention_days: row.get::<_, Option<i64>>(21)?.unwrap_or(90).max(1) as u32,
        last_started: row.get(22)?,
        source_password: row.get(23)?,
        dest_password: row.get(24)?,
        transfer: row.get::<_, Option<String>>(25)?.unwrap_or_else(default_transfer),
        dest_endpoint: row.get(26)?,
        sources: row.get::<_, Option<String>>(27)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default(),
    })
}

fn run_from_row(row: &rusqlite::Row) -> rusqlite::Result<RunRecord> {
    let n = |i: usize| -> rusqlite::Result<u64> { Ok(row.get::<_, Option<i64>>(i)?.unwrap_or(0).max(0) as u64) };
    Ok(RunRecord {
        id: row.get(0)?,
        profile_id: row.get(1)?,
        profile_name: row.get(2)?,
        kind: row.get(3)?,
        trigger: row.get(4)?,
        started_at: row.get(5)?,
        finished_at: row.get(6)?,
        status: row.get(7)?,
        counts: RunCounts { copied: n(8)?, updated: n(9)?, removed: n(10)?, versions_kept: n(11)?, failed: n(12)?, bytes: n(13)? },
        warnings: n(14)?,
        duration_ms: n(15)?,
        message: row.get(16)?,
    })
}

/// Opens the place a backup path points at: a local (or mounted) folder, or an SFTP address
pub fn store_for(path: &str, password: Option<&str>, endpoint: Option<&str>, sftp: &crate::config::SftpConfig) -> Result<Arc<dyn Store>, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("No folder chosen".into());
    }
    let lower = path.to_lowercase();
    if lower.starts_with("sftp://") || lower.starts_with("ssh://") {
        return Ok(Arc::new(super::sftp_store::SftpStore::new(path, password, sftp)?));
    }
    if lower.starts_with("rclone://") {
        return Ok(Arc::new(super::rclone_store::RcloneStore::new(path)?));
    }
    if lower.starts_with("fleet://") {
        let endpoint = endpoint.ok_or("A Fleet machine can only be where backups are kept, not what is backed up")?;
        return Ok(Arc::new(super::fleet_store::FleetStore::new(path, endpoint, password)?));
    }
    if path.contains("://") {
        let scheme = path.split("://").next().unwrap_or_default();
        return Err(format!("{scheme}:// places are not supported for backups. Use a folder on this machine, a mounted share, SFTP (sftp://) or an rclone remote (rclone://)."));
    }
    Ok(Arc::new(LocalStore::new(path)))
}

/// What a backup copies from: its folder, or several folders each under its own name
fn source_store(p: &BackupProfile, sftp: &crate::config::SftpConfig) -> Result<Arc<dyn Store>, String> {
    let sources = p.all_sources();
    if sources.len() == 1 {
        return store_for(&p.source_dir, p.source_password.as_deref(), None, sftp);
    }
    let names = super::store::source_names(&sources);
    let parts = names.into_iter().zip(sources.iter()).map(|(name, path)| Ok((name, store_for(path, None, None, sftp)?))).collect::<Result<Vec<_>, String>>()?;
    Ok(Arc::new(super::store::MultiStore::new(parts)))
}

/// `sftp://user:secret@host/path` becomes `sftp://user@host/path`, with the secret kept as the password
fn move_password_out_of_address(path: &mut String, password: &mut Option<String>) {
    let Some((scheme, rest)) = path.split_once("://") else { return };
    let Some((userinfo, host_path)) = rest.rsplit_once('@') else { return };
    if host_path.contains('/') && userinfo.contains('/') {
        return; // the "@" belongs to the path, not to a login
    }
    if let Some((user, secret)) = userinfo.split_once(':') {
        if !secret.is_empty() {
            *password = Some(crate::vfs::sftp::percent_decode(secret));
        }
        *path = format!("{scheme}://{user}@{host_path}");
    }
}

fn spec_for(p: &BackupProfile, sftp: &crate::config::SftpConfig) -> Result<JobSpec, String> {
    let (mode, implied_never_remove) = Mode::parse(&p.profile_mode);
    Ok(JobSpec {
        source: source_store(p, sftp)?,
        dest: store_for(&p.dest_dir, p.dest_password.as_deref(), p.dest_endpoint.as_deref(), sftp)?,
        mode,
        never_remove: (p.never_remove || implied_never_remove) && mode != Mode::TwoWay,
        verify: p.verify_checksum,
        partial_updates: p.block_delta,
        exclusions: Exclusions::parse(p.exclusions.as_deref().unwrap_or("")),
        versions_dir: p.versions_dir(),
        retention_days: p.retention_days,
    })
}

/// Same place, ignoring a trailing slash
fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// Checks that make sense before saving or previewing a backup
pub fn check_paths(p: &BackupProfile) -> Result<(), String> {
    let sources = p.all_sources();
    if sources.len() > 1 {
        if p.mode() == Mode::TwoWay {
            return Err("Keeping two folders the same works with one folder at a time. Use Copy or Copy and keep old versions for several folders.".into());
        }
        if p.transfer == "rsync" {
            return Err("rsync works with one folder at a time. Choose Brum's own copier for several folders.".into());
        }
        for (i, a) in sources.iter().enumerate() {
            if a.contains("://") {
                return Err(format!("With several folders, each must be a folder on this machine or a mounted share ({a} is not)"));
            }
            for b in sources.iter().skip(i + 1) {
                let (a2, b2) = (a.trim_end_matches('/'), b.trim_end_matches('/'));
                if b2.starts_with(&format!("{a2}/")) || a2.starts_with(&format!("{b2}/")) {
                    return Err(format!("{b} and {a}: one folder is inside the other, so it would be backed up twice"));
                }
            }
        }
        for src in &sources {
            let mut one = p.clone();
            one.sources = vec![src.clone()];
            one.source_dir = src.clone();
            check_paths(&one)?;
        }
        return Ok(());
    }
    let (s, d) = (p.source_dir.trim_end_matches('/'), p.dest_dir.trim_end_matches('/'));
    if s.is_empty() || d.is_empty() {
        return Err("Choose both what to back up and where to keep it".into());
    }
    if same_path(s, d) {
        return Err("What to back up and where to keep it are the same folder".into());
    }
    if d.starts_with(&format!("{s}/")) {
        return Err("The backup cannot be kept inside the folder it backs up".into());
    }
    if s.starts_with(&format!("{d}/")) && p.mode() == Mode::TwoWay {
        return Err("One folder cannot be inside the other for two-way sync".into());
    }
    Ok(())
}

#[derive(Clone)]
pub struct BackupManager {
    db: Arc<Mutex<Connection>>,
    running: Arc<Mutex<HashMap<String, Running>>>,
    /// Host key checking for SFTP locations, from Brum's storage settings
    sftp: Arc<crate::config::SftpConfig>,
    /// Makes a short-lived login for a user, for Fleet machines that have no token of their own
    login_minter: Arc<std::sync::OnceLock<LoginMinter>>,
}

pub type LoginMinter = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

impl BackupManager {
    pub fn new(db: Arc<Mutex<Connection>>) -> Result<Self, String> {
        {
            let conn = db.lock().map_err(|e| e.to_string())?;
            Self::migrate(&conn).map_err(|e| format!("Backup tables: {e}"))?;
        }
        Ok(Self { db, running: Arc::new(Mutex::new(HashMap::new())), sftp: Arc::new(crate::config::SftpConfig::default()), login_minter: Arc::new(std::sync::OnceLock::new()) })
    }

    /// For Fleet machines without a token: log in as the backup's owner, the way the Fleet proxy
    /// logs in as the person using it (this works where the machines accept each other's logins)
    pub fn set_login_minter(&self, f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
        let _ = self.login_minter.set(Arc::new(f));
    }

    /// The backup with a login for its Fleet destination filled in when it has none of its own
    fn with_fleet_login(&self, p: &BackupProfile) -> BackupProfile {
        let mut p = p.clone();
        if p.dest_dir.to_lowercase().starts_with("fleet://") && p.dest_password.as_deref().unwrap_or("").trim().is_empty() {
            if let (Some(owner), Some(mint)) = (p.owner.as_deref(), self.login_minter.get()) {
                p.dest_password = mint(owner);
            }
        }
        p
    }

    /// Uses Brum's SFTP settings (host key checking, known hosts file) for remote locations
    pub fn with_sftp_config(mut self, cfg: crate::config::SftpConfig) -> Self {
        self.sftp = Arc::new(cfg);
        self
    }

    fn migrate(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
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
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS backup_runs (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                profile_name TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'backup',
                trigger TEXT NOT NULL DEFAULT 'manual',
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                status TEXT NOT NULL,
                copied INTEGER DEFAULT 0,
                updated INTEGER DEFAULT 0,
                removed INTEGER DEFAULT 0,
                versions_kept INTEGER DEFAULT 0,
                failed INTEGER DEFAULT 0,
                bytes INTEGER DEFAULT 0,
                warnings INTEGER DEFAULT 0,
                duration_ms INTEGER DEFAULT 0,
                message TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_backup_runs_profile ON backup_runs(profile_id, started_at);
            CREATE TABLE IF NOT EXISTS backup_run_files (
                run_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                path TEXT NOT NULL,
                action TEXT NOT NULL,
                size INTEGER DEFAULT 0,
                ok INTEGER DEFAULT 1,
                detail TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_backup_run_files_run ON backup_run_files(run_id, seq);
            CREATE TABLE IF NOT EXISTS backup_state (
                profile_id TEXT NOT NULL,
                path TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                PRIMARY KEY (profile_id, path)
            );",
        )?;

        let mut have: Vec<String> = Vec::new();
        {
            let mut stmt = conn.prepare("PRAGMA table_info(backup_profiles)")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for r in rows.flatten() {
                have.push(r);
            }
        }
        for (col, def) in [
            ("exclusions", "TEXT"),
            ("owner", "TEXT"),
            ("never_remove", "INTEGER DEFAULT 0"),
            ("log_retention_days", "INTEGER DEFAULT 90"),
            ("last_started", "INTEGER"),
            ("source_password", "TEXT"),
            ("dest_password", "TEXT"),
            ("transfer", "TEXT DEFAULT 'builtin'"),
            ("dest_endpoint", "TEXT"),
            ("sources", "TEXT"),
        ] {
            if !have.iter().any(|c| c == col) {
                conn.execute(&format!("ALTER TABLE backup_profiles ADD COLUMN {col} {def}"), [])?;
            }
        }

        // Old tool's modes and schedules, in today's words
        conn.execute_batch(
            "UPDATE backup_profiles SET profile_mode = 'copy', never_remove = 1 WHERE profile_mode IN ('contribute', 'additive');
             UPDATE backup_profiles SET profile_mode = 'copy', source_dir = dest_dir, dest_dir = source_dir WHERE profile_mode = 'mirror_right_to_left';
             UPDATE backup_profiles SET profile_mode = 'copy' WHERE profile_mode IN ('echo', 'mirror', 'mirror_left_to_right', '1way');
             UPDATE backup_profiles SET profile_mode = 'versions' WHERE profile_mode IN ('subscribe', 'archive', 'versioning');
             UPDATE backup_profiles SET profile_mode = 'two_way' WHERE profile_mode IN ('synchronize', 'sync', 'bidirectional_newer', '2way');
             UPDATE backup_profiles SET schedule_type = 'interval', schedule_interval_mins = 5 WHERE schedule_type = 'realtime';",
        )?;

        // Old history becomes runs (once); the old table is left in place
        let old_history: bool = conn
            .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'backup_history'", [], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if old_history {
            conn.execute(
                "INSERT OR IGNORE INTO backup_runs (id, profile_id, profile_name, kind, trigger, started_at, finished_at, status, copied, updated, removed, versions_kept, bytes, duration_ms, message)
                 SELECT id, profile_id, profile_name, 'backup', 'manual', started_at, finished_at,
                        CASE status WHEN 'success' THEN 'ok' ELSE status END,
                        files_copied, files_updated, files_deleted, files_archived, bytes_transferred, duration_ms, error_message
                 FROM backup_history",
                [],
            )?;
        }

        // Runs that were going when Brum stopped
        conn.execute(
            "UPDATE backup_runs SET status = 'failed', message = 'Brum stopped during this run' WHERE status = 'running'",
            [],
        )?;
        Ok(())
    }

    // ---------------- Profiles ----------------

    pub fn list_profiles(&self) -> Vec<BackupProfile> {
        let Ok(conn) = self.db.lock() else { return Vec::new() };
        let sql = format!("SELECT {PROFILE_COLUMNS} FROM backup_profiles ORDER BY name COLLATE NOCASE");
        let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
        let rows = stmt.query_map([], profile_from_row);
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    pub fn get_profile(&self, id: &str) -> Option<BackupProfile> {
        let conn = self.db.lock().ok()?;
        conn.query_row(&format!("SELECT {PROFILE_COLUMNS} FROM backup_profiles WHERE id = ?1"), params![id], profile_from_row)
            .optional()
            .ok()
            .flatten()
    }

    pub fn save_profile(&self, mut p: BackupProfile) -> Result<BackupProfile, String> {
        p.name = p.name.trim().to_string();
        if p.name.is_empty() {
            return Err("Give the backup a name".into());
        }
        p.profile_mode = Mode::parse(&p.profile_mode).0.as_str().to_string();
        if !matches!(p.schedule_type.as_str(), "manual" | "interval" | "daily") {
            p.schedule_type = "manual".into();
        }
        if p.schedule_type == "daily" {
            let t = p.schedule_time.clone().unwrap_or_default();
            if chrono::NaiveTime::parse_from_str(&t, "%H:%M").is_err() {
                return Err("Choose a time of day for the daily backup".into());
            }
        }
        p.schedule_interval_mins = p.schedule_interval_mins.clamp(1, 7 * 24 * 60);
        // One list of sources; the first is also kept as source_dir
        let mut list: Vec<String> = Vec::new();
        for src in p.all_sources() {
            if !list.iter().any(|x| same_path(x, &src)) {
                list.push(src);
            }
        }
        if let Some(first) = list.first() {
            p.source_dir = first.clone();
        }
        p.sources = list;
        move_password_out_of_address(&mut p.source_dir, &mut p.source_password);
        move_password_out_of_address(&mut p.dest_dir, &mut p.dest_password);
        // Only remote places have passwords
        if !p.source_dir.contains("://") {
            p.source_password = None;
        }
        if !p.dest_dir.contains("://") {
            p.dest_password = None;
        }
        if !p.dest_dir.to_lowercase().starts_with("fleet://") {
            p.dest_endpoint = None;
        }
        check_paths(&p)?;
        if p.transfer != "rsync" {
            p.transfer = default_transfer();
        }
        let existing = if p.id.is_empty() { None } else { self.get_profile(&p.id) };
        if p.id.is_empty() {
            p.id = Uuid::new_v4().to_string();
        }
        if let Some(old) = &existing {
            p.created_at = old.created_at;
            p.last_run = old.last_run;
            p.last_status = old.last_status.clone();
            p.last_result = old.last_result.clone();
            p.last_started = old.last_started;
            // The browser never sees saved passwords; an empty field keeps them while the place stays the same
            if p.source_password.as_deref().unwrap_or("").is_empty() && same_path(&old.source_dir, &p.source_dir) {
                p.source_password = old.source_password.clone();
            }
            if p.dest_password.as_deref().unwrap_or("").is_empty() && same_path(&old.dest_dir, &p.dest_dir) {
                p.dest_password = old.dest_password.clone();
            }
            if p.dest_endpoint.is_none() && same_path(&old.dest_dir, &p.dest_dir) {
                p.dest_endpoint = old.dest_endpoint.clone();
            }
            // A different pair of folders starts a two-way sync from scratch
            if !same_path(&old.source_dir, &p.source_dir) || !same_path(&old.dest_dir, &p.dest_dir) || old.mode() != p.mode() {
                self.clear_state(&p.id);
            }
        } else {
            p.created_at = Utc::now().timestamp();
        }
        if p.transfer == "rsync" {
            super::rsync::check(&p.dest_dir, p.dest_password.as_deref(), &p.source_dir, p.mode())?;
        }
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            &format!(
                "INSERT INTO backup_profiles ({PROFILE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, source_dir = excluded.source_dir, dest_dir = excluded.dest_dir,
                   profile_mode = excluded.profile_mode, block_delta = excluded.block_delta, verify_checksum = excluded.verify_checksum,
                   archive_dir = excluded.archive_dir, retention_days = excluded.retention_days, schedule_type = excluded.schedule_type,
                   schedule_interval_mins = excluded.schedule_interval_mins, schedule_time = excluded.schedule_time,
                   webhook_url = excluded.webhook_url, enabled = excluded.enabled, exclusions = excluded.exclusions,
                   owner = excluded.owner, never_remove = excluded.never_remove, log_retention_days = excluded.log_retention_days,
                   source_password = excluded.source_password, dest_password = excluded.dest_password, transfer = excluded.transfer, dest_endpoint = excluded.dest_endpoint, sources = excluded.sources"
            ),
            params![
                p.id,
                p.name,
                p.source_dir,
                p.dest_dir,
                p.profile_mode,
                p.block_delta as i64,
                p.verify_checksum as i64,
                p.archive_dir,
                p.retention_days as i64,
                p.schedule_type,
                p.schedule_interval_mins as i64,
                p.schedule_time,
                p.webhook_url,
                p.enabled as i64,
                p.last_run,
                p.last_status,
                p.last_result,
                p.created_at,
                p.exclusions,
                p.owner,
                p.never_remove as i64,
                p.log_retention_days.max(1) as i64,
                p.last_started,
                p.source_password,
                p.dest_password,
                p.transfer,
                p.dest_endpoint,
                serde_json::to_string(&p.sources).unwrap_or_else(|_| "[]".into()),
            ],
        )
        .map_err(|e| format!("Could not save the backup: {e}"))?;
        Ok(p)
    }

    /// Removes the backup's settings, history and sync memory. The backed-up files stay where they are.
    pub fn delete_profile(&self, id: &str) -> Result<(), String> {
        if self.is_running(id) {
            return Err("Stop the running backup first".into());
        }
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM backup_run_files WHERE run_id IN (SELECT id FROM backup_runs WHERE profile_id = ?1)", params![id]).map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM backup_runs WHERE profile_id = ?1", params![id]).map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM backup_state WHERE profile_id = ?1", params![id]).map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM backup_profiles WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Pauses or resumes the schedule; returns whether it is now active
    pub fn toggle_profile(&self, id: &str) -> Result<bool, String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        conn.execute("UPDATE backup_profiles SET enabled = 1 - enabled WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
        conn.query_row("SELECT enabled FROM backup_profiles WHERE id = ?1", params![id], |r| r.get::<_, i64>(0))
            .map(|v| v == 1)
            .map_err(|e| e.to_string())
    }

    pub fn view(&self, p: BackupProfile) -> ProfileView {
        let running = self.running_info(&p.id);
        // The card describes the backup itself; restores are listed in History
        let last = self.list_runs(Some(&p.id), 20).into_iter().find(|r| r.kind == "backup");
        let last_success_at = self.db.lock().ok().and_then(|c| {
            c.query_row(
                "SELECT MAX(COALESCE(finished_at, started_at)) FROM backup_runs WHERE profile_id = ?1 AND kind = 'backup' AND status IN ('ok', 'warning', 'nothing')",
                params![p.id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
        });
        let next_run = if p.enabled { next_due(&p) } else { None };
        let has_source_password = p.source_password.as_deref().is_some_and(|x| !x.is_empty());
        let has_dest_password = p.dest_password.as_deref().is_some_and(|x| !x.is_empty());
        ProfileView { profile: p, has_source_password, has_dest_password, running, next_run, last, last_success_at }
    }

    // ---------------- Running ----------------

    pub fn is_running(&self, id: &str) -> bool {
        self.running.lock().map(|m| m.contains_key(id)).unwrap_or(false)
    }

    pub fn running_info(&self, id: &str) -> Option<RunningInfo> {
        let map = self.running.lock().ok()?;
        let r = map.get(id)?;
        let progress = r.progress.lock().map(|p| p.clone()).unwrap_or_default();
        Some(RunningInfo { run_id: r.run_id.clone(), kind: r.kind.clone(), started_at: r.started_at, progress })
    }

    /// Asks a running backup or restore to stop after the current file
    pub fn stop(&self, id: &str) -> bool {
        match self.running.lock().ok().and_then(|m| m.get(id).map(|r| r.cancel.clone())) {
            Some(c) => {
                c.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Claims the right to run; fails when this backup is already running
    fn claim(&self, id: &str, kind: &str) -> Result<(String, Arc<Mutex<Progress>>, Arc<AtomicBool>), String> {
        let mut map = self.running.lock().map_err(|e| e.to_string())?;
        if let Some(r) = map.get(id) {
            return Err(if r.kind == "restore" { "A restore is running for this backup".into() } else { "This backup is already running".into() });
        }
        let run_id = Uuid::new_v4().to_string();
        let progress = Arc::new(Mutex::new(Progress { phase: "checking".into(), ..Default::default() }));
        let cancel = Arc::new(AtomicBool::new(false));
        map.insert(id.to_string(), Running { run_id: run_id.clone(), kind: kind.to_string(), started_at: Utc::now().timestamp(), progress: progress.clone(), cancel: cancel.clone() });
        Ok((run_id, progress, cancel))
    }

    fn release(&self, id: &str) {
        if let Ok(mut map) = self.running.lock() {
            map.remove(id);
        }
    }

    /// Starts a backup in the background; returns the run id
    pub fn start_backup(&self, id: &str, trigger: &str, tasks: Arc<TaskManager>) -> Result<String, String> {
        let p = self.get_profile(id).ok_or("Backup not found")?;
        let spec = spec_for(&self.with_fleet_login(&p), &self.sftp)?;
        let (run_id, progress, cancel) = self.claim(id, "backup")?;
        let started_at = Utc::now().timestamp();
        if let Err(e) = self.insert_run(&run_id, &p, "backup", trigger, started_at) {
            self.release(id);
            return Err(e);
        }
        if let Ok(conn) = self.db.lock() {
            let _ = conn.execute("UPDATE backup_profiles SET last_started = ?1 WHERE id = ?2", params![started_at, p.id]);
        }
        info!("Backup '{}' started ({})", p.name, trigger);
        let mgr = self.clone();
        let rid = run_id.clone();
        tokio::spawn(async move {
            let p2 = p.clone();
            let mgr2 = mgr.clone();
            let rid2 = rid.clone();
            let record = tokio::task::spawn_blocking(move || mgr2.run_backup(&p2, spec, &rid2, progress, cancel, tasks)).await;
            mgr.release(&p.id);
            match record {
                Ok(rec) => mgr.notify(&p, &rec).await,
                Err(e) => error!("Backup '{}' crashed: {e}", p.name),
            }
        });
        Ok(run_id)
    }

    fn run_backup(&self, p: &BackupProfile, spec: JobSpec, run_id: &str, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>, tasks: Arc<TaskManager>) -> RunRecord {
        let clock = Instant::now();
        let mut rec = self.get_run(run_id).unwrap_or_else(|| blank_run(run_id, p, "backup", "manual"));
        let state = (spec.mode == Mode::TwoWay).then(|| self.load_state(&p.id));
        let mut log = LogWriter::new(self.db.clone(), run_id);

        let plan = match engine::plan(&spec, state.as_ref(), &cancel) {
            Ok(plan) => plan,
            Err(e) => {
                rec.status = if cancel.load(Ordering::Relaxed) { "cancelled".into() } else { "failed".into() };
                rec.message = Some(if rec.status == "cancelled" { "Stopped".into() } else { e });
                return self.finish_run(rec, clock, &mut log, p);
            }
        };
        for w in plan.warnings.iter() {
            log.warning(w);
        }
        rec.warnings = plan.warnings.len() as u64;

        if plan.items.is_empty() {
            rec.status = "nothing".into();
            rec.message = Some(format!("Already up to date ({} files checked)", plan.summary.source_files));
            if let Some(s) = engine::execute(&spec, &plan, state.as_ref(), &|_| {}, &mut |e| log.push(e), &cancel).ok().and_then(|o| o.new_state) {
                self.save_state(&p.id, &s);
            }
            return self.finish_run(rec, clock, &mut log, p);
        }

        let task_id = tasks.sync_create_task(&format!("Backup: {}", p.name), "backup", &spec.source.label(), &spec.dest.label(), plan.summary.bytes);
        if let Some(owner) = &p.owner {
            tasks.set_owner(&task_id, owner);
        }
        let speed_clock = Instant::now();
        let report = |pr: Progress| {
            if tasks.sync_is_cancelled(&task_id) {
                cancel.store(true, Ordering::Relaxed);
            }
            let secs = speed_clock.elapsed().as_secs_f64().max(0.001);
            tasks.sync_update_stream_progress(&task_id, pr.current.as_deref(), 0, 0, pr.files_done, pr.files_total.max(1), pr.bytes_done, (pr.bytes_done as f64 / secs) as u64);
            if let Ok(mut g) = progress.lock() {
                *g = pr;
            }
        };
        if p.transfer == "rsync" && spec.mode != Mode::TwoWay {
            let job = super::rsync::RsyncJob {
                source_dir: &p.source_dir,
                dest_dir: &p.dest_dir,
                dest_password: p.dest_password.as_deref(),
                mode: spec.mode,
                never_remove: spec.never_remove,
                exclusions: p.exclusions.as_deref().unwrap_or(""),
                versions_dir: &spec.versions_dir,
                expected_files: plan.items.len() as u64,
                expected_bytes: plan.summary.bytes,
            };
            match super::rsync::run(&job, &report, &mut |e| log.push(e), cancel.clone()) {
                Ok((counts, warnings)) => {
                    for w in &warnings {
                        log.warning(w);
                    }
                    rec.warnings += warnings.len() as u64;
                    rec.counts = counts;
                    if spec.mode == Mode::Versions && spec.retention_days > 0 {
                        engine::prune_versions(spec.dest.as_ref(), &spec.versions_dir, spec.retention_days, &mut |e| log.push(e));
                    }
                    rec.status = if rec.counts.failed > 0 || rec.warnings > 0 { "warning".into() } else { "ok".into() };
                    rec.message = Some(format!("{} (with rsync)", summary_line(&rec, false)));
                    tasks.sync_complete_task(&task_id);
                }
                Err(e) if e == "Cancelled" => {
                    rec.status = "cancelled".into();
                    rec.message = Some("Stopped".into());
                    tasks.sync_fail_task(&task_id, "Stopped");
                }
                Err(e) => {
                    rec.status = "failed".into();
                    rec.message = Some(format!("rsync: {e}"));
                    tasks.sync_fail_task(&task_id, &e);
                }
            }
            return self.finish_run(rec, clock, &mut log, p);
        }
        match engine::execute(&spec, &plan, state.as_ref(), &report, &mut |e| log.push(e), &cancel) {
            Ok(outcome) => {
                rec.counts = outcome.counts;
                if let Some(s) = outcome.new_state {
                    self.save_state(&p.id, &s);
                }
                rec.status = if outcome.cancelled {
                    "cancelled".into()
                } else if rec.counts.failed > 0 || rec.warnings > 0 {
                    "warning".into()
                } else {
                    "ok".into()
                };
                rec.message = Some(summary_line(&rec, outcome.cancelled));
                if outcome.cancelled {
                    tasks.sync_fail_task(&task_id, "Stopped");
                } else {
                    tasks.sync_complete_task(&task_id);
                }
            }
            Err(e) => {
                rec.status = "failed".into();
                rec.message = Some(e.clone());
                tasks.sync_fail_task(&task_id, &e);
            }
        }
        self.finish_run(rec, clock, &mut log, p)
    }

    fn finish_run(&self, mut rec: RunRecord, clock: Instant, log: &mut LogWriter, p: &BackupProfile) -> RunRecord {
        log.close();
        rec.finished_at = Some(Utc::now().timestamp());
        rec.duration_ms = clock.elapsed().as_millis() as u64;
        if let Ok(conn) = self.db.lock() {
            let c = &rec.counts;
            let _ = conn.execute(
                "UPDATE backup_runs SET finished_at = ?2, status = ?3, copied = ?4, updated = ?5, removed = ?6, versions_kept = ?7, failed = ?8, bytes = ?9, warnings = ?10, duration_ms = ?11, message = ?12 WHERE id = ?1",
                params![rec.id, rec.finished_at, rec.status, c.copied as i64, c.updated as i64, c.removed as i64, c.versions_kept as i64, c.failed as i64, c.bytes as i64, rec.warnings as i64, rec.duration_ms as i64, rec.message],
            );
            if rec.kind == "backup" {
                let _ = conn.execute(
                    "UPDATE backup_profiles SET last_run = ?1, last_status = ?2, last_result = ?3 WHERE id = ?4",
                    params![rec.started_at, rec.status, rec.message, p.id],
                );
            }
        }
        match rec.status.as_str() {
            "failed" => warn!("{} '{}' failed: {}", rec.kind, p.name, rec.message.as_deref().unwrap_or("")),
            _ => info!("{} '{}': {}", rec.kind, p.name, rec.message.as_deref().unwrap_or(&rec.status)),
        }
        rec
    }

    async fn notify(&self, p: &BackupProfile, rec: &RunRecord) {
        let Some(url) = p.webhook_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) else { return };
        if rec.status == "nothing" {
            return;
        }
        let payload = serde_json::json!({
            "event": if rec.status == "failed" { "backup_failed" } else { "backup_completed" },
            "kind": rec.kind,
            "profile_id": p.id,
            "profile_name": p.name,
            "status": rec.status,
            "message": rec.message,
            "files_copied": rec.counts.copied,
            "files_updated": rec.counts.updated,
            "files_removed": rec.counts.removed,
            "files_failed": rec.counts.failed,
            "bytes": rec.counts.bytes,
            "duration_ms": rec.duration_ms,
            "timestamp": rec.finished_at,
        });
        let _ = reqwest::Client::new().post(url).json(&payload).timeout(Duration::from_secs(5)).send().await;
    }

    // ---------------- Preview ----------------

    /// What a backup would do now, without changing anything
    pub fn preview(&self, p: &BackupProfile) -> Result<PreviewResult, String> {
        check_paths(p)?;
        let spec = spec_for(&self.with_fleet_login(p), &self.sftp)?;
        let state = if spec.mode == Mode::TwoWay {
            self.get_profile(&p.id)
                .filter(|old| same_path(&old.source_dir, &p.source_dir) && same_path(&old.dest_dir, &p.dest_dir) && old.mode() == Mode::TwoWay)
                .map(|_| self.load_state(&p.id))
        } else {
            None
        };
        let plan = engine::plan(&spec, state.as_ref(), &AtomicBool::new(false))?;
        const MAX_ITEMS: usize = 2000;
        const MAX_WARNINGS: usize = 200;
        Ok(PreviewResult {
            summary: plan.summary,
            total_items: plan.items.len(),
            items: plan.items.into_iter().take(MAX_ITEMS).collect(),
            total_warnings: plan.warnings.len(),
            warnings: plan.warnings.into_iter().take(MAX_WARNINGS).collect(),
        })
    }

    // ---------------- Two-way memory ----------------

    fn load_state(&self, id: &str) -> Listing {
        let mut out = Listing::new();
        let Ok(conn) = self.db.lock() else { return out };
        let Ok(mut stmt) = conn.prepare("SELECT path, size, mtime FROM backup_state WHERE profile_id = ?1") else { return out };
        if let Ok(rows) = stmt.query_map(params![id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))) {
            for (path, size, mtime) in rows.flatten() {
                out.insert(path, FileMeta { size: size.max(0) as u64, mtime });
            }
        }
        out
    }

    fn save_state(&self, id: &str, state: &Listing) {
        let Ok(mut conn) = self.db.lock() else { return };
        let Ok(tx) = conn.transaction() else { return };
        let _ = tx.execute("DELETE FROM backup_state WHERE profile_id = ?1", params![id]);
        if let Ok(mut stmt) = tx.prepare("INSERT INTO backup_state (profile_id, path, size, mtime) VALUES (?1, ?2, ?3, ?4)") {
            for (path, m) in state {
                let _ = stmt.execute(params![id, path, m.size as i64, m.mtime]);
            }
        }
        let _ = tx.commit();
    }

    fn clear_state(&self, id: &str) {
        if let Ok(conn) = self.db.lock() {
            let _ = conn.execute("DELETE FROM backup_state WHERE profile_id = ?1", params![id]);
        }
    }

    // ---------------- History and logs ----------------

    fn insert_run(&self, run_id: &str, p: &BackupProfile, kind: &str, trigger: &str, started_at: i64) -> Result<(), String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO backup_runs (id, profile_id, profile_name, kind, trigger, started_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running')",
            params![run_id, p.id, p.name, kind, trigger, started_at],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    pub fn get_run(&self, run_id: &str) -> Option<RunRecord> {
        let conn = self.db.lock().ok()?;
        conn.query_row(&format!("SELECT {RUN_COLUMNS} FROM backup_runs WHERE id = ?1"), params![run_id], run_from_row).optional().ok().flatten()
    }

    pub fn list_runs(&self, profile_id: Option<&str>, limit: usize) -> Vec<RunRecord> {
        let Ok(conn) = self.db.lock() else { return Vec::new() };
        let lim = limit.clamp(1, 500) as i64;
        let result = match profile_id {
            Some(pid) => conn
                .prepare(&format!("SELECT {RUN_COLUMNS} FROM backup_runs WHERE profile_id = ?1 ORDER BY started_at DESC LIMIT ?2"))
                .and_then(|mut s| s.query_map(params![pid, lim], run_from_row).map(|r| r.flatten().collect())),
            None => conn
                .prepare(&format!("SELECT {RUN_COLUMNS} FROM backup_runs ORDER BY started_at DESC LIMIT ?1"))
                .and_then(|mut s| s.query_map(params![lim], run_from_row).map(|r| r.flatten().collect())),
        };
        result.unwrap_or_default()
    }

    /// Per-file log lines of a run; `problems_only` keeps failures and warnings
    pub fn run_files(&self, run_id: &str, offset: usize, limit: usize, problems_only: bool) -> (Vec<RunFile>, u64) {
        let Ok(conn) = self.db.lock() else { return (Vec::new(), 0) };
        let filter = if problems_only { " AND (ok = 0 OR action = 'warning')" } else { "" };
        let total: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM backup_run_files WHERE run_id = ?1{filter}"), params![run_id], |r| r.get(0))
            .unwrap_or(0);
        let rows = conn
            .prepare(&format!("SELECT seq, path, action, size, ok, detail FROM backup_run_files WHERE run_id = ?1{filter} ORDER BY seq LIMIT ?2 OFFSET ?3"))
            .and_then(|mut s| {
                s.query_map(params![run_id, limit.clamp(1, 5000) as i64, offset as i64], |r| {
                    Ok(RunFile { seq: r.get(0)?, path: r.get(1)?, action: r.get(2)?, size: r.get::<_, i64>(3)?.max(0) as u64, ok: r.get::<_, i64>(4)? == 1, detail: r.get(5)? })
                })
                .map(|r| r.flatten().collect())
            })
            .unwrap_or_default();
        (rows, total.max(0) as u64)
    }

    /// The whole log of a run as plain text, for downloading
    pub fn run_log_text(&self, run_id: &str) -> Option<String> {
        let rec = self.get_run(run_id)?;
        let when = |t: i64| Local.timestamp_opt(t, 0).single().map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
        let mut out = String::new();
        out.push_str(&format!("{} of \"{}\"\n", if rec.kind == "restore" { "Restore" } else { "Backup" }, rec.profile_name));
        out.push_str(&format!("Started:  {} ({})\n", when(rec.started_at), rec.trigger));
        if let Some(f) = rec.finished_at {
            out.push_str(&format!("Finished: {}\n", when(f)));
        }
        out.push_str(&format!("Result:   {}\n", rec.status));
        if let Some(m) = &rec.message {
            out.push_str(&format!("Summary:  {m}\n"));
        }
        out.push('\n');
        let mut offset = 0;
        loop {
            let (rows, _) = self.run_files(run_id, offset, 5000, false);
            if rows.is_empty() {
                break;
            }
            offset += rows.len();
            for r in rows {
                let mark = if r.ok { "    " } else { "FAIL" };
                let detail = r.detail.map(|d| format!("  ({d})")).unwrap_or_default();
                out.push_str(&format!("{mark} {:<12} {}{}\n", action_words(&r.action), r.path, detail));
            }
        }
        Some(out)
    }

    /// Removes run logs older than each backup's log retention
    pub fn prune_logs(&self) {
        let profiles = self.list_profiles();
        let Ok(conn) = self.db.lock() else { return };
        for p in profiles {
            let limit = Utc::now().timestamp() - p.log_retention_days.max(1) as i64 * 86400;
            let _ = conn.execute(
                "DELETE FROM backup_run_files WHERE run_id IN (SELECT id FROM backup_runs WHERE profile_id = ?1 AND started_at < ?2 AND status != 'running')",
                params![p.id, limit],
            );
            let _ = conn.execute("DELETE FROM backup_runs WHERE profile_id = ?1 AND started_at < ?2 AND status != 'running'", params![p.id, limit]);
        }
    }

    // ---------------- Restore ----------------

    /// "Latest" plus every dated folder of old versions
    pub fn restore_points(&self, p: &BackupProfile) -> Result<Vec<RestorePoint>, String> {
        let p = &self.with_fleet_login(p);
        let dest = store_for(&p.dest_dir, p.dest_password.as_deref(), p.dest_endpoint.as_deref(), &self.sftp)?;
        let last_ok = self.view(p.clone()).last_success_at;
        let mut points = vec![RestorePoint { id: "latest".into(), label: "Latest backup".into(), when: last_ok }];
        if let Ok(entries) = dest.list_dir(&p.versions_dir()) {
            let mut dated: Vec<RestorePoint> = entries
                .into_iter()
                .filter(|e| e.is_dir)
                .filter_map(|e| {
                    let when = NaiveDateTime::parse_from_str(&e.name, STAMP_FORMAT).ok()?;
                    let ts = Local.from_local_datetime(&when).single()?.timestamp();
                    Some(RestorePoint { label: format!("Old versions saved {}", when.format("%Y-%m-%d %H:%M")), id: e.name, when: Some(ts) })
                })
                .collect();
            dated.sort_by(|a, b| b.when.cmp(&a.when));
            points.extend(dated);
        }
        Ok(points)
    }

    fn point_base(&self, p: &BackupProfile, point: &str) -> Result<String, String> {
        if point == "latest" || point.is_empty() {
            return Ok(String::new());
        }
        if NaiveDateTime::parse_from_str(point, STAMP_FORMAT).is_err() {
            return Err("Unknown restore point".into());
        }
        Ok(join_rel(&p.versions_dir(), point))
    }

    /// One folder level inside a restore point
    pub fn browse(&self, p: &BackupProfile, point: &str, path: &str) -> Result<Vec<DirItem>, String> {
        let p = &self.with_fleet_login(p);
        let dest = store_for(&p.dest_dir, p.dest_password.as_deref(), p.dest_endpoint.as_deref(), &self.sftp)?;
        let base = self.point_base(p, point)?;
        let rel = safe_rel(path)?;
        let at_top = base.is_empty() && rel.is_empty();
        let versions = p.versions_dir();
        let items = dest.list_dir(&join_rel(&base, &rel))?;
        Ok(items
            .into_iter()
            .filter(|i| !i.name.ends_with(PART_SUFFIX))
            .filter(|i| !(at_top && (i.name == versions || versions.starts_with(&format!("{}/", i.name)))))
            .collect())
    }

    /// Default "separate folder" for a restore: next to the original, with the date in its name
    pub fn default_restore_folder(p: &BackupProfile) -> String {
        let src = p.source_dir.trim_end_matches('/');
        let (parent, name) = src.rsplit_once('/').unwrap_or(("", src));
        let stamp = Local::now().format("%Y-%m-%d %H%M");
        // Several folders are restored together, under the backup's name
        let safe_backup_name: String = p.name.chars().map(|c| if c == '/' || c == '\\' { '-' } else { c }).collect();
        let name = if p.is_multi_source() { safe_backup_name.as_str() } else { name };
        let name = if name.is_empty() { "Backup" } else { name };
        format!("{parent}/{name} (restored {stamp})")
    }

    /// Restores files from a backup into `target_path` (already checked by the caller).
    /// Into the original folder, existing files are only replaced with `confirm_overwrite`.
    pub fn restore(&self, p: &BackupProfile, req: &RestoreRequest, target_path: &str, tasks: Arc<TaskManager>) -> Result<RestorePreview, String> {
        let with_login = self.with_fleet_login(p);
        let p = &with_login;
        let dest = store_for(&p.dest_dir, p.dest_password.as_deref(), p.dest_endpoint.as_deref(), &self.sftp)?;
        // Restoring into the original place of a remote source uses that place's password
        let target_password = same_path(target_path, &p.source_dir).then(|| p.source_password.as_deref()).flatten();
        let target = if p.is_multi_source() && same_path(target_path, &p.source_dir) {
            source_store(p, &self.sftp)?
        } else {
            store_for(target_path, target_password, None, &self.sftp)?
        };
        let base = self.point_base(p, &req.point)?;
        let versions = p.versions_dir();
        let skip = |rel: &str, _d: bool| rel.ends_with(PART_SUFFIX) || (base.is_empty() && (rel == versions || rel.starts_with(&format!("{versions}/"))));
        let cancel_none = AtomicBool::new(false);
        let mut warnings = Vec::new();

        // Files to restore, keyed by their path inside the backup
        let mut files = Listing::new();
        if req.items.is_empty() {
            files = dest.list(&base, &skip, &cancel_none, &mut warnings)?;
        } else {
            for item in &req.items {
                let item = safe_rel(item)?;
                if item.is_empty() {
                    files.extend(dest.list(&base, &skip, &cancel_none, &mut warnings)?);
                    continue;
                }
                let full = join_rel(&base, &item);
                if dest.is_dir(&full) {
                    for (rel, m) in dest.list(&full, &|r, d| skip(&join_rel(&item, r), d), &cancel_none, &mut warnings)? {
                        files.insert(join_rel(&item, &rel), m);
                    }
                } else if let Some(m) = dest.stat(&full) {
                    files.insert(item, m);
                }
            }
        }
        if files.is_empty() {
            return Err("Nothing to restore here".into());
        }
        // Files that are already exactly as in the backup need no restoring
        let before = files.len();
        files.retain(|rel, m| !target.stat(rel).is_some_and(|t| t.same_as(m)));
        let already_same = (before - files.len()) as u64;
        if files.is_empty() {
            return Err(format!("All {before} files are already exactly as in the backup; nothing to restore"));
        }
        let overwrites = files.iter().filter(|(rel, _)| target.stat(rel).is_some()).count() as u64;
        let mut preview = RestorePreview {
            files: files.len() as u64,
            bytes: files.values().map(|m| m.size).sum(),
            overwrites,
            target_path: target_path.to_string(),
            needs_confirmation: overwrites > 0 && !req.confirm_overwrite,
            run_id: None,
        };
        if req.dry_run || preview.needs_confirmation {
            return Ok(preview);
        }

        let (run_id, progress, cancel) = self.claim(&p.id, "restore")?;
        let started_at = Utc::now().timestamp();
        if let Err(e) = self.insert_run(&run_id, p, "restore", "manual", started_at) {
            self.release(&p.id);
            return Err(e);
        }
        preview.run_id = Some(run_id.clone());
        let (mgr, p, base, verify) = (self.clone(), p.clone(), base, p.verify_checksum);
        let total_bytes = preview.bytes;
        let target_label = target_path.to_string();
        let warn_list = warnings;
        tokio::spawn(async move {
            let mgr2 = mgr.clone();
            let p2 = p.clone();
            let res = tokio::task::spawn_blocking(move || {
                let clock = Instant::now();
                let mut rec = mgr2.get_run(&run_id).unwrap_or_else(|| blank_run(&run_id, &p2, "restore", "manual"));
                let mut log = LogWriter::new(mgr2.db.clone(), &run_id);
                for w in &warn_list {
                    log.warning(w);
                }
                rec.warnings = warn_list.len() as u64;
                let task_id = tasks.sync_create_task(&format!("Restore: {}", p2.name), "restore", &dest.label(), &target_label, total_bytes);
                if let Some(o) = &p2.owner {
                    tasks.set_owner(&task_id, o);
                }
                let mut pr = Progress { phase: "restoring".into(), files_total: files.len() as u64, bytes_total: total_bytes, ..Default::default() };
                let mut stopped = false;
                if let Err(e) = target.ensure_root() {
                    rec.status = "failed".into();
                    rec.message = Some(e.clone());
                    tasks.sync_fail_task(&task_id, &e);
                    return mgr2.finish_run(rec, clock, &mut log, &p2);
                }
                for (rel, meta) in &files {
                    if cancel.load(Ordering::Relaxed) || tasks.sync_is_cancelled(&task_id) {
                        stopped = true;
                        break;
                    }
                    pr.current = Some(rel.clone());
                    if let Ok(mut g) = progress.lock() {
                        *g = pr.clone();
                    }
                    tasks.sync_update_stream_progress(&task_id, Some(rel), 0, meta.size, pr.files_done, pr.files_total, pr.bytes_done, 0);
                    let existed = target.stat(rel).is_some();
                    match restore_one(dest.as_ref(), &join_rel(&base, rel), target.as_ref(), rel, *meta, verify, &cancel) {
                        Ok(bytes) => {
                            rec.counts.copied += 1;
                            rec.counts.bytes += bytes;
                            log.push(LogEntry { path: rel.clone(), action: "restore".into(), size: meta.size, ok: true, detail: existed.then(|| "replaced the existing file".to_string()) });
                        }
                        Err(e) if e == "Cancelled" => {
                            stopped = true;
                            break;
                        }
                        Err(e) => {
                            rec.counts.failed += 1;
                            log.push(LogEntry { path: rel.clone(), action: "restore".into(), size: meta.size, ok: false, detail: Some(e) });
                        }
                    }
                    pr.files_done += 1;
                    pr.bytes_done += meta.size;
                }
                rec.status = if stopped { "cancelled".into() } else if rec.counts.failed > 0 || rec.warnings > 0 { "warning".into() } else { "ok".into() };
                rec.message = Some(format!(
                    "{}{} restored to {}{}{}",
                    if stopped { "Stopped: " } else { "" },
                    plural(rec.counts.copied, "file", "files"),
                    target_label,
                    if rec.counts.failed > 0 { format!(", {} failed", rec.counts.failed) } else { String::new() },
                    if already_same > 0 { format!(" ({already_same} already up to date)") } else { String::new() }
                ));
                if stopped {
                    tasks.sync_fail_task(&task_id, "Stopped");
                } else {
                    tasks.sync_complete_task(&task_id);
                }
                mgr2.finish_run(rec, clock, &mut log, &p2)
            })
            .await;
            mgr.release(&p.id);
            match res {
                Ok(rec) => mgr.notify(&p, &rec).await,
                Err(e) => error!("Restore for '{}' crashed: {e}", p.name),
            }
        });
        Ok(preview)
    }

    // ---------------- Schedule ----------------

    /// Checks every 15 seconds which backups are due; one run per backup at a time
    pub fn start_scheduler(self: Arc<Self>, task_manager: Arc<TaskManager>) {
        tokio::spawn(async move {
            info!("Backup & Sync scheduler started");
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            let mut last_prune = Instant::now() - Duration::from_secs(3600);
            loop {
                tick.tick().await;
                let now = Utc::now().timestamp();
                for p in self.list_profiles() {
                    if p.enabled && !self.is_running(&p.id) && next_due(&p).is_some_and(|due| due <= now) {
                        if let Err(e) = self.start_backup(&p.id, "schedule", task_manager.clone()) {
                            // Record why, once per due time, so a failing schedule is visible in History
                            warn!("Scheduled backup '{}' could not start: {e}", p.name);
                            if let Ok(conn) = self.db.lock() {
                                let _ = conn.execute("UPDATE backup_profiles SET last_started = ?1 WHERE id = ?2", params![now, p.id]);
                            }
                            let run_id = Uuid::new_v4().to_string();
                            if self.insert_run(&run_id, &p, "backup", "schedule", now).is_ok() {
                                let mut rec = blank_run(&run_id, &p, "backup", "schedule");
                                rec.started_at = now;
                                rec.status = "failed".into();
                                rec.message = Some(e);
                                let mut log = LogWriter::new(self.db.clone(), &run_id);
                                self.finish_run(rec, Instant::now(), &mut log, &p);
                            }
                        }
                    }
                }
                if last_prune.elapsed() >= Duration::from_secs(3600) {
                    last_prune = Instant::now();
                    self.prune_logs();
                }
            }
        });
    }
}

/// When a backup should run next (None for manual ones)
pub fn next_due(p: &BackupProfile) -> Option<i64> {
    let last = p.last_started.unwrap_or(0);
    match p.schedule_type.as_str() {
        "interval" => Some(if last == 0 { p.created_at } else { last + p.schedule_interval_mins.max(1) as i64 * 60 }),
        "daily" => {
            let t = chrono::NaiveTime::parse_from_str(p.schedule_time.as_deref()?, "%H:%M").ok()?;
            let since = last.max(p.created_at);
            let since_local = Local.timestamp_opt(since, 0).single()?;
            let mut day = since_local.date_naive();
            loop {
                let slot = Local.from_local_datetime(&day.and_time(t)).earliest()?.timestamp();
                if slot > since {
                    return Some(slot);
                }
                day = day.succ_opt()?;
            }
        }
        _ => None,
    }
}

fn blank_run(id: &str, p: &BackupProfile, kind: &str, trigger: &str) -> RunRecord {
    RunRecord {
        id: id.to_string(),
        profile_id: p.id.clone(),
        profile_name: p.name.clone(),
        kind: kind.into(),
        trigger: trigger.into(),
        started_at: Utc::now().timestamp(),
        finished_at: None,
        status: "running".into(),
        counts: RunCounts::default(),
        warnings: 0,
        duration_ms: 0,
        message: None,
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn format_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{b} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

fn summary_line(rec: &RunRecord, stopped: bool) -> String {
    let c = &rec.counts;
    let mut parts = Vec::new();
    if c.copied > 0 {
        parts.push(format!("copied {}", plural(c.copied, "file", "files")));
    }
    if c.updated > 0 {
        parts.push(format!("updated {}", c.updated));
    }
    if c.removed > 0 {
        parts.push(format!("removed {}", c.removed));
    }
    if c.bytes > 0 {
        parts.push(format_bytes(c.bytes));
    }
    if c.failed > 0 {
        parts.push(format!("{} failed", c.failed));
    }
    if rec.warnings > 0 {
        parts.push(plural(rec.warnings, "warning", "warnings"));
    }
    let body = if parts.is_empty() { "nothing changed".to_string() } else { parts.join(", ") };
    let mut s = if stopped { format!("Stopped: {body}") } else { body };
    if let Some(first) = s.get(0..1) {
        s = first.to_uppercase() + &s[1..];
    }
    s
}

fn action_words(a: &str) -> &str {
    match a {
        "copy" => "copied",
        "update" => "updated",
        "remove" => "removed",
        "copy_back" => "copied back",
        "update_back" => "updated back",
        "remove_back" => "removed back",
        "restore" => "restored",
        "prune" => "cleaned up",
        "warning" => "warning",
        other => other,
    }
}

fn restore_one(from: &dyn Store, from_rel: &str, to: &dyn Store, to_rel: &str, meta: FileMeta, verify: bool, cancel: &AtomicBool) -> Result<u64, String> {
    let (part, bytes) = engine::copy_to_part(from, from_rel, to, to_rel, meta, verify, &mut |_| !cancel.load(Ordering::Relaxed))?;
    to.rename(&part, to_rel).map_err(|e| {
        let _ = to.remove_file(&part);
        e
    })?;
    Ok(bytes)
}

/// Writes per-file log lines to the database in batches
struct LogWriter {
    db: Arc<Mutex<Connection>>,
    run_id: String,
    seq: i64,
    buf: Vec<LogEntry>,
    warnings_logged: usize,
    warnings_hidden: usize,
}

impl LogWriter {
    const MAX_WARNINGS: usize = 1000;

    fn new(db: Arc<Mutex<Connection>>, run_id: &str) -> Self {
        Self { db, run_id: run_id.to_string(), seq: 0, buf: Vec::new(), warnings_logged: 0, warnings_hidden: 0 }
    }

    fn push(&mut self, e: LogEntry) {
        self.buf.push(e);
        if self.buf.len() >= 500 {
            self.flush();
        }
    }

    fn warning(&mut self, text: &str) {
        if self.warnings_logged >= Self::MAX_WARNINGS {
            self.warnings_hidden += 1;
            return;
        }
        self.warnings_logged += 1;
        let (path, detail) = match text.split_once(": ") {
            Some((what, rest)) => (rest.to_string(), what.to_string()),
            None => (String::new(), text.to_string()),
        };
        self.push(LogEntry { path, action: "warning".into(), size: 0, ok: true, detail: Some(detail) });
    }

    /// Writes what is left, with a note about warnings that were too many to list
    fn close(&mut self) {
        if self.warnings_hidden > 0 {
            let more = std::mem::take(&mut self.warnings_hidden);
            self.buf.push(LogEntry { path: String::new(), action: "warning".into(), size: 0, ok: true, detail: Some(format!("{more} more warnings not listed")) });
        }
        self.flush();
    }

    fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let Ok(mut conn) = self.db.lock() else { return };
        let Ok(tx) = conn.transaction() else { return };
        if let Ok(mut stmt) = tx.prepare("INSERT INTO backup_run_files (run_id, seq, path, action, size, ok, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)") {
            for e in self.buf.drain(..) {
                self.seq += 1;
                let _ = stmt.execute(params![self.run_id, self.seq, e.path, e.action, e.size as i64, e.ok as i64, e.detail]);
            }
        }
        let _ = tx.commit();
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        self.close();
    }
}
