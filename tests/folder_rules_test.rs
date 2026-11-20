//! Folder rules (#193): who may read, write or not touch which folders on this machine.

use brum::config::{AppConfig, FleetNodeConfig};
use brum::start_background_server;
use brum::tools::access::{Access, Rule};
use serde_json::{json, Value};
use tempfile::tempdir;

fn rule(path: &std::path::Path, access: Access, users: &[&str], roles: &[&str]) -> Rule {
    Rule { id: String::new(), path: path.to_string_lossy().to_string(), access, users: users.iter().map(|s| s.to_string()).collect(), roles: roles.iter().map(|s| s.to_string()).collect(), note: String::new() }
}

#[tokio::test]
async fn folder_rules_limit_and_grant() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("files");
    let (shared, team, private, bob_home) = (f.join("shared"), f.join("shared/team"), f.join("shared/private"), f.join("bob"));
    for d in [&team, &private, &bob_home] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(shared.join("doc.txt"), "doc").unwrap();
    std::fs::write(private.join("secret.txt"), "secret").unwrap();

    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".into();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.server.jwt_secret = "folder-rules-test-secret-0123456789abcdef0123".into();
    config.storage.allow_entire_system = true;
    config.auth.default_admin_user = "admin".into();
    config.auth.default_admin_pass = "AdminSecretPass123!".into();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("uplugins").to_string_lossy().to_string();
    // From config.toml: everyone in role "user" may read the shared folder; nobody but admins sees private
    config.access.rules = vec![rule(&shared, Access::Read, &[], &["user"]), rule(&private, Access::None, &[], &[])];
    config.fleet.nodes.push(FleetNodeConfig { id: "nas".into(), name: "NAS".into(), url: "http://192.168.1.50:3140".into(), ..Default::default() });
    let (port, _) = start_background_server(config).await.unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let c = reqwest::Client::new();
    let s = |p: &std::path::Path| p.to_string_lossy().to_string();
    let login = |user: &'static str, pass: &'static str| {
        let (c, base) = (c.clone(), base.clone());
        async move {
            let v: Value = c.post(format!("{base}/api/auth/login")).json(&json!({ "username": user, "password": pass })).send().await.unwrap().json().await.unwrap();
            v["token"].as_str().unwrap_or_else(|| panic!("{user}: {v}")).to_string()
        }
    };
    let admin = login("admin", "AdminSecretPass123!").await;
    // bob may only use his own folder; ann may use everything
    for (name, roots) in [("bob", json!([s(&bob_home)])), ("ann", json!(["*"]))] {
        let st = c.post(format!("{base}/api/auth/users")).bearer_auth(&admin).json(&json!({ "username": name, "password": "UserSecretPass123!", "role": "user", "home_dir": s(&bob_home), "allowed_roots": roots, "must_change_password": false })).send().await.unwrap().status();
        assert_eq!(st.as_u16(), 200);
    }
    let bob = login("bob", "UserSecretPass123!").await;
    let ann = login("ann", "UserSecretPass123!").await;

    let get = |token: &str, path: &str, q: &std::path::Path| {
        let (c, url, token) = (c.clone(), format!("{base}{path}?path={}", urlencode(&s(q))), token.to_string());
        async move { c.get(url).bearer_auth(token).send().await.unwrap().status().as_u16() }
    };
    let post = |token: &str, path: &str, body: Value| {
        let (c, url, token) = (c.clone(), format!("{base}{path}"), token.to_string());
        async move { c.post(url).bearer_auth(token).json(&body).send().await.unwrap().status().as_u16() }
    };

    // A rule grants bob reading outside his own folder, but not writing
    assert_eq!(get(&bob, "/api/fs/read", &shared.join("doc.txt")).await, 200);
    assert_eq!(post(&bob, "/api/fs/write", json!({ "path": s(&shared.join("new.txt")), "content": "x" })).await, 403);
    // Without a rule, his account decides
    assert_eq!(get(&bob, "/api/fs/list", &f).await, 403);

    // A rule saved in the Admin Panel lets bob write in team
    let r: Value = c.post(format!("{base}/api/admin/access-rules")).bearer_auth(&admin).json(&json!({ "path": s(&team), "access": "write", "users": ["bob"] })).send().await.unwrap().json().await.unwrap();
    let rule_id = r["id"].as_str().unwrap().to_string();
    assert_eq!(r["who"], "bob");
    assert_eq!(post(&bob, "/api/fs/write", json!({ "path": s(&team.join("plan.txt")), "content": "plan" })).await, 200);
    // Only admins manage rules
    assert_eq!(c.get(format!("{base}/api/admin/access-rules")).bearer_auth(&bob).send().await.unwrap().status().as_u16(), 403);
    let list: Value = c.get(format!("{base}/api/admin/access-rules")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert_eq!(list["rules"].as_array().unwrap().len(), 3);
    assert_eq!(list["rules"][0]["from_config"], true);

    // No access wins for private, even for ann who may use everything
    assert_eq!(get(&bob, "/api/fs/list", &private).await, 403);
    assert_eq!(get(&ann, "/api/fs/read", &private.join("secret.txt")).await, 403);
    // ...and a folder holding it cannot be copied, downloaded or deleted as a whole
    assert_eq!(post(&ann, "/api/fs/copy", json!({ "sources": [s(&shared)], "destination": s(&bob_home) })).await, 403);
    assert_eq!(get(&ann, "/api/fs/download", &shared).await, 403);
    assert!(matches!(post(&ann, "/api/fs/delete", json!({ "paths": [s(&shared)], "use_trash": false })).await, 207 | 403));
    assert!(private.join("secret.txt").exists());
    // Search leaves it out
    let found: Value = c.post(format!("{base}/api/tools/search")).bearer_auth(&ann).json(&json!({ "path": s(&f), "name_pattern": "*.txt" })).send().await.unwrap().json().await.unwrap();
    let names: Vec<&str> = found.as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"doc.txt") && !names.contains(&"secret.txt"), "{names:?}");
    // Admins are not limited by rules
    assert_eq!(get(&admin, "/api/fs/read", &private.join("secret.txt")).await, 200);

    // Removing the rule takes effect at once
    assert_eq!(c.delete(format!("{base}/api/admin/access-rules/{rule_id}")).bearer_auth(&admin).send().await.unwrap().status().as_u16(), 200);
    assert_eq!(post(&bob, "/api/fs/write", json!({ "path": s(&team.join("plan2.txt")), "content": "x" })).await, 403);

    // Visitors who are not signed in do not see the Fleet's machines
    let public: Value = c.get(format!("{base}/api/config")).send().await.unwrap().json().await.unwrap();
    assert_eq!(public["fleet"]["nodes"], json!([]));
    assert_eq!(public["fleet"]["nodes_hidden"], true);
    let signed_in: Value = c.get(format!("{base}/api/config")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert_eq!(signed_in["fleet"]["nodes"][0]["id"], "nas");
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
