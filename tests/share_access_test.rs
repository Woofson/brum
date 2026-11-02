use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::json;
use tempfile::tempdir;

struct TestServer {
    base: String,
    token: String,
    dir: tempfile::TempDir,
    client: reqwest::Client,
}

async fn start() -> TestServer {
    let dir = tempdir().expect("Failed to create tempdir");
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.storage.allow_entire_system = true;
    config.server.enable_auth = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = "AdminSecretPass123!".to_string();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    let (port, _) = start_background_server(config).await.expect("Failed to start test server");

    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    let login: serde_json::Value = client
        .post(format!("{}/api/auth/login", base))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap()
        .json().await.unwrap();
    let token = login["token"].as_str().unwrap().to_string();
    TestServer { base, token, dir, client }
}

impl TestServer {
    async fn create_share(&self, path: &std::path::Path, extra: serde_json::Value) -> String {
        let mut body = json!({
            "path": path.to_string_lossy(),
            "name": "Test Share",
            "is_dir": path.is_dir(),
            "allow_view": true,
            "allow_download": true,
        });
        body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let resp = self.client
            .post(format!("{}/api/shares", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send().await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let share: serde_json::Value = resp.json().await.unwrap();
        share["token"].as_str().unwrap().to_string()
    }

    fn public(&self, share: &str, rest: &str) -> String {
        format!("{}/api/public/shares/{}{}", self.base, share, rest)
    }

    async fn upload(&self, share: &str, name: &str, content: &str) -> reqwest::StatusCode {
        let boundary = "brumtestboundary";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{n}\"\r\nContent-Type: text/plain\r\n\r\n{c}\r\n--{b}--\r\n",
            b = boundary, n = name, c = content
        );
        self.client
            .post(self.public(share, "/upload"))
            .header("Content-Type", format!("multipart/form-data; boundary={}", boundary))
            .body(body)
            .send().await.unwrap()
            .status()
    }
}

/// The `name=value` part of a Set-Cookie header
fn cookie_pair(resp: &reqwest::Response) -> String {
    let raw = resp.headers().get("set-cookie").expect("no share pass cookie").to_str().unwrap();
    raw.split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn test_share_password_travels_as_cookie_not_in_url() {
    let srv = start().await;
    let folder = srv.dir.path().join("shared");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("secret.txt"), "hello").unwrap();
    let share = srv.create_share(&folder, json!({ "password": "SharePass123!" })).await;

    // Before unlocking, the folder's contents stay hidden
    let meta: serde_json::Value = srv.client.get(srv.public(&share, "")).send().await.unwrap().json().await.unwrap();
    assert_eq!(meta["unlocked"], false);
    assert_eq!(meta["files"].as_array().unwrap().len(), 0, "file names leaked before the password was given");

    // A password in the address is no longer accepted, so it can't be guessed there
    let url_pass = srv.client.get(srv.public(&share, "/download?file=secret.txt&password=SharePass123!")).send().await.unwrap();
    assert_eq!(url_pass.status(), reqwest::StatusCode::UNAUTHORIZED);

    // A made-up pass is refused
    let forged = srv.client.get(srv.public(&share, "/download?file=secret.txt"))
        .header("Cookie", format!("brum_share_pass={}.{}", i64::MAX, "00".repeat(32)))
        .send().await.unwrap();
    assert_eq!(forged.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The right password hands out a pass limited to this share
    let verify = srv.client.post(srv.public(&share, "/verify"))
        .json(&json!({ "password": "SharePass123!" }))
        .send().await.unwrap();
    assert_eq!(verify.status(), reqwest::StatusCode::OK);
    let raw_cookie = verify.headers().get("set-cookie").unwrap().to_str().unwrap().to_string();
    assert!(raw_cookie.contains("HttpOnly"), "share pass must be hidden from page scripts");
    assert!(raw_cookie.contains(&format!("Path=/api/public/shares/{}", share)));
    assert!(!raw_cookie.contains("SharePass123"), "the pass must not contain the password");
    let cookie = cookie_pair(&verify);

    let download = srv.client.get(srv.public(&share, "/download?file=secret.txt"))
        .header("Cookie", &cookie)
        .send().await.unwrap();
    assert_eq!(download.status(), reqwest::StatusCode::OK);
    assert_eq!(download.text().await.unwrap(), "hello");

    let meta: serde_json::Value = srv.client.get(srv.public(&share, "")).header("Cookie", &cookie)
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(meta["unlocked"], true);
    assert_eq!(meta["files"].as_array().unwrap().len(), 1);

    // The pass belongs to one share only
    let other = srv.create_share(&folder, json!({ "password": "SharePass123!" })).await;
    let cross = srv.client.get(srv.public(&other, "/download?file=secret.txt"))
        .header("Cookie", &cookie)
        .send().await.unwrap();
    assert_eq!(cross.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_share_email_gate_uses_signed_cookie() {
    let srv = start().await;
    let file = srv.dir.path().join("report.txt");
    std::fs::write(&file, "report").unwrap();
    let share = srv.create_share(&file, json!({ "require_email": true, "allowed_emails": "guest@example.com" })).await;

    let url_email = srv.client.get(srv.public(&share, "/download?email=guest@example.com")).send().await.unwrap();
    assert_eq!(url_email.status(), reqwest::StatusCode::FORBIDDEN);

    let meta: serde_json::Value = srv.client.get(srv.public(&share, "")).send().await.unwrap().json().await.unwrap();
    assert_eq!(meta["needs_email"], true);

    let verify = srv.client.post(srv.public(&share, "/verify-email"))
        .json(&json!({ "email": "guest@example.com" }))
        .send().await.unwrap();
    assert_eq!(verify.status(), reqwest::StatusCode::OK);
    let cookie = cookie_pair(&verify);

    let download = srv.client.get(srv.public(&share, "/download")).header("Cookie", &cookie).send().await.unwrap();
    assert_eq!(download.status(), reqwest::StatusCode::OK);

    // Changing the email inside the signed pass breaks it
    let value = cookie.split_once('=').unwrap().1;
    let mut parts: Vec<&str> = value.split('.').collect();
    let other_email = hex::encode("intruder@example.com");
    parts[1] = &other_email;
    let tampered = format!("brum_share_email={}", parts.join("."));
    let denied = srv.client.get(srv.public(&share, "/download")).header("Cookie", tampered).send().await.unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_share_upload_never_overwrites_and_drops_leading_dots() {
    let srv = start().await;
    let folder = srv.dir.path().join("dropbox");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), "original").unwrap();
    let share = srv.create_share(&folder, json!({ "allow_upload": true })).await;

    assert_eq!(srv.upload(&share, "notes.txt", "replacement").await, reqwest::StatusCode::OK);
    assert_eq!(srv.upload(&share, "notes.txt", "second").await, reqwest::StatusCode::OK);
    assert_eq!(std::fs::read_to_string(folder.join("notes.txt")).unwrap(), "original");
    assert_eq!(std::fs::read_to_string(folder.join("notes (1).txt")).unwrap(), "replacement");
    assert_eq!(std::fs::read_to_string(folder.join("notes (2).txt")).unwrap(), "second");

    assert_eq!(srv.upload(&share, ".bashrc", "evil").await, reqwest::StatusCode::OK);
    assert!(!folder.join(".bashrc").exists(), "hidden start-up file was created");
    assert_eq!(std::fs::read_to_string(folder.join("bashrc")).unwrap(), "evil");

    assert_eq!(srv.upload(&share, "...", "dots").await, reqwest::StatusCode::OK);
    assert!(folder.join("upload.bin").exists());
}

#[tokio::test]
async fn test_share_page_strict_policy_and_token_check() {
    let srv = start().await;
    let file = srv.dir.path().join("photo.txt");
    std::fs::write(&file, "x").unwrap();
    let share = srv.create_share(&file, json!({})).await;

    let page = srv.client.get(format!("{}/share/{}", srv.base, share)).send().await.unwrap();
    assert_eq!(page.status(), reqwest::StatusCode::OK);
    let csp = page.headers().get("content-security-policy").unwrap().to_str().unwrap().to_string();
    assert!(csp.contains("script-src 'nonce-"), "share page lacks a strict script policy: {}", csp);
    assert!(!csp.contains("unsafe-eval"));
    let html = page.text().await.unwrap();
    let nonce = csp.split("'nonce-").nth(1).unwrap().split('\'').next().unwrap();
    assert!(html.contains(&format!("<script nonce=\"{}\">", nonce)));
    for handler in ["onclick=", "onkeydown=", "oninput=", "onchange=", "onerror=", "oncontextmenu="] {
        assert!(!html.contains(handler), "share page still has an inline '{}' handler", handler);
    }

    // Anything that isn't a plain share token gets no page
    let crafted = srv.client.get(format!("{}/share/abc%22%3Balert(1)%2F%2F", srv.base)).send().await.unwrap();
    assert_eq!(crafted.status(), reqwest::StatusCode::NOT_FOUND);
    assert!(!crafted.text().await.unwrap().contains("alert(1)"));
}
