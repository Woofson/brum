use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

struct Srv {
    base: String,
    dir: tempfile::TempDir,
    client: reqwest::Client,
    admin: String,
}

async fn start() -> Srv {
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
    let admin = login(&client, &base, "admin", "AdminSecretPass123!").await;
    Srv { base, dir, client, admin }
}

async fn login(client: &reqwest::Client, base: &str, user: &str, pass: &str) -> String {
    let body: Value = client.post(format!("{}/api/auth/login", base))
        .json(&json!({ "username": user, "password": pass }))
        .send().await.unwrap().json().await.unwrap();
    body["token"].as_str().unwrap_or_else(|| panic!("login failed for {}: {}", user, body)).to_string()
}

impl Srv {
    fn path(&self, rel: &str) -> String {
        self.dir.path().join(rel).to_string_lossy().to_string()
    }

    /// A Brum-only user limited to their own folder inside the test directory
    async fn add_user(&self, name: &str) -> String {
        let home = self.path(name);
        std::fs::create_dir_all(&home).unwrap();
        let resp = self.client.post(format!("{}/api/auth/users", self.base))
            .bearer_auth(&self.admin)
            .json(&json!({ "username": name, "password": "UserPass123!", "role": "user", "home_dir": home, "allowed_roots": [home] }))
            .send().await.unwrap();
        assert!(resp.status().is_success(), "creating {} failed: {}", name, resp.text().await.unwrap());
        login(&self.client, &self.base, name, "UserPass123!").await
    }

    async fn get(&self, token: &str, path: &str) -> (reqwest::StatusCode, Value) {
        let resp = self.client.get(format!("{}{}", self.base, path)).bearer_auth(token).send().await.unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn post(&self, token: &str, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
        let resp = self.client.post(format!("{}{}", self.base, path)).bearer_auth(token).json(&body).send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

fn task_ids(list: &Value) -> Vec<String> {
    list.as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn test_tasks_belong_to_their_user() {
    let srv = start().await;
    let bob = srv.add_user("bob").await;
    let carol = srv.add_user("carol").await;
    std::fs::write(srv.path("bob/a.txt"), "a").unwrap();
    std::fs::create_dir_all(srv.path("bob/copies")).unwrap();

    let (status, started) = srv.post(&bob, "/api/fs/copy", json!({ "sources": [srv.path("bob/a.txt")], "destination": srv.path("bob/copies") })).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{}", started);
    let id = started["task_id"].as_str().unwrap().to_string();

    let (_, bobs) = srv.get(&bob, "/api/tasks").await;
    assert!(task_ids(&bobs).contains(&id));
    let (_, admins) = srv.get(&srv.admin, "/api/tasks").await;
    assert!(task_ids(&admins).contains(&id), "admins see every task");
    let (_, carols) = srv.get(&carol, "/api/tasks").await;
    assert!(!task_ids(&carols).contains(&id), "carol can see bob's task");

    let (status, _) = srv.get(&carol, &format!("/api/tasks/{}", id)).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    let (status, _) = srv.post(&carol, &format!("/api/tasks/{}/cancel", id), json!({})).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    let (status, _) = srv.post(&carol, &format!("/api/tasks/{}/pause", id), json!({})).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    // Clearing finished tasks only clears your own
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    srv.post(&carol, "/api/tasks/clear-completed", json!({})).await;
    let (_, bobs) = srv.get(&bob, "/api/tasks").await;
    assert!(task_ids(&bobs).contains(&id), "carol cleared bob's finished task");
}

#[tokio::test]
async fn test_backup_profiles_belong_to_their_user() {
    let srv = start().await;
    let bob = srv.add_user("bob").await;
    let carol = srv.add_user("carol").await;
    std::fs::create_dir_all(srv.path("bob/src")).unwrap();
    std::fs::create_dir_all(srv.path("bob/dst")).unwrap();

    let profile = json!({
        "id": "", "name": "Bob's backup", "source_dir": srv.path("bob/src"), "dest_dir": srv.path("bob/dst"),
        "profile_mode": "contribute", "block_delta": true, "verify_checksum": false, "archive_dir": null,
        "retention_days": 30, "schedule_type": "manual", "schedule_interval_mins": 60, "schedule_time": null,
        "webhook_url": null, "enabled": true, "last_run": null, "last_status": null, "last_result": null,
        "created_at": 0, "owner": "carol"
    });
    let (status, saved) = srv.post(&bob, "/api/tools/sync/profiles", profile.clone()).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{}", saved);
    assert_eq!(saved["owner"], "bob", "the owner comes from the login, not the request");
    let id = saved["id"].as_str().unwrap().to_string();

    let (_, carols) = srv.get(&carol, "/api/tools/sync/profiles").await;
    assert!(carols.as_array().unwrap().is_empty());
    let (_, admins) = srv.get(&srv.admin, "/api/tools/sync/profiles").await;
    assert_eq!(admins.as_array().unwrap().len(), 1);

    for action in ["run", "toggle"] {
        let (status, _) = srv.post(&carol, &format!("/api/tools/sync/profiles/{}/{}", id, action), json!({})).await;
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND, "carol could {} bob's profile", action);
    }
    let resp = srv.client.delete(format!("{}/api/tools/sync/profiles/{}", srv.base, id)).bearer_auth(&carol).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let (status, _) = srv.get(&carol, &format!("/api/tools/sync/history?profile_id={}", id)).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    // Saving over someone else's profile by its id is refused
    let mut hijack = profile.clone();
    hijack["id"] = json!(id);
    hijack["source_dir"] = json!(srv.path("carol"));
    hijack["dest_dir"] = json!(srv.path("carol"));
    let (status, _) = srv.post(&carol, "/api/tools/sync/profiles", hijack).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    // Editing your own keeps you as owner
    let mut edit = profile.clone();
    edit["id"] = json!(id);
    edit["name"] = json!("Renamed");
    let (status, saved) = srv.post(&bob, "/api/tools/sync/profiles", edit).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(saved["owner"], "bob");
}

#[tokio::test]
async fn test_tags_only_listed_for_reachable_paths() {
    let srv = start().await;
    let bob = srv.add_user("bob").await;
    std::fs::write(srv.path("bob/mine.txt"), "x").unwrap();
    std::fs::write(srv.path("secret.txt"), "x").unwrap();

    let (status, _) = srv.post(&srv.admin, "/api/fs/tags/set", json!({ "paths": [srv.path("secret.txt")], "add_tags": ["salary"] })).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let (status, _) = srv.post(&bob, "/api/fs/tags/set", json!({ "paths": [srv.path("bob/mine.txt")], "add_tags": ["todo"] })).await;
    assert_eq!(status, reqwest::StatusCode::OK);

    let paths = |v: &Value| -> Vec<String> {
        v["tags"].as_array().unwrap().iter().map(|t| t["path"].as_str().unwrap().to_string()).collect()
    };
    let (_, bobs) = srv.get(&bob, "/api/fs/tags/all").await;
    assert_eq!(paths(&bobs), vec![srv.path("bob/mine.txt")]);
    let (_, bobs_plain) = srv.get(&bob, "/api/fs/tags").await;
    assert_eq!(paths(&bobs_plain), vec![srv.path("bob/mine.txt")]);
    let (_, admins) = srv.get(&srv.admin, "/api/fs/tags/all").await;
    assert_eq!(paths(&admins).len(), 2);
}

#[tokio::test]
async fn test_git_push_needs_own_system_account() {
    let srv = start().await;
    // A Brum-only account, with no system account behind it
    let bob = srv.add_user("brumonly_bob").await;
    let repo = srv.path("brumonly_bob");
    let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).output();
    if git(&["init"]).is_err() {
        return;
    }

    let (status, _) = srv.get(&bob, &format!("/api/git/status?path={}", repo)).await;
    assert_eq!(status, reqwest::StatusCode::OK, "status stays available");
    for action in ["push", "pull"] {
        let (status, body) = srv.post(&bob, &format!("/api/git/{}", action), json!({ "path": repo })).await;
        assert_eq!(status, reqwest::StatusCode::FORBIDDEN, "{} allowed: {}", action, body);
        assert!(body.as_str().unwrap_or("").contains("system account"));
    }

    // Admins pass the gate (the push itself fails here: there's no remote)
    let (status, _) = srv.post(&srv.admin, "/api/git/push", json!({ "path": repo })).await;
    assert_eq!(status, reqwest::StatusCode::OK);
}
