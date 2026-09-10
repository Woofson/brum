use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub themes: ThemeConfig,
    #[serde(default)]
    pub paranoid: ParanoidConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub desktop: DesktopConfig,
    #[serde(default)]
    pub custom_actions: Vec<CustomAction>,
    #[serde(default = "default_open_with")]
    pub open_with: Vec<OpenWithRule>,
    #[serde(default)]
    pub bookmarks: Vec<BookmarkConfig>,
    #[serde(default)]
    pub syncthing: crate::tools::syncthing::SyncthingConfig,
    #[serde(default)]
    pub notedog: NoteDogConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            auth: AuthConfig::default(),
            storage: StorageConfig::default(),
            themes: ThemeConfig::default(),
            paranoid: ParanoidConfig::default(),
            ui: UiConfig::default(),
            desktop: DesktopConfig::default(),
            custom_actions: default_custom_actions(),
            open_with: default_open_with(),
            bookmarks: default_bookmarks(),
            syncthing: crate::tools::syncthing::SyncthingConfig::default(),
            notedog: NoteDogConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_root_path")]
    pub root_path: String,
    #[serde(default = "default_upload_max_mb")]
    pub upload_max_size_mb: usize,
    #[serde(default = "default_true")]
    pub enable_auth: bool,
    #[serde(default)]
    pub standalone: bool,
    #[serde(default = "default_jwt_secret")]
    pub jwt_secret: String,
    #[serde(default = "default_session_hours")]
    pub session_duration_hours: u64,
    #[serde(default = "default_db_path")]
    pub database_path: String,
    #[serde(default)]
    pub server_name: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            root_path: default_root_path(),
            upload_max_size_mb: default_upload_max_mb(),
            enable_auth: true,
            standalone: false,
            jwt_secret: default_jwt_secret(),
            session_duration_hours: default_session_hours(),
            database_path: default_db_path(),
            server_name: String::new(),
        }
    }
}

fn default_host() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 3140 }
fn default_root_path() -> String {
    #[cfg(windows)]
    {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
        "C:\\".to_string()
    }
    #[cfg(not(windows))]
    {
        "/".to_string()
    }
}
fn default_upload_max_mb() -> usize { 10240 } // 10 GB
fn default_true() -> bool { true }
fn default_jwt_secret() -> String { "brum-super-secret-jwt-key-2026".to_string() }
fn default_session_hours() -> u64 { 72 }
fn default_db_path() -> String {
    if let Ok(env_path) = std::env::var("BRUM_DATABASE_PATH").or_else(|_| std::env::var("CD_DATABASE_PATH")) {
        if !env_path.trim().is_empty() {
            return env_path;
        }
    }
    if Path::new("/data").is_dir() {
        if Path::new("/data/commanderdog.db").is_file() && !Path::new("/data/brum.db").is_file() {
            "/data/commanderdog.db".to_string()
        } else {
            "/data/brum.db".to_string()
        }
    } else {
        if Path::new("commanderdog.db").is_file() && !Path::new("brum.db").is_file() {
            "commanderdog.db".to_string()
        } else {
            "brum.db".to_string()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_auth_mode")]
    pub mode: String, // "mixed", "builtin", "pam", "none"
    #[serde(default = "default_pam_service")]
    pub pam_service: String,
    #[serde(default = "default_false")]
    pub allow_guest: bool,
    #[serde(default = "default_admin_username")]
    pub default_admin_user: String,
    #[serde(default = "default_admin_password")]
    pub default_admin_pass: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: default_auth_mode(),
            pam_service: default_pam_service(),
            allow_guest: false,
            default_admin_user: default_admin_username(),
            default_admin_pass: default_admin_password(),
        }
    }
}

fn default_auth_mode() -> String { "mixed".to_string() }
fn default_pam_service() -> String { "login".to_string() }
fn default_false() -> bool { false }
fn default_admin_username() -> String { "admin".to_string() }
fn default_admin_password() -> String { "brum".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_true")]
    pub allow_entire_system: bool,
    #[serde(default = "default_user_home_template")]
    pub default_user_home_template: String,
    #[serde(default = "default_storage_roots")]
    pub roots: Vec<StorageRoot>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            allow_entire_system: true,
            default_user_home_template: default_user_home_template(),
            roots: default_storage_roots(),
        }
    }
}

fn default_user_home_template() -> String {
    #[cfg(windows)]
    {
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            let parent = Path::new(&userprofile).parent().unwrap_or(Path::new("C:\\Users"));
            return format!("{}/{{username}}", parent.to_string_lossy().replace('\\', "/"));
        }
        "C:/Users/{username}".to_string()
    }
    #[cfg(not(windows))]
    {
        if Path::new("/data").exists() {
            "/data/users/{username}".to_string()
        } else {
            "/home/{username}".to_string()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageRoot {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub allowed_roles: Vec<String>,
}

fn default_storage_roots() -> Vec<StorageRoot> {
    let mut list = Vec::new();
    if Path::new("/data").exists() {
        list.push(StorageRoot {
            id: "data".to_string(),
            name: "Application Data".to_string(),
            path: "/data".to_string(),
            read_only: false,
            allowed_roles: vec![],
        });
    }
    if Path::new("/mnt").exists() {
        list.push(StorageRoot {
            id: "mnt".to_string(),
            name: "Mounts Storage".to_string(),
            path: "/mnt".to_string(),
            read_only: false,
            allowed_roles: vec![],
        });
    }
    list
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_theme_name")]
    pub default_theme: String,
    #[serde(default = "default_themes")]
    pub themes: Vec<ThemeDefinition>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            default_theme: default_theme_name(),
            themes: default_themes(),
        }
    }
}

fn default_theme_name() -> String { "amber-charcoal".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDefinition {
    pub id: String,
    pub name: String,
    pub bg_dark: String,
    pub bg_panel: String,
    pub bg_active: String,
    pub accent: String,
    pub accent_hover: String,
    pub text_main: String,
    pub text_muted: String,
    pub border: String,
}

fn default_themes() -> Vec<ThemeDefinition> {
    vec![
        ThemeDefinition {
            id: "amber-charcoal".to_string(),
            name: "Woofsons Amber Charcoal".to_string(),
            bg_dark: "#121214".to_string(),
            bg_panel: "#18181b".to_string(),
            bg_active: "#27272a".to_string(),
            accent: "#f59e0b".to_string(),
            accent_hover: "#fbbf24".to_string(),
            text_main: "#f4f4f5".to_string(),
            text_muted: "#a1a1aa".to_string(),
            border: "#3f3f46".to_string(),
        },
        ThemeDefinition {
            id: "zink".to_string(),
            name: "Woofsons Amber Zink".to_string(),
            bg_dark: "#fafafa".to_string(),
            bg_panel: "#ffffff".to_string(),
            bg_active: "#e4e4e7".to_string(),
            accent: "#d97706".to_string(),
            accent_hover: "#b45309".to_string(),
            text_main: "#18181b".to_string(),
            text_muted: "#52525b".to_string(),
            border: "#d4d4d8".to_string(),
        },
        ThemeDefinition {
            id: "gruvbox".to_string(),
            name: "Gruvbox Dark".to_string(),
            bg_dark: "#1d2021".to_string(),
            bg_panel: "#282828".to_string(),
            bg_active: "#3c3836".to_string(),
            accent: "#fabd2f".to_string(),
            accent_hover: "#fe8019".to_string(),
            text_main: "#ebdbb2".to_string(),
            text_muted: "#a89984".to_string(),
            border: "#504945".to_string(),
        },
        ThemeDefinition {
            id: "catppuccin-mocha".to_string(),
            name: "Catppuccin Mocha".to_string(),
            bg_dark: "#181825".to_string(),
            bg_panel: "#1e1e2e".to_string(),
            bg_active: "#313244".to_string(),
            accent: "#cba6f7".to_string(),
            accent_hover: "#f5c2e7".to_string(),
            text_main: "#cdd6f4".to_string(),
            text_muted: "#a6adc8".to_string(),
            border: "#45475a".to_string(),
        },
        ThemeDefinition {
            id: "catppuccin-latte".to_string(),
            name: "Catppuccin Latte (Light)".to_string(),
            bg_dark: "#dce0e8".to_string(),
            bg_panel: "#eff1f5".to_string(),
            bg_active: "#e6e9ef".to_string(),
            accent: "#8839ef".to_string(),
            accent_hover: "#1e66f5".to_string(),
            text_main: "#4c4f69".to_string(),
            text_muted: "#6c6f85".to_string(),
            border: "#bcc0cc".to_string(),
        },
        ThemeDefinition {
            id: "tokyo-night".to_string(),
            name: "Tokyo Night".to_string(),
            bg_dark: "#16161e".to_string(),
            bg_panel: "#1a1b26".to_string(),
            bg_active: "#24283b".to_string(),
            accent: "#7aa2f7".to_string(),
            accent_hover: "#7dcfff".to_string(),
            text_main: "#c0caf5".to_string(),
            text_muted: "#9aa5ce".to_string(),
            border: "#3b4261".to_string(),
        },
        ThemeDefinition {
            id: "monokai".to_string(),
            name: "Monokai Pro".to_string(),
            bg_dark: "#1e1f1c".to_string(),
            bg_panel: "#272822".to_string(),
            bg_active: "#3e3d32".to_string(),
            accent: "#ffd866".to_string(),
            accent_hover: "#a9dc76".to_string(),
            text_main: "#f8f8f2".to_string(),
            text_muted: "#939293".to_string(),
            border: "#49483e".to_string(),
        },
        ThemeDefinition {
            id: "solarized-dark".to_string(),
            name: "Solarized Dark".to_string(),
            bg_dark: "#00212b".to_string(),
            bg_panel: "#002b36".to_string(),
            bg_active: "#073642".to_string(),
            accent: "#268bd2".to_string(),
            accent_hover: "#2aa198".to_string(),
            text_main: "#839496".to_string(),
            text_muted: "#657b83".to_string(),
            border: "#586e75".to_string(),
        },
        ThemeDefinition {
            id: "ayu-dark".to_string(),
            name: "Ayu Dark".to_string(),
            bg_dark: "#0b0e14".to_string(),
            bg_panel: "#0f1419".to_string(),
            bg_active: "#1f2430".to_string(),
            accent: "#e6b450".to_string(),
            accent_hover: "#ffb454".to_string(),
            text_main: "#e6e1cf".to_string(),
            text_muted: "#707a8c".to_string(),
            border: "#252e37".to_string(),
        },
        ThemeDefinition {
            id: "nord".to_string(),
            name: "Nord Frost".to_string(),
            bg_dark: "#242933".to_string(),
            bg_panel: "#2e3440".to_string(),
            bg_active: "#3b4252".to_string(),
            accent: "#88c0d0".to_string(),
            accent_hover: "#81a1c1".to_string(),
            text_main: "#eceff4".to_string(),
            text_muted: "#d8dee9".to_string(),
            border: "#4c566a".to_string(),
        },
        ThemeDefinition {
            id: "dracula".to_string(),
            name: "Dracula Dark".to_string(),
            bg_dark: "#1e1f29".to_string(),
            bg_panel: "#282a36".to_string(),
            bg_active: "#44475a".to_string(),
            accent: "#bd93f9".to_string(),
            accent_hover: "#ff79c6".to_string(),
            text_main: "#f8f8f2".to_string(),
            text_muted: "#6272a4".to_string(),
            border: "#6272a4".to_string(),
        },
        ThemeDefinition {
            id: "midnight-blue".to_string(),
            name: "Midnight Commander Blue".to_string(),
            bg_dark: "#000044".to_string(),
            bg_panel: "#000088".to_string(),
            bg_active: "#0000aa".to_string(),
            accent: "#00ffff".to_string(),
            accent_hover: "#ffffff".to_string(),
            text_main: "#ffffff".to_string(),
            text_muted: "#a0a0ff".to_string(),
            border: "#00aaff".to_string(),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParanoidConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_checksum_algo")]
    pub checksum_algorithm: String, // "sha256", "md5", "sha1"
    #[serde(default = "default_true")]
    pub verify_after_transfer: bool,
    #[serde(default = "default_true")]
    pub atomic_writes: bool,
    #[serde(default = "default_true")]
    pub trash_enabled: bool,
    pub custom_trash_dir: Option<String>,
    #[serde(default = "default_true")]
    pub confirm_delete: bool,
    #[serde(default = "default_true")]
    pub confirm_overwrite: bool,
}

impl Default for ParanoidConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            checksum_algorithm: default_checksum_algo(),
            verify_after_transfer: true,
            atomic_writes: true,
            trash_enabled: true,
            custom_trash_dir: None,
            confirm_delete: true,
            confirm_overwrite: true,
        }
    }
}

fn default_checksum_algo() -> String { "sha256".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_pane_count")]
    pub default_pane_count: usize, // 1 to 4
    #[serde(default = "default_layout_name")]
    pub default_layout: String, // "quad", "dual-vertical", "dual-horizontal", "single", "triple"
    #[serde(default = "default_true")]
    pub show_hidden_files: bool,
    #[serde(default = "default_view_mode")]
    pub default_view_mode: String, // "details", "compact", "grid"
    #[serde(default = "default_true")]
    pub window_decorations: bool, // Tiled WM / Hyprland toggle (decorations on/off)
    #[serde(default = "default_false")]
    pub show_global_refresh: bool, // Global header refresh button toggle
    #[serde(default = "default_true")]
    pub show_hostname_badge: bool, // Top header hostname badge toggle
    #[serde(default)]
    pub hostname_badge: String, // Custom label for hostname badge (empty = auto)
    #[serde(default)]
    pub hostname_color: String, // "amber", "emerald", "sky", "purple", "rose", "orange", "cyan", "slate"
    #[serde(default)]
    pub hostname_style: String, // "subtle", "solid", "outline", "pill", "glow"
    #[serde(default)]
    pub hostname_icon: String,  // "server", "hard-drive", "cpu", "terminal", "cloud", "shield", "box", "home", "globe", "radio", "none"
    #[serde(default)]
    pub hostname_size: String,  // "sm", "md", "lg"
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            default_pane_count: default_pane_count(),
            default_layout: default_layout_name(),
            show_hidden_files: true,
            default_view_mode: default_view_mode(),
            window_decorations: true,
            show_global_refresh: false,
            show_hostname_badge: true,
            hostname_badge: String::new(),
            hostname_color: "amber".to_string(),
            hostname_style: "subtle".to_string(),
            hostname_icon: "server".to_string(),
            hostname_size: "md".to_string(),
        }
    }
}

fn default_pane_count() -> usize { 2 }
fn default_layout_name() -> String { "dual-vertical".to_string() }
fn default_view_mode() -> String { "details".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDogConfig {
    #[serde(default = "default_notes_folder")]
    pub notes_folder: String,
}

impl Default for NoteDogConfig {
    fn default() -> Self {
        Self {
            notes_folder: default_notes_folder(),
        }
    }
}

pub fn default_notes_folder() -> String {
    "~/Notes".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    #[serde(default = "default_true")]
    pub minimize_to_tray: bool,
    #[serde(default = "default_true")]
    pub enable_tray: bool,
    #[serde(default = "default_summon_hotkey")]
    pub global_summon_hotkey: String, // "Super+C", "Ctrl+Alt+Space", etc.
    #[serde(default = "default_false")]
    pub start_minimized: bool,
    #[serde(default)]
    pub external_editor: Option<String>,
    #[serde(default)]
    pub external_viewer: Option<String>,
    #[serde(default)]
    pub external_terminal: Option<String>,
    #[serde(default = "default_false")]
    pub use_external_editor_f4: bool,
    #[serde(default = "default_false")]
    pub use_external_viewer_f3: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            minimize_to_tray: true,
            enable_tray: true,
            global_summon_hotkey: default_summon_hotkey(),
            start_minimized: false,
            external_editor: None,
            external_viewer: None,
            external_terminal: None,
            use_external_editor_f4: false,
            use_external_viewer_f3: false,
        }
    }
}

fn default_summon_hotkey() -> String { "Super+C".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomAction {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub command: String,
    pub applicable_to: String, // "file", "folder", "archive", "all"
    #[serde(default)]
    pub in_background: bool,
}

fn default_custom_actions() -> Vec<CustomAction> {
    vec![
        CustomAction {
            id: "sha256-calc".to_string(),
            label: "Calculate SHA-256 Checksum".to_string(),
            icon: "shield-check".to_string(),
            command: "builtin:checksum:sha256".to_string(),
            applicable_to: "file".to_string(),
            in_background: false,
        },
        CustomAction {
            id: "folder-diff".to_string(),
            label: "Compare with Other Pane (Diff)".to_string(),
            icon: "columns-2".to_string(),
            command: "builtin:diff".to_string(),
            applicable_to: "all".to_string(),
            in_background: false,
        },
        CustomAction {
            id: "compress-zip".to_string(),
            label: "Compress to .zip".to_string(),
            icon: "archive".to_string(),
            command: "builtin:archive:zip".to_string(),
            applicable_to: "all".to_string(),
            in_background: true,
        },
        CustomAction {
            id: "compress-targz".to_string(),
            label: "Compress to .tar.gz".to_string(),
            icon: "archive".to_string(),
            command: "builtin:archive:targz".to_string(),
            applicable_to: "all".to_string(),
            in_background: true,
        },
        CustomAction {
            id: "compress-7z".to_string(),
            label: "Compress to .7z".to_string(),
            icon: "archive".to_string(),
            command: "builtin:archive:7z".to_string(),
            applicable_to: "all".to_string(),
            in_background: true,
        },
        CustomAction {
            id: "extract-here".to_string(),
            label: "Extract Archive Here".to_string(),
            icon: "unarchive".to_string(),
            command: "builtin:archive:extract".to_string(),
            applicable_to: "archive".to_string(),
            in_background: true,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenWithRule {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    pub command: String,
    pub icon: String,
    #[serde(default)]
    pub is_default: bool,
}

pub fn default_open_with() -> Vec<OpenWithRule> {
    vec![
        OpenWithRule {
            id: "editor-code".to_string(),
            name: "VS Code / Cursor".to_string(),
            extensions: vec![
                "rs".to_string(), "js".to_string(), "ts".to_string(), "py".to_string(),
                "json".to_string(), "toml".to_string(), "md".to_string(), "txt".to_string(),
                "html".to_string(), "css".to_string(), "sh".to_string(), "c".to_string(), "cpp".to_string(),
            ],
            command: "code \"%1\"".to_string(),
            icon: "code".to_string(),
            is_default: false,
        },
        OpenWithRule {
            id: "media-vlc".to_string(),
            name: "VLC Media Player".to_string(),
            extensions: vec![
                "mp4".to_string(), "mkv".to_string(), "avi".to_string(), "webm".to_string(),
                "mov".to_string(), "mp3".to_string(), "flac".to_string(), "wav".to_string(),
                "ogg".to_string(), "m4a".to_string(),
            ],
            command: "vlc \"%1\"".to_string(),
            icon: "film".to_string(),
            is_default: false,
        },
        OpenWithRule {
            id: "image-viewer".to_string(),
            name: "System Default Viewer".to_string(),
            extensions: vec![
                "png".to_string(), "jpg".to_string(), "jpeg".to_string(), "webp".to_string(),
                "svg".to_string(), "gif".to_string(), "bmp".to_string(), "ico".to_string(),
            ],
            command: "open \"%1\"".to_string(),
            icon: "image".to_string(),
            is_default: false,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkConfig {
    pub id: String,
    pub name: String,
    pub protocol: String, // "local", "sftp", "webdav"
    pub path: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
}

fn default_bookmarks() -> Vec<BookmarkConfig> {
    #[cfg(windows)]
    let (root_name, root_path) = ("System Drive (C:)", "C:\\");
    #[cfg(not(windows))]
    let (root_name, root_path) = ("Root Filesystem", "/");

    vec![
        BookmarkConfig {
            id: "home".to_string(),
            name: "Home Directory".to_string(),
            protocol: "local".to_string(),
            path: dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| {
                #[cfg(windows)]
                { "C:\\Users".to_string() }
                #[cfg(not(windows))]
                { "/home".to_string() }
            }),
            host: None,
            port: None,
            username: None,
        },
        BookmarkConfig {
            id: "root".to_string(),
            name: root_name.to_string(),
            protocol: "local".to_string(),
            path: root_path.to_string(),
            host: None,
            port: None,
            username: None,
        },
    ]
}

/// Config & External Theme Manager
pub struct ConfigManager;

impl ConfigManager {
    /// Loads configuration with sub-millisecond fast-path resolution:
    /// 1. User config: ~/.config/brum/config.toml (or ~/.config/commanderdog/config.toml)
    /// 2. Working dir config: ./config.toml / ./brum.toml
    /// 3. System-wide config: /etc/brum/config.toml / /etc/commanderdog/config.toml
    /// 4. Embedded fallback defaults: AppConfig::default()
    ///
    /// Also scans external theme directories (~/.config/brum/themes/*.toml).
    pub fn load_all() -> AppConfig {
        // Ensure user config and themes directory exists
        if let Some(user_config_dir) = dirs::config_dir().map(|d| d.join("brum")) {
            let _ = fs::create_dir_all(user_config_dir.join("themes"));
        }

        // Fast-path candidate paths in strict priority order
        let candidate_paths = vec![
            dirs::config_dir().map(|d| d.join("brum").join("config.toml")),
            dirs::config_dir().map(|d| d.join("commanderdog").join("config.toml")),
            Some(PathBuf::from("/data/config.toml")),
            Some(PathBuf::from("./brum.toml")),
            Some(PathBuf::from("./config.toml")),
            Some(PathBuf::from("/etc/brum/config.toml")),
            Some(PathBuf::from("/etc/commanderdog/config.toml")),
        ];

        let mut config = AppConfig::default();

        for candidate in candidate_paths.into_iter().flatten() {
            if candidate.is_file() {
                info!("Loading master configuration: {}", candidate.display());
                match fs::read_to_string(&candidate) {
                    Ok(content) => match toml::from_str::<AppConfig>(&content) {
                        Ok(parsed) => {
                            config = parsed;
                            break; // Stop immediately on first matching priority config
                        }
                        Err(e) => {
                            warn!("Failed to parse config {}: {}, falling back to defaults", candidate.display(), e);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to read config {}: {}", candidate.display(), e);
                    }
                }
            }
        }

        // Ensure all built-in default themes (e.g. zink, amber-charcoal) are present
        // even if the user has an existing older config.toml file on disk.
        for dt in default_themes() {
            if !config.themes.themes.iter().any(|t| t.id == dt.id) {
                config.themes.themes.push(dt);
            }
        }

        // Discover and load external themes from themes/ directories
        Self::load_external_themes(&mut config);

        // Environment Variable Overrides for Docker & Cloud Deployments
        if let Ok(p) = std::env::var("BRUM_PORT").or_else(|_| std::env::var("CD_PORT")).or_else(|_| std::env::var("PORT")) {
            if let Ok(port_num) = p.parse::<u16>() {
                config.server.port = port_num;
            }
        }
        if let Ok(h) = std::env::var("BRUM_BIND").or_else(|_| std::env::var("BRUM_HOST")).or_else(|_| std::env::var("CD_BIND")).or_else(|_| std::env::var("CD_HOST")).or_else(|_| std::env::var("HOST")) {
            if !h.trim().is_empty() {
                config.server.host = h.trim().to_string();
            }
        }
        if let Ok(db) = std::env::var("BRUM_DATABASE_PATH").or_else(|_| std::env::var("CD_DATABASE_PATH")).or_else(|_| std::env::var("DATABASE_PATH")) {
            if !db.trim().is_empty() {
                config.server.database_path = db.trim().to_string();
            }
        }
        if let Ok(jwt) = std::env::var("BRUM_JWT_SECRET").or_else(|_| std::env::var("CD_JWT_SECRET")).or_else(|_| std::env::var("JWT_SECRET")) {
            if !jwt.trim().is_empty() {
                config.server.jwt_secret = jwt.trim().to_string();
            }
        }

        config
    }

    fn collect_toml_files_sorted(dir: &Path, files: &mut Vec<PathBuf>) {
        if dir.exists() && dir.is_dir() {
            if let Ok(entries) = fs::read_dir(dir) {
                let mut dir_files: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().map_or(false, |ext| ext == "toml"))
                    .collect();
                dir_files.sort();
                files.extend(dir_files);
            }
        }
    }

    /// Scans external theme directories and loads theme definitions
    fn load_external_themes(config: &mut AppConfig) {
        let mut theme_dirs = vec![
            PathBuf::from("/etc/brum/themes"),
            PathBuf::from("/etc/commanderdog/themes"),
            PathBuf::from("./themes"),
        ];
        if let Some(d) = dirs::config_dir() {
            theme_dirs.push(d.join("brum").join("themes"));
            theme_dirs.push(d.join("commanderdog").join("themes"));
        }

        let mut theme_files = Vec::new();
        for dir in theme_dirs {
            Self::collect_toml_files_sorted(&dir, &mut theme_files);
        }

        for file_path in theme_files {
            info!("Loading external theme definition: {}", file_path.display());
            if let Ok(content) = fs::read_to_string(&file_path) {
                let stem = file_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "custom".to_string());
                Self::parse_and_insert_themes(config, &content, &stem);
            }
        }
    }

    fn parse_and_insert_themes(config: &mut AppConfig, content: &str, file_stem: &str) {
        // Try parsing as multi-theme array struct: [[themes]] or [themes] themes = [...]
        #[derive(Deserialize)]
        struct MultiThemeContainer {
            themes: Option<Vec<ThemeDefinition>>,
            theme: Option<ThemeDefinition>,
        }

        if let Ok(container) = toml::from_str::<MultiThemeContainer>(content) {
            if let Some(list) = container.themes {
                for t in list {
                    Self::upsert_theme(&mut config.themes.themes, t);
                }
                return;
            }
            if let Some(t) = container.theme {
                Self::upsert_theme(&mut config.themes.themes, t);
                return;
            }
        }

        // Try parsing directly as a single flat ThemeDefinition:
        // id = "...", name = "...", bg_dark = "...", ...
        #[derive(Deserialize)]
        struct FlatTheme {
            id: Option<String>,
            name: Option<String>,
            bg_dark: String,
            bg_panel: String,
            bg_active: String,
            accent: String,
            accent_hover: Option<String>,
            text_main: String,
            text_muted: String,
            border: String,
        }

        if let Ok(flat) = toml::from_str::<FlatTheme>(content) {
            let id = flat.id.unwrap_or_else(|| file_stem.to_string());
            let name = flat.name.unwrap_or_else(|| {
                file_stem
                    .split(['-', '_'])
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        }
                    })
                    .collect::<Vec<String>>()
                    .join(" ")
            });
            let accent = flat.accent.clone();
            let accent_hover = flat.accent_hover.unwrap_or(accent);

            let theme = ThemeDefinition {
                id,
                name,
                bg_dark: flat.bg_dark,
                bg_panel: flat.bg_panel,
                bg_active: flat.bg_active,
                accent: flat.accent,
                accent_hover,
                text_main: flat.text_main,
                text_muted: flat.text_muted,
                border: flat.border,
            };
            Self::upsert_theme(&mut config.themes.themes, theme);
        }
    }

    fn upsert_theme(themes: &mut Vec<ThemeDefinition>, new_theme: ThemeDefinition) {
        if let Some(existing) = themes.iter_mut().find(|t| t.id == new_theme.id) {
            *existing = new_theme;
        } else {
            themes.push(new_theme);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_external_theme_flat_parsing() {
        let mut config = AppConfig::default();
        let sample_toml = r##"
            id = "hyprland-cyan"
            name = "Hyprland Cyan"
            bg_dark = "#0b0f14"
            bg_panel = "#111822"
            bg_active = "#1b2533"
            accent = "#00e5ff"
            accent_hover = "#33ebff"
            text_main = "#e1e7ec"
            text_muted = "#7a889b"
            border = "#00e5ff"
        "##;

        ConfigManager::parse_and_insert_themes(&mut config, sample_toml, "hyprland-cyan");
        let theme = config.themes.themes.iter().find(|t| t.id == "hyprland-cyan");
        assert!(theme.is_some());
        let t = theme.unwrap();
        assert_eq!(t.name, "Hyprland Cyan");
        assert_eq!(t.accent, "#00e5ff");
    }

    #[test]
    fn test_external_theme_multi_parsing() {
        let mut config = AppConfig::default();
        let sample_toml = r##"
            [[themes]]
            id = "custom-one"
            name = "Custom One"
            bg_dark = "#101010"
            bg_panel = "#202020"
            bg_active = "#303030"
            accent = "#ff0055"
            accent_hover = "#ff3377"
            text_main = "#ffffff"
            text_muted = "#888888"
            border = "#444444"
        "##;

        ConfigManager::parse_and_insert_themes(&mut config, sample_toml, "themes");
        let theme = config.themes.themes.iter().find(|t| t.id == "custom-one");
        assert!(theme.is_some());
        assert_eq!(theme.unwrap().accent, "#ff0055");
    }

    #[test]
    fn test_master_config_parsing() {
        let sample_toml = r##"
            [server]
            host = "127.0.0.1"
            port = 9090

            [ui]
            window_decorations = false
            show_global_refresh = false

            [desktop]
            minimize_to_tray = true
            global_summon_hotkey = "Super+C"
            external_editor = "code \"%1\""
            use_external_editor_f4 = true

            [themes]
            default_theme = "catppuccin-mocha"
        "##;

        let config: AppConfig = toml::from_str(sample_toml).unwrap();
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.ui.window_decorations, false);
        assert_eq!(config.ui.show_global_refresh, false);
        assert_eq!(config.desktop.global_summon_hotkey, "Super+C");
        assert_eq!(config.desktop.external_editor, Some("code \"%1\"".to_string()));
        assert_eq!(config.desktop.use_external_editor_f4, true);
        assert_eq!(config.themes.default_theme, "catppuccin-mocha");
    }

    #[test]
    fn test_storage_config_parsing() {
        let sample_toml = r##"
            [storage]
            allow_entire_system = false
            default_user_home_template = "/users/{username}"

            [[storage.roots]]
            id = "vault"
            name = "Secure Vault"
            path = "/mnt/vault"
            read_only = true
            allowed_roles = ["admin"]

            [[storage.roots]]
            id = "share"
            name = "Public Share"
            path = "/mnt/share"
            read_only = false
        "##;

        let config: AppConfig = toml::from_str(sample_toml).unwrap();
        assert_eq!(config.storage.allow_entire_system, false);
        assert_eq!(config.storage.default_user_home_template, "/users/{username}");
        assert_eq!(config.storage.roots.len(), 2);
        assert_eq!(config.storage.roots[0].id, "vault");
        assert_eq!(config.storage.roots[0].read_only, true);
        assert_eq!(config.storage.roots[1].name, "Public Share");
    }

    #[test]
    fn test_open_with_and_custom_actions_parsing() {
        let sample_toml = r##"
            [[open_with]]
            id = "custom-vlc"
            name = "VLC Player"
            extensions = ["mp4", "mkv"]
            command = "vlc %1"
            icon = "film"
            is_default = true

            [[custom_actions]]
            id = "git-pull"
            label = "Git Pull"
            icon = "git-pull-request"
            command = "git -C {dir} pull"
            applicable_to = "folder"
            in_background = false
        "##;

        let config: AppConfig = toml::from_str(sample_toml).unwrap();
        assert_eq!(config.open_with.len(), 1);
        assert_eq!(config.open_with[0].id, "custom-vlc");
        assert_eq!(config.open_with[0].extensions, vec!["mp4", "mkv"]);
        assert_eq!(config.custom_actions.len(), 1);
        assert_eq!(config.custom_actions[0].command, "git -C {dir} pull");
    }
}
