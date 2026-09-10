fn main() {
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
            let _ = res.compile();
        }
    }
}
