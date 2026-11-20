//! #193: a login from another Fleet machine is limited by this machine's own settings for the
//! account, not by the folders it may use on the machine where it signed in.

use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

const SECRET: &str = "fleet-own-limits-test-secret-0123456789abcdef";

fn config(dir: &TempDir) -> AppConfig {
    let mut c = AppConfig::default();
    c.server.host = "127.0.0.1".into();
    c.server.port = 0;
    c.server.database_path = dir.path().join("brum.db").to_string_lossy().to_string();
    c.server.enable_auth = true;
    c.server.jwt_secret = SECRET.into();
    c.storage.allow_entire_system = true;
    c.auth.default_admin_user = "admin".into();
    c.auth.default_admin_pass = "AdminSecretPass123!".into();
    c.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    c.plugins.user_directory = dir.path().join("uplugins").to_string_lossy().to_string();
    c
}

#[tokio::test]
async fn each_machine_applies_its_own_limits() {
    let files = tempdir().unwrap();
    let (carl_home, other) = (files.path().join("carl"), files.path().join("other"));
    std::fs::create_dir_all(&carl_home).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let s = |p: &std::path::Path| p.to_string_lossy().to_string();
    let (dm, dn) = (tempdir().unwrap(), tempdir().unwrap());
    let (pm, _) = start_background_server(config(&dm)).await.unwrap();
    let (pn, _) = start_background_server(config(&dn)).await.unwrap();
    let c = reqwest::Client::new();
    let login = |port: u16, user: &'static str, pass: &'static str| {
        let c = c.clone();
        async move {
            let v: Value = c.post(format!("http://127.0.0.1:{port}/api/auth/login")).json(&json!({ "username": user, "password": pass })).send().await.unwrap().json().await.unwrap();
            v["token"].as_str().unwrap_or_else(|| panic!("{v}")).to_string()
        }
    };
    // carl may use everything on the main Brum, only his folder on the other machine
    for (port, roots) in [(pm, json!(["*"])), (pn, json!([s(&carl_home)]))] {
        let admin = login(port, "admin", "AdminSecretPass123!").await;
        let st = c.post(format!("http://127.0.0.1:{port}/api/auth/users")).bearer_auth(&admin).json(&json!({ "username": "carl", "password": "CarlSecretPass123!", "role": "user", "home_dir": s(&carl_home), "allowed_roots": roots, "must_change_password": false })).send().await.unwrap().status();
        assert_eq!(st.as_u16(), 200);
    }
    let carl = login(pm, "carl", "CarlSecretPass123!").await;
    let list = |port: u16, p: &std::path::Path| {
        let (c, url, t) = (c.clone(), format!("http://127.0.0.1:{port}/api/fs/list?path={}", s(p)), carl.clone());
        async move { c.get(url).bearer_auth(t).send().await.unwrap().status().as_u16() }
    };
    assert_eq!(list(pm, &other).await, 200, "the main Brum lets carl in everywhere");
    assert_eq!(list(pn, &carl_home).await, 200, "his login is accepted on the other machine");
    assert_eq!(list(pn, &other).await, 403, "but there his own folder is all he may use");
}
