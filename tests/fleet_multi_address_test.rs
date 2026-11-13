//! One Fleet entry for a machine reachable under several addresses (a multi-boot box with one
//! hostname per system, #187): the hub uses the address that answers, and the page gets every
//! address and the start folder for each system.

use brum::config::{AppConfig, FleetNodeConfig};
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

const SHARED_SECRET: &str = "fleet-multi-address-test-secret-0123456789abcdef";

fn node_config(dir: &TempDir) -> AppConfig {
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.server.jwt_secret = SHARED_SECRET.to_string();
    config.storage.allow_remote_loopback = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = "AdminSecretPass123!".to_string();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    config
}

#[tokio::test]
async fn hub_uses_the_address_that_answers() {
    let dead_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };

    // The machine, currently up under its second address
    let node_dir = tempdir().unwrap();
    let (node_port, _) = start_background_server(node_config(&node_dir)).await.unwrap();
    let live = format!("http://127.0.0.1:{node_port}");
    let dead = format!("http://127.0.0.1:{dead_port}");

    let hub_dir = tempdir().unwrap();
    let mut hub = node_config(&hub_dir);
    hub.fleet.nodes.push(FleetNodeConfig {
        id: "griffon".to_string(),
        name: "Griffon".to_string(),
        urls: vec![dead.clone(), format!("{live}/")],
        start_path: Some("/srv".to_string()),
        start_paths: [("linux".to_string(), "/home/bolt".to_string()), ("windows".to_string(), r"C:\Users\bolt".to_string())]
            .into_iter()
            .collect(),
        ..Default::default()
    });
    let (hub_port, _) = start_background_server(hub).await.unwrap();
    let base = format!("http://127.0.0.1:{hub_port}");
    let client = reqwest::Client::new();

    let body: Value = client.post(format!("{base}/api/auth/login"))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap().json().await.unwrap();
    let token = body["token"].as_str().unwrap().to_string();

    // Ping finds the address that answers
    let ping: Value = client.get(format!("{base}/api/fleet/ping/griffon"))
        .bearer_auth(&token)
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(ping["status"], "online", "{ping}");
    assert_eq!(ping["endpoint_url"], live.as_str(), "{ping}");
    assert_eq!(ping["data"]["os"], std::env::consts::OS);

    // Requests through the hub reach the machine, signed in with the shared secret
    let resp = client.get(format!("{base}/api/fleet/proxy/griffon/api/storage/roots"))
        .bearer_auth(&token)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200, "{}", resp.text().await.unwrap());

    // The page gets every address and the folder per system
    let cfg: Value = client.get(format!("{base}/api/config"))
        .bearer_auth(&token)
        .send().await.unwrap().json().await.unwrap();
    let node = cfg["fleet"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "griffon").unwrap().clone();
    assert_eq!(node["urls"], json!([dead, live]));
    assert_eq!(node["url"], dead.as_str(), "url stays the first address for older pages");
    assert_eq!(node["start_paths"]["windows"], r"C:\Users\bolt");
    assert_eq!(node["start_paths"]["linux"], "/home/bolt");
}

#[test]
fn addresses_and_start_folders() {
    let n = FleetNodeConfig {
        id: "box".into(),
        name: "Box".into(),
        url: "http://griffon:3140/".into(),
        urls: vec!["http://wingriff:3140".into(), "HTTP://GRIFFON:3140".into(), "  ".into()],
        start_path: Some("/data".into()),
        start_paths: [("windows".to_string(), r"C:\Users\bolt".to_string())].into_iter().collect(),
        ..Default::default()
    };
    assert_eq!(n.addresses(), vec!["http://griffon:3140", "http://wingriff:3140"]);
    assert_eq!(n.start_path_for("windows").as_deref(), Some(r"C:\Users\bolt"));
    assert_eq!(n.start_path_for("Windows").as_deref(), Some(r"C:\Users\bolt"));
    assert_eq!(n.start_path_for("linux").as_deref(), Some("/data"), "falls back to start_path");

    let parsed: AppConfig = toml::from_str(
        "[[fleet.nodes]]\nid = \"g\"\nname = \"G\"\nurls = [\"http://a:3140\", \"http://b:3140\"]\n[fleet.nodes.start_paths]\nlinux = \"/home/bolt\"\nwindows = 'C:\\Users\\bolt'\n",
    )
    .unwrap();
    assert_eq!(parsed.fleet.nodes[0].addresses(), vec!["http://a:3140", "http://b:3140"]);
    assert_eq!(parsed.fleet.nodes[0].start_path_for("windows").as_deref(), Some(r"C:\Users\bolt"));
}
