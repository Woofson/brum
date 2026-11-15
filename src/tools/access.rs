//! Folder rules (#193): on each machine, who may read a folder, write in it, or not touch it.
//!
//! Rules come from `config.toml` (`[[access.rules]]`) and from the Admin Panel (kept in Brum's
//! database). For a path, the rule for the most specific folder that applies to the person wins;
//! a rule can limit access (read only, no access) or grant it (read, or read and write, in a
//! folder outside their own folders). Admins are not limited by rules. Without a matching rule,
//! the usual account settings decide.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    None,
    Read,
    Write,
}

impl Access {
    pub fn describe(&self) -> &'static str {
        match self {
            Access::None => "no access",
            Access::Read => "read only",
            Access::Write => "read and write",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Set for rules saved in the Admin Panel; empty for rules from config.toml
    #[serde(default)]
    pub id: String,
    pub path: String,
    pub access: Access,
    /// Account names it applies to; with `roles` both empty it applies to everyone
    #[serde(default)]
    pub users: Vec<String>,
    /// Roles it applies to ("user", "readonly")
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub note: String,
}

impl Rule {
    fn applies_to(&self, user: &str, role: &str) -> bool {
        (self.users.is_empty() && self.roles.is_empty())
            || self.users.iter().any(|u| u == user)
            || self.roles.iter().any(|r| r.eq_ignore_ascii_case(role))
    }

    /// Who it is for, in words
    pub fn who(&self) -> String {
        if self.users.is_empty() && self.roles.is_empty() {
            return "everyone".into();
        }
        let mut parts: Vec<String> = self.users.clone();
        parts.extend(self.roles.iter().map(|r| format!("role {r}")));
        parts.join(", ")
    }
}

/// The rule that decides for `path` (already normalised), if any
pub fn decide<'a>(rules: impl Iterator<Item = &'a Rule>, path: &Path, user: &str, role: &str, within: impl Fn(&Path, &Path) -> bool) -> Option<&'a Rule> {
    rules
        .filter(|r| !r.path.trim().is_empty() && r.applies_to(user, role))
        .filter(|r| within(path, Path::new(r.path.trim())))
        // The most specific folder wins; at the same folder, the stricter rule
        .max_by_key(|r| (Path::new(r.path.trim()).components().count(), match r.access { Access::None => 2, Access::Read => 1, Access::Write => 0 }))
}

// ---------- Rules saved in the Admin Panel ----------

struct Store {
    db_path: String,
    rules: RwLock<Vec<Rule>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS access_rules (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL,
            access TEXT NOT NULL,
            users TEXT NOT NULL DEFAULT '[]',
            roles TEXT NOT NULL DEFAULT '[]',
            note TEXT NOT NULL DEFAULT '',
            created_by TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0
        );",
    )
}

fn load(conn: &Connection) -> Vec<Rule> {
    let Ok(mut stmt) = conn.prepare("SELECT id, path, access, users, roles, note FROM access_rules ORDER BY path") else { return Vec::new() };
    stmt.query_map([], |r| {
        let access: String = r.get(2)?;
        let users: String = r.get(3)?;
        let roles: String = r.get(4)?;
        Ok(Rule {
            id: r.get(0)?,
            path: r.get(1)?,
            access: serde_json::from_value(serde_json::Value::String(access)).unwrap_or(Access::None),
            users: serde_json::from_str(&users).unwrap_or_default(),
            roles: serde_json::from_str(&roles).unwrap_or_default(),
            note: r.get(5)?,
        })
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

pub fn init(db_path: &str) {
    if db_path.is_empty() || db_path == ":memory:" || STORE.get().is_some() {
        return;
    }
    match Connection::open(db_path).and_then(|c| migrate(&c).map(|_| c)) {
        Ok(conn) => {
            let rules = load(&conn);
            let _ = STORE.set(Store { db_path: db_path.to_string(), rules: RwLock::new(rules) });
        }
        Err(e) => tracing::warn!("Folder rules from the Admin Panel are off: cannot open the database: {}", e),
    }
}

pub fn saved() -> Vec<Rule> {
    STORE.get().and_then(|s| s.rules.read().ok().map(|r| r.clone())).unwrap_or_default()
}

/// Runs `f` over the saved rules without copying them
pub fn with_saved<T>(f: impl FnOnce(&[Rule]) -> T) -> T {
    match STORE.get().and_then(|s| s.rules.read().ok()) {
        Some(r) => f(&r),
        None => f(&[]),
    }
}

pub fn add(mut rule: Rule, by: &str) -> Result<Rule, String> {
    let store = STORE.get().ok_or("Folder rules are not available")?;
    if rule.path.trim().is_empty() {
        return Err("Choose a folder".into());
    }
    rule.path = rule.path.trim().to_string();
    rule.users = rule.users.iter().map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).collect();
    rule.roles = rule.roles.iter().map(|u| u.trim().to_lowercase()).filter(|u| !u.is_empty()).collect();
    rule.id = uuid::Uuid::new_v4().to_string();
    let conn = Connection::open(&store.db_path).map_err(|e| e.to_string())?;
    migrate(&conn).map_err(|e| e.to_string())?;
    let access = serde_json::to_value(rule.access).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "none".into());
    conn.execute(
        "INSERT INTO access_rules (id, path, access, users, roles, note, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![rule.id, rule.path, access, serde_json::to_string(&rule.users).unwrap_or_default(), serde_json::to_string(&rule.roles).unwrap_or_default(), rule.note, by, chrono::Utc::now().timestamp()],
    )
    .map_err(|e| e.to_string())?;
    if let Ok(mut r) = store.rules.write() {
        r.push(rule.clone());
    }
    Ok(rule)
}

pub fn remove(id: &str) -> Result<Option<Rule>, String> {
    let store = STORE.get().ok_or("Folder rules are not available")?;
    let conn = Connection::open(&store.db_path).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM access_rules WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    let mut removed = None;
    if let Ok(mut r) = store.rules.write() {
        if let Some(i) = r.iter().position(|x| x.id == id) {
            removed = Some(r.remove(i));
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(path: &str, access: Access, users: &[&str], roles: &[&str]) -> Rule {
        Rule { id: String::new(), path: path.into(), access, users: users.iter().map(|s| s.to_string()).collect(), roles: roles.iter().map(|s| s.to_string()).collect(), note: String::new() }
    }

    fn within(p: &Path, root: &Path) -> bool {
        p.starts_with(root)
    }

    #[test]
    fn the_most_specific_rule_wins() {
        let rules = [
            rule("/srv", Access::Read, &[], &[]),
            rule("/srv/media", Access::Write, &["bob"], &[]),
            rule("/srv/media/private", Access::None, &[], &["user"]),
            rule("/srv/archive", Access::Read, &[], &[]),
            rule("/srv/archive", Access::None, &["eve"], &[]),
        ];
        let d = |p: &str, u: &str, role: &str| decide(rules.iter(), Path::new(p), u, role, within).map(|r| r.access);
        assert_eq!(d("/srv/docs/a.txt", "bob", "user"), Some(Access::Read));
        assert_eq!(d("/srv/media/film.mkv", "bob", "user"), Some(Access::Write));
        assert_eq!(d("/srv/media/film.mkv", "ann", "user"), Some(Access::Read), "the bob rule is not for ann");
        assert_eq!(d("/srv/media/private/x", "bob", "user"), Some(Access::None));
        assert_eq!(d("/srv/archive/old", "eve", "user"), Some(Access::None), "same folder: the stricter rule");
        assert_eq!(d("/home/bob/x", "bob", "user"), None, "no rule: the account decides");
        assert_eq!(d("/srvx/file", "bob", "user"), None, "only whole folder names match");
    }
}
