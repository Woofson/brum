use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::json;
use tempfile::tempdir;

async fn setup_test_server(enable_auth: bool, standalone: bool) -> (u16, tempfile::TempDir, String, String) {
    let dir = tempdir().expect("Failed to create tempdir");
    let db_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    let admin_user = "admin".to_string();
    let admin_pass = "AdminSecretPass123!".to_string();

    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0; // Dynamic port allocation
    config.server.database_path = db_path;
    config.server.enable_auth = enable_auth;
    config.server.standalone = standalone;
    config.auth.default_admin_user = admin_user.clone();
    config.auth.default_admin_pass = admin_pass.clone();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();

    let bound_port = start_background_server(config).await.expect("Failed to start test server");
    (bound_port, dir, admin_user, admin_pass)
}

#[tokio::test]
async fn test_security_headers_injected() {
    let (port, _dir, _admin_user, _admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let resp = client.get(format!("{}/api/health", base_url)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let headers = resp.headers();

    // Verify all mandatory security headers
    assert_eq!(
        headers.get("x-content-type-options").and_then(|v| v.to_str().ok()),
        Some("nosniff"),
        "Missing or incorrect X-Content-Type-Options"
    );
    assert_eq!(
        headers.get("x-frame-options").and_then(|v| v.to_str().ok()),
        Some("SAMEORIGIN"),
        "Missing or incorrect X-Frame-Options"
    );
    assert_eq!(
        headers.get("referrer-policy").and_then(|v| v.to_str().ok()),
        Some("strict-origin-when-cross-origin"),
        "Missing or incorrect Referrer-Policy"
    );
    assert!(
        headers.get("content-security-policy").is_some(),
        "Missing Content-Security-Policy header"
    );
    assert!(
        headers.get("permissions-policy").is_some(),
        "Missing Permissions-Policy header"
    );
}

#[tokio::test]
async fn test_login_rate_limiting_and_uniform_errors() {
    let (port, _dir, admin_user, _admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Send 10 failed login attempts with wrong password
    for _ in 0..10 {
        let resp = client
            .post(format!("{}/api/auth/login", base_url))
            .json(&json!({
                "username": admin_user,
                "password": "WrongPasswordAttempt!"
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
        let text = resp.text().await.unwrap();
        assert!(text.contains("Invalid username or password"));
    }

    // 11th attempt should trigger 429 Too Many Requests
    let rate_limited_resp = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&json!({
            "username": admin_user,
            "password": "WrongPasswordAttempt!"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        rate_limited_resp.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "Login endpoint did not enforce 429 rate limit after 10 failed attempts"
    );
}

#[tokio::test]
async fn test_session_unlock_hardening_no_fallback() {
    let (port, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // 1. Successful login
    let login_resp = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&json!({
            "username": admin_user,
            "password": admin_pass
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(login_resp.status(), reqwest::StatusCode::OK);
    let login_json: serde_json::Value = login_resp.json().await.unwrap();
    let token = login_json["token"].as_str().unwrap();

    // 2. Unlock with wrong password should fail with 401 "Invalid credentials"
    let unlock_fail = client
        .post(format!("{}/api/auth/unlock", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "password": "IncorrectPassword999!"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(unlock_fail.status(), reqwest::StatusCode::UNAUTHORIZED);
    let fail_text = unlock_fail.text().await.unwrap();
    assert!(fail_text.contains("Invalid credentials"));

    // 3. Unlock with correct password should succeed
    let unlock_ok = client
        .post(format!("{}/api/auth/unlock", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "password": admin_pass
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(unlock_ok.status(), reqwest::StatusCode::OK);
    let unlock_json: serde_json::Value = unlock_ok.json().await.unwrap();
    assert_eq!(unlock_json["success"], true);
}

#[tokio::test]
async fn test_share_token_128bit_entropy() {
    let (port, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Login as admin
    let login_resp = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&json!({
            "username": admin_user,
            "password": admin_pass
        }))
        .send()
        .await
        .unwrap();
    let login_json: serde_json::Value = login_resp.json().await.unwrap();
    let token = login_json["token"].as_str().unwrap();

    // Create a share
    let share_resp = client
        .post(format!("{}/api/shares", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "path": "/",
            "name": "Audit Test Share",
            "is_dir": true,
            "allow_upload": false,
            "allow_view": true,
            "allow_download": true,
            "password": "SharePassword123!",
            "expires_at": null,
            "max_downloads": 100
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(share_resp.status(), reqwest::StatusCode::OK);
    let share_json: serde_json::Value = share_resp.json().await.unwrap();
    let share_token = share_json["token"].as_str().unwrap();

    // Verify token is at least 32 characters (128-bit hex string)
    assert!(
        share_token.len() >= 32,
        "Share token '{}' length is {} (expected >= 32 for 128-bit entropy)",
        share_token,
        share_token.len()
    );

    // Verify share password rate limiting
    for _ in 0..10 {
        let verify_fail = client
            .post(format!("{}/api/public/shares/{}/verify", base_url, share_token))
            .json(&json!({ "password": "WrongSharePass!" }))
            .send()
            .await
            .unwrap();
        assert_eq!(verify_fail.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    // 11th attempt must be 429
    let verify_limited = client
        .post(format!("{}/api/public/shares/{}/verify", base_url, share_token))
        .json(&json!({ "password": "WrongSharePass!" }))
        .send()
        .await
        .unwrap();
    assert_eq!(verify_limited.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_websocket_origin_security() {
    use brum::server::terminal::is_ws_origin_allowed;
    use axum::http::{HeaderMap, HeaderValue};
    use brum::server::AppState;
    use brum::config::AppConfig;
    use std::sync::Arc;

    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ws_test.db").to_string_lossy().to_string();
    let mut config = AppConfig::default();
    config.server.host = "192.168.1.100".to_string();
    config.server.port = 8080;
    config.server.database_path = db_path.clone();

    let auth = brum::auth::AuthManager::new(
        &db_path,
        "jwt-sec",
        8,
        "database",
        "brum",
        "admin",
        "pass",
    ).unwrap();
    let auth_arc = Arc::new(auth);
    let task_mgr = Arc::new(brum::tools::tasks::TaskManager::new());
    let tag_mgr = Arc::new(brum::tools::tags::TagManager::new(auth_arc.db()).unwrap());
    let vault_mgr = Arc::new(brum::vfs::vault::VaultManager::new());
    let backup_mgr = Arc::new(brum::tools::sync::BackupManager::new(auth_arc.db()).unwrap());
    let plugin_mgr = Arc::new(brum::plugins::PluginManager::new(
        dir.path().join("p1"),
        dir.path().join("p2"),
        false,
        "allow_all".to_string(),
        vec![],
        vec![],
    ));
    let oidc_mgr = Arc::new(brum::auth::oidc::OidcManager::new(config.auth.oidc.clone(), auth_arc.clone()));

    let state = AppState {
        config: Arc::new(config),
        auth: auth_arc,
        oidc: oidc_mgr,
        tasks: task_mgr,
        tags: tag_mgr,
        vaults: vault_mgr,
        backup: backup_mgr,
        plugins: plugin_mgr,
        rate_limiter: Arc::new(brum::server::RateLimiter::new()),
    };

    let mut headers = HeaderMap::new();
    headers.insert("host", HeaderValue::from_static("192.168.1.100:8080"));

    // 1. Same-origin matches
    assert!(is_ws_origin_allowed("http://192.168.1.100:8080", &headers, &state));
    assert!(is_ws_origin_allowed("https://192.168.1.100:8080", &headers, &state));

    // 2. Loopback allowed
    assert!(is_ws_origin_allowed("http://localhost:8080", &headers, &state));
    assert!(is_ws_origin_allowed("http://127.0.0.1:8080", &headers, &state));

    // 3. Cross-origin / malicious attacker domain MUST BE REJECTED
    assert!(!is_ws_origin_allowed("http://malicious-attacker.com", &headers, &state));
    assert!(!is_ws_origin_allowed("https://evil-hacker.xyz:8080", &headers, &state));
    assert!(!is_ws_origin_allowed("http://phishing.site", &headers, &state));
}
