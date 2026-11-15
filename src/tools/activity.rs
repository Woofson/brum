//! Activity log (#193): who did what to which files, when, on this machine.
//!
//! Entries are handed to a writer thread, so recording never slows a request down. They are
//! kept in Brum's database for `retention_days` (90 by default) and can be searched and
//! downloaded by admins. Operations that come in through the Fleet are recorded here too, under
//! the name of the person who did them, because each machine keeps its own complete record.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    /// Seconds since 1970 (UTC)
    pub at: i64,
    pub user: String,
    /// A short name for what happened: "delete", "rename", "copy", "upload", ...
    pub action: String,
    /// The file or folder it happened to (the first one, when there were several)
    pub target: String,
    /// More about it: where it went, how many items, why it failed
    pub detail: String,
    pub ok: bool,
}

struct Pending {
    at: i64,
    user: String,
    action: String,
    target: String,
    detail: String,
    ok: bool,
}

struct Log {
    tx: Mutex<Sender<Pending>>,
    db_path: String,
}

static LOG: OnceLock<Log> = OnceLock::new();

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS activity_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            user TEXT NOT NULL,
            action TEXT NOT NULL,
            target TEXT NOT NULL DEFAULT '',
            detail TEXT NOT NULL DEFAULT '',
            ok INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_activity_at ON activity_log(at);",
    )
}

fn prune(conn: &Connection, retention_days: u32) {
    if retention_days == 0 {
        return;
    }
    let cutoff = chrono::Utc::now().timestamp() - i64::from(retention_days) * 86_400;
    if let Ok(n) = conn.execute("DELETE FROM activity_log WHERE at < ?1", params![cutoff]) {
        if n > 0 {
            tracing::debug!("Activity log: removed {} entries older than {} days", n, retention_days);
        }
    }
}

/// Starts the activity log for the database at `db_path`. Without this (tests, tools),
/// `record` does nothing.
pub fn init(db_path: &str, retention_days: u32) {
    if db_path.is_empty() || db_path == ":memory:" || LOG.get().is_some() {
        return;
    }
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Activity log is off: cannot open the database: {}", e);
            return;
        }
    };
    let _ = conn.busy_timeout(Duration::from_secs(10));
    if let Err(e) = migrate(&conn) {
        tracing::warn!("Activity log is off: {}", e);
        return;
    }
    prune(&conn, retention_days);
    let (tx, rx) = mpsc::channel::<Pending>();
    std::thread::Builder::new()
        .name("activity-log".into())
        .spawn(move || {
            let mut last_prune = Instant::now();
            while let Ok(first) = rx.recv() {
                // Write what has piled up in one go
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                    if batch.len() >= 500 {
                        break;
                    }
                }
                let write = || -> rusqlite::Result<()> {
                    let tx = conn.unchecked_transaction()?;
                    {
                        let mut stmt = tx.prepare_cached("INSERT INTO activity_log (at, user, action, target, detail, ok) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
                        for p in &batch {
                            stmt.execute(params![p.at, p.user, p.action, p.target, p.detail, p.ok as i64])?;
                        }
                    }
                    tx.commit()
                };
                if let Err(e) = write() {
                    tracing::warn!("Activity log: could not save {} entries: {}", batch.len(), e);
                }
                if last_prune.elapsed() > Duration::from_secs(6 * 3600) {
                    prune(&conn, retention_days);
                    last_prune = Instant::now();
                }
            }
        })
        .ok();
    let _ = LOG.set(Log { tx: Mutex::new(tx), db_path: db_path.to_string() });
}

/// Records one operation. Passwords in addresses are masked; long texts are shortened.
pub fn record(user: &str, action: &str, target: &str, detail: &str, ok: bool) {
    let Some(log) = LOG.get() else { return };
    let clip = |s: &str, n: usize| -> String {
        let s = crate::tools::tasks::sanitize_credentials(s);
        if s.chars().count() > n {
            format!("{}…", s.chars().take(n).collect::<String>())
        } else {
            s
        }
    };
    let p = Pending {
        at: chrono::Utc::now().timestamp(),
        user: clip(if user.is_empty() { "?" } else { user }, 120),
        action: clip(action, 60),
        target: clip(target, 1000),
        detail: clip(detail, 2000),
        ok,
    };
    if let Ok(tx) = log.tx.lock() {
        let _ = tx.send(p);
    }
}

/// Records an operation on several paths: the first is the target, the count goes in the detail
pub fn record_many(user: &str, action: &str, paths: &[String], detail: &str, ok: bool) {
    let target = paths.first().cloned().unwrap_or_default();
    let more = if paths.len() > 1 { format!("{} items", paths.len()) } else { String::new() };
    let detail = match (more.is_empty(), detail.is_empty()) {
        (true, _) => detail.to_string(),
        (false, true) => more,
        (false, false) => format!("{more}; {detail}"),
    };
    record(user, action, &target, &detail, ok);
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Filter {
    /// Words that must appear in the user, action, path or detail
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
    /// Only failures
    #[serde(default)]
    pub failed: Option<bool>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

/// Entries matching the filter, newest first, and how many match in all
pub fn query(f: &Filter) -> Result<(Vec<Entry>, usize), String> {
    let log = LOG.get().ok_or("The activity log is not running")?;
    let conn = Connection::open(&log.db_path).map_err(|e| e.to_string())?;
    let _ = conn.busy_timeout(Duration::from_secs(5));
    let mut wh = vec!["1=1".to_string()];
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(u) = f.user.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        wh.push("user = ?".into());
        args.push(Box::new(u.to_string()));
    }
    if let Some(from) = f.from {
        wh.push("at >= ?".into());
        args.push(Box::new(from));
    }
    if let Some(to) = f.to {
        wh.push("at <= ?".into());
        args.push(Box::new(to));
    }
    if f.failed == Some(true) {
        wh.push("ok = 0".into());
    }
    for word in f.q.as_deref().unwrap_or("").split_whitespace().take(8) {
        let like = format!("%{}%", word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        wh.push("(user LIKE ? ESCAPE '\\' OR action LIKE ? ESCAPE '\\' OR target LIKE ? ESCAPE '\\' OR detail LIKE ? ESCAPE '\\')".into());
        for _ in 0..4 {
            args.push(Box::new(like.clone()));
        }
    }
    let where_sql = wh.join(" AND ");
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
    let total: usize = conn
        .query_row(&format!("SELECT COUNT(*) FROM activity_log WHERE {where_sql}"), refs.as_slice(), |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())? as usize;
    let limit = f.limit.unwrap_or(200).clamp(1, 100_000);
    let offset = f.offset.unwrap_or(0);
    let mut stmt = conn
        .prepare(&format!("SELECT id, at, user, action, target, detail, ok FROM activity_log WHERE {where_sql} ORDER BY at DESC, id DESC LIMIT {limit} OFFSET {offset}"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(refs.as_slice(), |r| {
            Ok(Entry { id: r.get(0)?, at: r.get(1)?, user: r.get(2)?, action: r.get(3)?, target: r.get(4)?, detail: r.get(5)?, ok: r.get::<_, i64>(6)? != 0 })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok((rows, total))
}

/// The matching entries as a CSV file (opens in any spreadsheet)
pub fn to_csv(entries: &[Entry]) -> String {
    let cell = |s: &str| {
        // A leading = + - @ would be run as a formula by spreadsheets
        let s = if s.starts_with(['=', '+', '-', '@']) { format!("'{s}") } else { s.to_string() };
        format!("\"{}\"", s.replace('"', "\"\""))
    };
    let mut out = String::from("time (UTC),user,action,path,detail,result\n");
    for e in entries {
        let when = chrono::DateTime::from_timestamp(e.at, 0).map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
        out.push_str(&format!("{},{},{},{},{},{}\n", cell(&when), cell(&e.user), cell(&e.action), cell(&e.target), cell(&e.detail), if e.ok { "ok" } else { "failed" }));
    }
    out
}

/// Waits until everything recorded so far is saved (for tests)
pub fn flush() {
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_entries_are_removed() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let now = chrono::Utc::now().timestamp();
        for (at, user) in [(now - 91 * 86_400, "old"), (now - 89 * 86_400, "recent"), (now, "today")] {
            conn.execute("INSERT INTO activity_log (at, user, action) VALUES (?1, ?2, 'save')", params![at, user]).unwrap();
        }
        prune(&conn, 90);
        let left: Vec<String> = conn.prepare("SELECT user FROM activity_log ORDER BY at").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(left, ["recent", "today"]);
        prune(&conn, 0); // 0 keeps everything
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM activity_log", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn csv_cells_are_safe_in_spreadsheets() {
        let e = Entry { id: 1, at: 0, user: "bob".into(), action: "delete".into(), target: "=cmd|' /C calc'!A1".into(), detail: "say \"hi\"".into(), ok: false };
        let csv = to_csv(&[e]);
        assert!(csv.contains("\"'=cmd|' /C calc'!A1\""), "{csv}");
        assert!(csv.contains("\"say \"\"hi\"\"\""), "{csv}");
        assert!(csv.trim_end().ends_with("failed"));
    }
}
