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

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub home_dir: String,
    pub is_pam: bool,
    #[serde(default = "default_allowed_roots_claims")]
    pub allowed_roots: Option<String>,
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
                password_hash TEXT,
                expires_at TEXT,
                created_by TEXT NOT NULL,
                created_at TEXT NOT NULL,
                download_count INTEGER NOT NULL DEFAULT 0,
                max_downloads INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

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

        // Safe migrations for newly added columns
        let _ = conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN email TEXT", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN avatar_url TEXT", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN allowed_services TEXT DEFAULT '[\"*\"]'", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN allowed_roots TEXT DEFAULT '[\"*\"]'", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN is_pam INTEGER DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE users ADD COLUMN is_disabled INTEGER DEFAULT 0", []);

        let auth = Self {
            db: Arc::new(Mutex::new(conn)),
            jwt_secret: jwt_secret.to_string(),
            session_hours,
            auth_mode: auth_mode.to_string(),
            pam_service: pam_service.to_string(),
        };

        // Seed default admin user if database is empty
        if auth.count_users()? == 0 {
            info!("No users found in database. Creating default admin user: {}", default_admin_user);
            auth.create_user(default_admin_user, default_admin_pass, "admin", "/", Some("[\"*\"]"))?;
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
            "INSERT INTO users (username, password_hash, role, home_dir, allowed_services, allowed_roots, is_pam, is_disabled, created_at) VALUES (?1, ?2, ?3, ?4, '[\"*\"]', ?5, 0, 0, ?6)",
            params![username, password_hash, role, home_dir, roots_json, now],
        )?;

        let id = conn.last_insert_rowid();

        Ok(User {
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
        })
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<User>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare(
            "SELECT id, username, nickname, email, avatar_url, role, home_dir, is_pam, is_disabled, allowed_services, allowed_roots FROM users WHERE username = ?1"
        )?;

        let user = stmt.query_row(params![username], |row| {
            Ok(User {
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
            })
        }).optional()?;

        Ok(user)
    }

    pub fn list_users(&self) -> Result<Vec<User>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut stmt = conn.prepare(
            "SELECT id, username, nickname, email, avatar_url, role, home_dir, is_pam, is_disabled, allowed_services, allowed_roots FROM users ORDER BY username ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(User {
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
            })
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
        self.get_user_by_username(username)?
            .ok_or_else(|| "Failed to retrieve synced PAM user".into())
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
        is_disabled: bool,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let dis_int = if is_disabled { 1 } else { 0 };
        let roots_str = allowed_roots.unwrap_or("[\"*\"]");

        let affected = if let Some(hd) = home_dir {
            conn.execute(
                "UPDATE users SET role = ?1, allowed_services = ?2, allowed_roots = ?3, home_dir = ?4, is_disabled = ?5 WHERE username = ?6",
                params![role, allowed_services, roots_str, hd, dis_int, username],
            )?
        } else {
            conn.execute(
                "UPDATE users SET role = ?1, allowed_services = ?2, allowed_roots = ?3, is_disabled = ?4 WHERE username = ?5",
                params![role, allowed_services, roots_str, dis_int, username],
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
                    "SELECT id, username, password_hash, role, home_dir, nickname, email, avatar_url, allowed_services, allowed_roots, is_pam, is_disabled FROM users WHERE username = ?1"
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
                        row.get::<_, i64>(10)? != 0,
                        row.get::<_, i64>(11)? != 0,
                    ))
                }).optional()?
            };

            if let Some((id, uname, hash_str, role, home_dir, nickname, email, avatar_url, allowed_services, allowed_roots, is_pam, is_disabled)) = user_res {
                if is_disabled {
                    return Err("Account is disabled. Please contact an administrator.".into());
                }

                // Only verify Argon2 if this is a native DB user (not PAM_MANAGED)
                if !is_pam && hash_str != "PAM_MANAGED" {
                    if let Ok(parsed_hash) = PasswordHash::new(&hash_str) {
                        if Argon2::default().verify_password(password.as_bytes(), &parsed_hash).is_ok() {
                            return Ok(User {
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
                            });
                        }
                    }
                }
            }
        }

        // 2. If auth mode is mixed or pam, try PAM authentication
        if self.auth_mode == "pam" || self.auth_mode == "mixed" {
            let mut services = vec![self.pam_service.as_str()];
            for fallback in &["common-auth", "sudo", "other", "passwd", "login"] {
                if !services.contains(fallback) {
                    services.push(fallback);
                }
            }

            for svc in services {
                if pam::authenticate(svc, username, password).is_ok() {
                    let (home_dir, def_role) = get_linux_user_info(username);
                    
                    // Check if this PAM user already has a linked DB record
                    if let Ok(Some(mut existing)) = self.get_user_by_username(username) {
                        if existing.is_disabled {
                            return Err("Account is disabled. Please contact an administrator.".into());
                        }
                        existing.is_pam = true;
                        return Ok(existing);
                    }

                    // Auto-link new PAM user to DB profile
                    let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
                    let now = Utc::now().to_rfc3339();
                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO users (username, password_hash, role, home_dir, allowed_services, allowed_roots, is_pam, is_disabled, created_at) VALUES (?1, 'PAM_MANAGED', ?2, ?3, '[\"*\"]', '[\"*\"]', 1, 0, ?4)",
                        params![username, def_role, home_dir, now],
                    );
                    let id = conn.last_insert_rowid();

                    return Ok(User {
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
                    });
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
            exp: expiration,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        Ok(token)
    }

    pub fn verify_token(&self, token: &str) -> Result<Claims, Box<dyn std::error::Error + Send + Sync>> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )?;

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

    // ---------------- LINK SHARING & DROPBOX ----------------
    pub fn create_share(
        &self,
        username: &str,
        path: &str,
        name: &str,
        is_dir: bool,
        allow_upload: bool,
        password: Option<&str>,
        expires_at: Option<&str>,
        max_downloads: u64,
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
            "INSERT INTO shares (token, path, name, is_dir, allow_upload, password_hash, expires_at, created_by, created_at, download_count, max_downloads)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10)",
            params![
                token,
                path,
                name,
                if is_dir { 1 } else { 0 },
                if allow_upload { 1 } else { 0 },
                password_hash,
                expires_at,
                username,
                now,
                max_downloads as i64,
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
            has_password,
            password_hash: None,
            expires_at: expires_at.map(|s| s.to_string()),
            created_by: username.to_string(),
            created_at: now,
            download_count: 0,
            max_downloads,
        })
    }

    pub fn list_shares(&self, username: &str, is_admin: bool) -> Result<Vec<ShareItem>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        let mut list = Vec::new();

        if is_admin {
            let mut stmt = conn.prepare("SELECT id, token, path, name, is_dir, allow_upload, password_hash, expires_at, created_by, created_at, download_count, max_downloads FROM shares ORDER BY id DESC")?;
            let rows = stmt.query_map([], |r| {
                let pass_hash: Option<String> = r.get(6)?;
                Ok(ShareItem {
                    id: r.get(0)?,
                    token: r.get(1)?,
                    path: r.get(2)?,
                    name: r.get(3)?,
                    is_dir: r.get::<_, i64>(4)? == 1,
                    allow_upload: r.get::<_, i64>(5)? == 1,
                    has_password: pass_hash.is_some(),
                    password_hash: None,
                    expires_at: r.get(7)?,
                    created_by: r.get(8)?,
                    created_at: r.get(9)?,
                    download_count: r.get::<_, i64>(10)? as u64,
                    max_downloads: r.get::<_, i64>(11)? as u64,
                })
            })?;
            for r in rows {
                list.push(r?);
            }
        } else {
            let mut stmt = conn.prepare("SELECT id, token, path, name, is_dir, allow_upload, password_hash, expires_at, created_by, created_at, download_count, max_downloads FROM shares WHERE created_by = ?1 ORDER BY id DESC")?;
            let rows = stmt.query_map(params![username], |r| {
                let pass_hash: Option<String> = r.get(6)?;
                Ok(ShareItem {
                    id: r.get(0)?,
                    token: r.get(1)?,
                    path: r.get(2)?,
                    name: r.get(3)?,
                    is_dir: r.get::<_, i64>(4)? == 1,
                    allow_upload: r.get::<_, i64>(5)? == 1,
                    has_password: pass_hash.is_some(),
                    password_hash: None,
                    expires_at: r.get(7)?,
                    created_by: r.get(8)?,
                    created_at: r.get(9)?,
                    download_count: r.get::<_, i64>(10)? as u64,
                    max_downloads: r.get::<_, i64>(11)? as u64,
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
        let mut stmt = conn.prepare("SELECT id, token, path, name, is_dir, allow_upload, password_hash, expires_at, created_by, created_at, download_count, max_downloads FROM shares WHERE token = ?1")?;
        let mut rows = stmt.query(params![token])?;

        if let Some(r) = rows.next()? {
            let pass_hash: Option<String> = r.get(6)?;
            Ok(Some(ShareItem {
                id: r.get(0)?,
                token: r.get(1)?,
                path: r.get(2)?,
                name: r.get(3)?,
                is_dir: r.get::<_, i64>(4)? == 1,
                allow_upload: r.get::<_, i64>(5)? == 1,
                has_password: pass_hash.is_some(),
                password_hash: pass_hash,
                expires_at: r.get(7)?,
                created_by: r.get(8)?,
                created_at: r.get(9)?,
                download_count: r.get::<_, i64>(10)? as u64,
                max_downloads: r.get::<_, i64>(11)? as u64,
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

    pub fn increment_share_downloads(&self, token: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.db.lock().map_err(|_| "DB lock poisoned")?;
        conn.execute("UPDATE shares SET download_count = download_count + 1 WHERE token = ?1", params![token])?;
        Ok(())
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareItem {
    pub id: i64,
    pub token: String,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub allow_upload: bool,
    pub has_password: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<String>,
    pub expires_at: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub download_count: u64,
    pub max_downloads: u64,
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
}
