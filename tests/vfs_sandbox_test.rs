use brum::config::AppConfig;
use brum::plugins::PluginManager;
use brum::server::validate_path_access;
use brum::vfs::archive::ArchiveHandler;
use reqwest::header::HeaderMap;
use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

fn create_raw_tar_with_path(archive_path: &std::path::Path, entry_name: &str, content: &[u8]) {
    let mut file = File::create(archive_path).unwrap();
    let mut header = [0u8; 512];

    // Name (0..100)
    let name_bytes = entry_name.as_bytes();
    header[..name_bytes.len()].copy_from_slice(name_bytes);

    // Mode (100..108)
    header[100..107].copy_from_slice(b"0000644");
    // UID (108..116)
    header[108..115].copy_from_slice(b"0001000");
    // GID (116..124)
    header[116..123].copy_from_slice(b"0001000");
    // Size (124..136)
    let size_str = format!("{:011o}", content.len());
    header[124..135].copy_from_slice(size_str.as_bytes());
    // Mtime (136..148)
    header[136..147].copy_from_slice(b"14000000000");
    // Typeflag (156) -> regular file '0'
    header[156] = b'0';
    // Magic (257..263)
    header[257..263].copy_from_slice(b"ustar\0");
    // Version (263..265)
    header[263..265].copy_from_slice(b"00");

    // Checksum (148..156)
    // Fill checksum with spaces first
    for i in 148..156 {
        header[i] = b' ';
    }
    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    let chksum_str = format!("{:06o}\0 ", sum);
    header[148..156].copy_from_slice(chksum_str.as_bytes());

    file.write_all(&header).unwrap();
    file.write_all(content).unwrap();

    // Pad content to 512-byte boundary
    let remainder = content.len() % 512;
    if remainder > 0 {
        let pad = vec![0u8; 512 - remainder];
        file.write_all(&pad).unwrap();
    }

    // Two 512-byte zero blocks at end of tar archive
    let end_blocks = [0u8; 1024];
    file.write_all(&end_blocks).unwrap();
}

#[test]
fn test_tar_slip_path_traversal_rejected() {
    let tmp = tempdir().unwrap();
    let archive_path = tmp.path().join("malicious.tar");
    let target_dir = tmp.path().join("target");
    fs::create_dir_all(&target_dir).unwrap();

    create_raw_tar_with_path(&archive_path, "../escape.txt", b"pwned tar");

    let extract_res = ArchiveHandler::extract_archive(
        &archive_path.to_string_lossy(),
        &target_dir.to_string_lossy(),
    );

    assert!(extract_res.is_err(), "Tar extraction should have rejected path traversal entry!");
    assert!(!tmp.path().join("escape.txt").exists(), "Malicious file escaped extraction directory!");
}

#[test]
fn test_zip_slip_path_traversal_rejected() {
    let tmp = tempdir().unwrap();
    let archive_path = tmp.path().join("malicious.zip");
    let target_dir = tmp.path().join("target");
    fs::create_dir_all(&target_dir).unwrap();

    // Create a zip archive with a path traversal entry
    {
        let file = File::create(&archive_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        zip.start_file("../zip_escape.txt", options).unwrap();
        zip.write_all(b"pwned zip").unwrap();
        zip.finish().unwrap();
    }

    let extract_res = ArchiveHandler::extract_archive(
        &archive_path.to_string_lossy(),
        &target_dir.to_string_lossy(),
    );

    assert!(extract_res.is_err(), "Zip extraction should have rejected path traversal entry!");
    assert!(!tmp.path().join("zip_escape.txt").exists(), "Malicious file escaped extraction directory!");
}

#[test]
fn test_plugin_installer_zip_slip_rejected() {
    let tmp = tempdir().unwrap();
    let user_plugins_dir = tmp.path().join("user_plugins");
    fs::create_dir_all(&user_plugins_dir).unwrap();

    let plugin_mgr = PluginManager::new(
        tmp.path().join("sys_plugins"),
        user_plugins_dir.clone(),
        true,
        "allow".to_string(),
        vec![],
        vec![],
    );

    // Create a .grr archive with a traversal entry
    let grr_path = tmp.path().join("evil.grr");
    {
        let file = File::create(&grr_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        zip.start_file("plugin.toml", options).unwrap();
        zip.write_all(br#"
[plugin]
id = "evil-plugin"
name = "Evil Plugin"
version = "1.0.0"
author = "Hacker"
"#).unwrap();

        zip.start_file("index.html", options).unwrap();
        zip.write_all(b"<h1>Evil</h1>").unwrap();

        // Traversal entry
        zip.start_file("../evil_escape.txt", options).unwrap();
        zip.write_all(b"malicious payload").unwrap();

        zip.finish().unwrap();
    }

    let grr_bytes = fs::read(&grr_path).unwrap();
    let install_res = plugin_mgr.install_grr(&grr_bytes, "admin", true);

    assert!(install_res.is_err(), "Plugin installer should have rejected zip slip package!");
    assert!(!tmp.path().join("evil_escape.txt").exists(), "Plugin entry escaped plugin directory!");
}

#[tokio::test]
async fn test_vfs_path_access_scheme_and_roots_fail_closed() {
    let tmp = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.server.enable_auth = true;
    config.server.standalone = false;
    config.server.database_path = tmp.path().join("test.db").to_string_lossy().to_string();
    config.storage.allow_entire_system = false;
    config.storage.roots.push(brum::config::StorageRoot {
        id: "safe-root".to_string(),
        name: "Safe Root".to_string(),
        path: tmp.path().join("safe").to_string_lossy().to_string(),
        read_only: false,
        allowed_roles: vec!["user".to_string()],
    });

    let state = brum::create_app_state(&config).unwrap();

    // 1. Path with fake scheme in filename (/safe/test://../../etc/passwd) must NOT bypass checks
    let mut headers = HeaderMap::new();
    let user = state.auth.create_user(
        "testuser",
        "pass123",
        "user",
        &tmp.path().join("home").to_string_lossy(),
        Some("[\"safe-root\"]"),
    ).unwrap();
    let token = state.auth.generate_token(&user).unwrap();

    headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());

    // Fake scheme inside a local path
    let fake_scheme_path = "/home/test://../../etc/shadow";
    let res = validate_path_access(&state, &headers, fake_scheme_path, false);
    assert!(res.is_err(), "Fake scheme embedding inside path must not bypass sandbox validation!");

    // Real remote scheme prefix
    let real_remote = "sftp://user@host/path";
    let res_remote = validate_path_access(&state, &headers, real_remote, false);
    assert!(res_remote.is_ok(), "Real leading protocol prefix must be allowed!");
}
