//! Joining the Fleet with a pairing code (#193).
//!
//! On the main Brum an admin asks for a code: eight characters, usable once, valid for ten
//! minutes. The new machine sends the code (with its name and addresses) to the main Brum,
//! which answers with the secret it signs logins with. From then on both machines accept each
//! other's logins, and each lists the other as a Fleet machine. Brum never rewrites
//! `config.toml`: paired machines are kept in Brum's database, and the new machine keeps the
//! Fleet secret in `fleet-secret.key` next to its database.

use crate::config::FleetNodeConfig;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// How long a code can be used
pub const CODE_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// After this many wrong codes within ten minutes, every open code is cancelled
const MAX_WRONG_CODES: u32 = 10;
/// Easy to read out and type: no 0/O, 1/I/L
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const SECRET_FILE: &str = "fleet-secret.key";

struct Code {
    expires: Instant,
    created_by: String,
    /// The machine that used it
    joined: Option<String>,
}

#[derive(Default)]
struct Codes {
    open: HashMap<String, Code>,
    wrong: u32,
    wrong_since: Option<Instant>,
}

static CODES: OnceLock<Mutex<Codes>> = OnceLock::new();

fn codes() -> &'static Mutex<Codes> {
    CODES.get_or_init(|| Mutex::new(Codes::default()))
}

/// `ABCD-EFGH` → `ABCDEFGH`, ignoring case, spaces and dashes
pub fn normalize(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

pub fn display(code: &str) -> String {
    if code.len() == 8 {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_string()
    }
}

/// A new code, made by `by` (an admin)
pub fn new_code(by: &str) -> String {
    use rand::Rng;
    let mut rng = rand::rngs::OsRng;
    let code: String = (0..8).map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char).collect();
    if let Ok(mut c) = codes().lock() {
        let now = Instant::now();
        c.open.retain(|_, v| v.expires > now || v.joined.is_some() && v.expires + CODE_LIFETIME > now);
        c.open.insert(code.clone(), Code { expires: now + CODE_LIFETIME, created_by: by.to_string(), joined: None });
    }
    code
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CodeStatus {
    Waiting { seconds_left: u64 },
    Joined { machine: String },
    Expired,
}

pub fn status(code: &str) -> CodeStatus {
    let code = normalize(code);
    let Ok(c) = codes().lock() else { return CodeStatus::Expired };
    match c.open.get(&code) {
        Some(Code { joined: Some(m), .. }) => CodeStatus::Joined { machine: m.clone() },
        Some(v) if v.expires > Instant::now() => CodeStatus::Waiting { seconds_left: v.expires.saturating_duration_since(Instant::now()).as_secs() },
        _ => CodeStatus::Expired,
    }
}

/// Uses up a code for the machine `machine`; returns who made it. Every wrong code counts, and
/// too many cancel all open codes, so guessing does not work.
pub fn redeem(code: &str, machine: &str) -> Result<String, String> {
    use subtle::ConstantTimeEq;
    let wanted = normalize(code);
    let mut c = codes().lock().map_err(|_| "Busy".to_string())?;
    let now = Instant::now();
    let found = c
        .open
        .iter()
        .find(|(k, v)| v.joined.is_none() && v.expires > now && k.as_bytes().ct_eq(wanted.as_bytes()).into())
        .map(|(k, _)| k.clone());
    match found {
        Some(k) => {
            let v = c.open.get_mut(&k).unwrap();
            v.joined = Some(machine.to_string());
            Ok(v.created_by.clone())
        }
        None => {
            if c.wrong_since.is_none_or(|t| now.duration_since(t) > CODE_LIFETIME) {
                c.wrong_since = Some(now);
                c.wrong = 0;
            }
            c.wrong += 1;
            if c.wrong >= MAX_WRONG_CODES {
                c.open.retain(|_, v| v.joined.is_some());
                tracing::warn!("Fleet pairing: {} wrong codes in a short time; all open pairing codes were cancelled", c.wrong);
            }
            Err("The pairing code is wrong or has expired. Make a new one on the main Brum.".to_string())
        }
    }
}

// ---------- Machines paired with this one ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedMachine {
    pub id: String,
    pub name: String,
    pub urls: Vec<String>,
    #[serde(default)]
    pub os: String,
    /// "joined" (it joined this Brum) or "main" (this Brum joined it)
    pub role: String,
    pub paired_at: i64,
    pub paired_by: String,
}

struct Store {
    db_path: String,
    list: RwLock<Vec<PairedMachine>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fleet_paired (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            urls TEXT NOT NULL,
            os TEXT NOT NULL DEFAULT '',
            role TEXT NOT NULL,
            paired_at INTEGER NOT NULL,
            paired_by TEXT NOT NULL DEFAULT ''
        );",
    )
}

fn load(conn: &Connection) -> Vec<PairedMachine> {
    let Ok(mut stmt) = conn.prepare("SELECT id, name, urls, os, role, paired_at, paired_by FROM fleet_paired ORDER BY name") else { return Vec::new() };
    stmt.query_map([], |r| {
        let urls: String = r.get(2)?;
        Ok(PairedMachine { id: r.get(0)?, name: r.get(1)?, urls: serde_json::from_str(&urls).unwrap_or_default(), os: r.get(3)?, role: r.get(4)?, paired_at: r.get(5)?, paired_by: r.get(6)? })
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Loads the paired machines of the database at `db_path`
pub fn init(db_path: &str) {
    if db_path.is_empty() || db_path == ":memory:" || STORE.get().is_some() {
        return;
    }
    let list = match Connection::open(db_path).and_then(|c| migrate(&c).map(|_| c)) {
        Ok(conn) => load(&conn),
        Err(e) => {
            tracing::warn!("Fleet pairing: cannot open the database: {}", e);
            return;
        }
    };
    let _ = STORE.set(Store { db_path: db_path.to_string(), list: RwLock::new(list) });
}

pub fn list() -> Vec<PairedMachine> {
    STORE.get().and_then(|s| s.list.read().ok().map(|l| l.clone())).unwrap_or_default()
}

/// A Fleet id from a machine name: `My NAS` → `my-nas`, unique among `taken`
pub fn make_id(name: &str, taken: &[String]) -> String {
    let mut base: String = name.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() { "machine".to_string() } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t == &id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Adds a machine (or replaces the one with the same addresses or id)
pub fn add(m: PairedMachine) -> Result<(), String> {
    let store = STORE.get().ok_or("Pairing is not available")?;
    let conn = Connection::open(&store.db_path).map_err(|e| e.to_string())?;
    migrate(&conn).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO fleet_paired (id, name, urls, os, role, paired_at, paired_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![m.id, m.name, serde_json::to_string(&m.urls).unwrap_or_default(), m.os, m.role, m.paired_at, m.paired_by],
    )
    .map_err(|e| e.to_string())?;
    if let Ok(mut l) = store.list.write() {
        l.retain(|x| x.id != m.id);
        l.push(m);
        l.sort_by_key(|m| m.name.to_lowercase());
    }
    Ok(())
}

pub fn remove(id: &str) -> Result<bool, String> {
    let store = STORE.get().ok_or("Pairing is not available")?;
    let conn = Connection::open(&store.db_path).map_err(|e| e.to_string())?;
    let n = conn.execute("DELETE FROM fleet_paired WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    if let Ok(mut l) = store.list.write() {
        l.retain(|x| x.id != id);
    }
    Ok(n > 0)
}

impl PairedMachine {
    /// As a Fleet machine, like an entry in `config.toml`
    pub fn to_node(&self) -> FleetNodeConfig {
        FleetNodeConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            url: self.urls.first().cloned().unwrap_or_default(),
            urls: self.urls.iter().skip(1).cloned().collect(),
            tags: vec!["paired".to_string()],
            ..Default::default()
        }
    }
}

// ---------- The Fleet secret on a machine that joined ----------

pub fn secret_path(db_path: &str) -> Option<PathBuf> {
    crate::logging::credentials_note_dir(db_path).map(|d| d.join(SECRET_FILE))
}

/// The Fleet secret this machine got when it joined, if any
pub fn joined_secret(db_path: &str) -> Option<String> {
    let s = std::fs::read_to_string(secret_path(db_path)?).ok()?;
    let s = s.trim().to_string();
    (s.len() >= 32).then_some(s)
}

/// Keeps the Fleet secret next to the database, readable only by Brum
pub fn save_secret(db_path: &str, secret: &str) -> Result<PathBuf, String> {
    let path = secret_path(db_path).ok_or("No database folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&path).map_err(|e| e.to_string())?;
        f.write_all(secret.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&path).map_err(|e| e.to_string())?;
        crate::win_perms::restrict_file_to_owner(&path).map_err(|e| e.to_string())?;
        f.write_all(secret.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

// ---------- The new machine's side ----------

#[derive(Debug, Serialize, Deserialize)]
pub struct JoinRequest {
    pub code: String,
    pub name: String,
    pub urls: Vec<String>,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JoinAnswer {
    pub secret: String,
    /// The main Brum's name and system, so the new machine can list it
    pub name: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub version: String,
}

/// The addresses other machines can reach this Brum on: its name, and the network address it
/// uses towards the main Brum
pub fn own_addresses(port: u16, main_url: &str) -> Vec<String> {
    let mut out = Vec::new();
    let host = crate::server::system_handlers::get_system_hostname();
    if !host.is_empty() && host != "localhost" {
        out.push(format!("http://{}:{}", host.to_lowercase(), port));
    }
    if let Some((_, h)) = crate::server::middleware::extract_remote_scheme_host(main_url) {
        let target_port = main_url.rsplit(':').next().and_then(|p| p.trim_end_matches('/').parse::<u16>().ok()).unwrap_or(80);
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if sock.connect((h.trim_matches(['[', ']']), target_port)).is_ok() {
                if let Ok(local) = sock.local_addr() {
                    let ip = local.ip();
                    if !ip.is_unspecified() && !ip.is_loopback() {
                        out.push(match ip {
                            std::net::IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
                            v4 => format!("http://{v4}:{port}"),
                        });
                    }
                }
            }
        }
    }
    out.dedup();
    out
}

/// Joins the Fleet of the Brum at `main_url` with `code`: asks it for the Fleet secret, keeps
/// the secret next to this machine's database and lists the main Brum as a Fleet machine.
/// Returns the main Brum's name. Brum must restart afterwards to sign logins with the secret.
pub fn join(main_url: &str, code: &str, my_name: &str, my_urls: Vec<String>, db_path: &str) -> Result<String, String> {
    let main_url = main_url.trim().trim_end_matches('/').to_string();
    let main_url = if main_url.starts_with("http://") || main_url.starts_with("https://") { main_url } else { format!("http://{main_url}") };
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let body = JoinRequest { code: code.to_string(), name: my_name.to_string(), urls: my_urls, os: std::env::consts::OS.to_string(), version: env!("CARGO_PKG_VERSION").to_string() };
    let resp = client.post(format!("{main_url}/api/public/fleet/join")).json(&body).send().map_err(|e| format!("Cannot reach {main_url}: {e}"))?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from)).unwrap_or(text);
        return Err(match status.as_u16() {
            404 => format!("{main_url} does not offer pairing (Brum 1.23 or newer is needed there)"),
            _ => msg,
        });
    }
    let answer: JoinAnswer = serde_json::from_str(&text).map_err(|_| "The main Brum sent an answer Brum cannot read".to_string())?;
    if answer.secret.trim().len() < 32 {
        return Err("The main Brum sent no usable Fleet secret".into());
    }
    save_secret(db_path, answer.secret.trim())?;
    let taken: Vec<String> = list().into_iter().map(|m| m.id).collect();
    let existing = list().into_iter().find(|m| m.urls.iter().any(|u| u == &main_url));
    add(PairedMachine {
        id: existing.map(|m| m.id).unwrap_or_else(|| make_id(&answer.name, &taken)),
        name: answer.name.clone(),
        urls: vec![main_url],
        os: answer.os,
        role: "main".into(),
        paired_at: chrono::Utc::now().timestamp(),
        paired_by: String::new(),
    })?;
    Ok(answer.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_work_once_and_resist_guessing() {
        let code = new_code("admin");
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| ALPHABET.contains(&b)));
        assert!(matches!(status(&code), CodeStatus::Waiting { .. }));
        // Typed in lower case with a dash
        assert_eq!(redeem(&display(&code).to_lowercase(), "nas").unwrap(), "admin");
        assert_eq!(status(&code), CodeStatus::Joined { machine: "nas".into() });
        assert!(redeem(&code, "again").is_err(), "a code works once");
        // Ten wrong codes cancel every open code
        let other = new_code("admin");
        for _ in 0..MAX_WRONG_CODES {
            let _ = redeem("WRONG123", "x");
        }
        assert!(redeem(&other, "late").is_err());
    }

    #[test]
    fn ids_from_names() {
        assert_eq!(make_id("My NAS", &[]), "my-nas");
        assert_eq!(make_id("my nas!", &["my-nas".into()]), "my-nas-2");
        assert_eq!(make_id("  ", &[]), "machine");
    }
}
