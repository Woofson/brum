//! Brum's built-in manuals open in Notes for every user, and nothing else can be read through them.

use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

async fn login(client: &reqwest::Client, base: &str, user: &str, pass: &str) -> String {
    let body: Value = client.post(format!("{}/api/auth/login", base))
        .json(&json!({ "username": user, "password": pass }))
        .send().await.unwrap().json().await.unwrap();
    body["token"].as_str().unwrap_or_else(|| panic!("login failed for {}: {}", user, body)).to_string()
}

async fn read(client: &reqwest::Client, base: &str, token: &str, path: &str) -> (u16, Value) {
    let resp = client.get(format!("{}/api/fs/read", base))
        .query(&[("path", path)])
        .bearer_auth(token)
        .send().await.unwrap();
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn manuals_open_for_admins_and_users_and_stay_read_only() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.storage.allow_entire_system = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = "AdminSecretPass123!".to_string();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    let (port, _) = start_background_server(config).await.unwrap();
    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    let admin = login(&client, &base, "admin", "AdminSecretPass123!").await;

    let home = dir.path().join("bob");
    std::fs::create_dir_all(&home).unwrap();
    let home_s = home.to_string_lossy().to_string();
    let resp = client.post(format!("{}/api/auth/users", base))
        .bearer_auth(&admin)
        .json(&json!({ "username": "bob", "password": "UserPass123!", "role": "user", "home_dir": home_s, "allowed_roots": [home_s] }))
        .send().await.unwrap();
    assert!(resp.status().is_success());
    let bob = login(&client, &base, "bob", "UserPass123!").await;

    // Every manual listed in Notes opens, for both users
    let list: Value = client.get(format!("{}/api/manuals", base)).bearer_auth(&admin)
        .send().await.unwrap().json().await.unwrap();
    let mut paths = Vec::new();
    for sec in list["sections"].as_array().unwrap() {
        for note in sec["notes"].as_array().unwrap() {
            paths.push(note["path"].as_str().unwrap().to_string());
        }
    }
    assert!(paths.len() > 10, "expected the full manual list");
    for token in [&admin, &bob] {
        for p in &paths {
            let (status, body) = read(&client, &base, token, p).await;
            assert_eq!(status, 200, "{p} did not open: {body}");
            assert!(!body["content"].as_str().unwrap().is_empty(), "{p} is empty");
        }
    }

    // Only shipped manuals: no other files through the manual address
    let secret = dir.path().join("secret.md");
    std::fs::write(&secret, "secret").unwrap();
    for p in [
        format!("manual://{}", secret.display()),
        "manual://../Cargo.toml".to_string(),
        "manual://....//Cargo.toml".to_string(),
        "manual://missing.md".to_string(),
    ] {
        let (status, _) = read(&client, &base, &bob, &p).await;
        assert_eq!(status, 404, "{p} should not be readable");
    }

    // Manuals cannot be changed
    let resp = client.post(format!("{}/api/fs/write", base))
        .bearer_auth(&admin)
        .json(&json!({ "path": "manual://README.md", "content": "changed" }))
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 403);
}
