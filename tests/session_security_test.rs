use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::json;
use tempfile::tempdir;

const ADMIN_PASS: &str = "AdminSecretPass123!";

async fn start() -> (String, tempfile::TempDir) {
    let dir = tempdir().expect("Failed to create tempdir");
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.storage.allow_entire_system = true;
    config.server.enable_auth = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = ADMIN_PASS.to_string();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    let (port, _) = start_background_server(config).await.expect("Failed to start test server");
    (format!("http://127.0.0.1:{}", port), dir)
}

async fn login(client: &reqwest::Client, base: &str, password: &str) -> String {
    let resp = client
        .post(format!("{}/api/auth/login", base))
        .json(&json!({ "username": "admin", "password": password }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    body["token"].as_str().unwrap().to_string()
}

async fn me_status(client: &reqwest::Client, base: &str, token: &str) -> reqwest::StatusCode {
    client.get(format!("{}/api/tasks", base))
        .header("Authorization", format!("Bearer {}", token))
        .send().await.unwrap()
        .status()
}

#[tokio::test]
async fn test_logout_ends_session_on_server() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let kept = login(&client, &base, ADMIN_PASS).await;
    let token = login(&client, &base, ADMIN_PASS).await;
    assert_eq!(me_status(&client, &base, &token).await, reqwest::StatusCode::OK);

    let out = client.post(format!("{}/api/auth/logout", base))
        .header("Authorization", format!("Bearer {}", token))
        .send().await.unwrap();
    assert_eq!(out.status(), reqwest::StatusCode::OK);

    assert_eq!(me_status(&client, &base, &token).await, reqwest::StatusCode::UNAUTHORIZED, "token still works after logout");
    // Logging out in one browser leaves the other logins alone
    assert_eq!(me_status(&client, &base, &kept).await, reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_password_change_ends_other_sessions() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let other_device = login(&client, &base, ADMIN_PASS).await;
    let this_device = login(&client, &base, ADMIN_PASS).await;

    let resp = client.post(format!("{}/api/auth/profile", base))
        .header("Authorization", format!("Bearer {}", this_device))
        .json(&json!({ "new_password": "BrandNewPass456!" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let fresh = body["token"].as_str().expect("password change must hand this browser a fresh login").to_string();

    assert_eq!(me_status(&client, &base, &other_device).await, reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(me_status(&client, &base, &this_device).await, reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(me_status(&client, &base, &fresh).await, reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_login_token_not_accepted_from_address() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let token = login(&client, &base, ADMIN_PASS).await;

    let in_url = client.get(format!("{}/api/tasks?token={}", base, token)).send().await.unwrap();
    assert_eq!(in_url.status(), reqwest::StatusCode::UNAUTHORIZED);

    let in_cookie = client.get(format!("{}/api/tasks", base))
        .header("Cookie", format!("cd_token={}", token))
        .send().await.unwrap();
    assert_eq!(in_cookie.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_cross_site_requests_blocked() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let token = login(&client, &base, ADMIN_PASS).await;
    let host = base.trim_start_matches("http://").to_string();
    let url = format!("{}/api/tasks/clear-completed", base);

    let post = |origin: &str, site: Option<&str>| {
        let mut req = client.post(&url)
            .header("Cookie", format!("cd_token={}", token))
            .header("Origin", origin);
        if let Some(s) = site {
            req = req.header("Sec-Fetch-Site", s);
        }
        req.send()
    };

    // Another site, or a sibling service under the same domain, riding on the login cookie
    assert_eq!(post("https://evil.example", Some("cross-site")).await.unwrap().status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(post("https://jellyfin.home.example", Some("same-site")).await.unwrap().status(), reqwest::StatusCode::FORBIDDEN);
    // An older browser without Sec-Fetch-Site still sends Origin
    assert_eq!(post("https://evil.example", None).await.unwrap().status(), reqwest::StatusCode::FORBIDDEN);

    // Brum's own page
    assert_eq!(post(&format!("http://{}", host), Some("same-origin")).await.unwrap().status(), reqwest::StatusCode::OK);
    assert_eq!(post(&format!("http://{}", host), None).await.unwrap().status(), reqwest::StatusCode::OK);

    // Scripts and Fleet send their own token, which another site can't attach
    let scripted = client.post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Origin", "https://other-node.example")
        .header("Sec-Fetch-Site", "cross-site")
        .send().await.unwrap();
    assert_eq!(scripted.status(), reqwest::StatusCode::OK);

    // Reading stays possible; only changes are guarded
    let read = client.get(format!("{}/api/tasks", base))
        .header("Cookie", format!("cd_token={}", token))
        .header("Sec-Fetch-Site", "cross-site")
        .send().await.unwrap();
    assert_eq!(read.status(), reqwest::StatusCode::OK);
}

#[test]
fn test_generated_admin_password_note_beside_database() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("brum.db");
    brum::auth::AuthManager::new(&db.to_string_lossy(), "jwt-secret-for-test-0123456789abcdef", 8, "builtin", "login", "admin", "brum").unwrap();
    let note = dir.path().join("initial_admin_credentials.txt");
    assert!(note.is_file(), "credentials note should sit next to the database");
    let text = std::fs::read_to_string(&note).unwrap();
    assert!(text.contains("Username  : admin"));

    brum::logging::cleanup_initial_admin_credentials(&db.to_string_lossy());
    assert!(!note.exists());
}

fn login_off_config(host: &str, proxies: Vec<String>) -> (AppConfig, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.server.host = host.to_string();
    config.server.port = 0;
    config.server.enable_auth = false;
    config.server.trusted_proxies = proxies;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.auth.default_admin_pass = ADMIN_PASS.to_string();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    (config, dir)
}

#[tokio::test]
async fn test_login_off_refused_on_network_address() {
    let (config, _dir) = login_off_config("0.0.0.0", Vec::new());
    let err = start_background_server(config).await.err().expect("login off on 0.0.0.0 must not start");
    assert!(err.to_string().contains("Refusing to start"), "{}", err);

    // On this machine only, or behind a login-handling proxy, it may start
    assert!(login_off_config("127.0.0.1", Vec::new()).0.check_login_off_is_safe().is_ok());
    assert!(login_off_config("0.0.0.0", vec!["10.0.0.5".to_string()]).0.check_login_off_is_safe().is_ok());
}

#[tokio::test]
async fn test_main_page_policy_has_no_eval() {
    let (base, _dir) = start().await;
    let resp = reqwest::get(format!("{}/", base)).await.unwrap();
    let csp = resp.headers().get("content-security-policy").unwrap().to_str().unwrap().to_string();
    assert!(!csp.contains("unsafe-eval"), "{}", csp);
    assert!(csp.contains("object-src 'none'"), "{}", csp);
    assert!(csp.contains("base-uri 'self'"), "{}", csp);
}
