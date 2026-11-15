//! Joining the Fleet with a pairing code (#193), with real Brum programs: the main Brum makes a
//! code, a new machine joins with `brum join`, and afterwards each accepts the other's logins
//! and lists the other as a Fleet machine.

use serde_json::{json, Value};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const PASS: &str = "AdminSecretPass123!";

struct Brum(Child);

impl Drop for Brum {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn write_config(dir: &Path, port: u16) {
    std::fs::create_dir_all(dir).unwrap();
    let d = dir.display();
    std::fs::write(
        dir.join("config.toml"),
        format!(
            r#"[server]
host = "127.0.0.1"
port = {port}
enable_auth = true
database_path = "{d}/brum.db"

[auth]
default_admin_user = "admin"
default_admin_pass = "{PASS}"

[storage]
allow_entire_system = true
allow_remote_loopback = true

[logging]
file_enabled = false
console_enabled = false

[plugins]
directory = "{d}/plugins"
user_directory = "{d}/uplugins"
"#
        ),
    )
    .unwrap();
}

fn brum(dir: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_brum"));
    c.current_dir(dir).args(args).env_remove("BRUM_JWT_SECRET").env_remove("BRUM_JWT_SECRET_FILE").env("HOME", dir);
    c
}

async fn start(dir: &Path, port: u16) -> Brum {
    // Stopped (and waited for) when the guard is dropped, also if it never answers
    let guard = Brum(brum(dir, &["--server"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    let url = format!("http://127.0.0.1:{port}/api/health");
    for _ in 0..100 {
        if reqwest::get(&url).await.is_ok() {
            return guard;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("Brum in {} did not start", dir.display());
}

async fn login(port: u16) -> String {
    let v: Value = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/auth/login"))
        .json(&json!({ "username": "admin", "password": PASS }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["token"].as_str().unwrap_or_else(|| panic!("login failed: {v}")).to_string()
}

#[tokio::test]
async fn a_new_machine_joins_with_a_code() {
    let tmp = tempfile::tempdir().unwrap();
    let (main_dir, new_dir, third_dir) = (tmp.path().join("main"), tmp.path().join("new"), tmp.path().join("third"));
    let (main_port, new_port, third_port) = (free_port(), free_port(), free_port());
    write_config(&main_dir, main_port);
    write_config(&new_dir, new_port);
    write_config(&third_dir, third_port);
    let c = reqwest::Client::new();
    let main = format!("http://127.0.0.1:{main_port}");

    let _main = start(&main_dir, main_port).await;
    let admin = login(main_port).await;

    // Only admins make codes
    let code: Value = c.post(format!("{main}/api/fleet/pair/code")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    let code = code["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 9, "ABCD-EFGH: {code}");

    // A wrong code is refused
    let out = brum(&new_dir, &["join", &main, "WRNG-CODE", "--name", "newbox"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("wrong or has expired"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!new_dir.join("fleet-secret.key").exists());

    // The new machine joins
    let out = brum(&new_dir, &["join", &main, &code.to_lowercase(), "--name", "newbox"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Joined the Fleet of"), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    let secret_file = new_dir.join("fleet-secret.key");
    assert!(secret_file.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&secret_file).unwrap().permissions().mode() & 0o077, 0, "only Brum may read the secret");
    }

    // The code is used up, and the main Brum says who joined
    let st: Value = c.get(format!("{main}/api/fleet/pair/code/{code}")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert_eq!(st["state"], "joined", "{st}");
    assert_eq!(st["machine"], "newbox");
    let out = brum(&third_dir, &["join", &main, &code, "--name", "again"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("wrong or has expired"), "a code works once");

    // The main Brum lists the new machine, also as a Fleet machine
    let paired: Value = c.get(format!("{main}/api/fleet/paired")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    let newbox = paired.as_array().unwrap().iter().find(|m| m["name"] == "newbox").unwrap_or_else(|| panic!("{paired}")).clone();
    assert_eq!(newbox["role"], "joined");
    assert_eq!(newbox["paired_by"], "admin");
    let nodes: Value = c.get(format!("{main}/api/fleet/nodes")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert!(nodes.as_array().unwrap().iter().any(|n| n["id"] == newbox["id"]), "{nodes}");
    // ...but not to visitors who are not signed in
    let public: Value = c.get(format!("{main}/api/config")).send().await.unwrap().json().await.unwrap();
    assert!(!public["fleet"]["nodes"].to_string().contains("newbox"), "{public}");

    // After a restart the new machine accepts logins from the main Brum
    let _new = start(&new_dir, new_port).await;
    let resp = c.get(format!("http://127.0.0.1:{new_port}/api/storage/roots")).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200, "a login from the main Brum works on the new machine");
    // and lists the main Brum
    let paired: Value = c.get(format!("http://127.0.0.1:{new_port}/api/fleet/paired")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert!(paired.as_array().unwrap().iter().any(|m| m["role"] == "main" && m["urls"][0] == main.as_str()), "{paired}");

    // The activity log on the main Brum shows it
    tokio::time::sleep(Duration::from_millis(500)).await;
    let log: Value = c.get(format!("{main}/api/admin/activity?q=joined")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert!(log["entries"].to_string().contains("newbox"), "{log}");

    // A third machine joins from its own Settings
    let _third = start(&third_dir, third_port).await;
    let third_admin = login(third_port).await;
    let code: Value = c.post(format!("{main}/api/fleet/pair/code")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    let resp = c
        .post(format!("http://127.0.0.1:{third_port}/api/fleet/join"))
        .bearer_auth(&third_admin)
        .json(&json!({ "main_url": main, "code": code["code"] }))
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let v: Value = resp.json().await.unwrap();
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["restart"], true);
    assert!(v["warning"].as_str().unwrap_or("").contains("127.0.0.1"), "it listens on this machine only: {v}");
    assert!(third_dir.join("fleet-secret.key").is_file());

    // Removing a paired machine (admins only)
    let id = newbox["id"].as_str().unwrap();
    let resp = c.delete(format!("{main}/api/fleet/paired/{id}")).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let paired: Value = c.get(format!("{main}/api/fleet/paired")).bearer_auth(&admin).send().await.unwrap().json().await.unwrap();
    assert!(!paired.to_string().contains("newbox"), "{paired}");
}
