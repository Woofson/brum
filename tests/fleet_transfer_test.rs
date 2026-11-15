//! Copies and moves between Fleet machines run on the servers (#190): straight from machine to
//! machine when they can reach each other, through the asking Brum when not, checked with
//! checksums, and retried where they stopped.

use brum::config::{AppConfig, FleetNodeConfig};
use brum::start_background_server;
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};

const SHARED_SECRET: &str = "fleet-transfer-test-secret-0123456789abcdef0123";

fn node_config(dir: &TempDir, loopback: bool) -> AppConfig {
    let mut config = AppConfig::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;
    config.server.database_path = dir.path().join("brum_test.db").to_string_lossy().to_string();
    config.server.enable_auth = true;
    config.server.jwt_secret = SHARED_SECRET.to_string();
    config.storage.allow_remote_loopback = loopback;
    config.storage.allow_entire_system = true;
    config.auth.default_admin_user = "admin".to_string();
    config.auth.default_admin_pass = "AdminSecretPass123!".to_string();
    config.plugins.directory = dir.path().join("plugins").to_string_lossy().to_string();
    config.plugins.user_directory = dir.path().join("user_plugins").to_string_lossy().to_string();
    config
}

fn node(id: &str, url: &str) -> FleetNodeConfig {
    FleetNodeConfig { id: id.to_string(), name: id.to_string(), url: url.to_string(), ..Default::default() }
}

async fn login(client: &reqwest::Client, base: &str) -> String {
    let body: Value = client.post(format!("{base}/api/auth/login"))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap().json().await.unwrap();
    body["token"].as_str().unwrap().to_string()
}

async fn start(client: &reqwest::Client, base: &str, token: &str, body: Value) -> (u16, Value) {
    let resp = client.post(format!("{base}/api/transfer/start")).bearer_auth(token).json(&body).send().await.unwrap();
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(json!({ "text": text })))
}

async fn wait(client: &reqwest::Client, base: &str, token: &str, id: &str) -> Value {
    for _ in 0..300 {
        let t: Value = client.get(format!("{base}/api/tasks/{id}")).bearer_auth(token).send().await.unwrap().json().await.unwrap();
        if t["status"] != "running" && t["status"] != "paused" {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("transfer did not finish");
}

#[tokio::test]
async fn copies_run_between_the_machines() {
    let files = tempdir().unwrap();
    let (a_dir, b_dir, c_dir, hub_dir) = (files.path().join("a"), files.path().join("b"), files.path().join("c"), files.path().join("hub"));
    for d in [&a_dir, &b_dir, &c_dir, &hub_dir] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::create_dir_all(a_dir.join("photos/empty")).unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(a_dir.join("photos/big.bin"), &big).unwrap();
    std::fs::write(a_dir.join("photos/note.txt"), b"hello").unwrap();
    std::fs::write(a_dir.join("move-me.txt"), b"moving").unwrap();

    // Two Fleet machines that may reach each other, and one that refuses loopback addresses
    // (stands in for a machine on another network)
    let (da, db, dc, dh) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
    let (pa, _) = start_background_server(node_config(&da, true)).await.unwrap();
    let (pb, _) = start_background_server(node_config(&db, true)).await.unwrap();
    let (pc, _) = start_background_server(node_config(&dc, false)).await.unwrap();
    // A machine with an older Brum: answers, but knows nothing of copy jobs
    let old = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let po = old.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(old, axum::Router::new()).await.unwrap() });

    let mut hub = node_config(&dh, true);
    for (id, port) in [("alpha", pa), ("bravo", pb), ("charlie", pc), ("oldie", po)] {
        hub.fleet.nodes.push(node(id, &format!("http://127.0.0.1:{port}")));
    }
    let (ph, _) = start_background_server(hub).await.unwrap();
    let base = format!("http://127.0.0.1:{ph}");
    let client = reqwest::Client::new();
    let token = login(&client, &base).await;
    let s = |p: &std::path::Path| p.to_string_lossy().to_string();

    // 1. alpha → bravo: bravo fetches the files straight from alpha
    let (st, r) = start(&client, &base, &token, json!({
        "action": "copy",
        "source": { "node": "alpha", "folder": s(&a_dir), "items": [{ "name": "photos" }] },
        "dest": { "node": "bravo", "folder": s(&b_dir) },
    })).await;
    assert_eq!(st, 200, "{r}");
    assert_eq!(r["mode"], "direct", "{r}");
    let t = wait(&client, &base, &token, r["task_id"].as_str().unwrap()).await;
    assert_eq!(t["status"], "completed", "{t}");
    assert_eq!(std::fs::read(b_dir.join("photos/big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(b_dir.join("photos/note.txt")).unwrap(), b"hello");
    assert!(b_dir.join("photos/empty").is_dir(), "empty folders are copied too");
    assert_eq!(t["verified_files"], 2, "{t}");
    assert!(t["log_entries"].to_string().contains("fetches the files straight from alpha"), "{t}");

    // 2. alpha → charlie: charlie cannot reach alpha, so the hub passes the files between them
    let (st, r) = start(&client, &base, &token, json!({
        "action": "copy",
        "source": { "node": "alpha", "folder": s(&a_dir), "items": [{ "name": "photos" }] },
        "dest": { "node": "charlie", "folder": s(&c_dir) },
    })).await;
    assert_eq!(st, 200, "{r}");
    assert_eq!(r["mode"], "relay", "{r}");
    let t = wait(&client, &base, &token, r["task_id"].as_str().unwrap()).await;
    assert_eq!(t["status"], "completed", "{t}");
    assert_eq!(std::fs::read(c_dir.join("photos/big.bin")).unwrap(), big);
    assert!(t["log_entries"].to_string().contains("cannot reach alpha directly"), "{t}");

    // 3. Move alpha → this machine: the original goes only after the copy is checked
    let (st, r) = start(&client, &base, &token, json!({
        "action": "move",
        "source": { "node": "alpha", "folder": s(&a_dir), "items": [{ "name": "move-me.txt" }] },
        "dest": { "node": "local", "folder": s(&hub_dir) },
    })).await;
    assert_eq!(st, 200, "{r}");
    let t = wait(&client, &base, &token, r["task_id"].as_str().unwrap()).await;
    assert_eq!(t["status"], "completed", "{t}");
    assert_eq!(std::fs::read(hub_dir.join("move-me.txt")).unwrap(), b"moving");
    assert!(!a_dir.join("move-me.txt").exists());

    // 4. Keep both: a same-named item gets a new name instead of replacing the old one
    let (st, r) = start(&client, &base, &token, json!({
        "action": "copy",
        "source": { "node": "local", "folder": s(&hub_dir), "items": [{ "name": "move-me.txt", "conflict": "rename" }] },
        "dest": { "node": "bravo", "folder": s(&b_dir) },
    })).await;
    assert_eq!(st, 200, "{r}");
    assert_eq!(wait(&client, &base, &token, r["task_id"].as_str().unwrap()).await["status"], "completed");
    std::fs::write(b_dir.join("existing.txt"), b"old").unwrap();
    std::fs::write(hub_dir.join("existing.txt"), b"new").unwrap();
    let (_, r) = start(&client, &base, &token, json!({
        "action": "copy",
        "source": { "node": "local", "folder": s(&hub_dir), "items": [{ "name": "existing.txt", "conflict": "rename" }] },
        "dest": { "node": "bravo", "folder": s(&b_dir) },
    })).await;
    assert_eq!(wait(&client, &base, &token, r["task_id"].as_str().unwrap()).await["status"], "completed");
    assert_eq!(std::fs::read(b_dir.join("existing.txt")).unwrap(), b"old");
    assert_eq!(std::fs::read(b_dir.join("existing (1).txt")).unwrap(), b"new");

    // 5. A machine with an older Brum: the page is told to copy the old way
    let (st, r) = start(&client, &base, &token, json!({
        "action": "copy",
        "source": { "node": "local", "folder": s(&hub_dir), "items": [{ "name": "existing.txt" }] },
        "dest": { "node": "oldie", "folder": "/tmp" },
    })).await;
    assert_eq!(st, 409, "{r}");
    assert_eq!(r["fallback"], true, "{r}");

    // 6. Names must stay inside the chosen folder
    let (st, _) = start(&client, &base, &token, json!({
        "action": "copy",
        "source": { "node": "alpha", "folder": s(&a_dir), "items": [{ "name": "../etc" }] },
        "dest": { "node": "bravo", "folder": s(&b_dir) },
    })).await;
    assert_eq!(st, 400);
}

#[cfg(unix)]
#[tokio::test]
async fn retry_copies_what_failed() {
    use std::os::unix::fs::PermissionsExt;
    let files = tempdir().unwrap();
    let (src, dst) = (files.path().join("src"), files.path().join("dst"));
    std::fs::create_dir_all(src.join("set")).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(src.join("set/ok.txt"), b"fine").unwrap();
    std::fs::write(src.join("set/locked.txt"), b"later").unwrap();
    std::fs::set_permissions(src.join("set/locked.txt"), std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(src.join("set/locked.txt")).is_ok() {
        return; // running as root: nothing can be locked
    }

    let (da, dh) = (tempdir().unwrap(), tempdir().unwrap());
    let (pa, _) = start_background_server(node_config(&da, true)).await.unwrap();
    let mut hub = node_config(&dh, true);
    hub.fleet.nodes.push(node("alpha", &format!("http://127.0.0.1:{pa}")));
    let (ph, _) = start_background_server(hub).await.unwrap();
    let base = format!("http://127.0.0.1:{ph}");
    let client = reqwest::Client::new();
    let token = login(&client, &base).await;

    let (st, r) = start(&client, &base, &token, json!({
        "action": "copy",
        "source": { "node": "alpha", "folder": src.to_string_lossy(), "items": [{ "name": "set" }] },
        "dest": { "node": "local", "folder": dst.to_string_lossy() },
    })).await;
    assert_eq!(st, 200, "{r}");
    let id = r["task_id"].as_str().unwrap().to_string();
    let t = wait(&client, &base, &token, &id).await;
    assert_eq!(t["status"], "failed", "{t}");
    assert!(t["error_message"].as_str().unwrap().contains("1 of 2"), "{t}");
    assert!(dst.join("set/ok.txt").exists());

    std::fs::set_permissions(src.join("set/locked.txt"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let resp = client.post(format!("{base}/api/transfer/{id}/retry")).bearer_auth(&token).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200, "{}", resp.text().await.unwrap());
    let t = wait(&client, &base, &token, &id).await;
    assert_eq!(t["status"], "completed", "{t}");
    assert_eq!(std::fs::read(dst.join("set/locked.txt")).unwrap(), b"later");
}

#[tokio::test]
async fn an_interrupted_copy_continues_on_the_other_machine() {
    let files = tempdir().unwrap();
    let (src, dst) = (files.path().join("src"), files.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    let data: Vec<u8> = (0..4_000_000u32).map(|i| (i % 241) as u8).collect();
    std::fs::write(src.join("video.mkv"), &data).unwrap();
    // Part of it arrived before the connection broke
    std::fs::write(dst.join(".video.mkv.brum-part"), &data[..1_500_000]).unwrap();

    let (db, dh) = (tempdir().unwrap(), tempdir().unwrap());
    let (pb, _) = start_background_server(node_config(&db, true)).await.unwrap();
    let mut hub = node_config(&dh, true);
    hub.fleet.nodes.push(node("bravo", &format!("http://127.0.0.1:{pb}")));
    let (ph, _) = start_background_server(hub).await.unwrap();
    let base = format!("http://127.0.0.1:{ph}");
    let client = reqwest::Client::new();
    let token = login(&client, &base).await;

    let (st, r) = start(&client, &base, &token, json!({
        "action": "copy",
        "source": { "node": "local", "folder": src.to_string_lossy(), "items": [{ "name": "video.mkv" }] },
        "dest": { "node": "bravo", "folder": dst.to_string_lossy() },
    })).await;
    assert_eq!(st, 200, "{r}");
    let t = wait(&client, &base, &token, r["task_id"].as_str().unwrap()).await;
    assert_eq!(t["status"], "completed", "{t}");
    assert!(t["log_entries"].to_string().contains("Continuing video.mkv"), "{t}");
    assert_eq!(std::fs::read(dst.join("video.mkv")).unwrap(), data);
    assert!(!dst.join(".video.mkv.brum-part").exists());
}
