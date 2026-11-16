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
    // With nothing saved in Brum, the full file shipped up to 1.23 gives exactly what it gave before
    let text = include_str!("fixtures/config-full-1.23.toml");
    let file: toml::Table = toml::from_str(text).unwrap();
    let before = ConfigManager::parse_config_str(text).unwrap();
    let after = saved::compose(Some(&file), &Default::default()).unwrap();
    assert_eq!(serde_json::to_value(&before).unwrap(), serde_json::to_value(&after).unwrap());
}

#[tokio::test]
async fn a_new_port_is_checked_and_tried_out() {
    let dir = tempdir().unwrap();
    let b = Brum::start(config(&dir, None)).await;

    // A port another program holds is refused before anything is saved
    let busy = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let busy_port = busy.local_addr().unwrap().port();
    let (st, body) = b.save(json!({ "server.port": busy_port, "server.host": "0.0.0.0" })).await;
    assert_eq!(st, 400, "{body}");
    assert!(body.contains("cannot be used"), "{body}");
    drop(busy);
    // So is an address this machine does not have
    let (st, body) = b.save(json!({ "server.host": "203.0.113.77" })).await;
    assert_eq!(st, 400, "{body}");

    // A free port is saved and remembered as something to try out after the restart
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let r = b.c.post(b.url()).bearer_auth(&b.token).json(&json!({ "changes": { "server.port": free } })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["network_change"]["port"], json!(free));
    let conn = rusqlite::Connection::open(dir.path().join("brum.db")).unwrap();
    let trial: String = conn.query_row("SELECT value FROM server_settings WHERE key = 'config_trial'", [], |r| r.get(0)).unwrap();
    assert!(trial.contains("server.port"), "{trial}");

    // Not running yet (that starts with the restart), and "keep it" clears it
    let t: Value = b.c.get(format!("{}/trial", b.url())).bearer_auth(&b.token).send().await.unwrap().json().await.unwrap();
    assert_eq!(t["seconds_left"], Value::Null);
    let r = b.c.post(format!("{}/keep", b.url())).bearer_auth(&b.token).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let left: Option<String> = conn.query_row("SELECT value FROM server_settings WHERE key = 'config_trial'", [], |r| r.get(0)).ok();
    assert_eq!(left, None);
    assert_eq!(next_start(&dir, None).server.port, free);
}

#[test]
fn the_new_short_file_works_like_the_old_full_one() {
    // Fresh installs get the short file; they must work like the full one did, apart from the
    // defaults that are smarter on purpose
    let old = serde_json::to_value(ConfigManager::parse_config_str(include_str!("fixtures/config-full-1.23.toml")).unwrap()).unwrap();
    let new = serde_json::to_value(ConfigManager::parse_config_str(include_str!("../config.toml")).unwrap()).unwrap();
    let on_purpose = [
        "storage.default_user_home_template", // C:/Users/{username} on Windows, /data/users in Docker
        "auth.oidc.default_home_template",
        "syncthing.enabled",                  // on when Syncthing is found on the machine
        "bookmarks",                          // the home folder written out instead of "~"
    ];
    let mut differ = Vec::new();
    fn walk(p: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for (k, v) in x {
                    walk(&if p.is_empty() { k.clone() } else { format!("{p}.{k}") }, v, y.get(k).unwrap_or(&Value::Null), out)
                }
            }
            _ if a != b => out.push(p.to_string()),
            _ => {}
        }
    }
    walk("", &old, &new, &mut differ);
    differ.retain(|k| !on_purpose.contains(&k.as_str()));
    assert!(differ.is_empty(), "the short file changes: {differ:?}");
}

#[tokio::test]
async fn move_my_settings_into_brum() {
    let dir = tempdir().unwrap();
    let original = include_str!("fixtures/config-full-1.23.toml");
    let cfg = config(&dir, Some(original));
    let file = cfg.server.config_file.clone().unwrap();
    let before = serde_json::to_value(next_start(&dir, Some(&file))).unwrap();
    let b = Brum::start(cfg).await;
    let movable = b.get().await["movable"].as_u64().unwrap();
    assert!(movable > 40, "{movable}");

    let r = b.c.post(format!("{}/move-from-file", b.url())).bearer_auth(&b.token).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["moved"].as_array().unwrap().len() as u64, movable, "{v}");

    // The old file is kept as it was, the new one keeps only the rest
    assert_eq!(std::fs::read_to_string(v["backup"].as_str().unwrap()).unwrap(), original);
    let now = std::fs::read_to_string(&file).unwrap();
    assert!(!now.contains("upload_max_size_mb") && !now.contains("[paranoid]") && !now.contains("custom_actions"), "{now}");
    for kept in ["database_path", "[plugins]", "[[bookmarks]]"] {
        assert!(now.contains(kept), "{kept} stays in the file:\n{now}");
    }
    // Brum works exactly the same, and the settings are now changeable in the page
    assert_eq!(serde_json::to_value(next_start(&dir, Some(&file))).unwrap(), before);
    let all = b.get().await;
    assert_eq!(setting(&all, "server.upload_max_size_mb")["source"], json!("brum"));
    assert_eq!(all["movable"], json!(0));
    let (st, body) = b.save(json!({ "server.upload_max_size_mb": 1234 })).await;
    assert_eq!(st, 200, "{body}");
}

#[tokio::test]
async fn settings_that_cannot_be_used_stay_and_a_read_only_file_is_left_alone() {
    let dir = tempdir().unwrap();
    let text = "[server]\nserver_name = \"Den\"\n[[storage.roots]]\npath = \"/no/such/folder/for/brum\"\n";
    let cfg = config(&dir, Some(text));
    let file = cfg.server.config_file.clone().unwrap();
    let b = Brum::start(cfg).await;

    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&file, perms.clone()).unwrap();
    let r = b.c.post(format!("{}/move-from-file", b.url())).bearer_auth(&b.token).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 403);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    assert_eq!(setting(&b.get().await, "server.server_name")["source"], json!("file"), "nothing was saved");
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&file, perms).unwrap();

    let v: Value = b.c.post(format!("{}/move-from-file", b.url())).bearer_auth(&b.token).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["moved"], json!(["server.server_name"]));
    assert_eq!(v["stayed"][0]["key"], json!("storage.roots"));
    let now = std::fs::read_to_string(&file).unwrap();
    assert!(now.contains("/no/such/folder/for/brum") && !now.contains("Den"), "{now}");
}
