//! #195 step 5: Fleet machines written by hand in config.toml move into Brum's list with
//! "Move them into Brum" (tokens encrypted), and admins add, change and remove such machines
//! in Manage Fleet without a restart. Its own test program: the machine list is one per process.

use brum::config::{AppConfig, ConfigManager};
use brum::start_background_server;
use serde_json::{json, Value};

const ADMIN_PASS: &str = "AdminSecretPass123!";

const FILE: &str = r#"
[server]
server_name = "Main"

[[fleet.nodes]]
id = "nas"
name = "NAS"
url = "http://192.0.2.10:3140"
urls = ["http://nas.lan:3140"]
token = "nas-api-token-0123456789"
read_only = true
color_accent = "emerald"
tags = ["servers"]
[fleet.nodes.start_paths]
linux = "/mnt/storage"

[[fleet.nodes]]
id = "broken"
name = "No address"
token = "kept-token-for-broken"
"#;

async fn login(c: &reqwest::Client, port: u16, user: &str, pass: &str) -> String {
    let v: Value = c.post(format!("http://127.0.0.1:{port}/api/auth/login")).json(&json!({ "username": user, "password": pass })).send().await.unwrap().json().await.unwrap();
    v["token"].as_str().unwrap_or_else(|| panic!("{v}")).to_string()
}

#[tokio::test]
async fn hand_written_machines_move_into_brum_and_are_managed_there() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    std::fs::write(&file, FILE).unwrap();
    let mut c = ConfigManager::parse_config_str(FILE).unwrap();
    c.server.config_file = Some(file.clone());
    c.server.host = "127.0.0.1".into();
    c.server.port = 0;
    c.server.database_path = dir.path().join("brum.db").to_string_lossy().to_string();
    c.server.jwt_secret = "fleet-machines-test-secret-0123456789abcdef".into();
    c.auth.default_admin_user = "admin".into();
    c.auth.default_admin_pass = ADMIN_PASS.into();
    c.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    c.plugins.user_directory = dir.path().join("uplugins").to_string_lossy().to_string();
    let cfg: AppConfig = c;
    let (port, _) = start_background_server(cfg).await.unwrap();
    let http = reqwest::Client::new();
    let admin = login(&http, port, "admin", ADMIN_PASS).await;
    let url = |p: &str| format!("http://127.0.0.1:{port}{p}");
    let nodes = |t: String| {
        let (http, u) = (http.clone(), url("/api/fleet/nodes"));
        async move { http.get(u).bearer_auth(t).send().await.unwrap().json::<Value>().await.unwrap() }
    };
    let before = nodes(admin.clone()).await;
    let nas_before = before.as_array().unwrap().iter().find(|n| n["id"] == "nas").cloned().unwrap();

    // The settings page counts the machine as something to move
    let s: Value = http.get(url("/api/admin/server-settings")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert!(s["movable"].as_u64().unwrap() >= 2, "{}", s["movable"]);

    let v: Value = http.post(url("/api/admin/server-settings/move-from-file")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["machines"], json!(["NAS"]), "{v}");
    assert!(v["stayed"].as_array().unwrap().iter().any(|x| x["machine"] == "No address"), "{v}");

    // The file keeps the machine that could not move, token and all; the moved token is gone from it
    let now = std::fs::read_to_string(&file).unwrap();
    assert!(!now.contains("nas-api-token") && !now.contains("192.0.2.10"), "{now}");
    assert!(now.contains("kept-token-for-broken") && now.contains("No address"), "{now}");
    // Not readable in the database file either
    let conn = rusqlite::Connection::open(dir.path().join("brum.db")).unwrap();
    let stored: String = conn.query_row("SELECT token FROM fleet_paired WHERE id = 'nas'", [], |r| r.get(0)).unwrap();
    assert!(!stored.is_empty() && !stored.contains("nas-api-token"), "{stored}");

    // The machine is listed exactly as before, and the browser never sees the token
    let after = nodes(admin.clone()).await;
    let nas_after = after.as_array().unwrap().iter().find(|n| n["id"] == "nas").cloned().unwrap();
    assert_eq!(nas_after, nas_before);
    let paired: Value = http.get(url("/api/fleet/paired")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    let nas = paired.as_array().unwrap().iter().find(|m| m["id"] == "nas").cloned().unwrap();
    assert_eq!((nas["role"].as_str(), nas["has_token"].as_bool()), (Some("manual"), Some(true)));
    assert!(!paired.to_string().contains("nas-api-token"));

    // Change it: the token is kept when the page sends the mask, removed when it sends nothing
    let put = |body: Value| {
        let (http, u, t) = (http.clone(), url("/api/fleet/machines/nas"), admin.clone());
        async move { http.put(u).bearer_auth(t).json(&body).send().await.unwrap().status().as_u16() }
    };
    assert_eq!(put(json!({ "name": "Big NAS", "urls": ["http://192.0.2.10:3140"], "token": "********", "details": { "tags": ["servers"] } })).await, 200);
    let has_token = |p: Value| p.as_array().unwrap().iter().find(|m| m["id"] == "nas").map(|m| (m["name"].clone(), m["has_token"].clone())).unwrap();
    let p: Value = http.get(url("/api/fleet/paired")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert_eq!(has_token(p), (json!("Big NAS"), json!(true)));
    assert_eq!(put(json!({ "name": "Big NAS", "urls": ["http://192.0.2.10:3140"], "token": "" })).await, 200);
    let p: Value = http.get(url("/api/fleet/paired")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert_eq!(has_token(p), (json!("Big NAS"), json!(false)));

    // Add one by address; bad and taken addresses are refused
    let post = |body: Value, t: String| {
        let (http, u) = (http.clone(), url("/api/fleet/machines"));
        async move { http.post(u).bearer_auth(t).json(&body).send().await.unwrap().status().as_u16() }
    };
    assert_eq!(post(json!({ "name": "Media PC", "urls": ["http://192.0.2.20:3140/"] }), admin.clone()).await, 200);
    assert_eq!(post(json!({ "name": "Again", "urls": ["http://192.0.2.20:3140"] }), admin.clone()).await, 409);
    assert_eq!(post(json!({ "name": "Bad", "urls": ["nas:3140"] }), admin.clone()).await, 400);
    assert_eq!(post(json!({ "name": "", "urls": [] }), admin.clone()).await, 400);
    let listed = nodes(admin.clone()).await;
    assert!(listed.as_array().unwrap().iter().any(|n| n["name"] == "Media PC" && n["url"] == "http://192.0.2.20:3140"), "{listed}");

    // Only admins
    let st = http.post(url("/api/auth/users")).bearer_auth(&admin).json(&json!({ "username": "dana", "password": "DanaSecretPass123!", "role": "user", "home_dir": dir.path() })).send().await.unwrap().status();
    assert_eq!(st.as_u16(), 200);
    let dana = login(&http, port, "dana", "DanaSecretPass123!").await;
    assert_eq!(post(json!({ "name": "Sneaky", "urls": ["http://192.0.2.30:3140"] }), dana.clone()).await, 403);
    assert_eq!(http.put(url("/api/fleet/machines/nas")).bearer_auth(&dana).json(&json!({ "name": "x", "urls": ["http://192.0.2.10:3140"] })).send().await.unwrap().status().as_u16(), 403);

    // Remove
    assert_eq!(http.delete(url("/api/fleet/paired/nas")).bearer_auth(&admin).send().await.unwrap().status().as_u16(), 200);
    let gone = nodes(admin).await;
    assert!(!gone.as_array().unwrap().iter().any(|n| n["id"] == "nas"), "{gone}");
}
