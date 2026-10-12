use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::json;
use tempfile::tempdir;

async fn setup_test_server_with_proxies(
    enable_auth: bool,
    standalone: bool,
    trusted_proxies: Vec<String>,
) -> (u16, tempfile::TempDir, String, String) {
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
    config.server.trusted_proxies = trusted_proxies;
    config.auth.default_admin_user = admin_user.clone();
    config.auth.default_admin_pass = admin_pass.clone();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();

    let (bound_port, _token) = start_background_server(config).await.expect("Failed to start test server");
    (bound_port, dir, admin_user, admin_pass)
}

async fn setup_test_server(enable_auth: bool, standalone: bool) -> (u16, tempfile::TempDir, String, String) {
    setup_test_server_with_proxies(enable_auth, standalone, Vec::new()).await
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
        instance_info: None,
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
    assert!(!is_ws_origin_allowed("http://localhost.attacker.com", &headers, &state));
    assert!(!is_ws_origin_allowed("http://127.0.0.1.attacker.com", &headers, &state));
}

#[tokio::test]
async fn test_rate_limiter_bounded_capacity_pruning() {
    let limiter = brum::server::RateLimiter::new();

    // Insert 6000 rotating keys (simulating memory exhaustion attack)
    for i in 0..6000 {
        limiter.record(&format!("attack_key_{}", i), 60);
    }

    // Capacity must be strictly bounded (capped at max 5000)
    assert!(limiter.len() <= 5000, "RateLimiter leaked memory: size was {}", limiter.len());
}

#[tokio::test]
async fn test_login_rate_limiting_ignores_spoofed_x_forwarded_for() {
    let (port, _dir, admin_user, _admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Send 10 failed login attempts, each with a different spoofed X-Forwarded-For header
    for i in 1..=10 {
        let resp = client
            .post(format!("{}/api/auth/login", base_url))
            .header("X-Forwarded-For", format!("198.51.100.{}", i))
            .header("X-Real-IP", format!("198.51.100.{}", i))
            .json(&json!({
                "username": admin_user,
                "password": "WrongPasswordAttempt!"
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    // 11th attempt with yet another spoofed IP must still be blocked by peer address
    let rate_limited_resp = client
        .post(format!("{}/api/auth/login", base_url))
        .header("X-Forwarded-For", "203.0.113.99")
        .header("X-Real-IP", "203.0.113.99")
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
        "Spoofed X-Forwarded-For allowed rate limit bypass!"
    );
}

#[tokio::test]
async fn test_login_user_lockout_on_failed_attempts() {
    let (port, _dir, _admin_user, _admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let target_user = "brute_force_target";

    // 10 failed attempts against the specific user account
    for _ in 0..10 {
        let resp = client
            .post(format!("{}/api/auth/login", base_url))
            .json(&json!({
                "username": target_user,
                "password": "WrongPasswordAttempt!"
            }))
            .send()
            .await
            .unwrap();

        // Could be 401 Unauthorized (or 429 if IP limit hits at the same threshold)
        assert!(resp.status() == reqwest::StatusCode::UNAUTHORIZED || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS);
    }

    // Next attempt must be rate-limited
    let resp = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&json!({
            "username": target_user,
            "password": "WrongPasswordAttempt!"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_security_headers_present_on_401_unauthorized() {
    let (port, _dir, _admin_user, _admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Request protected route without token (triggers 401 Unauthorized from auth_middleware)
    let resp = client.get(format!("{}/api/auth/me", base_url)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let headers = resp.headers();
    assert_eq!(
        headers.get("x-content-type-options").and_then(|v| v.to_str().ok()),
        Some("nosniff"),
        "401 response missing X-Content-Type-Options"
    );
    assert_eq!(
        headers.get("x-frame-options").and_then(|v| v.to_str().ok()),
        Some("SAMEORIGIN"),
        "401 response missing X-Frame-Options"
    );
    assert_eq!(
        headers.get("referrer-policy").and_then(|v| v.to_str().ok()),
        Some("strict-origin-when-cross-origin"),
        "401 response missing Referrer-Policy"
    );
    assert!(
        headers.get("content-security-policy").is_some(),
        "401 response missing Content-Security-Policy"
    );
    assert!(
        headers.get("permissions-policy").is_some(),
        "401 response missing Permissions-Policy"
    );
}

#[tokio::test]
async fn test_cors_restricts_unauthorized_foreign_origin() {
    let (port, _dir, _admin_user, _admin_pass) = setup_test_server(true, false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Request with foreign origin (e.g. malicious site)
    let foreign_resp = client
        .get(format!("{}/api/health", base_url))
        .header("Origin", "https://malicious-attacker.com")
        .send()
        .await
        .unwrap();

    assert_eq!(foreign_resp.status(), reqwest::StatusCode::OK);
    assert!(
        foreign_resp.headers().get("access-control-allow-origin").is_none(),
        "Foreign origin must NOT receive Access-Control-Allow-Origin header"
    );

    // Request with null origin (sandboxed iframe / data URI / file URI)
    let null_resp = client
        .get(format!("{}/api/health", base_url))
        .header("Origin", "null")
        .send()
        .await
        .unwrap();

    assert_eq!(null_resp.status(), reqwest::StatusCode::OK);
    assert!(
        null_resp.headers().get("access-control-allow-origin").is_none(),
        "Origin: null must NOT receive Access-Control-Allow-Origin header"
    );

    // Request with DNS-rebinding attempt (matching evil Host and Origin)
    let rebinding_resp = client
        .get(format!("{}/api/health", base_url))
        .header("Host", "evil.attacker.com")
        .header("Origin", "https://evil.attacker.com")
        .send()
        .await
        .unwrap();

    assert_eq!(rebinding_resp.status(), reqwest::StatusCode::OK);
    assert!(
        rebinding_resp.headers().get("access-control-allow-origin").is_none(),
        "DNS rebinding Host/Origin pair must NOT receive Access-Control-Allow-Origin header"
    );

    // Request with loopback origin
    let loopback_resp = client
        .get(format!("{}/api/health", base_url))
        .header("Origin", format!("http://127.0.0.1:{}", port))
        .send()
        .await
        .unwrap();

    assert_eq!(loopback_resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        loopback_resp.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()),
        Some(format!("http://127.0.0.1:{}", port).as_str())
    );
}

#[test]
fn test_resolve_client_ip_rightmost_untrusted_logic() {
    use axum::http::{HeaderMap, HeaderValue};
    use brum::server::middleware::resolve_client_ip;
    use std::net::SocketAddr;

    let trusted = vec!["127.0.0.1".to_string(), "10.0.0.1".to_string(), "10.42.0.0/16".to_string()];
    let peer_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();

    // 1. Rightmost untrusted IP selected when spoofed prefix is prepended
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("1.2.3.4, 203.0.113.195, 10.0.0.1"),
    );
    let resolved = resolve_client_ip(&headers, Some(peer_addr), &trusted);
    assert_eq!(resolved, "203.0.113.195");

    // 2. Untrusted peer direct connection ignores X-Forwarded-For
    let untrusted_peer: SocketAddr = "198.51.100.50:50000".parse().unwrap();
    let resolved_untrusted = resolve_client_ip(&headers, Some(untrusted_peer), &trusted);
    assert_eq!(resolved_untrusted, "198.51.100.50");

    // 3. Peer within CIDR subnet (10.42.1.88 within 10.42.0.0/16) is recognized as trusted proxy
    let cidr_peer: SocketAddr = "10.42.1.88:43210".parse().unwrap();
    let resolved_cidr = resolve_client_ip(&headers, Some(cidr_peer), &trusted);
    assert_eq!(resolved_cidr, "203.0.113.195");

    // 4. Peer outside CIDR subnet (10.43.1.88 outside 10.42.0.0/16) is untrusted
    let outside_cidr_peer: SocketAddr = "10.43.1.88:43210".parse().unwrap();
    let resolved_outside = resolve_client_ip(&headers, Some(outside_cidr_peer), &trusted);
    assert_eq!(resolved_outside, "10.43.1.88");
}

#[tokio::test]
async fn test_unlock_and_share_separate_rate_limit_buckets_per_peer() {
    let (port, _dir, admin_user, admin_pass) = setup_test_server_with_proxies(true, false, vec!["127.0.0.1".to_string()]).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // 1. Login to get token for unlock test
    let login_resp = client
        .post(format!("{}/api/auth/login", base_url))
        .header("X-Forwarded-For", "198.51.100.99")
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

    // 2. Peer A performs 10 failed unlock attempts
    for _ in 0..10 {
        let resp = client
            .post(format!("{}/api/auth/unlock", base_url))
            .header("Authorization", format!("Bearer {}", token))
            .header("X-Forwarded-For", "198.51.100.10")
            .json(&json!({ "password": "WrongPasswordAttempt!" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    // 11th attempt from Peer A must be rate-limited (429)
    let peer_a_limited = client
        .post(format!("{}/api/auth/unlock", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Forwarded-For", "198.51.100.10")
        .json(&json!({ "password": "WrongPasswordAttempt!" }))
        .send()
        .await
        .unwrap();
    assert_eq!(peer_a_limited.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);

    // Peer B attempting unlock MUST NOT be blocked by Peer A's rate limit
    let peer_b_unlock = client
        .post(format!("{}/api/auth/unlock", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Forwarded-For", "198.51.100.20")
        .json(&json!({ "password": "WrongPasswordAttempt!" }))
        .send()
        .await
        .unwrap();
    assert_eq!(peer_b_unlock.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 3. Create a share for share verification test
    let share_resp = client
        .post(format!("{}/api/shares", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "path": "/",
            "name": "Peer Rate Test Share",
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

    // Peer A performs 10 failed share verify attempts
    for _ in 0..10 {
        let resp = client
            .post(format!("{}/api/public/shares/{}/verify", base_url, share_token))
            .header("X-Forwarded-For", "198.51.100.10")
            .json(&json!({ "password": "WrongSharePass!" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    // 11th attempt from Peer A must be rate-limited (429)
    let peer_a_share_limited = client
        .post(format!("{}/api/public/shares/{}/verify", base_url, share_token))
        .header("X-Forwarded-For", "198.51.100.10")
        .json(&json!({ "password": "WrongSharePass!" }))
        .send()
        .await
        .unwrap();
    assert_eq!(peer_a_share_limited.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);

    // Peer B attempting share verify MUST NOT be blocked by Peer A's rate limit
    let peer_b_share = client
        .post(format!("{}/api/public/shares/{}/verify", base_url, share_token))
        .header("X-Forwarded-For", "198.51.100.20")
        .json(&json!({ "password": "WrongSharePass!" }))
        .send()
        .await
        .unwrap();
    assert_eq!(peer_b_share.status(), reqwest::StatusCode::UNAUTHORIZED);
}

