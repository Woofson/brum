//! Compressed disk and CD images open like the uncompressed ones (#159), and the admin
//! controls how long unpacked copies are kept.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use brum::config::AppConfig;
use brum::start_background_server;
use serde_json::{json, Value};
use tempfile::tempdir;

/// Unpacked copies go to a throw-away cache folder, never the real one
fn use_test_cache() -> PathBuf {
    static CACHE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("brum-test-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CACHE_HOME", &dir);
        dir
    }).clone()
}

fn big_data() -> Vec<u8> {
    (0..300_000u32).map(|i| (i * 13 % 241) as u8).collect()
}

/// A 4 MiB FAT disk with DOCS/README.TXT and BIG.BIN
fn fat_image(path: &Path) {
    let mut f = std::fs::OpenOptions::new().create(true).read(true).write(true).truncate(true).open(path).unwrap();
    f.set_len(4 * 1024 * 1024).unwrap();
    fatfs::format_volume(&mut f, fatfs::FormatVolumeOptions::new()).unwrap();
    let fs = fatfs::FileSystem::new(f, fatfs::FsOptions::new()).unwrap();
    let root = fs.root_dir();
    let docs = root.create_dir("DOCS").unwrap();
    docs.create_file("README.TXT").unwrap().write_all(b"old disk file").unwrap();
    root.create_file("BIG.BIN").unwrap().write_all(&big_data()).unwrap();
}

/// A SquashFS image with notes.txt
fn squashfs_image(path: &Path) {
    use backhand::{compression::Compressor, FilesystemCompressor, FilesystemWriter, NodeHeader};
    let h = NodeHeader { permissions: 0o644, uid: 1000, gid: 1000, mtime: 1_700_000_000 };
    let mut fs = FilesystemWriter::default();
    fs.set_compressor(FilesystemCompressor::new(Compressor::Gzip, None).unwrap());
    fs.push_file(Cursor::new(b"inside squashfs".to_vec()), "notes.txt", h).unwrap();
    let mut out = std::fs::File::create(path).unwrap();
    fs.write(&mut out).unwrap();
}

fn compress(src: &Path, dst: &Path) {
    let data = std::fs::read(src).unwrap();
    let name = dst.to_string_lossy().to_string();
    let out = if name.ends_with(".zst") {
        zstd::encode_all(&data[..], 3).unwrap()
    } else if name.ends_with(".gz") {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&data).unwrap();
        e.finish().unwrap()
    } else if name.ends_with(".xz") {
        let mut e = liblzma::write::XzEncoder::new(Vec::new(), 6);
        e.write_all(&data).unwrap();
        e.finish().unwrap()
    } else if name.ends_with(".bz2") {
        let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        e.write_all(&data).unwrap();
        e.finish().unwrap()
    } else {
        unreachable!()
    };
    std::fs::write(dst, out).unwrap();
}

struct Srv {
    base: String,
    client: reqwest::Client,
    admin: String,
    _dir: tempfile::TempDir,
    home: PathBuf,
}

async fn start() -> Srv {
    use_test_cache();
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
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
    let (port, _) = start_background_server(config).await.unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    let body: Value = client.post(format!("{base}/api/auth/login"))
        .json(&json!({ "username": "admin", "password": "AdminSecretPass123!" }))
        .send().await.unwrap().json().await.unwrap();
    let admin = body["token"].as_str().unwrap().to_string();
    Srv { base, client, admin, _dir: dir, home }
}

impl Srv {
    async fn list(&self, path: &str) -> Value {
        let resp = self.client.get(format!("{}/api/fs/list", self.base))
            .query(&[("path", path)]).bearer_auth(&self.admin).send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        assert!(status.is_success(), "list {path}: {status} {text}");
        serde_json::from_str(&text).unwrap()
    }

    async fn read(&self, path: &str) -> Value {
        let resp = self.client.get(format!("{}/api/fs/read", self.base))
            .query(&[("path", path)]).bearer_auth(&self.admin).send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        assert!(status.is_success(), "read {path}: {status} {text}");
        serde_json::from_str(&text).unwrap()
    }

    /// Follows single folders (a partition) down to the folder holding `want`
    async fn find_dir_with(&self, start: &str, want: &str) -> (String, Value) {
        let mut path = start.to_string();
        for _ in 0..4 {
            let listing = self.list(&path).await;
            let entries = listing["entries"].as_array().unwrap();
            if entries.iter().any(|e| e["name"].as_str().unwrap().eq_ignore_ascii_case(want)) {
                return (path, listing);
            }
            let dirs: Vec<&Value> = entries.iter().filter(|e| e["is_dir"].as_bool().unwrap_or(false)).collect();
            assert!(!dirs.is_empty(), "{want} not found under {start}: {listing}");
            path = dirs[0]["path"].as_str().unwrap().to_string();
        }
        panic!("{want} not found under {start}");
    }
}

#[tokio::test]
async fn compressed_disk_and_squashfs_images_open() {
    let srv = start().await;
    let work = tempdir().unwrap();
    let fat = work.path().join("disk.img");
    fat_image(&fat);
    let sq = work.path().join("rootfs.squashfs");
    squashfs_image(&sq);

    for ext in ["zst", "gz", "xz", "bz2"] {
        // A FAT disk image
        let img = srv.home.join(format!("disk.img.{ext}"));
        compress(&fat, &img);
        let original = img.to_string_lossy().to_string();
        let root = format!("archive://{original}#");
        let (dir, listing) = srv.find_dir_with(&root, "DOCS").await;
        // Addresses use the original file, never the hidden cache copy
        let text = listing.to_string();
        assert!(!text.contains("partclone_"), "cache path leaked: {text}");
        assert!(listing["current_path"].as_str().unwrap().starts_with(&format!("archive://{original}#")));
        let docs = listing["entries"].as_array().unwrap().iter()
            .find(|e| e["name"].as_str().unwrap().eq_ignore_ascii_case("DOCS")).unwrap()["path"].as_str().unwrap().to_string();
        let inner = srv.list(&docs).await;
        let readme = inner["entries"].as_array().unwrap().iter()
            .find(|e| e["name"].as_str().unwrap().eq_ignore_ascii_case("README.TXT")).expect("README.TXT")["path"].as_str().unwrap().to_string();
        let content = srv.read(&readme).await;
        assert_eq!(content["content"], "old disk file", "{ext}");
        assert!(content["path"].as_str().unwrap().contains(&original), "{ext}: {content}");

        // The big file downloads byte for byte
        let big = format!("{}/BIG.BIN", dir.trim_end_matches('/'));
        let big = if dir.ends_with('#') { format!("{dir}BIG.BIN") } else { big };
        let bytes = srv.client.get(format!("{}/api/fs/download", srv.base))
            .query(&[("path", big.as_str())]).bearer_auth(&srv.admin).send().await.unwrap().bytes().await.unwrap();
        assert_eq!(bytes.as_ref(), big_data().as_slice(), "{ext}: BIG.BIN differs");

        // A SquashFS image inside a compressed file
        let sqz = srv.home.join(format!("rootfs.img.{ext}"));
        compress(&sq, &sqz);
        let sq_root = format!("archive://{}#", sqz.to_string_lossy());
        let listing = srv.list(&sq_root).await;
        assert!(listing["entries"].as_array().unwrap().iter().any(|e| e["name"] == "notes.txt"), "{ext}: {listing}");
        let notes = srv.read(&format!("{sq_root}notes.txt")).await;
        assert_eq!(notes["content"], "inside squashfs");
    }

    // Extracting a compressed image writes its files
    let out = srv.home.join("extracted");
    let resp = srv.client.post(format!("{}/api/fs/archive/extract", srv.base)).bearer_auth(&srv.admin)
        .json(&json!({ "archive_path": srv.home.join("rootfs.img.xz").to_string_lossy(), "target_dir": out.to_string_lossy() }))
        .send().await.unwrap();
    assert!(resp.status().is_success(), "{}", resp.text().await.unwrap());
    assert_eq!(std::fs::read_to_string(walk_find(&out, "notes.txt").expect("extracted notes.txt")).unwrap(), "inside squashfs");

    // Ordinary compressed files are not treated as images
    std::fs::write(srv.home.join("notes.txt.gz"), b"x").unwrap();
    let home_listing = srv.list(&srv.home.to_string_lossy()).await;
    let entry = home_listing["entries"].as_array().unwrap().iter().find(|e| e["name"] == "notes.txt.gz").unwrap().clone();
    assert_eq!(entry["is_archive"], false);
    let entry = home_listing["entries"].as_array().unwrap().iter().find(|e| e["name"] == "disk.img.xz").unwrap().clone();
    assert_eq!(entry["is_archive"], true);

    // Admin cache settings: read, change, clear
    let status: Value = srv.client.get(format!("{}/api/system/image-cache", srv.base)).bearer_auth(&srv.admin)
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(status["retention_days"], 7);
    assert!(status["files"].as_u64().unwrap() >= 8, "{status}");
    // A copy nobody opened for 3 days goes when the limit is 1 day; the rest stay
    let cache = use_test_cache().join("brum").join("partclone_cache");
    let mut copies: Vec<PathBuf> = std::fs::read_dir(&cache).unwrap().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "raw")).collect();
    copies.sort();
    let three_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 3600);
    std::fs::OpenOptions::new().write(true).open(&copies[0]).unwrap().set_modified(three_days_ago).unwrap();
    let saved: Value = srv.client.post(format!("{}/api/system/image-cache", srv.base)).bearer_auth(&srv.admin)
        .json(&json!({ "retention_days": 1 })).send().await.unwrap().json().await.unwrap();
    assert_eq!(saved["retention_days"], 1);
    assert!(!copies[0].exists(), "expired copy should be removed");
    assert!(copies[1..].iter().all(|p| p.exists()), "recent copies should stay");
    let saved: Value = srv.client.post(format!("{}/api/system/image-cache", srv.base)).bearer_auth(&srv.admin)
        .json(&json!({ "retention_days": 30 })).send().await.unwrap().json().await.unwrap();
    assert_eq!(saved["retention_days"], 30);
    let bad = srv.client.post(format!("{}/api/system/image-cache", srv.base)).bearer_auth(&srv.admin)
        .json(&json!({ "retention_days": 99999 })).send().await.unwrap();
    assert_eq!(bad.status().as_u16(), 400);
    let cleared: Value = srv.client.post(format!("{}/api/system/image-cache/clear", srv.base)).bearer_auth(&srv.admin)
        .send().await.unwrap().json().await.unwrap();
    assert!(cleared["removed"].as_u64().unwrap() >= 7, "{cleared}");
    assert_eq!(cleared["files"], 0);
    // Opening again unpacks again
    let again = srv.read(&format!("archive://{}#notes.txt", srv.home.join("rootfs.img.gz").to_string_lossy())).await;
    assert_eq!(again["content"], "inside squashfs");

    // Only admins may change it
    let home = srv.home.to_string_lossy().to_string();
    let resp = srv.client.post(format!("{}/api/auth/users", srv.base)).bearer_auth(&srv.admin)
        .json(&json!({ "username": "bob", "password": "UserPass123!", "role": "user", "home_dir": home, "allowed_roots": [home] }))
        .send().await.unwrap();
    assert!(resp.status().is_success());
    let bob: Value = srv.client.post(format!("{}/api/auth/login", srv.base))
        .json(&json!({ "username": "bob", "password": "UserPass123!" })).send().await.unwrap().json().await.unwrap();
    let bob = bob["token"].as_str().unwrap();
    for (method, path) in [("GET", "/api/system/image-cache"), ("POST", "/api/system/image-cache/clear")] {
        let req = if method == "GET" { srv.client.get(format!("{}{path}", srv.base)) } else { srv.client.post(format!("{}{path}", srv.base)) };
        assert_eq!(req.bearer_auth(bob).send().await.unwrap().status().as_u16(), 403, "{path}");
    }
    let _ = std::fs::remove_dir_all(use_test_cache());
}

fn walk_find(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(found) = walk_find(&p, name) {
                return Some(found);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}
