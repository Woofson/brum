//! Requests must never reach files outside the folder they are meant for.

use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

struct Srv {
    base: String,
    port: u16,
    dir: tempfile::TempDir,
    client: reqwest::Client,
}

async fn start() -> Srv {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.storage.allow_entire_system = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = "AdminSecretPass123!".to_string();
    config.notes.notes_folder = dir.path().join("Notes").to_string_lossy().to_string();
    // The bundled Chewtoys from the source tree
    config.plugins.directory = concat!(env!("CARGO_MANIFEST_DIR"), "/plugins").to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    let (port, _) = start_background_server(config).await.unwrap();
    Srv { base: format!("http://127.0.0.1:{}", port), port, dir, client: reqwest::Client::new() }
}

async fn login(srv: &Srv, user: &str, pass: &str) -> String {
    let body: Value = srv.client.post(format!("{}/api/auth/login", srv.base))
        .json(&json!({ "username": user, "password": pass }))
        .send().await.unwrap().json().await.unwrap();
    body["token"].as_str().unwrap_or_else(|| panic!("login failed: {body}")).to_string()
}

async fn add_user(srv: &Srv, admin: &str, name: &str) -> String {
    let home = srv.dir.path().join(name);
    std::fs::create_dir_all(&home).unwrap();
    let home = home.to_string_lossy().to_string();
    let resp = srv.client.post(format!("{}/api/auth/users", srv.base))
        .bearer_auth(admin)
        .json(&json!({ "username": name, "password": "UserPass123!", "role": "user", "home_dir": home, "allowed_roots": [home] }))
        .send().await.unwrap();
    assert!(resp.status().is_success());
    login(srv, name, "UserPass123!").await
}

/// Sends the path exactly as written (HTTP clients would remove `..` first)
async fn raw_get(port: u16, path: &str, token: Option<&str>) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Connection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), s.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, text)
}

#[tokio::test]
async fn page_files_stay_inside_the_frontend_folder() {
    let srv = start().await;
    // Tests run from the source tree, where Brum reads `frontend/` from disk
    let (status, _) = raw_get(srv.port, "/index.html", None).await;
    assert_eq!(status, 200);
    for path in ["/../Cargo.toml", "/assets/../../Cargo.toml", "/assets/../../../../../../../../etc/hostname", "/%2e%2e/Cargo.toml", "/..%2fCargo.toml"] {
        let (_, body) = raw_get(srv.port, path, None).await;
        assert!(!body.contains("[package]") && !body.contains("name = \"brum\""), "{path} served a file outside frontend/");
        let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
        if !host.trim().is_empty() {
            assert!(!body.contains(host.trim()) || body.contains("<!DOCTYPE"), "{path} served /etc/hostname");
        }
    }
}

#[tokio::test]
async fn chewtoy_assets_stay_inside_the_plugin_folder() {
    let srv = start().await;
    let admin = login(&srv, "admin", "AdminSecretPass123!").await;
    let bob = add_user(&srv, &admin, "bob").await;
    std::fs::write(srv.dir.path().join("secret.txt"), "TOP-SECRET").unwrap();
    let plugin_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/tetrion"));
    let rel = pathdiff(plugin_dir, &srv.dir.path().join("secret.txt"));

    for token in [&admin, &bob] {
        let (ok, _) = raw_get(srv.port, "/api/plugins/tetrion/assets/main.js", Some(token)).await;
        assert_eq!(ok, 200, "normal asset must load");
        for path in [
            format!("/api/plugins/tetrion/assets/{}", rel.replace('/', "%2F")),
            format!("/api/plugins/tetrion/assets/{rel}"),
            "/api/plugins/tetrion/assets/..%2F..%2F..%2F..%2F..%2F..%2F..%2F..%2Fetc%2Fpasswd".to_string(),
        ] {
            let (_, body) = raw_get(srv.port, &path, Some(token)).await;
            assert!(!body.contains("TOP-SECRET") && !body.contains("root:"), "{path} escaped the plugin folder");
        }
    }
}

#[tokio::test]
async fn notes_export_writes_only_inside_the_notes_folder() {
    let srv = start().await;
    let admin = login(&srv, "admin", "AdminSecretPass123!").await;
    let bob = add_user(&srv, &admin, "bob").await;
    let resp = srv.client.post(format!("{}/api/notes", srv.base))
        .bearer_auth(&bob)
        .json(&json!({ "title": "escape", "content": "outside", "category": "../../escaped", "section": "../x" }))
        .send().await.unwrap();
    assert!(resp.status().is_success());
    let resp = srv.client.post(format!("{}/api/notes/migrate/export", srv.base)).bearer_auth(&bob).send().await.unwrap();
    assert!(resp.status().is_success());

    let notes = srv.dir.path().join("Notes");
    let mut found = Vec::new();
    for entry in walk(srv.dir.path()) {
        if entry.file_name().map(|n| n == "escape.md").unwrap_or(false) {
            found.push(entry);
        }
    }
    assert_eq!(found.len(), 1, "expected one exported note: {found:?}");
    assert!(found[0].starts_with(&notes), "note written outside the notes folder: {:?}", found[0]);
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { out.extend(walk(&p)); } else { out.push(p); }
        }
    }
    out
}

/// Relative path from `from` (a folder) to `to`
fn pathdiff(from: &std::path::Path, to: &std::path::Path) -> String {
    let from: Vec<_> = from.canonicalize().unwrap().components().map(|c| c.as_os_str().to_owned()).collect();
    let to: Vec<_> = to.canonicalize().unwrap().components().map(|c| c.as_os_str().to_owned()).collect();
    let common = from.iter().zip(to.iter()).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    parts.extend(to[common..].iter().map(|s| s.to_string_lossy().to_string()));
    parts.join("/")
}
