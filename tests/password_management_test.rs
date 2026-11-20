//! #205: passwords of Brum accounts. Admin reset (generated or set), "must choose a new password
//! at next sign-in" blocking everything else, own change needing the current password, the
//! password rules, accounts whose password is kept elsewhere, and admins not locking themselves out.

use base64::Engine;
use brum::config::AppConfig;
use brum::start_background_server;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

const ADMIN_PASS: &str = "AdminSecretPass123!";

struct Brum {
    base: String,
    c: reqwest::Client,
    admin: String,
    _dir: TempDir,
}

impl Brum {
    async fn start() -> Self {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::default();
        config.server.host = "127.0.0.1".into();
        config.server.port = 0;
        config.server.database_path = dir.path().join("brum.db").to_string_lossy().to_string();
        config.server.enable_auth = true;
        config.storage.allow_entire_system = true;
        config.auth.default_admin_user = "admin".into();
        config.auth.default_admin_pass = ADMIN_PASS.into();
        config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
        config.plugins.user_directory = dir.path().join("uplugins").to_string_lossy().to_string();
        let (port, _) = start_background_server(config).await.unwrap();
        let base = format!("http://127.0.0.1:{port}");
        let c = reqwest::Client::new();
        let admin = login(&c, &base, "admin", ADMIN_PASS).await.1["token"].as_str().unwrap().to_string();
        Brum { base, c, admin, _dir: dir }
    }
    async fn post(&self, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let r = self.c.post(format!("{}{path}", self.base)).bearer_auth(token).json(&body).send().await.unwrap();
        let st = r.status();
        let text = r.text().await.unwrap();
        (st, serde_json::from_str(&text).unwrap_or(json!({ "text": text })))
    }
    async fn get(&self, token: &str, path: &str) -> (StatusCode, Value) {
        let r = self.c.get(format!("{}{path}", self.base)).bearer_auth(token).send().await.unwrap();
        let st = r.status();
        let text = r.text().await.unwrap();
        (st, serde_json::from_str(&text).unwrap_or(json!({ "text": text })))
    }
    async fn create(&self, name: &str, pass: &str, must_change: Option<bool>) -> (StatusCode, Value) {
        let mut body = json!({ "username": name, "password": pass, "role": "user" });
        if let Some(m) = must_change {
            body["must_change_password"] = json!(m);
        }
        self.post(&self.admin, "/api/auth/users", body).await
    }
}

async fn login(c: &reqwest::Client, base: &str, user: &str, pass: &str) -> (StatusCode, Value) {
    let r = c.post(format!("{base}/api/auth/login")).json(&json!({ "username": user, "password": pass })).send().await.unwrap();
    let st = r.status();
    (st, r.json().await.unwrap_or(Value::Null))
}

fn token_of(v: &Value) -> String {
    v["token"].as_str().unwrap_or_else(|| panic!("no token in {v}")).to_string()
}

/// The login's own fields, as every Fleet machine reads them
fn claims(token: &str) -> Value {
    let part = token.split('.').nth(1).unwrap();
    serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
}

#[tokio::test]
async fn new_accounts_follow_the_rules_and_choose_their_own_password() {
    let b = Brum::start().await;
    // Rules: at least 12 characters, not the username
    let (st, v) = b.create("carol", "short", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("at least 12"), "{v}");
    let (st, v) = b.create("carolcarolcarol", "CarolCarolCarol", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

    // A new account must choose its own password first (on unless switched off)
    let (st, v) = b.create("carol", "Temporary-Pass-1", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = login(&b.c, &b.base, "carol", "Temporary-Pass-1").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["must_change_password"], json!(true), "{v}");
    let carol = token_of(&v);
    assert_eq!(claims(&carol)["pwc"], json!(true), "other Fleet machines must see the mark too");

    // Everything else is refused, the page can still ask who is signed in
    let (st, v) = b.get(&carol, "/api/tasks").await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["code"], json!("password_change_required"), "{v}");
    let (st, v) = b.get(&carol, "/api/auth/me").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["must_change_password"], json!(true), "{v}");

    // Choosing the new password: the current one is needed, and the rules apply
    let (st, _) = b.post(&carol, "/api/auth/password", json!({ "current_password": "wrong-one", "new_password": "Carols-Own-Pass-2" })).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, v) = b.post(&carol, "/api/auth/password", json!({ "current_password": "Temporary-Pass-1", "new_password": "Temporary-Pass-1" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "same as before: {v}");
    let (st, v) = b.post(&carol, "/api/auth/password", json!({ "current_password": "Temporary-Pass-1", "new_password": "Carols-Own-Pass-2" })).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let fresh = token_of(&v);
    assert!(claims(&fresh).get("pwc").is_none(), "the fresh login has no mark");
    assert_eq!(b.get(&fresh, "/api/tasks").await.0, StatusCode::OK);
    assert_eq!(b.get(&carol, "/api/tasks").await.0, StatusCode::UNAUTHORIZED, "the old login ended");
    let (_, v) = login(&b.c, &b.base, "carol", "Carols-Own-Pass-2").await;
    assert!(v.get("must_change_password").is_none(), "{v}");

    // The admin can switch it off when creating an account
    let (st, _) = b.create("dave", "Daves-Pass-12345", Some(false)).await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = login(&b.c, &b.base, "dave", "Daves-Pass-12345").await;
    assert_eq!(b.get(&token_of(&v), "/api/tasks").await.0, StatusCode::OK);
}

#[tokio::test]
async fn admin_reset_generated_or_set() {
    let b = Brum::start().await;
    b.create("erin", "Erins-First-Pass1", Some(false)).await;
    let (_, v) = login(&b.c, &b.base, "erin", "Erins-First-Pass1").await;
    let erin_before = token_of(&v);

    // Generated: shown once in the answer, all of erin's logins end, erin must choose a new one
    let (st, v) = b.post(&b.admin, "/api/auth/users/erin/password", json!({ "mode": "generate" })).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let generated = v["password"].as_str().unwrap().to_string();
    assert_eq!(generated.chars().count(), 20);
    assert_eq!(b.get(&erin_before, "/api/tasks").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&b.c, &b.base, "erin", "Erins-First-Pass1").await.0, StatusCode::UNAUTHORIZED);
    let (st, v) = login(&b.c, &b.base, "erin", &generated).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["must_change_password"], json!(true));

    // Set: follows the rules; "must choose a new one" can be switched off; no password comes back
    let (st, _) = b.post(&b.admin, "/api/auth/users/erin/password", json!({ "mode": "set", "password": "erin" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = b.post(&b.admin, "/api/auth/users/erin/password", json!({ "mode": "set", "password": "Set-By-The-Admin-1", "must_change": false })).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v.get("password").is_none(), "{v}");
    let (_, v) = login(&b.c, &b.base, "erin", "Set-By-The-Admin-1").await;
    assert!(v.get("must_change_password").is_none(), "{v}");

    // Only admins
    let erin = token_of(&v);
    let (st, _) = b.post(&erin, "/api/auth/users/admin/password", json!({ "mode": "generate" })).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn own_change_needs_the_current_password_and_the_profile_no_longer_changes_it() {
    let b = Brum::start().await;
    let (st, v) = b.post(&b.admin, "/api/auth/profile", json!({ "nickname": "Boss", "new_password": "Sneaky-New-Pass-1" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(login(&b.c, &b.base, "admin", ADMIN_PASS).await.0, StatusCode::OK, "password unchanged");
    let (st, _) = b.post(&b.admin, "/api/auth/password", json!({ "current_password": "", "new_password": "Sneaky-New-Pass-1" })).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admins_cannot_lock_themselves_out() {
    let b = Brum::start().await;
    let (st, v) = b.post(&b.admin, "/api/auth/users/admin", json!({ "role": "user" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v) = b.post(&b.admin, "/api/auth/users/admin", json!({ "role": "admin", "is_disabled": true })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let r = b.c.delete(format!("{}/api/auth/users/admin", b.base)).bearer_auth(&b.admin).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    // Another admin can
    b.create("frank", "Franks-Pass-12345", Some(false)).await;
    let (st, _) = b.post(&b.admin, "/api/auth/users/frank", json!({ "role": "readonly" })).await;
    assert_eq!(st, StatusCode::OK);
    // Still an admin, still signed in
    assert_eq!(b.get(&b.admin, "/api/auth/users").await.0, StatusCode::OK);
}

#[tokio::test]
async fn computer_accounts_have_no_password_in_brum() {
    let b = Brum::start().await;
    // A Linux account Brum has seen (as after its first sign-in)
    let db = b._dir.path().join("brum.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO users (username, password_hash, role, home_dir, is_pam, is_disabled, created_at) VALUES ('linuxer', 'PAM_MANAGED', 'user', '/home/linuxer', 1, 0, '')",
        [],
    ).unwrap();
    let (st, v) = b.post(&b.admin, "/api/auth/users/linuxer/password", json!({ "mode": "generate" })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v.to_string().contains("computer"), "{v}");
}
