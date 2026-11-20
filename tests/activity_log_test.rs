//! The activity log (#193): changes are recorded with who made them and how they went, and only
//! admins can read or download the record.

use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

#[tokio::test]
async fn changes_are_recorded_for_admins() {
    let dir = tempdir().unwrap();
    let files = dir.path().join("files");
    let bob_home = dir.path().join("bob");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::create_dir_all(&bob_home).unwrap();
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".into();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.server.jwt_secret = "activity-log-test-secret-0123456789abcdef0123".into();
    config.storage.allow_entire_system = true;
    config.auth.default_admin_user = "admin".into();
    config.auth.default_admin_pass = "AdminSecretPass123!".into();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("uplugins").to_string_lossy().to_string();
    let (port, _) = start_background_server(config).await.unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let c = reqwest::Client::new();
    let login = |user: &'static str, pass: &'static str| {
        let (c, base) = (c.clone(), base.clone());
        async move {
            let v: Value = c.post(format!("{base}/api/auth/login")).json(&json!({ "username": user, "password": pass })).send().await.unwrap().json().await.unwrap();
            v["token"].as_str().unwrap_or_else(|| panic!("login {user}: {v}")).to_string()
        }
    };
    let admin = login("admin", "AdminSecretPass123!").await;
    let s = |p: std::path::PathBuf| p.to_string_lossy().to_string();
    let post = |token: String, path: &'static str, body: Value| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}{path}")).bearer_auth(token).json(&body).send().await.unwrap().status().as_u16() }
    };

    // An account that may only use its own folder
    assert_eq!(post(admin.clone(), "/api/auth/users", json!({ "username": "bob", "password": "BobSecretPass123!", "role": "user", "home_dir": s(bob_home.clone()), "allowed_roots": [s(bob_home.clone())], "must_change_password": false })).await, 200);
    let bob = login("bob", "BobSecretPass123!").await;

    // Changes by the admin
    assert_eq!(post(admin.clone(), "/api/fs/mkdir", json!({ "path": s(files.join("reports")) })).await, 200);
    assert_eq!(post(admin.clone(), "/api/fs/write", json!({ "path": s(files.join("reports/q1.txt")), "content": "numbers" })).await, 200);
    assert_eq!(post(admin.clone(), "/api/fs/rename", json!({ "from": s(files.join("reports/q1.txt")), "to": s(files.join("reports/q1-final.txt")) })).await, 200);
    assert_eq!(post(admin.clone(), "/api/fs/delete", json!({ "paths": [s(files.join("reports/q1-final.txt"))], "use_trash": false })).await, 200);
    // Bob tries to delete something outside his folder: refused, and recorded as a failure
    let st = post(bob.clone(), "/api/fs/delete", json!({ "paths": [s(files.join("reports"))], "use_trash": false })).await;
    assert!(matches!(st, 207 | 400 | 403), "bob delete status {st}");
    assert!(files.join("reports").is_dir());
    brum::tools::activity::flush();

    let list = |token: String, query: &'static str| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.get(format!("{base}/api/admin/activity{query}")).bearer_auth(token).send().await.unwrap() }
    };
    let v: Value = list(admin.clone(), "").await.json().await.unwrap();
    let entries = v["entries"].as_array().unwrap();
    let has = |user: &str, action: &str, target_end: &str, ok: bool| entries.iter().any(|e| e["user"] == user && e["action"] == action && e["target"].as_str().unwrap().ends_with(target_end) && e["ok"] == ok);
    assert!(has("admin", "account created", "bob", true), "{v}");
    assert!(has("admin", "new folder", "reports", true), "{v}");
    assert!(has("admin", "save", "q1.txt", true), "{v}");
    assert!(has("admin", "rename", "q1.txt", true), "{v}");
    assert!(has("admin", "delete", "q1-final.txt", true), "{v}");
    assert!(has("bob", "delete", "reports", false), "{v}");
    assert_eq!(v["retention_days"], 90);
    // Newest first
    assert_eq!(entries[0]["user"], "bob", "{v}");

    // Search and "only failures"
    let v: Value = list(admin.clone(), "?q=q1-final").await.json().await.unwrap();
    assert!(v["entries"].as_array().unwrap().iter().all(|e| e.to_string().contains("q1-final")), "{v}");
    let v: Value = list(admin.clone(), "?failed=true").await.json().await.unwrap();
    assert_eq!(v["total"], 1, "{v}");
    let v: Value = list(admin.clone(), "?user=bob").await.json().await.unwrap();
    assert!(v["entries"].as_array().unwrap().iter().all(|e| e["user"] == "bob"), "{v}");

    // Only admins may read it
    assert_eq!(list(bob.clone(), "").await.status().as_u16(), 403);
    let resp = c.get(format!("{base}/api/admin/activity/csv")).bearer_auth(&bob).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 403);

    // Download as a spreadsheet
    let resp = c.get(format!("{base}/api/admin/activity/csv?user=admin")).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert!(resp.headers()["content-type"].to_str().unwrap().starts_with("text/csv"));
    assert!(resp.headers()["content-disposition"].to_str().unwrap().contains("brum-activity-"));
    let csv = resp.text().await.unwrap();
    assert!(csv.starts_with("time (UTC),user,action,path,detail,result"));
    assert!(csv.contains("\"new folder\""), "{csv}");
    assert!(!csv.contains("\"bob\",\"delete\""), "filtered to admin: {csv}");
}
