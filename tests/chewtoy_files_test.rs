//! Sandboxed Chewtoys load their files below a signed address, without the login cookie.

use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

#[tokio::test]
async fn signed_chewtoy_addresses_open_only_their_own_files() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = "AdminSecretPass123!".to_string();
    config.plugins.directory = concat!(env!("CARGO_MANIFEST_DIR"), "/plugins").to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    let (port, _) = start_background_server(config).await.unwrap();
    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    let body: Value = client.post(format!("{base}/api/auth/login"))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap().json().await.unwrap();
    let token = body["token"].as_str().unwrap().to_string();

    // Without login there is no address
    let resp = client.get(format!("{base}/api/plugins/colours/launch")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 401);

    let launch: Value = client.get(format!("{base}/api/plugins/colours/launch")).bearer_auth(&token)
        .send().await.unwrap().json().await.unwrap();
    let files = launch["base"].as_str().unwrap().to_string();
    assert!(files.starts_with("/api/plugins/colours/files/"), "{files}");

    // The page and the files it loads, with no login at all (as from the sandbox)
    for name in ["index.html", "main.js", "style.css", "assets/icon.svg"] {
        let resp = client.get(format!("{base}{files}{name}")).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 200, "{name}");
        let csp = resp.headers().get("content-security-policy").unwrap().to_str().unwrap().to_string();
        assert!(csp.starts_with("sandbox"), "{name} must stay sandboxed");
    }

    // A changed signature, or the address of one Chewtoy used for another, is refused
    let sig = files.trim_end_matches('/').rsplit('/').next().unwrap().to_string();
    let mut forged = sig.clone();
    forged.replace_range(forged.len() - 1.., if sig.ends_with('0') { "1" } else { "0" });
    for path in [
        format!("/api/plugins/colours/files/{forged}/main.js"),
        format!("/api/plugins/tetrion/files/{sig}/main.js"),
        "/api/plugins/colours/files/1.61.00/main.js".to_string(),
    ] {
        let resp = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 403, "{path}");
    }

    // Still no way out of the plugin folder
    let resp = client.get(format!("{base}{files}..%2F..%2F..%2FCargo.toml")).send().await.unwrap();
    assert!(!resp.status().is_success());
}
