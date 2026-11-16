//! #195: server settings saved inside Brum. Admins save them in the web page; config.toml and
//! environment variables still win; the "settings only in the file" switch blocks the page;
//! secrets never come back to the browser; saved settings are used at the next start.

use brum::config::{saved, AppConfig, ConfigManager};
use brum::start_background_server;
use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

const SECRET: &str = "server-settings-test-secret-0123456789abcdef";
const ADMIN_PASS: &str = "AdminSecretPass123!";

fn config(dir: &TempDir, file_text: Option<&str>) -> AppConfig {
    let mut c = AppConfig::default();
    if let Some(text) = file_text {
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        c = ConfigManager::parse_config_str(text).unwrap();
        c.server.config_file = Some(path);
    }
    c.server.host = "127.0.0.1".into();
    c.server.port = 0;
    c.server.database_path = dir.path().join("brum.db").to_string_lossy().to_string();
    c.server.enable_auth = true;
    c.server.jwt_secret = SECRET.into();
    c.auth.default_admin_user = "admin".into();
    c.auth.default_admin_pass = ADMIN_PASS.into();
    c.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    c.plugins.user_directory = dir.path().join("uplugins").to_string_lossy().to_string();
    c
}

struct Brum {
    port: u16,
    token: String,
    c: reqwest::Client,
}

impl Brum {
    async fn start(cfg: AppConfig) -> Self {
        let (port, _) = start_background_server(cfg).await.unwrap();
        let c = reqwest::Client::new();
        let token = login(&c, port, "admin", ADMIN_PASS).await;
        Brum { port, token, c }
    }
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/api/admin/server-settings", self.port)
    }
    async fn get(&self) -> Value {
        let r = self.c.get(self.url()).bearer_auth(&self.token).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
        r.json().await.unwrap()
    }
    async fn save(&self, changes: Value) -> (u16, String) {
        let r = self.c.post(self.url()).bearer_auth(&self.token).json(&json!({ "changes": changes })).send().await.unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }
}

async fn login(c: &reqwest::Client, port: u16, user: &str, pass: &str) -> String {
    let v: Value = c.post(format!("http://127.0.0.1:{port}/api/auth/login")).json(&json!({ "username": user, "password": pass })).send().await.unwrap().json().await.unwrap();
    v["token"].as_str().unwrap_or_else(|| panic!("{v}")).to_string()
}

fn setting<'a>(all: &'a Value, key: &str) -> &'a Value {
    all["settings"].as_array().unwrap().iter().find(|s| s["key"] == key).unwrap_or_else(|| panic!("no {key}"))
}

/// What Brum would start with next time, read the way start-up reads it
fn next_start(dir: &TempDir, file: Option<&PathBuf>) -> AppConfig {
    let db = dir.path().join("brum.db").to_string_lossy().to_string();
    let table = ConfigManager::read_config_file_table(file.map(|p| p.as_path()));
    saved::compose(table.as_ref(), &saved::read_from_db_file(&db)).unwrap()
}

#[tokio::test]
async fn saved_settings_survive_a_restart_and_the_file_wins() {
    let dir = tempdir().unwrap();
    let cfg = config(&dir, Some("[server]\nserver_name = \"From the file\"\n"));
    let file = cfg.server.config_file.clone();
    let b = Brum::start(cfg).await;

    let all = b.get().await;
    assert_eq!(all["editable"], json!(true));
    assert_eq!(setting(&all, "server.server_name")["source"], json!("file"));
    assert_eq!(setting(&all, "fleet.transfer_jobs")["source"], json!("default"));

    let (st, body) = b.save(json!({ "fleet.transfer_jobs": 5, "paranoid.confirm_delete": false, "ui.default_view_mode": "grid" })).await;
    assert_eq!(st, 200, "{body}");
    let all = b.get().await;
    let jobs = setting(&all, "fleet.transfer_jobs");
    assert_eq!((&jobs["value"], &jobs["source"], &jobs["restart_needed"]), (&json!(5), &json!("brum"), &json!(true)));

    // Set in the file: refused, and the file's value is what Brum starts with
    let (st, body) = b.save(json!({ "server.server_name": "From the page" })).await;
    assert_eq!(st, 409, "{body}");
    assert!(body.contains("config.toml"), "{body}");

    // Brum starts again: the saved settings are used, the file still wins
    let next = next_start(&dir, file.as_ref());
    assert_eq!(next.fleet.transfer_jobs, 5);
    assert!(!next.paranoid.confirm_delete);
    assert_eq!(next.ui.default_view_mode, "grid");
    assert_eq!(next.server.server_name, "From the file");

    // Back to the default
    let (st, body) = b.save(json!({ "fleet.transfer_jobs": null })).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(next_start(&dir, file.as_ref()).fleet.transfer_jobs, 2);
    assert_eq!(setting(&b.get().await, "fleet.transfer_jobs")["source"], json!("default"));
}

#[tokio::test]
async fn bad_values_and_file_only_settings_are_refused() {
    let dir = tempdir().unwrap();
    let b = Brum::start(config(&dir, None)).await;
    for (changes, why) in [
        (json!({ "server.port": 70000 }), "port out of range"),
        (json!({ "auth.mode": "none" }), "logins cannot be switched off"),
        (json!({ "server.database_path": "/tmp/x.db" }), "file-only"),
        (json!({ "server.allow_web_config_edit": false }), "file-only"),
        (json!({ "server.enable_auth": false }), "file-only"),
        (json!({ "terminal.default_shell": "/bin/sh" }), "names a program"),
        (json!({ "no.such.setting": 1 }), "unknown"),
        (json!({ "storage.roots": [{ "path": "/no/such/folder/for/brum" }] }), "missing folder"),
        (json!({ "server.trusted_proxies": ["not an address"] }), "bad address"),
    ] {
        let (st, body) = b.save(changes).await;
        assert_eq!(st, 400, "{why}: {body}");
    }
    // One bad value means nothing is saved
    let (st, _) = b.save(json!({ "fleet.transfer_jobs": 3, "server.port": 0 })).await;
    assert_eq!(st, 400);
    assert_eq!(setting(&b.get().await, "fleet.transfer_jobs")["source"], json!("default"));
}

#[tokio::test]
async fn secrets_are_stored_encrypted_and_never_sent_back() {
    let dir = tempdir().unwrap();
    let b = Brum::start(config(&dir, None)).await;
    let (st, body) = b.save(json!({ "auth.oidc.client_secret": "very-secret-value" })).await;
    assert_eq!(st, 200, "{body}");
    let all = b.get().await;
    assert_eq!(setting(&all, "auth.oidc.client_secret")["value"], json!("********"));
    assert!(!all.to_string().contains("very-secret-value"));

    // Not readable in the database file itself
    let conn = rusqlite::Connection::open(dir.path().join("brum.db")).unwrap();
    let stored: String = conn.query_row("SELECT value FROM server_settings WHERE key = 'config:auth.oidc.client_secret'", [], |r| r.get(0)).unwrap();
    assert!(!stored.contains("very-secret-value"), "{stored}");

    // Sending the mask back keeps it; the next start gets the real value
    let (st, _) = b.save(json!({ "auth.oidc.client_secret": "********", "auth.oidc.provider_name": "Keycloak" })).await;
    assert_eq!(st, 200);
    let next = next_start(&dir, None);
    assert_eq!(next.auth.oidc.client_secret, "very-secret-value");
    assert_eq!(next.auth.oidc.provider_name, "Keycloak");
}

#[tokio::test]
async fn only_admins_and_never_when_locked() {
    let dir = tempdir().unwrap();
    let b = Brum::start(config(&dir, None)).await;
    let st = b.c.post(format!("http://127.0.0.1:{}/api/auth/users", b.port)).bearer_auth(&b.token)
        .json(&json!({ "username": "dana", "password": "DanaSecretPass123!", "role": "user", "home_dir": dir.path() })).send().await.unwrap().status();
    assert_eq!(st.as_u16(), 200);
    let dana = login(&b.c, b.port, "dana", "DanaSecretPass123!").await;
    assert_eq!(b.c.get(b.url()).bearer_auth(&dana).send().await.unwrap().status().as_u16(), 403);
    let r = b.c.post(b.url()).bearer_auth(&dana).json(&json!({ "changes": { "fleet.transfer_jobs": 4 } })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 403);
    assert_eq!(b.c.get(b.url()).send().await.unwrap().status().as_u16(), 401, "not logged in");

    // The lock switch in config.toml blocks the page entirely
    let locked_dir = tempdir().unwrap();
    let locked = Brum::start(config(&locked_dir, Some("[server]\nallow_web_config_edit = false\n"))).await;
    assert_eq!(locked.get().await["editable"], json!(false));
    let (st, body) = locked.save(json!({ "fleet.transfer_jobs": 4 })).await;
    assert_eq!(st, 403, "{body}");
}

#[test]
fn an_old_full_config_file_works_unchanged() {
    // With nothing saved in Brum, the shipped example file gives exactly what it gave before
    let text = include_str!("../config.toml");
    let file: toml::Table = toml::from_str(text).unwrap();
    let before = ConfigManager::parse_config_str(text).unwrap();
    let after = saved::compose(Some(&file), &Default::default()).unwrap();
    assert_eq!(serde_json::to_value(&before).unwrap(), serde_json::to_value(&after).unwrap());
}
