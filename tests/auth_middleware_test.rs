use brum::config::AppConfig;
use brum::start_background_server;
use reqwest::header::AUTHORIZATION;
use serde_json::json;
use tempfile::tempdir;

async fn setup_test_server(enable_auth: bool, standalone: bool) -> (u16, Option<String>, tempfile::TempDir, String, String) {
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

    let (bound_port, token) = start_background_server(config).await.expect("Failed to start test server");
    (bound_port, token, dir, admin_user, admin_pass)
}

#[tokio::test]
async fn test_public_endpoints_and_config_sanitization() {
    let (port, _token, _dir, _admin_user, _admin_pass) = setup_test_server(true, false).await;
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
    let (port, _token, _dir, _admin_user, _admin_pass) = setup_test_server(true, false).await;
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
    let (port, _token, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
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
    let (port, _token, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
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
    let (port, _token, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
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
    let (port, token_opt, _dir, _admin_user, _admin_pass) = setup_test_server(false, true).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    let inst_tok = token_opt.expect("Standalone server must produce an instance token");

    // 1. Without instance token -> 403 Forbidden (protects loopback from unauthorized local users)
    let resp_unauth = client.get(format!("{}/api/system/disks", base_url)).send().await.unwrap();
    assert_eq!(resp_unauth.status(), reqwest::StatusCode::FORBIDDEN);

    // 2. With valid instance token query parameter -> 200 OK
    let resp_auth = client.get(format!("{}/api/system/disks?instance_token={}", base_url, inst_tok)).send().await.unwrap();
    assert_eq!(resp_auth.status(), reqwest::StatusCode::OK);

    // 3. With valid instance token header -> 200 OK
    let resp_hdr = client.get(format!("{}/api/system/disks", base_url))
        .header("X-Brum-Instance-Token", &inst_tok)
        .send().await.unwrap();
    assert_eq!(resp_hdr.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_initial_admin_credentials_atomic_mode_and_cleanup_on_login() {
    let dir = tempdir().expect("Failed to create tempdir");
    let cred_dir = dir.path().join("logs");

    // 1. Verify file is created with 0600 mode
    let target = brum::logging::write_initial_admin_credentials(&cred_dir, "admin", "DynamicPass123!").expect("write creds failed");
    assert!(target.is_file());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&target).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600, "Initial admin credentials file was not created with mode 0600");
    }

    // 2. Set up test server
    let (port, _token, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Write credentials in default log directory to verify cleanup on login
    let default_log_dir = brum::config::LoggingConfig::default().resolve_log_directory();
    let default_cred_file = brum::logging::write_initial_admin_credentials(&default_log_dir, &admin_user, &admin_pass).unwrap();
    assert!(default_cred_file.is_file());

    // 3. Login as admin
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

    // 4. Credentials file must now be deleted
    assert!(!default_cred_file.exists(), "Initial admin credentials file was not cleaned up after admin login!");
}

#[tokio::test]
async fn test_standalone_mode_refuses_non_loopback_bind() {
    let dir = tempdir().expect("Failed to create tempdir");
    let db_path = dir.path().join("brum_test.db").to_string_lossy().to_string();

    let mut config = AppConfig::default();
    config.server.host = "192.168.1.50".to_string(); // Non-loopback address
    config.server.port = 0;
    config.server.database_path = db_path;
    config.server.standalone = true; // Standalone desktop mode requires loopback bind

    let res = start_background_server(config).await;
    assert!(res.is_err(), "Standalone mode must refuse non-loopback bind address");
    let err_str = res.unwrap_err().to_string();
    assert!(err_str.contains("Standalone desktop instance cannot bind to non-loopback address"), "Unexpected error: {}", err_str);
}

#[tokio::test]
async fn test_instance_token_constant_time_comparison() {
    let (port, token_opt, _dir, _admin_user, _admin_pass) = setup_test_server(false, true).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    let valid_token = token_opt.expect("Standalone server must produce an instance token");

    // 1. Valid token -> 200 OK
    let resp = client.get(format!("{}/api/system/disks", base_url))
        .header("X-Brum-Instance-Token", &valid_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // 2. Token with different length -> 403 Forbidden
    let resp_diff_len = client.get(format!("{}/api/system/disks", base_url))
        .header("X-Brum-Instance-Token", format!("{}short", valid_token))
        .send().await.unwrap();
    assert_eq!(resp_diff_len.status(), reqwest::StatusCode::FORBIDDEN);

    // 3. Token with same length but altered byte -> 403 Forbidden
    let mut bad_token = valid_token.clone();
    let last_char = bad_token.pop().unwrap();
    let replacement = if last_char == 'a' { 'b' } else { 'a' };
    bad_token.push(replacement);

    let resp_same_len = client.get(format!("{}/api/system/disks", base_url))
        .header("X-Brum-Instance-Token", &bad_token)
        .send().await.unwrap();
    assert_eq!(resp_same_len.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_query_token_scoping_and_cookie_auth() {
    let (port, _token, _dir, admin_user, admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // 1. Authenticate to obtain a valid JWT token
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

    // 2. Query parameter ?token=... on standard HTTP API must be REJECTED (401 Unauthorized)
    let resp_query = client
        .get(format!("{}/api/system/disks?token={}", base_url, token))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp_query.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Query parameter ?token= must not authenticate standard HTTP API requests"
    );

    // 3. Query parameter ?cd_token=... on standard HTTP API must also be REJECTED (401 Unauthorized)
    let resp_cd_query = client
        .get(format!("{}/api/system/disks?cd_token={}", base_url, token))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp_cd_query.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Query parameter ?cd_token= must not authenticate standard HTTP API requests"
    );

    // 4. Authorization: Bearer <token> header must SUCCEED (200 OK)
    let resp_bearer = client
        .get(format!("{}/api/system/disks", base_url))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_bearer.status(), reqwest::StatusCode::OK);

    // 5. Cookie cd_token=<token> header must SUCCEED (200 OK)
    let resp_cookie = client
        .get(format!("{}/api/system/disks", base_url))
        .header(reqwest::header::COOKIE, format!("cd_token={}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_cookie.status(), reqwest::StatusCode::OK);

    // 6. /api/auth/me accepts Cookie cd_token=<token>
    let resp_me = client
        .get(format!("{}/api/auth/me", base_url))
        .header(reqwest::header::COOKIE, format!("cd_token={}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_me.status(), reqwest::StatusCode::OK);
    let me_data: serde_json::Value = resp_me.json().await.unwrap();
    assert_eq!(me_data["username"], "admin");

    // 7. /api/auth/logout is publicly accessible and returns Set-Cookie with Max-Age=0
    let resp_logout = client
        .post(format!("{}/api/auth/logout", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_logout.status(), reqwest::StatusCode::OK);
    let set_cookie = resp_logout.headers().get(reqwest::header::SET_COOKIE).expect("Set-Cookie header missing on logout");
    let cookie_str = set_cookie.to_str().unwrap();
    assert!(cookie_str.contains("Max-Age=0"), "Logout must clear cd_token cookie");
}

#[test]
fn test_is_request_secure_detection() {
    let mut headers = reqwest::header::HeaderMap::new();
    assert!(!brum::server::is_request_secure(&headers));

    headers.insert("x-forwarded-proto", "https".parse().unwrap());
    assert!(brum::server::is_request_secure(&headers));

    let mut headers_ssl = reqwest::header::HeaderMap::new();
    headers_ssl.insert("x-forwarded-ssl", "on".parse().unwrap());
    assert!(brum::server::is_request_secure(&headers_ssl));
}

