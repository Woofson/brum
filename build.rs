use std::process::Command;

fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    let pkg_version = env!("CARGO_PKG_VERSION");

    // 1. Extract Git Commit Hash and dirty status
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { String::from_utf8(o.stdout).ok() } else { None })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "release".to_string());

    let is_dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(!o.stdout.is_empty()) } else { None })
        .unwrap_or(false);

    let commit_str = if is_dirty && commit != "release" {
        format!("{}-dirty", commit)
    } else {
        commit.clone()
    };

    // 2. Compute Commit Count / Build Number
    let commit_count: u64 = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { String::from_utf8(o.stdout).ok() } else { None })
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(1);

    // Read or increment local build counter
    let counter_file = ".build_counter";
    let previous_count = std::fs::read_to_string(counter_file)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    let build_number = std::cmp::max(commit_count, previous_count + 1);
    let _ = std::fs::write(counter_file, build_number.to_string());

    // 3. Formatted UTC timestamp
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    // 4. Full SemVer 2.0.0 string with build metadata (+build.<N>.git.<hash>)
    let semver_full = format!("{}+build.{}.git.{}", pkg_version, build_number, commit);

    // Export compile-time environment variables for the main crate
    println!("cargo:rustc-env=BRUM_BUILD_NUMBER={}", build_number);
    println!("cargo:rustc-env=BRUM_BUILD_COMMIT={}", commit_str);
    println!("cargo:rustc-env=BRUM_BUILD_TIMESTAMP={}", timestamp);
    println!("cargo:rustc-env=BRUM_BUILD_TARGET={}", target);
    println!("cargo:rustc-env=BRUM_SEMVER_FULL={}", semver_full);

    // Ensure cargo re-runs build script if git state changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    #[cfg(windows)]
    {
        // When building as part of the Tauri desktop app, tauri-build generates
        // and links the Windows resource (.rc/manifest/version info).
        // Compiling winres here would cause duplicate resource error (CVT1100).
        let is_desktop = std::env::var("CARGO_FEATURE_DESKTOP_APP").is_ok()
            || std::env::var("TAURI_ENV_TARGET_TRIPLE").is_ok()
            || std::env::var("TAURI_ENV_ARCH").is_ok()
            || std::env::var("TAURI_FAMILY").is_ok();

        if !is_desktop {
            let mut res = winres::WindowsResource::new();
            res.set_icon("assets/brum.ico");
            res.set("ProductName", "Brum");
            res.set("FileDescription", "Multi-Pane Web Environment (File Commander/Manager) - By Woofson");
            res.set("LegalCopyright", "Copyright (c) 2026 Bolt J Woofson");
            res.set("FileVersion", &format!("{}.{}", pkg_version, build_number));
            res.set("ProductVersion", &semver_full);
            let _ = res.compile();
        }
    }
}
