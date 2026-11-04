//! A Fleet node that does not answer must be reported as "cannot reach", so the web page can show
//! its "Can't reach" message instead of falling back to a folder from another machine (#158).

use brum::config::{AppConfig, FleetNodeConfig};
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

#[tokio::test]
async fn unreachable_fleet_node_answers_bad_gateway() {
    // A port that nobody listens on: bind, note the number, close it again
    let dead_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };

    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = "AdminSecretPass123!".to_string();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    config.fleet.nodes.push(FleetNodeConfig {
        id: "gone".to_string(),
        name: "Gone Node".to_string(),
        url: format!("http://127.0.0.1:{dead_port}"),
        token: None,
        start_path: Some("C:\\Users\\Bolt".to_string()),
        read_only: false,
        color_accent: None,
        tags: Vec::new(),
    });
    let (port, _) = start_background_server(config).await.unwrap();
    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();

    let body: Value = client.post(format!("{base}/api/auth/login"))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap().json().await.unwrap();
    let token = body["token"].as_str().unwrap().to_string();

    let resp = client.get(format!("{base}/api/fleet/proxy/gone/api/fs/list"))
        .query(&[("path", "C:\\Users\\Bolt")])
        .bearer_auth(&token)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 502);
    let text = resp.text().await.unwrap();
    // The web page looks for this wording to show its "Can't reach" message
    assert!(text.contains("Failed to reach remote fleet node"), "unexpected text: {text}");
    assert!(text.contains("Gone Node"), "the message should name the node: {text}");
}
