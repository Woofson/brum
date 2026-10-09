use brum::config::AppConfig;
use brum::start_background_server;
use reqwest::header::AUTHORIZATION;
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
async fn test_public_endpoints_and_config_sanitization() {
    let (port, _dir, _admin_user, _admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // 1. /api/health should be public
    let resp = client.get(format!("{}/api/health", base_url)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let health_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(health_json["status"], "ok");

    // 2. /api/system/status should be public
    let resp = client.get(format!("{}/api/system/status", base_url)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // 3. /api/config should be public and sanitize secrets
    let resp = client.get(format!("{}/api/config", base_url)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let config_text = resp.text().await.unwrap();
    let config_json: serde_json::Value = serde_json::from_str(&config_text).unwrap();

    // Verify secret omission
    assert!(config_json["server"]["jwt_secret"].is_null(), "jwt_secret leaked in /api/config!");
    assert!(config_json["auth"]["default_admin_pass"].is_null(), "default_admin_pass leaked in /api/config!");
    assert!(!config_text.contains("AdminSecretPass123!"), "Raw admin password string leaked in /api/config response!");
}

#[tokio::test]
async fn test_unauthenticated_requests_blocked() {
    let (port, _dir, _admin_user, _admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Protected endpoints must return 401 UNAUTHORIZED
    let endpoints = vec![
        ("/api/system/disks", reqwest::Method::GET),
        ("/api/auth/users", reqwest::Method::GET),
        ("/api/tasks", reqwest::Method::GET),
        ("/api/fs/list?path=/", reqwest::Method::GET),
    ];

    for (path, method) in endpoints {
        let req = client.request(method, format!("{}{}", base_url, path));
        let resp = req.send().await.unwrap();
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "Endpoint {} did not return 401 when unauthenticated",
            path
        );
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["code"], "UNAUTHORIZED");
    }
}

#[tokio::test]
async fn test_removed_endpoints_not_found() {
    let (port, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Authenticate as admin
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
    let login_data: serde_json::Value = login_resp.json().await.unwrap();
    let token = login_data["token"].as_str().unwrap();

    // Verify removed arbitrary shell execution endpoint /api/actions/run returns 404 NOT FOUND
    let resp = client
        .post(format!("{}/api/actions/run", base_url))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .json(&json!({
            "command": "echo test"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND, "Deprecated /api/actions/run was not removed!");
}

#[tokio::test]
async fn test_rbac_admin_vs_regular_user() {
    let (port, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // 1. Admin login
    let login_resp = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&json!({
            "username": admin_user,
            "password": admin_pass
        }))
        .send()
        .await
        .unwrap();
    let login_data: serde_json::Value = login_resp.json().await.unwrap();
    let admin_token = login_data["token"].as_str().unwrap();

    // 2. Admin creates regular user
    let user_name = "standarduser";
    let user_pass = "UserSecretPass456!";
    let create_resp = client
        .post(format!("{}/api/auth/users", base_url))
        .header(AUTHORIZATION, format!("Bearer {}", admin_token))
        .json(&json!({
            "username": user_name,
            "password": user_pass,
            "role": "user"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);

    // 3. User login
    let user_login_resp = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&json!({
            "username": user_name,
            "password": user_pass
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(user_login_resp.status(), reqwest::StatusCode::OK);
    let user_login_data: serde_json::Value = user_login_resp.json().await.unwrap();
    let user_token = user_login_data["token"].as_str().unwrap();

    // 4. Regular user accessing non-admin protected endpoint -> 200 OK
    let disks_resp = client
        .get(format!("{}/api/system/disks", base_url))
        .header(AUTHORIZATION, format!("Bearer {}", user_token))
        .send()
        .await
        .unwrap();
    assert_eq!(disks_resp.status(), reqwest::StatusCode::OK);

    // 5. Regular user accessing admin-only endpoint -> 403 FORBIDDEN
    let admin_endpoints = vec![
        (format!("{}/api/auth/users", base_url), reqwest::Method::GET),
        (format!("{}/api/system/run-custom-action", base_url), reqwest::Method::POST),
        (format!("{}/api/system/exit", base_url), reqwest::Method::POST),
        (format!("{}/api/system/restart", base_url), reqwest::Method::POST),
        (format!("{}/api/system/autostart", base_url), reqwest::Method::GET),
    ];

    for (url, method) in admin_endpoints {
        let mut req = client
            .request(method.clone(), &url)
            .header(AUTHORIZATION, format!("Bearer {}", user_token));
        if method == reqwest::Method::POST {
            if url.contains("run-custom-action") {
                req = req.json(&json!({
                    "command": "echo test",
                    "target_path": "/"
                }));
            } else {
                req = req.json(&json!({}));
            }
        }
        let resp = req.send().await.unwrap();
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::FORBIDDEN,
            "Regular user was not forbidden from accessing admin endpoint {}",
            url
        );
    }
}

#[tokio::test]
async fn test_user_disablement_revocation() {
    let (port, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // 1. Admin login
    let login_resp = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&json!({
            "username": admin_user,
            "password": admin_pass
        }))
        .send()
        .await
        .unwrap();
    let admin_token = login_resp.json::<serde_json::Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // 2. Create user and obtain token
    let user_name = "revokeme";
    let user_pass = "RevokePass789!";
    let create_resp = client
        .post(format!("{}/api/auth/users", base_url))
        .header(AUTHORIZATION, format!("Bearer {}", admin_token))
        .json(&json!({
            "username": user_name,
            "password": user_pass,
            "role": "user"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);

    let user_login = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&json!({
            "username": user_name,
            "password": user_pass
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(user_login.status(), reqwest::StatusCode::OK);
    let user_token = user_login.json::<serde_json::Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // 3. Verify user token works initially
    let test1 = client
        .get(format!("{}/api/system/disks", base_url))
        .header(AUTHORIZATION, format!("Bearer {}", user_token))
        .send()
        .await
        .unwrap();
    assert_eq!(test1.status(), reqwest::StatusCode::OK);

    // 4. Admin disables the user account
    let disable_resp = client
        .put(format!("{}/api/auth/users/{}", base_url, user_name))
        .header(AUTHORIZATION, format!("Bearer {}", admin_token))
        .json(&json!({
            "role": "user",
            "is_disabled": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(disable_resp.status(), reqwest::StatusCode::OK);

    // 5. Existing user token must now immediately fail with 401 UNAUTHORIZED
    let test2 = client
        .get(format!("{}/api/system/disks", base_url))
        .header(AUTHORIZATION, format!("Bearer {}", user_token))
        .send()
        .await
        .unwrap();
    assert_eq!(
        test2.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Disabled user's existing JWT token was not rejected!"
    );
}

#[tokio::test]
async fn test_standalone_mode_behavior() {
    let (port, _dir, _admin_user, _admin_pass) = setup_test_server(false, true).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // In standalone mode, protected endpoints succeed unauthenticated
    let resp = client.get(format!("{}/api/system/disks", base_url)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}
