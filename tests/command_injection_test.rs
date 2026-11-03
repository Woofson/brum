//! File names must never run as shell commands through Custom Actions or Open With.
#![cfg(unix)]

use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

const HOSTILE: &str = "a$(touch INJECTED)`touch INJECTED`\"; touch INJECTED; \"'; touch INJECTED; ' b.txt";

async fn start() -> (String, tempfile::TempDir, reqwest::Client, String) {
    let dir = tempdir().expect("Failed to create tempdir");
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
    let (port, _) = start_background_server(config).await.expect("Failed to start test server");
    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    let body: Value = client.post(format!("{}/api/auth/login", base))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap().json().await.unwrap();
    let token = body["token"].as_str().expect("admin login failed").to_string();
    (base, dir, client, token)
}

/// Commands are started in the background, so give them a moment to finish
async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
}

#[tokio::test]
async fn custom_actions_do_not_run_file_names() {
    let (base, dir, client, token) = start().await;
    let file = dir.path().join(HOSTILE);
    std::fs::write(&file, "one\ntwo\n").unwrap();
    let file_s = file.to_string_lossy().to_string();
    let out = dir.path().join("count.txt");

    for command in [
        // The built-in Shell Actions, as the right-click menu sends them
        "wc -l \"{file}\"".to_string(),
        "md5sum \"{file}\"".to_string(),
        // A user action that writes its result, to prove the command still ran
        format!("wc -l {{selection}} > '{}'", out.display()),
    ] {
        let resp = client.post(format!("{}/api/system/run-custom-action", base))
            .bearer_auth(&token)
            .json(&json!({
                "command": command,
                "target_path": file_s,
                "selection": [file_s],
                "target_pane_path": dir.path().to_string_lossy(),
            }))
            .send().await.unwrap();
        assert!(resp.status().is_success(), "action failed: {}", resp.text().await.unwrap());
    }
    settle().await;
    assert!(!dir.path().join("INJECTED").exists(), "a file name ran as a command");
    let counted = std::fs::read_to_string(&out).expect("the action itself did not run");
    assert!(counted.trim_start().starts_with('2'), "unexpected output: {counted}");
}

#[tokio::test]
async fn open_with_does_not_run_file_names() {
    let (base, dir, client, token) = start().await;
    let file = dir.path().join(HOSTILE);
    std::fs::write(&file, "x").unwrap();
    let resp = client.post(format!("{}/api/system/open-with", base))
        .bearer_auth(&token)
        .json(&json!({ "file_path": file.to_string_lossy(), "command": "cat \"%1\" > /dev/null" }))
        .send().await.unwrap();
    assert!(resp.status().is_success(), "open-with failed: {}", resp.text().await.unwrap());
    settle().await;
    assert!(!dir.path().join("INJECTED").exists(), "a file name ran as a command");
}
