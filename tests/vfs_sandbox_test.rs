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

    let state = brum::create_app_state(&config, None).unwrap();

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

    // Real remote scheme prefix without entitlement: non-admin MUST be rejected (403 Forbidden)
    let real_remote = "sftp://user@remote.example.com/path";
    let res_remote = validate_path_access(&state, &headers, real_remote, false);
    assert!(res_remote.is_err(), "Non-admin without explicit remote permission must be rejected!");
    assert_eq!(res_remote.unwrap_err().0.as_u16(), 403);

    // Admin user accessing valid remote scheme must be allowed
    let admin_user = state.auth.create_user(
        "adminuser",
        "pass123",
        "admin",
        &tmp.path().join("home").to_string_lossy(),
        Some("[\"*\"]"),
    ).unwrap();
    let admin_token = state.auth.generate_token(&admin_user).unwrap();
    let mut admin_headers = HeaderMap::new();
    admin_headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", admin_token).parse().unwrap());
    let res_admin = validate_path_access(&state, &admin_headers, "sftp://user@remote.example.com/path", false);
    assert!(res_admin.is_ok(), "Admin user must be allowed remote schemes!");
}

#[tokio::test]
async fn test_symlink_escape_nonexistent_file_rejected() {
    let tmp = tempdir().unwrap();
    let safe_root = tmp.path().join("safe");
    let outside_dir = tmp.path().join("outside");
    fs::create_dir_all(&safe_root).unwrap();
    fs::create_dir_all(&outside_dir).unwrap();

    let symlink_dir = safe_root.join("symlink_to_outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_dir, &symlink_dir).unwrap();
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_dir(&outside_dir, &symlink_dir);

    if !symlink_dir.exists() {
        return;
    }

    let mut config = AppConfig::default();
    config.server.enable_auth = true;
    config.server.standalone = false;
    config.server.database_path = tmp.path().join("test.db").to_string_lossy().to_string();
    config.storage.allow_entire_system = false;
    config.storage.roots.push(brum::config::StorageRoot {
        id: "safe-root".to_string(),
        name: "Safe Root".to_string(),
        path: safe_root.to_string_lossy().to_string(),
        read_only: false,
        allowed_roles: vec!["user".to_string()],
    });

    let state = brum::create_app_state(&config, None).unwrap();
    let user = state.auth.create_user(
        "testuser",
        "pass123",
        "user",
        &safe_root.to_string_lossy(),
        Some("[\"safe-root\"]"),
    ).unwrap();
    let token = state.auth.generate_token(&user).unwrap();

    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());

    // 1. Attempting to create a non-existent file through the symlink to outside must be FORBIDDEN
    let malicious_target = symlink_dir.join("new_escape_file.txt");
    let res_escape = validate_path_access(&state, &headers, &malicious_target.to_string_lossy(), true);
    assert!(
        res_escape.is_err(),
        "Creating new file through symlink pointing outside root must be rejected with 403!"
    );

    // 2. Legitimate non-existent file inside safe_root must be ALLOWED
    let legitimate_target = safe_root.join("valid_new_file.txt");
    let res_legit = validate_path_access(&state, &headers, &legitimate_target.to_string_lossy(), true);
    assert!(
        res_legit.is_ok(),
        "Creating new file directly inside authorized root must be permitted!"
    );
}

#[tokio::test]
async fn test_dangling_symlink_escape_rejected() {
    let tmp = tempdir().unwrap();
    let safe_root = tmp.path().join("safe");
    let outside_dir = tmp.path().join("outside");
    fs::create_dir_all(&safe_root).unwrap();
    fs::create_dir_all(&outside_dir).unwrap();

    let outside_target_file = outside_dir.join("nonexistent_escape.txt");
    let dangling_symlink = safe_root.join("dangling_escape_link.txt");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_target_file, &dangling_symlink).unwrap();
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_file(&outside_target_file, &dangling_symlink);

    // If symlink creation is not permitted on host (e.g. unprivileged Windows), skip gracefully
    if std::fs::symlink_metadata(&dangling_symlink).is_err() {
        return;
    }

    assert!(!dangling_symlink.exists(), "Target file must not exist (must be a dangling symlink)");
    assert!(
        std::fs::symlink_metadata(&dangling_symlink).unwrap().file_type().is_symlink(),
        "Link must exist as a symlink"
    );

    let mut config = AppConfig::default();
    config.server.enable_auth = true;
    config.server.standalone = false;
    config.server.database_path = tmp.path().join("test.db").to_string_lossy().to_string();
    config.storage.allow_entire_system = false;
    config.storage.roots.push(brum::config::StorageRoot {
        id: "safe-root".to_string(),
        name: "Safe Root".to_string(),
        path: safe_root.to_string_lossy().to_string(),
        read_only: false,
        allowed_roles: vec!["user".to_string()],
    });

    let state = brum::create_app_state(&config, None).unwrap();
    let user = state.auth.create_user(
        "testuser",
        "pass123",
        "user",
        &safe_root.to_string_lossy(),
        Some("[\"safe-root\"]"),
    ).unwrap();
    let token = state.auth.generate_token(&user).unwrap();

    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());

    // 1. Absolute dangling symlink pointing to non-existent file outside root must be 403 Forbidden
    let res_escape = validate_path_access(&state, &headers, &dangling_symlink.to_string_lossy(), true);
    assert!(
        res_escape.is_err(),
        "Accessing or creating a file through a dangling symlink pointing outside root must be rejected!"
    );
    assert_eq!(res_escape.unwrap_err().0.as_u16(), 403);

    // 2. Relative dangling symlink escaping root must also be 403 Forbidden
    let rel_dangling_symlink = safe_root.join("rel_dangle.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(std::path::Path::new("../outside/nonexistent_rel.txt"), &rel_dangling_symlink).unwrap();
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_file(std::path::Path::new(r"..\outside\nonexistent_rel.txt"), &rel_dangling_symlink);

    if std::fs::symlink_metadata(&rel_dangling_symlink).is_ok() {
        let res_rel_escape = validate_path_access(&state, &headers, &rel_dangling_symlink.to_string_lossy(), true);
        assert!(
            res_rel_escape.is_err(),
            "Relative dangling symlink escaping root must be rejected with 403!"
        );
        assert_eq!(res_rel_escape.unwrap_err().0.as_u16(), 403);
    }

    // 3. Dangling directory symlink pointing outside root with child path must also be 403 Forbidden
    let dangling_dir_symlink = safe_root.join("dangling_dir");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside_dir.join("nonexistent_subdir"), &dangling_dir_symlink).unwrap();
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_dir(outside_dir.join("nonexistent_subdir"), &dangling_dir_symlink);

    if std::fs::symlink_metadata(&dangling_dir_symlink).is_ok() {
        let child_target = dangling_dir_symlink.join("child_new_file.txt");
        let res_dir_escape = validate_path_access(&state, &headers, &child_target.to_string_lossy(), true);
        assert!(
            res_dir_escape.is_err(),
            "Child file through dangling directory symlink pointing outside root must be rejected with 403!"
        );
        assert_eq!(res_dir_escape.unwrap_err().0.as_u16(), 403);
    }

    // 4. Safe dangling symlink pointing to non-existent file strictly inside root must be allowed
    let safe_dangling_symlink = safe_root.join("safe_dangle.txt");
    let safe_future_file = safe_root.join("future_allowed_file.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&safe_future_file, &safe_dangling_symlink).unwrap();
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_file(&safe_future_file, &safe_dangling_symlink);

    if std::fs::symlink_metadata(&safe_dangling_symlink).is_ok() {
        let res_safe = validate_path_access(&state, &headers, &safe_dangling_symlink.to_string_lossy(), true);
        assert!(
            res_safe.is_ok(),
            "Dangling symlink pointing strictly inside root must be permitted!"
        );
    }
}

#[tokio::test]
async fn test_remote_network_schemes_authorization_and_ssrf() {
    let tmp = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.server.enable_auth = true;
    config.server.standalone = false;
    config.server.database_path = tmp.path().join("test_remote.db").to_string_lossy().to_string();
    config.storage.roots = vec![
        brum::config::StorageRoot {
            id: "safe-root".to_string(),
            name: "Safe Root".to_string(),
            path: tmp.path().to_string_lossy().to_string(),
            read_only: false,
            allowed_roles: vec![],
        },
    ];

    let state = brum::create_app_state(&config, None).unwrap();

    // 1. Regular user without remote permissions
    let normal_user = state.auth.create_user(
        "regular_joe",
        "pass123",
        "user",
        &tmp.path().to_string_lossy(),
        Some("[\"safe-root\"]"),
    ).unwrap();
    let user_token = state.auth.generate_token(&normal_user).unwrap();
    let mut user_headers = HeaderMap::new();
    user_headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", user_token).parse().unwrap());

    // Remote network schemes must all be blocked for non-admin
    let blocked_schemes = [
        "sftp://files.example.com/file.txt",
        "ssh://host.example.com/path",
        "smb://nas.corp/share",
        "samba://nas.corp/share",
        "webdav://storage.corp/data",
        "webdavs://storage.corp/data",
        "s3://my-bucket/key",
        "s3s://my-bucket/key",
        "nfs://nfs.corp/export",
        "http://files.corp/download",
        "https://files.corp/download",
    ];

    for uri in &blocked_schemes {
        let res = validate_path_access(&state, &user_headers, uri, false);
        assert!(res.is_err(), "Non-admin must be blocked from remote scheme: {}", uri);
        assert_eq!(res.unwrap_err().0.as_u16(), 403, "Status code must be 403 Forbidden for {}", uri);
    }

    // Local virtual schemes are permitted
    assert!(validate_path_access(&state, &user_headers, "vault://my-vault/secret", false).is_ok());
    assert!(validate_path_access(&state, &user_headers, "archive:///tmp/test.zip#inner", false).is_ok());
    assert!(validate_path_access(&state, &user_headers, "trash:///tmp/trash/item", false).is_ok());

    // 2. Entitled non-admin user with explicit remote entitlement
    let entitled_user = state.auth.create_user(
        "entitled_joe",
        "pass123",
        "user",
        &tmp.path().to_string_lossy(),
        Some("[\"sftp://allowed.remote.org/\"]"),
    ).unwrap();
    let entitled_token = state.auth.generate_token(&entitled_user).unwrap();
    let mut entitled_headers = HeaderMap::new();
    entitled_headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", entitled_token).parse().unwrap());

    assert!(
        validate_path_access(&state, &entitled_headers, "sftp://allowed.remote.org/file.txt", false).is_ok(),
        "Entitled user must be allowed on their granted remote root"
    );
    assert!(
        validate_path_access(&state, &entitled_headers, "sftp://forbidden.remote.org/file.txt", false).is_err(),
        "Entitled user must be blocked from other remote endpoints"
    );

    // 3. Admin user: allowed on legitimate external targets, but blocked from SSRF destinations
    let admin_user = state.auth.create_user(
        "superadmin",
        "pass123",
        "admin",
        &tmp.path().to_string_lossy(),
        Some("[\"*\"]"),
    ).unwrap();
    let admin_token = state.auth.generate_token(&admin_user).unwrap();
    let mut admin_headers = HeaderMap::new();
    admin_headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", admin_token).parse().unwrap());

    // Legitimate remote destinations allowed
    assert!(validate_path_access(&state, &admin_headers, "sftp://user@remote.example.com/path", false).is_ok());
    assert!(validate_path_access(&state, &admin_headers, "smb://user@nas.example.com/share", false).is_ok());

    // SSRF targets blocked even for admin
    let ssrf_targets = [
        "sftp://127.0.0.1/etc/passwd",
        "sftp://127.0.0.2:22/path",
        "sftp://localhost/path",
        "sftp://[::1]:22/path",
        "sftp://0.0.0.0/path",
        "sftp://169.254.169.254/latest/meta-data",
        "webdav://metadata.google.internal/computeMetadata",
        "smb://instance-data/share",
        "sftp://2130706433/path",     // Decimal 127.0.0.1
        "sftp://0177.0.0.1/path",      // Octal 127.0.0.1
        "sftp://0x7f.0.0.1/path",      // Hex 127.0.0.1
    ];

    for ssrf_uri in &ssrf_targets {
        let res = validate_path_access(&state, &admin_headers, ssrf_uri, false);
        assert!(res.is_err(), "SSRF target must be blocked: {}", ssrf_uri);
        assert_eq!(res.unwrap_err().0.as_u16(), 403, "Status code must be 403 for SSRF target: {}", ssrf_uri);
    }

    // 4. Configuration override: allow_remote_loopback = true allows 127.0.0.1, but metadata remains blocked
    let mut loopback_allowed_config = config.clone();
    loopback_allowed_config.storage.allow_remote_loopback = true;
    let loopback_state = brum::create_app_state(&loopback_allowed_config, None).unwrap();

    let res_loopback = validate_path_access(&loopback_state, &admin_headers, "sftp://127.0.0.1/path", false);
    assert!(res_loopback.is_ok(), "Loopback allowed when explicitly configured in storage config");

    let res_metadata = validate_path_access(&loopback_state, &admin_headers, "sftp://169.254.169.254/latest/meta-data", false);
    assert!(res_metadata.is_err(), "Cloud metadata must still be blocked even when allow_remote_loopback is enabled");
    assert_eq!(res_metadata.unwrap_err().0.as_u16(), 403);
}

#[tokio::test]
async fn test_archive_extraction_limits_and_subprocess_hardening() {
    let tmp = tempdir().unwrap();
    let safe_dir = tmp.path().join("extracted");
    fs::create_dir_all(&safe_dir).unwrap();

    // 1. Normal directory under limits passes
    let file1 = safe_dir.join("file1.txt");
    fs::write(&file1, b"hello world").unwrap();
    assert!(ArchiveHandler::verify_extracted_dir_limits(&safe_dir).is_ok());

    // 2. Extraction of archive with leading hyphen filename does not trigger flag parsing error
    let archive_dir = tmp.path().join("archives");
    fs::create_dir_all(&archive_dir).unwrap();
    let hyphen_archive = archive_dir.join("-test_archive.zip");
    
    // Create simple zip
    {
        let file = File::create(&hyphen_archive).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("sample.txt", SimpleFileOptions::default()).unwrap();
        zip.write_all(b"sample content").unwrap();
        zip.finish().unwrap();
    }

    let out_dir = tmp.path().join("hyphen_out");
    fs::create_dir_all(&out_dir).unwrap();
    let res_extract = ArchiveHandler::extract_archive(&hyphen_archive.to_string_lossy(), &out_dir.to_string_lossy());
    assert!(res_extract.is_ok(), "Archive with leading hyphen in filename must extract safely without CLI flag confusion");
    assert!(out_dir.join("sample.txt").exists());
}



