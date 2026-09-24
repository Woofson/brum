use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tracing::info;

pub mod pam;
pub mod oidc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub role: String, // "admin", "user", "readonly"
    pub home_dir: String,
    pub is_pam: bool,
    pub is_disabled: bool,
    pub allowed_services: String, // JSON array e.g. ["*"] or ["local","smb","s3","upload","download"]
    pub allowed_roots: String,    // JSON array e.g. ["*"] or ["data","storage","home"]
    pub can_install_plugins: bool,
    pub allowed_plugins: String,  // JSON array e.g. ["*"]
    pub blocked_plugins: String,  // JSON array e.g. []
}

impl User {
    pub fn resolve_avatar(&mut self) {
        if self.avatar_url.is_none() || self.avatar_url.as_deref() == Some("") {
            self.avatar_url = resolve_system_avatar(&self.username, &self.home_dir);
        }
    }
}

pub fn resolve_system_avatar(username: &str, home_dir: &str) -> Option<String> {
    #[cfg(unix)]
    {
        use base64::Engine;
        let home_path = std::path::Path::new(home_dir);
        let mut candidates = vec![
            home_path.join(".face"),
            home_path.join(".face.icon"),
            home_path.join(".face.png"),
            home_path.join(".face.jpg"),
            home_path.join(".face.jpeg"),
            home_path.join(".face.svg"),
            home_path.join(".face.webp"),
            home_path.join(".avatar"),
            home_path.join(".avatar.png"),
            home_path.join(".avatar.svg"),
            std::path::PathBuf::from(format!("/var/lib/AccountsService/icons/{}", username)),
        ];

        if !home_dir.contains(username) {
            candidates.push(std::path::PathBuf::from(format!("/home/{}/.face", username)));
            candidates.push(std::path::PathBuf::from(format!("/home/{}/.face.icon", username)));
            candidates.push(std::path::PathBuf::from(format!("/home/{}/.face.png", username)));
            candidates.push(std::path::PathBuf::from(format!("/home/{}/.face.svg", username)));
        }

        for path in &candidates {
            if path.exists() {
                if let Ok(bytes) = std::fs::read(path) {
                    if !bytes.is_empty() && bytes.len() <= 10 * 1024 * 1024 {
                        let trimmed = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
                        let is_svg = trimmed.starts_with(b"<?xml")
                            || trimmed.starts_with(b"<svg")
                            || (trimmed.len() > 10 && std::str::from_utf8(&trimmed[..std::cmp::min(trimmed.len(), 512)]).map(|s| s.contains("<svg")).unwrap_or(false));

                        let mime = if is_svg {
                            "image/svg+xml"
                        } else if trimmed.starts_with(&[0x89, b'P', b'N', b'G']) {
                            "image/png"
                        } else if trimmed.starts_with(&[0xFF, 0xD8, 0xFF]) {
                            "image/jpeg"
                        } else if trimmed.starts_with(b"RIFF") && trimmed.len() > 12 && &trimmed[8..12] == b"WEBP" {
                            "image/webp"
                        } else if trimmed.starts_with(b"GIF87a") || trimmed.starts_with(b"GIF89a") {
                            "image/gif"
                        } else if trimmed.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
                            "image/x-icon"
                        } else {
                            "image/png"
                        };

                        return Some(format!(
                            "data:{};base64,{}",
                            mime,
                            base64::engine::general_purpose::STANDARD.encode(&bytes)
                        ));
                    }
                }
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = (username, home_dir);
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalMount {
    pub id: i64,
    pub name: String,
    pub protocol: String,
    pub target_uri: String,
    pub options_json: String,
    pub allowed_users: String, // JSON array e.g. ["*"] or ["bolt", "alice"]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiTokenInfo {
    pub id: String,
    pub name: String,
    pub username: String,
    pub token_prefix: String,
    pub role: String,
    pub allowed_roots: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedApiToken {
    pub token: String,
    pub info: ApiTokenInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub home_dir: String,
    pub is_pam: bool,
    #[serde(default = "default_allowed_roots_claims")]
    pub allowed_roots: Option<String>,
    #[serde(default)]
    pub token_id: Option<String>,
    pub exp: i64,
}

fn default_allowed_roots_claims() -> Option<String> {
    Some("[\"*\"]".to_string())
}

pub struct AuthManager {
    db: Arc<Mutex<Connection>>,
    jwt_secret: String,
    session_hours: u64,
    auth_mode: String,
    #[allow(dead_code)]
    pam_service: String,
    cached_pam_service: Arc<std::sync::RwLock<Option<String>>>,
}

impl AuthManager {
    pub fn new(
        db_path: &str,
        jwt_secret: &str,
        session_hours: u64,
        auth_mode: &str,
        pam_service: &str,
        default_admin_user: &str,
        default_admin_pass: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let conn = Connection::open(db_path)?;

        // High-performance SQLite concurrency and memory pragmas
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA cache_size = -16000;
             PRAGMA busy_timeout = 5000;",
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'user',
                home_dir TEXT NOT NULL DEFAULT '/',
                nickname TEXT,
                email TEXT,
                avatar_url TEXT,
                allowed_services TEXT DEFAULT '[\"*\"]',
                allowed_roots TEXT DEFAULT '[\"*\"]',
                can_install_plugins INTEGER DEFAULT 0,
                allowed_plugins TEXT DEFAULT '[\"*\"]',
                blocked_plugins TEXT DEFAULT '[]',
                is_pam INTEGER DEFAULT 0,
                is_disabled INTEGER DEFAULT 0,
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS global_mounts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                protocol TEXT NOT NULL,
                target_uri TEXT NOT NULL,
                options_json TEXT DEFAULT '{}',
                allowed_users TEXT DEFAULT '[\"*\"]',
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS bookmarks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL,
                name TEXT NOT NULL,
                protocol TEXT NOT NULL,
                path TEXT NOT NULL,
                password TEXT,
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS security_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS shares (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                token TEXT UNIQUE NOT NULL,
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                is_dir INTEGER NOT NULL DEFAULT 0,
                allow_upload INTEGER NOT NULL DEFAULT 0,
                allow_view INTEGER NOT NULL DEFAULT 1,
                allow_download INTEGER NOT NULL DEFAULT 1,
                password_hash TEXT,
                expires_at TEXT,
                created_by TEXT NOT NULL,
                created_at TEXT NOT NULL,
                download_count INTEGER NOT NULL DEFAULT 0,
                max_downloads INTEGER NOT NULL DEFAULT 0,
                allowed_emails TEXT,
                require_email INTEGER NOT NULL DEFAULT 0,
                watermark_enabled INTEGER NOT NULL DEFAULT 0,
                watermark_text TEXT,
                status TEXT NOT NULL DEFAULT 'active'
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS share_access_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                share_id INTEGER NOT NULL,
                token TEXT NOT NULL,
                visitor_email TEXT,
                ip_address TEXT NOT NULL,
                user_agent TEXT,
                action TEXT NOT NULL,
                target_file TEXT,
                accessed_at TEXT NOT NULL,
                FOREIGN KEY(share_id) REFERENCES shares(id) ON DELETE CASCADE
            )",
            [],
        )?;
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_share_logs_token ON share_access_logs (token)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_share_logs_share_id ON share_access_logs (share_id)", []);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_preferences (
                username TEXT PRIMARY KEY,
                preferences_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS tetradog_scores (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL,
                player_name TEXT NOT NULL,
                score INTEGER NOT NULL,
                lines_cleared INTEGER NOT NULL,
                level INTEGER NOT NULL,
                duration_seconds INTEGER NOT NULL DEFAULT 0,
                mode TEXT NOT NULL DEFAULT 'marathon',
                created_at TEXT NOT NULL
            )",
            [],
        )?;
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_tetradog_scores ON tetradog_scores (score DESC)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_tetradog_user ON tetradog_scores (username)", []);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS api_tokens (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                username TEXT NOT NULL,
                token_prefix TEXT NOT NULL,
                role TEXT NOT NULL,
                allowed_roots TEXT DEFAULT '[\"*\"]',
                expires_at INTEGER,
                created_at INTEGER NOT NULL,
                last_used_at INTEGER
            )",
            [],
        )?;
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens (username)", []);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS db_notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                category TEXT NOT NULL DEFAULT 'General',
                section TEXT NOT NULL DEFAULT 'Default',
                tags TEXT DEFAULT '[]',
                is_pinned INTEGER NOT NULL DEFAULT 0,
                is_archived INTEGER NOT NULL DEFAULT 0,
                is_encrypted INTEGER NOT NULL DEFAULT 0,
                color TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_db_notes_user ON db_notes (username)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_db_notes_updated ON db_notes (updated_at DESC)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_db_notes_category ON db_notes (category)", []);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS db_note_attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                note_id INTEGER,
                username TEXT NOT NULL,
                filename TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                storage_path TEXT NOT NULL,
                sha256 TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(note_id) REFERENCES db_notes(id) ON DELETE CASCADE
            )",
            [],
        )?;
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_db_note_attachments_note ON db_note_attachments (note_id)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_db_note_attachments_user ON db_note_attachments (username)", []);

        // Safe migrations for newly added columns
        let _ = conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN email TEXT", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN avatar_url TEXT", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN allowed_services TEXT DEFAULT '[\"*\"]'", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN allowed_roots TEXT DEFAULT '[\"*\"]'", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN is_pam INTEGER DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN is_disabled INTEGER DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN can_install_plugins INTEGER DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN allowed_plugins TEXT DEFAULT '[\"*\"]'", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN blocked_plugins TEXT DEFAULT '[]'", []);

        let _ = conn.execute("ALTER TABLE shares ADD COLUMN allow_view INTEGER DEFAULT 1", []);
        let _ = conn.execute("ALTER TABLE shares ADD COLUMN allow_download INTEGER DEFAULT 1", []);
        let _ = conn.execute("ALTER TABLE shares ADD COLUMN allowed_emails TEXT", []);
        let _ = conn.execute("ALTER TABLE shares ADD COLUMN require_email INTEGER DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE shares ADD COLUMN watermark_enabled INTEGER DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE shares ADD COLUMN watermark_text TEXT", []);
        let _ = conn.execute("ALTER TABLE shares ADD COLUMN status TEXT DEFAULT 'active'", []);

        let auth = Self {
            db: Arc::new(Mutex::new(conn)),
            jwt_secret: jwt_secret.to_string(),
            session_hours,
            auth_mode: auth_mode.to_string(),
            pam_service: pam_service.to_string(),
            cached_pam_service: Arc::new(std::sync::RwLock::new(None)),
        };

        // Seed default admin user if database is empty
        if auth.count_users()? == 0 {
            info!("No users found in database. Creating default admin user: {}", default_admin_user);
            #[cfg(windows)]
            let def_home = dirs::home_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "C:\\".to_string());
            #[cfg(not(windows))]
            let def_home = "/".to_string();

            auth.create_user(default_admin_user, default_admin_pass, "admin", &def_home, Some("[\"*\"]"))?;
        }

        Ok(auth)
    }

    pub fn count_users(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM users")?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn db(&self) -> Arc<Mutex<Connection>> {
        self.db.clone()
    }

    pub fn create_user(
        &self,
        username: &str,
        password: &str,
        role: &str,
        home_dir: &str,
        allowed_roots: Option<&str>,
    ) -> Result<User, Box<dyn std::error::Error + Send + Sync>> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| format!("Password hashing failed: {}", e))?
            .to_string();

        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let now = Utc::now().to_rfc3339();
        let roots_json = allowed_roots.unwrap_or("[\"*\"]");

        conn.execute(
            "INSERT INTO users (username, password_hash, role, home_dir, allowed_services, allowed_roots, can_install_plugins, allowed_plugins, blocked_plugins, is_pam, is_disabled, created_at) VALUES (?1, ?2, ?3, ?4, '[\"*\"]', ?5, 0, '[\"*\"]', '[]', 0, 0, ?6)",
            params![username, password_hash, role, home_dir, roots_json, now],
        )?;

        let id = conn.last_insert_rowid();

        let mut user = User {
            id,
            username: username.to_string(),
            nickname: None,
            email: None,
            avatar_url: None,
            role: role.to_string(),
            home_dir: home_dir.to_string(),
            is_pam: false,
            is_disabled: false,
            allowed_services: "[\"*\"]".to_string(),
            allowed_roots: roots_json.to_string(),
            can_install_plugins: false,
            allowed_plugins: "[\"*\"]".to_string(),
            blocked_plugins: "[]".to_string(),
        };
        user.resolve_avatar();
        Ok(user)
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<User>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare(
            "SELECT id, username, nickname, email, avatar_url, role, home_dir, is_pam, is_disabled, allowed_services, allowed_roots, can_install_plugins, allowed_plugins, blocked_plugins FROM users WHERE username = ?1"
        )?;

        let user = stmt.query_row(params![username], |row| {
            let mut u = User {
                id: row.get(0)?,
                username: row.get(1)?,
                nickname: row.get(2)?,
                email: row.get(3)?,
                avatar_url: row.get(4)?,
                role: row.get(5)?,
                home_dir: row.get(6)?,
                is_pam: row.get::<_, i64>(7)? != 0,
                is_disabled: row.get::<_, i64>(8)? != 0,
                allowed_services: row.get::<_, Option<String>>(9)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                allowed_roots: row.get::<_, Option<String>>(10)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                can_install_plugins: row.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
                allowed_plugins: row.get::<_, Option<String>>(12)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                blocked_plugins: row.get::<_, Option<String>>(13)?.unwrap_or_else(|| "[]".to_string()),
            };
            u.resolve_avatar();
            Ok(u)
        }).optional()?;

        Ok(user)
    }

    pub fn list_users(&self) -> Result<Vec<User>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare(
            "SELECT id, username, nickname, email, avatar_url, role, home_dir, is_pam, is_disabled, allowed_services, allowed_roots, can_install_plugins, allowed_plugins, blocked_plugins FROM users ORDER BY username ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            let mut u = User {
                id: row.get(0)?,
                username: row.get(1)?,
                nickname: row.get(2)?,
                email: row.get(3)?,
                avatar_url: row.get(4)?,
                role: row.get(5)?,
                home_dir: row.get(6)?,
                is_pam: row.get::<_, i64>(7)? != 0,
                is_disabled: row.get::<_, i64>(8)? != 0,
                allowed_services: row.get::<_, Option<String>>(9)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                allowed_roots: row.get::<_, Option<String>>(10)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                can_install_plugins: row.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
                allowed_plugins: row.get::<_, Option<String>>(12)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                blocked_plugins: row.get::<_, Option<String>>(13)?.unwrap_or_else(|| "[]".to_string()),
            };
            u.resolve_avatar();
            Ok(u)
        })?;

        let mut users = Vec::new();
        for user in rows {
            users.push(user?);
        }
        Ok(users)
    }

    pub fn sync_pam_user_to_db(
        &self,
        username: &str,
        role: &str,
        home_dir: &str,
    ) -> Result<User, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO users (username, password_hash, role, home_dir, nickname, allowed_services, allowed_roots, is_pam, is_disabled, created_at)
             VALUES (?1, 'PAM_MANAGED', ?2, ?3, ?1, '[\"*\"]', '[\"*\"]', 1, 0, ?4)
             ON CONFLICT(username) DO UPDATE SET is_pam = 1",
            params![username, role, home_dir, now],
        )?;

        drop(conn);
        let mut user = self.get_user_by_username(username)?
            .ok_or_else(|| format!("Failed to retrieve synced PAM user"))?;
        user.resolve_avatar();
        Ok(user)
    }

    pub fn update_user_profile(
        &self,
        username: &str,
        nickname: Option<&str>,
        email: Option<&str>,
        avatar_url: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let affected = conn.execute(
            "UPDATE users SET nickname = ?1, email = ?2, avatar_url = ?3 WHERE username = ?4",
            params![nickname, email, avatar_url, username],
        )?;
        Ok(affected > 0)
    }

    pub fn update_user_password(
        &self,
        username: &str,
        new_password: &str,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(new_password.as_bytes(), &salt)
            .map_err(|e| format!("Password hashing failed: {}", e))?
            .to_string();

        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let affected = conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE username = ?2",
            params![password_hash, username],
        )?;
        Ok(affected > 0)
    }

    pub fn update_user_rbac(
        &self,
        username: &str,
        role: &str,
        allowed_services: &str,
        allowed_roots: Option<&str>,
        home_dir: Option<&str>,
        can_install_plugins: Option<bool>,
        allowed_plugins: Option<&str>,
        blocked_plugins: Option<&str>,
        is_disabled: bool,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let dis_int = if is_disabled { 1 } else { 0 };
        let roots_str = allowed_roots.unwrap_or("[\"*\"]");
        let can_inst_int = if can_install_plugins.unwrap_or(false) { 1 } else { 0 };
        let allowed_plugs_str = allowed_plugins.unwrap_or("[\"*\"]");
        let blocked_plugs_str = blocked_plugins.unwrap_or("[]");

        let affected = if let Some(hd) = home_dir {
            conn.execute(
                "UPDATE users SET role = ?1, allowed_services = ?2, allowed_roots = ?3, home_dir = ?4, can_install_plugins = ?5, allowed_plugins = ?6, blocked_plugins = ?7, is_disabled = ?8 WHERE username = ?9",
                params![role, allowed_services, roots_str, hd, can_inst_int, allowed_plugs_str, blocked_plugs_str, dis_int, username],
            )?
        } else {
            conn.execute(
                "UPDATE users SET role = ?1, allowed_services = ?2, allowed_roots = ?3, can_install_plugins = ?4, allowed_plugins = ?5, blocked_plugins = ?6, is_disabled = ?7 WHERE username = ?8",
                params![role, allowed_services, roots_str, can_inst_int, allowed_plugs_str, blocked_plugs_str, dis_int, username],
            )?
        };
        Ok(affected > 0)
    }

    pub fn delete_user(&self, username: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let affected = conn.execute("DELETE FROM users WHERE username = ?1", params![username])?;
        Ok(affected > 0)
    }

    pub fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<User, Box<dyn std::error::Error + Send + Sync>> {
        // 1. If auth mode is mixed or builtin, try builtin DB first
        if self.auth_mode == "builtin" || self.auth_mode == "mixed" {
            let user_res = {
                let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
                let mut stmt = conn.prepare(
                    "SELECT id, username, password_hash, role, home_dir, nickname, email, avatar_url, allowed_services, allowed_roots, can_install_plugins, allowed_plugins, blocked_plugins, is_pam, is_disabled FROM users WHERE username = ?1"
                )?;
                stmt.query_row(params![username], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                        row.get::<_, Option<String>>(9)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                        row.get::<_, Option<i64>>(10)?.unwrap_or(0) != 0,
                        row.get::<_, Option<String>>(11)?.unwrap_or_else(|| "[\"*\"]".to_string()),
                        row.get::<_, Option<String>>(12)?.unwrap_or_else(|| "[]".to_string()),
                        row.get::<_, i64>(13)? != 0,
                        row.get::<_, i64>(14)? != 0,
                    ))
                }).optional()?
            };

            if let Some((id, uname, hash_str, role, home_dir, nickname, email, avatar_url, allowed_services, allowed_roots, can_install_plugins, allowed_plugins, blocked_plugins, is_pam, is_disabled)) = user_res {
                if is_disabled {
                    return Err("Account is disabled. Please contact an administrator.".into());
                }

                // Only verify Argon2 if this is a native DB user (not PAM_MANAGED)
                if !is_pam && hash_str != "PAM_MANAGED" {
                    if let Ok(parsed_hash) = PasswordHash::new(&hash_str) {
                        if Argon2::default().verify_password(password.as_bytes(), &parsed_hash).is_ok() {
                            let mut u = User {
                                id,
                                username: uname,
                                nickname,
                                email,
                                avatar_url,
                                role,
                                home_dir,
                                is_pam,
                                is_disabled,
                                allowed_services,
                                allowed_roots,
                                can_install_plugins,
                                allowed_plugins,
                                blocked_plugins,
                            };
                            u.resolve_avatar();
                            return Ok(u);
                        } else {
                            return Err("Invalid username or password".into());
                        }
                    } else {
                        return Err("Invalid password hash format".into());
                    }
                }
            }
        }

        // 2. If auth mode is mixed or pam, try PAM authentication for system users
        if self.auth_mode == "pam" || self.auth_mode == "mixed" {
            #[cfg(unix)]
            let is_system_user = unsafe {
                if let Ok(c_user) = std::ffi::CString::new(username) {
                    !libc::getpwnam(c_user.as_ptr()).is_null()
                } else {
                    false
                }
            };
            #[cfg(not(unix))]
            let is_system_user = false;

            if is_system_user {
                let mut services: Vec<String> = Vec::new();

                // 1. If we have a verified working PAM service cached in memory, prioritize it
                if let Ok(guard) = self.cached_pam_service.read() {
                    if let Some(ref cached) = *guard {
                        services.push(cached.clone());
                    }
                }

                // 2. Add configured pam_service if not already tried and if exists
                if !services.contains(&self.pam_service) {
                    let path = format!("/etc/pam.d/{}", self.pam_service);
                    if std::path::Path::new(&path).exists() || services.is_empty() {
                        services.push(self.pam_service.clone());
                    }
                }

                // 3. Fallback candidates prioritizing standalone services present in /etc/pam.d/
                for candidate in &["login", "passwd", "common-auth", "sudo", "other"] {
                    let cand_str = candidate.to_string();
                    if !services.contains(&cand_str) {
                        let path = format!("/etc/pam.d/{}", candidate);
                        if std::path::Path::new(&path).exists() {
                            services.push(cand_str);
                        }
                    }
                }

                for svc in services {
                    if pam::authenticate(&svc, username, password).is_ok() {
                        // Cache the verified working PAM service for instantaneous future unlocks
                        if let Ok(mut guard) = self.cached_pam_service.write() {
                            *guard = Some(svc.clone());
                        }

                        let (home_dir, def_role) = get_linux_user_info(username);
                        
                        // Check if this PAM user already has a linked DB record
                        if let Ok(Some(mut existing)) = self.get_user_by_username(username) {
                            if existing.is_disabled {
                                return Err("Account is disabled. Please contact an administrator.".into());
                            }
                            existing.is_pam = true;
                            existing.resolve_avatar();
                            return Ok(existing);
                        }

                        // Auto-link new PAM user to DB profile
                        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
                        let now = Utc::now().to_rfc3339();
                        let _ = conn.execute(
                            "INSERT OR IGNORE INTO users (username, password_hash, role, home_dir, allowed_services, allowed_roots, can_install_plugins, allowed_plugins, blocked_plugins, is_pam, is_disabled, created_at) VALUES (?1, 'PAM_MANAGED', ?2, ?3, '[\"*\"]', '[\"*\"]', 0, '[\"*\"]', '[]', 1, 0, ?4)",
                            params![username, def_role, home_dir, now],
                        );
                        let id = conn.last_insert_rowid();

                        let mut u = User {
                            id,
                            username: username.to_string(),
                            nickname: Some(username.to_string()),
                            email: None,
                            avatar_url: None,
                            role: def_role,
                            home_dir,
                            is_pam: true,
                            is_disabled: false,
                            allowed_services: "[\"*\"]".to_string(),
                            allowed_roots: "[\"*\"]".to_string(),
                            can_install_plugins: false,
                            allowed_plugins: "[\"*\"]".to_string(),
                            blocked_plugins: "[]".to_string(),
                        };
                        u.resolve_avatar();
                        return Ok(u);
                    }
                }
            }
        }

        Err("Invalid username or password".into())
    }

    pub fn generate_token(&self, user: &User) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let expiration = Utc::now()
            .checked_add_signed(Duration::hours(self.session_hours as i64))
            .expect("valid timestamp")
            .timestamp();

        let claims = Claims {
            sub: user.username.clone(),
            role: user.role.clone(),
            home_dir: user.home_dir.clone(),
            is_pam: user.is_pam,
            allowed_roots: Some(user.allowed_roots.clone()),
            token_id: None,
            exp: expiration,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        Ok(token)
    }

    pub fn create_api_token(
        &self,
        username: &str,
        name: &str,
        role: &str,
        allowed_roots: Option<String>,
        expires_in_days: Option<i64>,
    ) -> Result<GeneratedApiToken, Box<dyn std::error::Error + Send + Sync>> {
        let token_id = format!("cdtok_{}", uuid::Uuid::new_v4().simple());
        let now = Utc::now().timestamp();

        let (exp_ts, exp_claim) = match expires_in_days {
            Some(days) if days > 0 => {
                let exp = Utc::now()
                    .checked_add_signed(Duration::days(days))
                    .expect("valid timestamp")
                    .timestamp();
                (Some(exp), exp)
            }
            _ => (None, 4_102_444_800), // Year 2099 far-future timestamp for non-expiring tokens
        };

        let roots = allowed_roots.unwrap_or_else(|| "[\"*\"]".to_string());

        let claims = Claims {
            sub: username.to_string(),
            role: role.to_string(),
            home_dir: "/".to_string(),
            is_pam: false,
            allowed_roots: Some(roots.clone()),
            token_id: Some(token_id.clone()),
            exp: exp_claim,
        };

        let raw_token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        let prefix = if raw_token.len() > 16 {
            format!("{}...{}", &raw_token[..8], &raw_token[raw_token.len() - 6..])
        } else {
            "cdtok_***".to_string()
        };

        {
            let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
            conn.execute(
                "INSERT INTO api_tokens (id, name, username, token_prefix, role, allowed_roots, expires_at, created_at, last_used_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
                params![
                    token_id,
                    name,
                    username,
                    prefix,
                    role,
                    roots,
                    exp_ts,
                    now
                ],
            )?;
        }

        let info = ApiTokenInfo {
            id: token_id,
            name: name.to_string(),
            username: username.to_string(),
            token_prefix: prefix,
            role: role.to_string(),
            allowed_roots: roots,
            expires_at: exp_ts,
            created_at: now,
            last_used_at: None,
        };

        Ok(GeneratedApiToken {
            token: raw_token,
            info,
        })
    }

    pub fn list_api_tokens(&self, username: Option<&str>) -> Result<Vec<ApiTokenInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut tokens = Vec::new();

        if let Some(user) = username {
            let mut stmt = conn.prepare(
                "SELECT id, name, username, token_prefix, role, allowed_roots, expires_at, created_at, last_used_at
                 FROM api_tokens WHERE username = ?1 ORDER BY created_at DESC"
            )?;
            let rows = stmt.query_map(params![user], |row| {
                Ok(ApiTokenInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    username: row.get(2)?,
                    token_prefix: row.get(3)?,
                    role: row.get(4)?,
                    allowed_roots: row.get(5)?,
                    expires_at: row.get(6)?,
                    created_at: row.get(7)?,
                    last_used_at: row.get(8)?,
                })
            })?;
            for r in rows {
                tokens.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, name, username, token_prefix, role, allowed_roots, expires_at, created_at, last_used_at
                 FROM api_tokens ORDER BY created_at DESC"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ApiTokenInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    username: row.get(2)?,
                    token_prefix: row.get(3)?,
                    role: row.get(4)?,
                    allowed_roots: row.get(5)?,
                    expires_at: row.get(6)?,
                    created_at: row.get(7)?,
                    last_used_at: row.get(8)?,
                })
            })?;
            for r in rows {
                tokens.push(r?);
            }
        }

        Ok(tokens)
    }

    pub fn revoke_api_token(&self, token_id: &str, username: Option<&str>) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let rows_affected = if let Some(user) = username {
            conn.execute(
                "DELETE FROM api_tokens WHERE id = ?1 AND username = ?2",
                params![token_id, user],
            )?
        } else {
            conn.execute(
                "DELETE FROM api_tokens WHERE id = ?1",
                params![token_id],
            )?
        };

        Ok(rows_affected > 0)
    }

    pub fn verify_token(&self, token: &str) -> Result<Claims, Box<dyn std::error::Error + Send + Sync>> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )?;

        // If it's a persistent API token with a token_id, check DB to ensure it's not revoked / expired
        if let Some(ref tid) = token_data.claims.token_id {
            let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
            let token_row: Option<(Option<i64>, String)> = conn.query_row(
                "SELECT expires_at, role FROM api_tokens WHERE id = ?1",
                params![tid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?;

            match token_row {
                Some((exp_opt, _)) => {
                    if let Some(exp) = exp_opt {
                        if exp > 0 && Utc::now().timestamp() > exp {
                            return Err("API token expired".into());
                        }
                    }
                    // Update last_used_at timestamp
                    let _ = conn.execute(
                        "UPDATE api_tokens SET last_used_at = ?1 WHERE id = ?2",
                        params![Utc::now().timestamp(), tid],
                    );
                }
                None => {
                    return Err("API token has been revoked".into());
                }
            }
        }

        Ok(token_data.claims)
    }

    pub fn verify_token_allow_expired(&self, token: &str) -> Result<Claims, Box<dyn std::error::Error + Send + Sync>> {
        let mut validation = Validation::default();
        validation.validate_exp = false;
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &validation,
        )?;

        // If it's a persistent API token with a token_id, check DB to ensure it's not revoked
        if let Some(ref tid) = token_data.claims.token_id {
            let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
            let token_row: Option<(Option<i64>, String)> = conn.query_row(
                "SELECT expires_at, role FROM api_tokens WHERE id = ?1",
                params![tid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?;

            if token_row.is_none() {
                return Err("API token has been revoked".into());
            }
        }

        Ok(token_data.claims)
    }

    pub fn list_accessible_mounts(
        &self,
        username: &str,
        is_admin: bool,
    ) -> Result<Vec<GlobalMount>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare("SELECT id, name, protocol, target_uri, options_json, allowed_users, created_at FROM global_mounts ORDER BY name ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(GlobalMount {
                id: row.get(0)?,
                name: row.get(1)?,
                protocol: row.get(2)?,
                target_uri: row.get(3)?,
                options_json: row.get(4)?,
                allowed_users: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;

        let mut accessible = Vec::new();
        for r in rows {
            let m = r?;
            if is_admin {
                accessible.push(m);
            } else {
                let allowed: Vec<String> = serde_json::from_str(&m.allowed_users).unwrap_or_else(|_| vec!["*".to_string()]);
                if allowed.contains(&"*".to_string()) || allowed.iter().any(|u| u.eq_ignore_ascii_case(username)) {
                    accessible.push(m);
                }
            }
        }

        Ok(accessible)
    }

    pub fn list_all_mounts(&self) -> Result<Vec<GlobalMount>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare("SELECT id, name, protocol, target_uri, options_json, allowed_users, created_at FROM global_mounts ORDER BY name ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(GlobalMount {
                id: row.get(0)?,
                name: row.get(1)?,
                protocol: row.get(2)?,
                target_uri: row.get(3)?,
                options_json: row.get(4)?,
                allowed_users: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;

        let mut all = Vec::new();
        for r in rows {
            all.push(r?);
        }
        Ok(all)
    }

    pub fn create_or_update_mount(
        &self,
        name: &str,
        protocol: &str,
        target_uri: &str,
        options_json: &str,
        allowed_users: &str,
    ) -> Result<GlobalMount, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO global_mounts (name, protocol, target_uri, options_json, allowed_users, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![name, protocol, target_uri, options_json, allowed_users, now],
        )?;

        let id = conn.last_insert_rowid();

        Ok(GlobalMount {
            id,
            name: name.to_string(),
            protocol: protocol.to_string(),
            target_uri: target_uri.to_string(),
            options_json: options_json.to_string(),
            allowed_users: allowed_users.to_string(),
            created_at: now,
        })
    }

    pub fn delete_mount(&self, id: i64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        conn.execute("DELETE FROM global_mounts WHERE id = ?1", params![id])?;
        Ok(())
    }

    // Bookmarks Management
    pub fn list_bookmarks(&self, username: &str) -> Result<Vec<UserBookmark>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare(
            "SELECT id, username, name, protocol, path, password, created_at
             FROM bookmarks WHERE username = ?1 OR username = '*' ORDER BY id ASC"
        )?;

        let rows = stmt.query_map(params![username], |row| {
            let pass: Option<String> = row.get(5)?;
            Ok(UserBookmark {
                id: row.get(0)?,
                username: row.get(1)?,
                name: row.get(2)?,
                protocol: row.get(3)?,
                path: row.get(4)?,
                has_password: pass.is_some() && !pass.as_ref().unwrap().is_empty(),
                password: pass,
                created_at: row.get(6)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn create_bookmark(
        &self,
        username: &str,
        name: &str,
        protocol: &str,
        path: &str,
        password: Option<&str>,
    ) -> Result<UserBookmark, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO bookmarks (username, name, protocol, path, password, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![username, name, protocol, path, password, now],
        )?;

        let id = conn.last_insert_rowid();

        Ok(UserBookmark {
            id,
            username: username.to_string(),
            name: name.to_string(),
            protocol: protocol.to_string(),
            path: path.to_string(),
            has_password: password.is_some() && !password.unwrap().is_empty(),
            password: password.map(|s| s.to_string()),
            created_at: now,
        })
    }

    pub fn delete_bookmark(&self, id: i64, username: &str, is_admin: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        if is_admin {
            conn.execute("DELETE FROM bookmarks WHERE id = ?1", params![id])?;
        } else {
            conn.execute("DELETE FROM bookmarks WHERE id = ?1 AND username = ?2", params![id, username])?;
        }
        Ok(())
    }

    // User Preferences Management (Cross-Device Sync)
    pub fn get_user_preferences(&self, username: &str) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare("SELECT preferences_json FROM user_preferences WHERE username = ?1")?;
        let mut rows = stmt.query(params![username])?;
        if let Some(row) = rows.next()? {
            let json_str: String = row.get(0)?;
            Ok(Some(json_str))
        } else {
            Ok(None)
        }
    }

    pub fn save_user_preferences(&self, username: &str, preferences_json: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO user_preferences (username, preferences_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(username) DO UPDATE SET preferences_json = ?2, updated_at = ?3",
            params![username, preferences_json, now],
        )?;
        Ok(())
    }

    pub fn reset_user_preferences(&self, username: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        conn.execute(
            "DELETE FROM user_preferences WHERE username = ?1",
            params![username],
        )?;
        Ok(())
    }

    // Security Settings
    pub fn get_security_settings(&self) -> Result<SecuritySettings, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let auto_lock_enabled = conn.query_row("SELECT value FROM security_settings WHERE key = 'auto_lock_enabled'", [], |r| r.get::<_, String>(0)).unwrap_or_else(|_| "true".to_string()) == "true";
        let auto_lock_minutes = conn.query_row("SELECT value FROM security_settings WHERE key = 'auto_lock_minutes'", [], |r| r.get::<_, String>(0)).unwrap_or_else(|_| "15".to_string()).parse::<u32>().unwrap_or(15);
        let session_timeout_hours = conn.query_row("SELECT value FROM security_settings WHERE key = 'session_timeout_hours'", [], |r| r.get::<_, String>(0)).unwrap_or_else(|_| "8".to_string()).parse::<u32>().unwrap_or(8);
        let lock_prevented_by_tasks = conn.query_row("SELECT value FROM security_settings WHERE key = 'lock_prevented_by_tasks'", [], |r| r.get::<_, String>(0)).unwrap_or_else(|_| "true".to_string()) == "true";

        Ok(SecuritySettings {
            auto_lock_enabled,
            auto_lock_minutes,
            session_timeout_hours,
            lock_prevented_by_tasks,
        })
    }

    pub fn update_security_settings(&self, settings: &SecuritySettings) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        conn.execute("INSERT OR REPLACE INTO security_settings (key, value) VALUES ('auto_lock_enabled', ?1)", params![settings.auto_lock_enabled.to_string()])?;
        conn.execute("INSERT OR REPLACE INTO security_settings (key, value) VALUES ('auto_lock_minutes', ?1)", params![settings.auto_lock_minutes.to_string()])?;
        conn.execute("INSERT OR REPLACE INTO security_settings (key, value) VALUES ('session_timeout_hours', ?1)", params![settings.session_timeout_hours.to_string()])?;
        conn.execute("INSERT OR REPLACE INTO security_settings (key, value) VALUES ('lock_prevented_by_tasks', ?1)", params![settings.lock_prevented_by_tasks.to_string()])?;
        Ok(())
    }

    // ---------------- LINK SHARING & ADVANCED SHARING CENTER ----------------
    pub fn create_share(
        &self,
        username: &str,
        path: &str,
        name: &str,
        is_dir: bool,
        allow_upload: bool,
        allow_view: bool,
        allow_download: bool,
        password: Option<&str>,
        expires_at: Option<&str>,
        max_downloads: u64,
        allowed_emails: Option<&str>,
        require_email: bool,
        watermark_enabled: bool,
        watermark_text: Option<&str>,
    ) -> Result<ShareItem, Box<dyn std::error::Error + Send + Sync>> {
        let token = uuid::Uuid::new_v4().to_string().replace('-', "")[..16].to_string();
        let now = Utc::now().to_rfc3339();

        let password_hash = if let Some(pass) = password {
            if !pass.trim().is_empty() {
                let salt = SaltString::generate(&mut OsRng);
                let argon2 = Argon2::default();
                Some(
                    argon2
                        .hash_password(pass.as_bytes(), &salt)
                        .map_err(|e| format!("Password hashing failed: {}", e))?
                        .to_string(),
                )
            } else {
                None
            }
        } else {
            None
        };

        let has_password = password_hash.is_some();
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;

        conn.execute(
            "INSERT INTO shares (token, path, name, is_dir, allow_upload, allow_view, allow_download, password_hash, expires_at, created_by, created_at, download_count, max_downloads, allowed_emails, require_email, watermark_enabled, watermark_text, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12, ?13, ?14, ?15, ?16, 'active')",
            params![
                token,
                path,
                name,
                if is_dir { 1 } else { 0 },
                if allow_upload { 1 } else { 0 },
                if allow_view { 1 } else { 0 },
                if allow_download { 1 } else { 0 },
                password_hash,
                expires_at,
                username,
                now,
                max_downloads as i64,
                allowed_emails,
                if require_email { 1 } else { 0 },
                if watermark_enabled { 1 } else { 0 },
                watermark_text,
            ],
        )?;

        let id = conn.last_insert_rowid();

        Ok(ShareItem {
            id,
            token,
            path: path.to_string(),
            name: name.to_string(),
            is_dir,
            allow_upload,
            allow_view,
            allow_download,
            has_password,
            password_hash: None,
            expires_at: expires_at.map(|s| s.to_string()),
            created_by: username.to_string(),
            created_at: now,
            download_count: 0,
            max_downloads,
            allowed_emails: allowed_emails.map(|s| s.to_string()),
            require_email,
            watermark_enabled,
            watermark_text: watermark_text.map(|s| s.to_string()),
            status: "active".to_string(),
            total_visits: 0,
            total_previews: 0,
        })
    }

    pub fn update_share(
        &self,
        id: i64,
        username: &str,
        is_admin: bool,
        name: &str,
        allow_upload: bool,
        allow_view: bool,
        allow_download: bool,
        new_password: Option<Option<&str>>,
        expires_at: Option<Option<&str>>,
        max_downloads: u64,
        allowed_emails: Option<&str>,
        require_email: bool,
        watermark_enabled: bool,
        watermark_text: Option<&str>,
        status: Option<&str>,
    ) -> Result<ShareItem, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;

        let existing = if is_admin {
            self.get_share_by_id_internal(&conn, id)?
        } else {
            let item = self.get_share_by_id_internal(&conn, id)?;
            if let Some(ref s) = item {
                if s.created_by != username {
                    return Err("Access denied".into());
                }
            }
            item
        }.ok_or("Share not found")?;

        let password_hash = match new_password {
            Some(Some(pass)) if !pass.trim().is_empty() => {
                let salt = SaltString::generate(&mut OsRng);
                let argon2 = Argon2::default();
                Some(Some(
                    argon2
                        .hash_password(pass.as_bytes(), &salt)
                        .map_err(|e| format!("Password hashing failed: {}", e))?
                        .to_string(),
                ))
            }
            Some(None) => Some(None),
            Some(Some(_)) => Some(None),
            None => None,
        };

        let resolved_exp = match expires_at {
            Some(opt) => opt.map(|s| s.to_string()),
            None => existing.expires_at.clone(),
        };

        let resolved_status = status.unwrap_or(&existing.status);

        if let Some(pass_opt) = password_hash {
            conn.execute(
                "UPDATE shares SET name = ?1, allow_upload = ?2, allow_view = ?3, allow_download = ?4, password_hash = ?5, expires_at = ?6, max_downloads = ?7, allowed_emails = ?8, require_email = ?9, watermark_enabled = ?10, watermark_text = ?11, status = ?12 WHERE id = ?13",
                params![
                    name,
                    if allow_upload { 1 } else { 0 },
                    if allow_view { 1 } else { 0 },
                    if allow_download { 1 } else { 0 },
                    pass_opt,
                    resolved_exp,
                    max_downloads as i64,
                    allowed_emails,
                    if require_email { 1 } else { 0 },
                    if watermark_enabled { 1 } else { 0 },
                    watermark_text,
                    resolved_status,
                    id,
                ],
            )?;
        } else {
            conn.execute(
                "UPDATE shares SET name = ?1, allow_upload = ?2, allow_view = ?3, allow_download = ?4, expires_at = ?5, max_downloads = ?6, allowed_emails = ?7, require_email = ?8, watermark_enabled = ?9, watermark_text = ?10, status = ?11 WHERE id = ?12",
                params![
                    name,
                    if allow_upload { 1 } else { 0 },
                    if allow_view { 1 } else { 0 },
                    if allow_download { 1 } else { 0 },
                    resolved_exp,
                    max_downloads as i64,
                    allowed_emails,
                    if require_email { 1 } else { 0 },
                    if watermark_enabled { 1 } else { 0 },
                    watermark_text,
                    resolved_status,
                    id,
                ],
            )?;
        }

        self.get_share_by_id(id)?.ok_or("Failed to fetch updated share".into())
    }

    fn get_share_by_id_internal(&self, conn: &Connection, id: i64) -> Result<Option<ShareItem>, Box<dyn std::error::Error + Send + Sync>> {
        let mut stmt = conn.prepare("SELECT id, token, path, name, is_dir, allow_upload, allow_view, allow_download, password_hash, expires_at, created_by, created_at, download_count, max_downloads, allowed_emails, require_email, watermark_enabled, watermark_text, status FROM shares WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;

        if let Some(r) = rows.next()? {
            let pass_hash: Option<String> = r.get(8)?;
            Ok(Some(ShareItem {
                id: r.get(0)?,
                token: r.get(1)?,
                path: r.get(2)?,
                name: r.get(3)?,
                is_dir: r.get::<_, i64>(4)? == 1,
                allow_upload: r.get::<_, i64>(5)? == 1,
                allow_view: r.get::<_, Option<i64>>(6)?.unwrap_or(1) == 1,
                allow_download: r.get::<_, Option<i64>>(7)?.unwrap_or(1) == 1,
                has_password: pass_hash.is_some(),
                password_hash: pass_hash,
                expires_at: r.get(9)?,
                created_by: r.get(10)?,
                created_at: r.get(11)?,
                download_count: r.get::<_, i64>(12)? as u64,
                max_downloads: r.get::<_, i64>(13)? as u64,
                allowed_emails: r.get(14)?,
                require_email: r.get::<_, Option<i64>>(15)?.unwrap_or(0) == 1,
                watermark_enabled: r.get::<_, Option<i64>>(16)?.unwrap_or(0) == 1,
                watermark_text: r.get(17)?,
                status: r.get::<_, Option<String>>(18)?.unwrap_or_else(|| "active".to_string()),
                total_visits: 0,
                total_previews: 0,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_share_by_id(&self, id: i64) -> Result<Option<ShareItem>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut item = self.get_share_by_id_internal(&conn, id)?;
        if let Some(ref mut s) = item {
            let mut v_stmt = conn.prepare("SELECT COUNT(1) FROM share_access_logs WHERE share_id = ?1 AND action = 'visit'")?;
            let visits: i64 = v_stmt.query_row(params![id], |r| r.get(0)).unwrap_or(0);
            let mut p_stmt = conn.prepare("SELECT COUNT(1) FROM share_access_logs WHERE share_id = ?1 AND action = 'preview'")?;
            let previews: i64 = p_stmt.query_row(params![id], |r| r.get(0)).unwrap_or(0);
            s.total_visits = visits as u64;
            s.total_previews = previews as u64;
        }
        Ok(item)
    }

    pub fn list_shares(&self, username: &str, is_admin: bool) -> Result<Vec<ShareItem>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut list = Vec::new();

        let query = if is_admin {
            "SELECT s.id, s.token, s.path, s.name, s.is_dir, s.allow_upload, s.allow_view, s.allow_download,
                    s.password_hash, s.expires_at, s.created_by, s.created_at, s.download_count, s.max_downloads,
                    s.allowed_emails, s.require_email, s.watermark_enabled, s.watermark_text, s.status,
                    (SELECT COUNT(1) FROM share_access_logs l WHERE l.share_id = s.id AND l.action = 'visit') as visits,
                    (SELECT COUNT(1) FROM share_access_logs l WHERE l.share_id = s.id AND l.action = 'preview') as previews
             FROM shares s ORDER BY s.id DESC"
        } else {
            "SELECT s.id, s.token, s.path, s.name, s.is_dir, s.allow_upload, s.allow_view, s.allow_download,
                    s.password_hash, s.expires_at, s.created_by, s.created_at, s.download_count, s.max_downloads,
                    s.allowed_emails, s.require_email, s.watermark_enabled, s.watermark_text, s.status,
                    (SELECT COUNT(1) FROM share_access_logs l WHERE l.share_id = s.id AND l.action = 'visit') as visits,
                    (SELECT COUNT(1) FROM share_access_logs l WHERE l.share_id = s.id AND l.action = 'preview') as previews
             FROM shares s WHERE s.created_by = ?1 ORDER BY s.id DESC"
        };

        if is_admin {
            let mut stmt = conn.prepare(query)?;
            let rows = stmt.query_map([], |r| {
                let pass_hash: Option<String> = r.get(8)?;
                Ok(ShareItem {
                    id: r.get(0)?,
                    token: r.get(1)?,
                    path: r.get(2)?,
                    name: r.get(3)?,
                    is_dir: r.get::<_, i64>(4)? == 1,
                    allow_upload: r.get::<_, i64>(5)? == 1,
                    allow_view: r.get::<_, Option<i64>>(6)?.unwrap_or(1) == 1,
                    allow_download: r.get::<_, Option<i64>>(7)?.unwrap_or(1) == 1,
                    has_password: pass_hash.is_some(),
                    password_hash: None,
                    expires_at: r.get(9)?,
                    created_by: r.get(10)?,
                    created_at: r.get(11)?,
                    download_count: r.get::<_, i64>(12)? as u64,
                    max_downloads: r.get::<_, i64>(13)? as u64,
                    allowed_emails: r.get(14)?,
                    require_email: r.get::<_, Option<i64>>(15)?.unwrap_or(0) == 1,
                    watermark_enabled: r.get::<_, Option<i64>>(16)?.unwrap_or(0) == 1,
                    watermark_text: r.get(17)?,
                    status: r.get::<_, Option<String>>(18)?.unwrap_or_else(|| "active".to_string()),
                    total_visits: r.get::<_, Option<i64>>(19)?.unwrap_or(0) as u64,
                    total_previews: r.get::<_, Option<i64>>(20)?.unwrap_or(0) as u64,
                })
            })?;
            for r in rows {
                list.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(query)?;
            let rows = stmt.query_map(params![username], |r| {
                let pass_hash: Option<String> = r.get(8)?;
                Ok(ShareItem {
                    id: r.get(0)?,
                    token: r.get(1)?,
                    path: r.get(2)?,
                    name: r.get(3)?,
                    is_dir: r.get::<_, i64>(4)? == 1,
                    allow_upload: r.get::<_, i64>(5)? == 1,
                    allow_view: r.get::<_, Option<i64>>(6)?.unwrap_or(1) == 1,
                    allow_download: r.get::<_, Option<i64>>(7)?.unwrap_or(1) == 1,
                    has_password: pass_hash.is_some(),
                    password_hash: None,
                    expires_at: r.get(9)?,
                    created_by: r.get(10)?,
                    created_at: r.get(11)?,
                    download_count: r.get::<_, i64>(12)? as u64,
                    max_downloads: r.get::<_, i64>(13)? as u64,
                    allowed_emails: r.get(14)?,
                    require_email: r.get::<_, Option<i64>>(15)?.unwrap_or(0) == 1,
                    watermark_enabled: r.get::<_, Option<i64>>(16)?.unwrap_or(0) == 1,
                    watermark_text: r.get(17)?,
                    status: r.get::<_, Option<String>>(18)?.unwrap_or_else(|| "active".to_string()),
                    total_visits: r.get::<_, Option<i64>>(19)?.unwrap_or(0) as u64,
                    total_previews: r.get::<_, Option<i64>>(20)?.unwrap_or(0) as u64,
                })
            })?;
            for r in rows {
                list.push(r?);
            }
        }

        Ok(list)
    }

    pub fn get_share_by_token(&self, token: &str) -> Result<Option<ShareItem>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare("SELECT id, token, path, name, is_dir, allow_upload, allow_view, allow_download, password_hash, expires_at, created_by, created_at, download_count, max_downloads, allowed_emails, require_email, watermark_enabled, watermark_text, status FROM shares WHERE token = ?1")?;
        let mut rows = stmt.query(params![token])?;

        if let Some(r) = rows.next()? {
            let pass_hash: Option<String> = r.get(8)?;
            Ok(Some(ShareItem {
                id: r.get(0)?,
                token: r.get(1)?,
                path: r.get(2)?,
                name: r.get(3)?,
                is_dir: r.get::<_, i64>(4)? == 1,
                allow_upload: r.get::<_, i64>(5)? == 1,
                allow_view: r.get::<_, Option<i64>>(6)?.unwrap_or(1) == 1,
                allow_download: r.get::<_, Option<i64>>(7)?.unwrap_or(1) == 1,
                has_password: pass_hash.is_some(),
                password_hash: pass_hash,
                expires_at: r.get(9)?,
                created_by: r.get(10)?,
                created_at: r.get(11)?,
                download_count: r.get::<_, i64>(12)? as u64,
                max_downloads: r.get::<_, i64>(13)? as u64,
                allowed_emails: r.get(14)?,
                require_email: r.get::<_, Option<i64>>(15)?.unwrap_or(0) == 1,
                watermark_enabled: r.get::<_, Option<i64>>(16)?.unwrap_or(0) == 1,
                watermark_text: r.get(17)?,
                status: r.get::<_, Option<String>>(18)?.unwrap_or_else(|| "active".to_string()),
                total_visits: 0,
                total_previews: 0,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_share(&self, id: i64, username: &str, is_admin: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        if is_admin {
            conn.execute("DELETE FROM shares WHERE id = ?1", params![id])?;
        } else {
            conn.execute("DELETE FROM shares WHERE id = ?1 AND created_by = ?2", params![id, username])?;
        }
        Ok(())
    }

    pub fn revoke_share(&self, id: i64, username: &str, is_admin: bool, revoke: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let status = if revoke { "revoked" } else { "active" };
        if is_admin {
            conn.execute("UPDATE shares SET status = ?1 WHERE id = ?2", params![status, id])?;
        } else {
            conn.execute("UPDATE shares SET status = ?1 WHERE id = ?2 AND created_by = ?3", params![status, id, username])?;
        }
        Ok(())
    }

    pub fn increment_share_downloads(&self, token: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        conn.execute("UPDATE shares SET download_count = download_count + 1 WHERE token = ?1", params![token])?;
        Ok(())
    }

    pub fn log_share_access(
        &self,
        share_id: i64,
        token: &str,
        visitor_email: Option<&str>,
        ip_address: &str,
        user_agent: Option<&str>,
        action: &str,
        target_file: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO share_access_logs (share_id, token, visitor_email, ip_address, user_agent, action, target_file, accessed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![share_id, token, visitor_email, ip_address, user_agent, action, target_file, now],
        )?;
        Ok(())
    }

    pub fn get_share_logs(
        &self,
        share_id: i64,
        username: &str,
        is_admin: bool,
    ) -> Result<Vec<ShareAccessLog>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut owner_check = if is_admin {
            conn.prepare("SELECT id FROM shares WHERE id = ?1")?
        } else {
            conn.prepare("SELECT id FROM shares WHERE id = ?1 AND created_by = ?2")?
        };
        let mut check_rows = if is_admin {
            owner_check.query(params![share_id])?
        } else {
            owner_check.query(params![share_id, username])?
        };
        if check_rows.next()?.is_none() {
            return Err("Share not found or access denied".into());
        }

        let mut stmt = conn.prepare(
            "SELECT id, share_id, token, visitor_email, ip_address, user_agent, action, target_file, accessed_at
             FROM share_access_logs WHERE share_id = ?1 ORDER BY id DESC LIMIT 500"
        )?;
        let rows = stmt.query_map(params![share_id], |r| {
            Ok(ShareAccessLog {
                id: r.get(0)?,
                share_id: r.get(1)?,
                token: r.get(2)?,
                visitor_email: r.get(3)?,
                ip_address: r.get(4)?,
                user_agent: r.get(5)?,
                action: r.get(6)?,
                target_file: r.get(7)?,
                accessed_at: r.get(8)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn verify_share_email(&self, token: &str, email: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(share) = self.get_share_by_token(token)? {
            if !share.require_email {
                return Ok(true);
            }
            if let Some(ref allowed) = share.allowed_emails {
                let email_clean = email.trim().to_lowercase();
                if email_clean.is_empty() {
                    return Ok(false);
                }
                if let Ok(emails_vec) = serde_json::from_str::<Vec<String>>(allowed) {
                    return Ok(emails_vec.iter().any(|e| {
                        let t = e.trim().to_lowercase();
                        t == email_clean || t == "*"
                    }));
                }
                let split_matches = allowed.split(|c| c == ',' || c == ';' || c == '\n' || c == ' ')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .any(|e| e == email_clean || e == "*");
                return Ok(split_matches);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn verify_share_password(&self, token: &str, password: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(share) = self.get_share_by_token(token)? {
            if let Some(hash_str) = share.password_hash {
                let parsed_hash = PasswordHash::new(&hash_str).map_err(|e| format!("Invalid hash format: {}", e))?;
                Ok(Argon2::default().verify_password(password.as_bytes(), &parsed_hash).is_ok())
            } else {
                Ok(true) // no password required
            }
        } else {
            Ok(false)
        }
    }

    // ---------------- PERSISTENT DATABASE NOTES ENGINE ----------------

    pub fn list_db_notes(
        &self,
        username: &str,
        is_admin: bool,
        search: Option<&str>,
        tag: Option<&str>,
        category: Option<&str>,
        section: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<DbNote>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|e| format!("DB lock error: {}", e))?;

        let mut query = String::from(
            "SELECT id, username, title, content, category, section, tags, is_pinned, is_archived, is_encrypted, color, created_at, updated_at
             FROM db_notes WHERE "
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if is_admin {
            query.push_str("(username = ?1 OR ?1 = 'admin')");
            params_vec.push(Box::new(username.to_string()));
        } else {
            query.push_str("username = ?1");
            params_vec.push(Box::new(username.to_string()));
        }

        if !include_archived {
            query.push_str(" AND is_archived = 0");
        }

        if let Some(cat) = category {
            let cat_clean = cat.trim();
            if !cat_clean.is_empty() && cat_clean != "All" {
                let idx = params_vec.len() + 1;
                query.push_str(&format!(" AND category = ?{}", idx));
                params_vec.push(Box::new(cat_clean.to_string()));
            }
        }

        if let Some(sec) = section {
            let sec_clean = sec.trim();
            if !sec_clean.is_empty() && sec_clean != "All" {
                let idx = params_vec.len() + 1;
                query.push_str(&format!(" AND section = ?{}", idx));
                params_vec.push(Box::new(sec_clean.to_string()));
            }
        }

        if let Some(t) = tag {
            let t_clean = t.trim();
            if !t_clean.is_empty() && t_clean != "All" {
                let idx = params_vec.len() + 1;
                query.push_str(&format!(" AND tags LIKE ?{}", idx));
                params_vec.push(Box::new(format!("%\"{}\"%", t_clean)));
            }
        }

        if let Some(s) = search {
            let clean = s.trim();
            if !clean.is_empty() {
                let idx = params_vec.len() + 1;
                query.push_str(&format!(" AND (title LIKE ?{0} OR content LIKE ?{0} OR tags LIKE ?{0})", idx));
                params_vec.push(Box::new(format!("%{}%", clean)));
            }
        }

        query.push_str(" ORDER BY is_pinned DESC, updated_at DESC");

        let mut stmt = conn.prepare(&query)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(DbNote {
                id: row.get(0)?,
                username: row.get(1)?,
                title: row.get(2)?,
                content: row.get(3)?,
                category: row.get(4)?,
                section: row.get(5)?,
                tags: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "[]".to_string()),
                is_pinned: row.get::<_, i32>(7)? != 0,
                is_archived: row.get::<_, i32>(8)? != 0,
                is_encrypted: row.get::<_, i32>(9)? != 0,
                color: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                attachments: Vec::new(),
            })
        })?;

        let mut notes = Vec::new();
        for r in rows {
            notes.push(r?);
        }

        // Attach attachments list for each note
        for note in &mut notes {
            if let Ok(mut att_stmt) = conn.prepare(
                "SELECT id, note_id, username, filename, mime_type, size_bytes, storage_path, sha256, created_at
                 FROM db_note_attachments WHERE note_id = ?1 ORDER BY created_at ASC"
            ) {
                if let Ok(att_rows) = att_stmt.query_map(params![note.id], |row| {
                    Ok(DbNoteAttachment {
                        id: row.get(0)?,
                        note_id: row.get(1)?,
                        username: row.get(2)?,
                        filename: row.get(3)?,
                        mime_type: row.get(4)?,
                        size_bytes: row.get(5)?,
                        storage_path: row.get(6)?,
                        sha256: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                }) {
                    for ar in att_rows.flatten() {
                        note.attachments.push(ar);
                    }
                }
            }
        }

        Ok(notes)
    }

    pub fn get_db_note(
        &self,
        id: i64,
        username: &str,
        is_admin: bool,
    ) -> Result<Option<DbNote>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, username, title, content, category, section, tags, is_pinned, is_archived, is_encrypted, color, created_at, updated_at
             FROM db_notes WHERE id = ?1"
        )?;

        let mut note = stmt.query_row(params![id], |row| {
            Ok(DbNote {
                id: row.get(0)?,
                username: row.get(1)?,
                title: row.get(2)?,
                content: row.get(3)?,
                category: row.get(4)?,
                section: row.get(5)?,
                tags: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "[]".to_string()),
                is_pinned: row.get::<_, i32>(7)? != 0,
                is_archived: row.get::<_, i32>(8)? != 0,
                is_encrypted: row.get::<_, i32>(9)? != 0,
                color: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                attachments: Vec::new(),
            })
        }).optional()?;

        if let Some(ref mut n) = note {
            if !is_admin && n.username != username && username != "admin" {
                return Ok(None);
            }
            let mut att_stmt = conn.prepare(
                "SELECT id, note_id, username, filename, mime_type, size_bytes, storage_path, sha256, created_at
                 FROM db_note_attachments WHERE note_id = ?1 ORDER BY created_at ASC"
            )?;
            let att_rows = att_stmt.query_map(params![id], |row| {
                Ok(DbNoteAttachment {
                    id: row.get(0)?,
                    note_id: row.get(1)?,
                    username: row.get(2)?,
                    filename: row.get(3)?,
                    mime_type: row.get(4)?,
                    size_bytes: row.get(5)?,
                    storage_path: row.get(6)?,
                    sha256: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })?;
            for ar in att_rows.flatten() {
                n.attachments.push(ar);
            }
        }

        Ok(note)
    }

    pub fn create_db_note(
        &self,
        username: &str,
        title: &str,
        content: &str,
        category: &str,
        section: &str,
        tags: &str,
        is_pinned: bool,
        is_encrypted: bool,
        color: Option<&str>,
    ) -> Result<DbNote, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let cat = if category.trim().is_empty() { "General" } else { category.trim() };
        let sec = if section.trim().is_empty() { "Default" } else { section.trim() };
        let t = if tags.trim().is_empty() { "[]" } else { tags.trim() };

        conn.execute(
            "INSERT INTO db_notes (username, title, content, category, section, tags, is_pinned, is_archived, is_encrypted, color, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10, ?10)",
            params![
                username,
                title,
                content,
                cat,
                sec,
                t,
                if is_pinned { 1 } else { 0 },
                if is_encrypted { 1 } else { 0 },
                color,
                now
            ],
        )?;

        let id = conn.last_insert_rowid();
        Ok(DbNote {
            id,
            username: username.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            category: cat.to_string(),
            section: sec.to_string(),
            tags: t.to_string(),
            is_pinned,
            is_archived: false,
            is_encrypted,
            color: color.map(|c| c.to_string()),
            created_at: now.clone(),
            updated_at: now,
            attachments: Vec::new(),
        })
    }

    pub fn update_db_note(
        &self,
        id: i64,
        username: &str,
        is_admin: bool,
        title: Option<&str>,
        content: Option<&str>,
        category: Option<&str>,
        section: Option<&str>,
        tags: Option<&str>,
        is_pinned: Option<bool>,
        is_archived: Option<bool>,
        color: Option<Option<&str>>,
    ) -> Result<DbNote, Box<dyn std::error::Error + Send + Sync>> {
        let existing = self.get_db_note(id, username, is_admin)?
            .ok_or_else(|| "Note not found or access denied".to_string())?;

        let now = chrono::Utc::now().to_rfc3339();
        let new_title = title.unwrap_or(&existing.title);
        let new_content = content.unwrap_or(&existing.content);
        let new_category = category.unwrap_or(&existing.category);
        let new_section = section.unwrap_or(&existing.section);
        let new_tags = tags.unwrap_or(&existing.tags);
        let new_pinned = is_pinned.unwrap_or(existing.is_pinned);
        let new_archived = is_archived.unwrap_or(existing.is_archived);
        let new_color = match color {
            Some(opt) => opt.map(|c| c.to_string()),
            None => existing.color.clone(),
        };

        let conn = self.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        conn.execute(
            "UPDATE db_notes SET title = ?1, content = ?2, category = ?3, section = ?4, tags = ?5,
                    is_pinned = ?6, is_archived = ?7, color = ?8, updated_at = ?9
             WHERE id = ?10",
            params![
                new_title,
                new_content,
                new_category,
                new_section,
                new_tags,
                if new_pinned { 1 } else { 0 },
                if new_archived { 1 } else { 0 },
                new_color,
                now,
                id
            ],
        )?;

        drop(conn);
        self.get_db_note(id, username, is_admin)?
            .ok_or_else(|| "Failed to reload updated note".to_string().into())
    }

    pub fn delete_db_note(
        &self,
        id: i64,
        username: &str,
        is_admin: bool,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let note = self.get_db_note(id, username, is_admin)?
            .ok_or_else(|| "Note not found or access denied".to_string())?;

        let attachments_to_delete: Vec<String> = note.attachments.into_iter().map(|a| a.storage_path).collect();

        let conn = self.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        conn.execute("DELETE FROM db_note_attachments WHERE note_id = ?1", params![id])?;
        conn.execute("DELETE FROM db_notes WHERE id = ?1", params![id])?;

        Ok(attachments_to_delete)
    }

    pub fn create_db_note_attachment(
        &self,
        note_id: Option<i64>,
        username: &str,
        filename: &str,
        mime_type: &str,
        size_bytes: i64,
        storage_path: &str,
        sha256: Option<&str>,
    ) -> Result<DbNoteAttachment, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO db_note_attachments (note_id, username, filename, mime_type, size_bytes, storage_path, sha256, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                note_id,
                username,
                filename,
                mime_type,
                size_bytes,
                storage_path,
                sha256,
                now
            ],
        )?;

        let id = conn.last_insert_rowid();

        if let Some(nid) = note_id {
            let _ = conn.execute("UPDATE db_notes SET updated_at = ?1 WHERE id = ?2", params![now, nid]);
        }

        Ok(DbNoteAttachment {
            id,
            note_id,
            username: username.to_string(),
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
            size_bytes,
            storage_path: storage_path.to_string(),
            sha256: sha256.map(|s| s.to_string()),
            created_at: now,
        })
    }

    pub fn get_db_note_attachment(
        &self,
        attachment_id: i64,
        username: &str,
        is_admin: bool,
    ) -> Result<Option<DbNoteAttachment>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, note_id, username, filename, mime_type, size_bytes, storage_path, sha256, created_at
             FROM db_note_attachments WHERE id = ?1"
        )?;

        let att = stmt.query_row(params![attachment_id], |row| {
            Ok(DbNoteAttachment {
                id: row.get(0)?,
                note_id: row.get(1)?,
                username: row.get(2)?,
                filename: row.get(3)?,
                mime_type: row.get(4)?,
                size_bytes: row.get(5)?,
                storage_path: row.get(6)?,
                sha256: row.get(7)?,
                created_at: row.get(8)?,
            })
        }).optional()?;

        if let Some(ref a) = att {
            if !is_admin && a.username != username && username != "admin" {
                return Ok(None);
            }
        }

        Ok(att)
    }

    pub fn delete_db_note_attachment(
        &self,
        attachment_id: i64,
        username: &str,
        is_admin: bool,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let att = self.get_db_note_attachment(attachment_id, username, is_admin)?;
        if let Some(a) = att {
            let conn = self.db.lock().map_err(|e| format!("DB lock error: {}", e))?;
            conn.execute("DELETE FROM db_note_attachments WHERE id = ?1", params![attachment_id])?;
            Ok(Some(a.storage_path))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbNote {
    pub id: i64,
    pub username: String,
    pub title: String,
    pub content: String,
    pub category: String,
    pub section: String,
    pub tags: String,
    pub is_pinned: bool,
    pub is_archived: bool,
    pub is_encrypted: bool,
    pub color: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub attachments: Vec<DbNoteAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbNoteAttachment {
    pub id: i64,
    pub note_id: Option<i64>,
    pub username: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub storage_path: String,
    pub sha256: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareItem {
    pub id: i64,
    pub token: String,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub allow_upload: bool,
    pub allow_view: bool,
    pub allow_download: bool,
    pub has_password: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<String>,
    pub expires_at: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub download_count: u64,
    pub max_downloads: u64,
    pub allowed_emails: Option<String>,
    pub require_email: bool,
    pub watermark_enabled: bool,
    pub watermark_text: Option<String>,
    pub status: String,
    #[serde(default)]
    pub total_visits: u64,
    #[serde(default)]
    pub total_previews: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareAccessLog {
    pub id: i64,
    pub share_id: i64,
    pub token: String,
    pub visitor_email: Option<String>,
    pub ip_address: String,
    pub user_agent: Option<String>,
    pub action: String,
    pub target_file: Option<String>,
    pub accessed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBookmark {
    pub id: i64,
    pub username: String,
    pub name: String,
    pub protocol: String,
    pub path: String,
    pub has_password: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySettings {
    pub auto_lock_enabled: bool,
    pub auto_lock_minutes: u32,
    pub session_timeout_hours: u32,
    pub lock_prevented_by_tasks: bool,
}

pub fn get_linux_user_info(username: &str) -> (String, String) {
    let mut home_dir = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("/home/{}", username));
    let mut is_admin = username == "root" || username.eq_ignore_ascii_case("administrator");

    if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 6 && parts[0] == username {
                home_dir = parts[5].to_string();
                break;
            }
        }
    }

    if let Ok(group) = std::fs::read_to_string("/etc/group") {
        for line in group.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 4 {
                let grp_name = parts[0];
                let members: Vec<&str> = parts[3].split(',').map(|s| s.trim()).collect();
                if (grp_name == "sudo" || grp_name == "wheel" || grp_name == "admin" || grp_name == "root")
                    && (members.contains(&username) || grp_name == username)
                {
                    is_admin = true;
                }
            }
        }
    }

    let role = if is_admin { "admin".to_string() } else { "user".to_string() };
    (home_dir, role)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_auth_sqlite_wal_pragmas() {
        let tmp = tempdir().unwrap();
        let db_file = tmp.path().join("test_auth.db");
        let auth = AuthManager::new(
            &db_file.to_string_lossy(),
            "secret-key-123456789012345678901234",
            24,
            "builtin",
            "login",
            "admin",
            "admin",
        ).unwrap();
        
        let db = auth.db();
        let conn = db.lock().unwrap();
        
        let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(journal_mode.to_uppercase(), "WAL");

        let busy_timeout: i64 = conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0)).unwrap();
        assert_eq!(busy_timeout, 5000);
    }

    #[test]
    fn test_user_preferences_persistence() {
        let tmp = tempdir().unwrap();
        let db_file = tmp.path().join("test_prefs.db");
        let auth = AuthManager::new(
            &db_file.to_string_lossy(),
            "secret-key-123456789012345678901234",
            24,
            "builtin",
            "login",
            "admin",
            "admin",
        ).unwrap();

        // 1. Initial should be None
        let initial = auth.get_user_preferences("admin").unwrap();
        assert!(initial.is_none());

        // 2. Save preferences JSON
        let prefs_json = r#"{"pane_names":["DOWNLOADS","SERVER",null,null],"pane_colors":{"0":"emerald","1":"sky"},"default_layout":"layout-dual-vertical"}"#;
        auth.save_user_preferences("admin", prefs_json).unwrap();

        // 3. Retrieve preferences
        let loaded = auth.get_user_preferences("admin").unwrap();
        assert_eq!(loaded.as_deref(), Some(prefs_json));

        // 4. Update preferences
        let updated_json = r#"{"pane_names":["MEDIA","STORAGE"],"pane_colors":{"0":"purple"}}"#;
        auth.save_user_preferences("admin", updated_json).unwrap();
        let loaded_updated = auth.get_user_preferences("admin").unwrap();
        assert_eq!(loaded_updated.as_deref(), Some(updated_json));

        // 5. Reset preferences
        auth.reset_user_preferences("admin").unwrap();
        let reset_result = auth.get_user_preferences("admin").unwrap();
        assert!(reset_result.is_none());
    }

    #[test]
    fn test_api_token_lifecycle_and_revocation() {
        let tmp = tempdir().unwrap();
        let db_file = tmp.path().join("test_tokens.db");
        let auth = AuthManager::new(
            &db_file.to_string_lossy(),
            "secret-key-123456789012345678901234",
            24,
            "builtin",
            "login",
            "admin",
            "admin",
        ).unwrap();

        // 1. Create API Token
        let gen = auth.create_api_token(
            "admin",
            "Proxmox LXC 104",
            "admin",
            Some("[\"/data\", \"/backup\"]".to_string()),
            Some(90),
        ).unwrap();

        assert!(gen.token.len() > 20);
        assert_eq!(gen.info.name, "Proxmox LXC 104");
        assert_eq!(gen.info.username, "admin");
        assert_eq!(gen.info.role, "admin");

        // 2. List API Tokens
        let tokens = auth.list_api_tokens(Some("admin")).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].id, gen.info.id);

        // 3. Verify API Token
        let claims = auth.verify_token(&gen.token).unwrap();
        assert_eq!(claims.sub, "admin");
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.token_id, Some(gen.info.id.clone()));

        // 4. Revoke API Token
        let revoked = auth.revoke_api_token(&gen.info.id, Some("admin")).unwrap();
        assert!(revoked);

        // 5. Verify Revoked Token Fails
        let verify_res = auth.verify_token(&gen.token);
        assert!(verify_res.is_err());
    }

    #[test]
    fn test_verify_token_allow_expired_for_session_unlock() {
        let tmp = tempdir().unwrap();
        let db_file = tmp.path().join("test_unlock.db");
        let auth = AuthManager::new(
            &db_file.to_string_lossy(),
            "secret-key-123456789012345678901234",
            24,
            "builtin",
            "login",
            "admin",
            "admin",
        ).unwrap();

        let user = auth.get_user_by_username("admin").unwrap().unwrap();
        let token = auth.generate_token(&user).unwrap();

        // 1. Valid token succeeds with both verify_token and verify_token_allow_expired
        let claims = auth.verify_token(&token).unwrap();
        assert_eq!(claims.sub, "admin");

        let claims_allow_exp = auth.verify_token_allow_expired(&token).unwrap();
        assert_eq!(claims_allow_exp.sub, "admin");
    }

    #[test]
    fn test_advanced_sharing_center_lifecycle() {
        let tmp = tempdir().unwrap();
        let db_file = tmp.path().join("test_shares.db");
        let auth = AuthManager::new(
            &db_file.to_string_lossy(),
            "secret-key-123456789012345678901234",
            24,
            "builtin",
            "login",
            "admin",
            "admin",
        ).unwrap();

        // 1. Create Showcase Share
        let share = auth.create_share(
            "admin",
            "/home/user/designs",
            "Architectural Showcase 2026",
            true,
            false,
            true,
            false,
            Some("Secret123"),
            None,
            10,
            Some("client@studio.com, partner@agency.ac"),
            true,
            true,
            Some("CONFIDENTIAL • {email} • {date}"),
        ).unwrap();

        assert_eq!(share.name, "Architectural Showcase 2026");
        assert!(share.allow_view);
        assert!(!share.allow_download);
        assert!(share.watermark_enabled);
        assert!(share.require_email);
        assert!(share.has_password);
        assert_eq!(share.status, "active");

        // 2. Email verification gate
        assert!(auth.verify_share_email(&share.token, "client@studio.com").unwrap());
        assert!(auth.verify_share_email(&share.token, "partner@agency.ac").unwrap());
        assert!(!auth.verify_share_email(&share.token, "intruder@domain.com").unwrap());

        // 3. Password verification gate
        assert!(auth.verify_share_password(&share.token, "Secret123").unwrap());
        assert!(!auth.verify_share_password(&share.token, "WrongPass").unwrap());

        // 4. Access Logging
        auth.log_share_access(share.id, &share.token, Some("client@studio.com"), "192.168.1.50", Some("Mozilla/5.0"), "visit", None).unwrap();
        auth.log_share_access(share.id, &share.token, Some("client@studio.com"), "192.168.1.50", Some("Mozilla/5.0"), "preview", Some("floorplan.png")).unwrap();

        let logs = auth.get_share_logs(share.id, "admin", true).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].action, "preview");
        assert_eq!(logs[0].target_file, Some("floorplan.png".to_string()));
        assert_eq!(logs[1].action, "visit");

        // 5. Aggregate metrics
        let fetched = auth.get_share_by_id(share.id).unwrap().unwrap();
        assert_eq!(fetched.total_visits, 1);
        assert_eq!(fetched.total_previews, 1);

        // 6. Revocation
        auth.revoke_share(share.id, "admin", true, true).unwrap();
        let revoked = auth.get_share_by_id(share.id).unwrap().unwrap();
        assert_eq!(revoked.status, "revoked");

        auth.revoke_share(share.id, "admin", true, false).unwrap();
        let reactivated = auth.get_share_by_id(share.id).unwrap().unwrap();
        assert_eq!(reactivated.status, "active");

        // 7. Delete
        auth.delete_share(share.id, "admin", true).unwrap();
        assert!(auth.get_share_by_id(share.id).unwrap().is_none());
    }

    #[test]
    fn test_db_notes_and_attachments_lifecycle() {
        let tmp = tempdir().unwrap();
        let db_file = tmp.path().join("test_notes.db");
        let auth = AuthManager::new(
            &db_file.to_string_lossy(),
            "secret-key-123456789012345678901234",
            24,
            "builtin",
            "login",
            "admin",
            "admin",
        ).unwrap();

        // 1. Create Note
        let note = auth.create_db_note(
            "admin",
            "Architecture Review 2026",
            "# Brum Core Design\n\nDatabase notes with attachments.",
            "Work",
            "Projects",
            "[\"design\", \"architecture\"]",
            true,
            false,
            Some("#f59e0b"),
        ).unwrap();

        assert_eq!(note.title, "Architecture Review 2026");
        assert_eq!(note.category, "Work");
        assert_eq!(note.section, "Projects");
        assert!(note.is_pinned);
        assert!(!note.is_archived);

        // 2. Add Attachment
        let att = auth.create_db_note_attachment(
            Some(note.id),
            "admin",
            "diagram.png",
            "image/png",
            1024,
            "/tmp/diagram.png",
            Some("fake-sha256-hash"),
        ).unwrap();

        assert_eq!(att.filename, "diagram.png");
        assert_eq!(att.note_id, Some(note.id));

        // 3. List and Get Note (with attachments included)
        let fetched = auth.get_db_note(note.id, "admin", true).unwrap().unwrap();
        assert_eq!(fetched.attachments.len(), 1);
        assert_eq!(fetched.attachments[0].filename, "diagram.png");

        let listed = auth.list_db_notes("admin", true, Some("Architecture"), None, None, None, false).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, note.id);

        // 4. Update Note
        let updated = auth.update_db_note(
            note.id,
            "admin",
            true,
            Some("Architecture Review 2026 (Updated)"),
            None,
            None,
            None,
            None,
            Some(false),
            None,
            None,
        ).unwrap();

        assert_eq!(updated.title, "Architecture Review 2026 (Updated)");
        assert!(!updated.is_pinned);

        // 5. Delete Attachment
        let del_att_path = auth.delete_db_note_attachment(att.id, "admin", true).unwrap().unwrap();
        assert_eq!(del_att_path, "/tmp/diagram.png");

        let re_fetched = auth.get_db_note(note.id, "admin", true).unwrap().unwrap();
        assert_eq!(re_fetched.attachments.len(), 0);

        // 6. Delete Note
        let paths = auth.delete_db_note(note.id, "admin", true).unwrap();
        assert_eq!(paths.len(), 0);
        assert!(auth.get_db_note(note.id, "admin", true).unwrap().is_none());
    }
}

