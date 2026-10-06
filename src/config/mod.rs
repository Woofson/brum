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
    #[serde(default)]
    pub terminal: TerminalConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub fleet: FleetConfig,
    #[serde(default)]
    pub sftp: Option<SftpConfig>,
}

impl AppConfig {
    pub fn get_sftp_config(&self) -> SftpConfig {
        if let Some(ref sftp) = self.sftp {
            sftp.clone()
        } else {
            self.storage.sftp.clone()
        }
    }
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
            terminal: TerminalConfig::default(),
            plugins: PluginsConfig::default(),
            fleet: FleetConfig::default(),
            sftp: None,
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
    #[cfg(windows)]
    {
        // 1. If explicit database file exists in current working directory, prefer it for local portable/dev use
        if Path::new("brum.db").is_file() {
            return "brum.db".to_string();
        }
        if Path::new("commanderdog.db").is_file() {
            return "commanderdog.db".to_string();
        }

        // 2. Check %PROGRAMDATA%\Brum\brum.db (e.g. C:\ProgramData\Brum\brum.db) - standard for Windows Service and installed apps
        if let Ok(progdata) = std::env::var("ProgramData") {
            let pdata_dir = Path::new(&progdata).join("Brum");
            let pdata_db = pdata_dir.join("brum.db");
            if pdata_db.is_file() {
                return pdata_db.to_string_lossy().to_string();
            }
            if pdata_dir.is_dir() || std::fs::create_dir_all(&pdata_dir).is_ok() {
                return pdata_db.to_string_lossy().to_string();
            }
        }

        // 3. Check %APPDATA%\Brum\brum.db
        if let Some(appdata) = dirs::data_dir() {
            let app_dir = appdata.join("Brum");
            let _ = std::fs::create_dir_all(&app_dir);
            let app_db = app_dir.join("brum.db");
            return app_db.to_string_lossy().to_string();
        }

        "brum.db".to_string()
    }
    #[cfg(not(windows))]
    {
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
    #[serde(default)]
    pub oidc: OidcConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_oidc_provider_name")]
    pub provider_name: String, // e.g. "Authentik", "Keycloak", "SSO"
    #[serde(default)]
    pub issuer_url: String, // e.g. "https://auth.example.com/application/o/brum/"
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default)]
    pub redirect_url: String, // e.g. "https://brum.example.com/api/auth/oidc/callback"
    #[serde(default = "default_oidc_scopes")]
    pub scopes: Vec<String>, // ["openid", "profile", "email", "groups"]
    #[serde(default = "default_true")]
    pub auto_provision: bool,
    #[serde(default = "default_oidc_admin_group")]
    pub admin_group: String, // e.g. "brum-admins" or "authentik Admins"
    #[serde(default = "default_oidc_default_role")]
    pub default_user_role: String, // "user", "readonly", "admin"
    #[serde(default = "default_user_home_template")]
    pub default_home_template: String,
    #[serde(default = "default_false")]
    pub force_sso_only: bool,
    #[serde(default = "default_oidc_button_icon")]
    pub button_icon: String, // e.g. "shield-check", "key-round", "lock"
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider_name: default_oidc_provider_name(),
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            redirect_url: String::new(),
            scopes: default_oidc_scopes(),
            auto_provision: true,
            admin_group: default_oidc_admin_group(),
            default_user_role: default_oidc_default_role(),
            default_home_template: default_user_home_template(),
            force_sso_only: false,
            button_icon: default_oidc_button_icon(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: default_auth_mode(),
            pam_service: default_pam_service(),
            allow_guest: false,
            default_admin_user: default_admin_username(),
            default_admin_pass: default_admin_password(),
            oidc: OidcConfig::default(),
        }
    }
}

fn default_auth_mode() -> String { "mixed".to_string() }
fn default_pam_service() -> String { "login".to_string() }
fn default_false() -> bool { false }
fn default_admin_username() -> String { "admin".to_string() }
fn default_admin_password() -> String { "brum".to_string() }
fn default_oidc_provider_name() -> String { "Authentik".to_string() }
fn default_oidc_scopes() -> Vec<String> { vec!["openid".to_string(), "profile".to_string(), "email".to_string(), "groups".to_string()] }
fn default_oidc_admin_group() -> String { "brum-admins".to_string() }
fn default_oidc_default_role() -> String { "user".to_string() }
fn default_oidc_button_icon() -> String { "shield-check".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SftpConfig {
    #[serde(default = "default_host_key_checking")]
    pub host_key_checking: String, // "auto_accept", "tofu", "strict"
    #[serde(default)]
    pub known_hosts_file: Option<String>,
}

fn default_host_key_checking() -> String {
    "tofu".to_string()
}

impl Default for SftpConfig {
    fn default() -> Self {
        Self {
            host_key_checking: default_host_key_checking(),
            known_hosts_file: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_true")]
    pub allow_entire_system: bool,
    #[serde(default = "default_user_home_template")]
    pub default_user_home_template: String,
    #[serde(default = "default_home_fallback_scheme")]
    pub home_fallback_scheme: String, // "auto", "roots_only", "strict"
    #[serde(default = "default_true")]
    pub auto_create_home_dirs: bool,
    #[serde(default)]
    pub sftp: SftpConfig,
    #[serde(default = "default_storage_roots")]
    pub roots: Vec<StorageRoot>,
}

fn default_home_fallback_scheme() -> String {
    "auto".to_string()
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            allow_entire_system: true,
            default_user_home_template: default_user_home_template(),
            home_fallback_scheme: default_home_fallback_scheme(),
            auto_create_home_dirs: true,
            sftp: SftpConfig::default(),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StorageRoot {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub allowed_roles: Vec<String>,
}

fn default_storage_roots() -> Vec<StorageRoot> {
    Vec::new()
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
    #[serde(default)]
    pub folder_color: Option<String>,
    #[serde(default)]
    pub radius: Option<String>,
    #[serde(default)]
    pub panel_gap_x: Option<String>,
    #[serde(default)]
    pub panel_gap_y: Option<String>,
    #[serde(default)]
    pub app_margin: Option<String>,
}

pub fn theme_def(
    id: &str,
    name: &str,
    bg_dark: &str,
    bg_panel: &str,
    bg_active: &str,
    accent: &str,
    accent_hover: &str,
    text_main: &str,
    text_muted: &str,
    border: &str,
) -> ThemeDefinition {
    ThemeDefinition {
        id: id.to_string(),
        name: name.to_string(),
        bg_dark: bg_dark.to_string(),
        bg_panel: bg_panel.to_string(),
        bg_active: bg_active.to_string(),
        accent: accent.to_string(),
        accent_hover: accent_hover.to_string(),
        text_main: text_main.to_string(),
        text_muted: text_muted.to_string(),
        border: border.to_string(),
        folder_color: None,
        radius: None,
        panel_gap_x: None,
        panel_gap_y: None,
        app_margin: None,
    }
}

fn default_themes() -> Vec<ThemeDefinition> {
    vec![
        theme_def(
            "amber-charcoal",
            "Woofsons Amber Charcoal",
            "#121214",
            "#18181b",
            "#27272a",
            "#f59e0b",
            "#fbbf24",
            "#f4f4f5",
            "#a1a1aa",
            "#3f3f46",
        ),
        theme_def(
            "zink",
            "Woofsons Amber Zink",
            "#fafafa",
            "#ffffff",
            "#e4e4e7",
            "#d97706",
            "#b45309",
            "#18181b",
            "#52525b",
            "#d4d4d8",
        ),
        theme_def(
            "gruvbox",
            "Gruvbox Dark",
            "#1d2021",
            "#282828",
            "#3c3836",
            "#fabd2f",
            "#fe8019",
            "#ebdbb2",
            "#a89984",
            "#504945",
        ),
        theme_def(
            "catppuccin-mocha",
            "Catppuccin Mocha",
            "#181825",
            "#1e1e2e",
            "#313244",
            "#cba6f7",
            "#f5c2e7",
            "#cdd6f4",
            "#a6adc8",
            "#45475a",
        ),
        theme_def(
            "catppuccin-latte",
            "Catppuccin Latte (Light)",
            "#dce0e8",
            "#eff1f5",
            "#e6e9ef",
            "#8839ef",
            "#1e66f5",
            "#4c4f69",
            "#6c6f85",
            "#bcc0cc",
        ),
        theme_def(
            "tokyo-night",
            "Tokyo Night",
            "#16161e",
            "#1a1b26",
            "#24283b",
            "#7aa2f7",
            "#7dcfff",
            "#c0caf5",
            "#9aa5ce",
            "#3b4261",
        ),
        theme_def(
            "monokai",
            "Monokai Pro",
            "#1e1f1c",
            "#272822",
            "#3e3d32",
            "#ffd866",
            "#a9dc76",
            "#f8f8f2",
            "#939293",
            "#49483e",
        ),
        theme_def(
            "solarized-dark",
            "Solarized Dark",
            "#00212b",
            "#002b36",
            "#073642",
            "#268bd2",
            "#2aa198",
            "#839496",
            "#657b83",
            "#586e75",
        ),
        theme_def(
            "ayu-dark",
            "Ayu Dark",
            "#0b0e14",
            "#0f1419",
            "#1f2430",
            "#e6b450",
            "#ffb454",
            "#e6e1cf",
            "#707a8c",
            "#252e37",
        ),
        theme_def(
            "nord",
            "Nord Frost",
            "#242933",
            "#2e3440",
            "#3b4252",
            "#88c0d0",
            "#81a1c1",
            "#eceff4",
            "#d8dee9",
            "#4c566a",
        ),
        theme_def(
            "dracula",
            "Dracula Dark",
            "#1e1f29",
            "#282a36",
            "#44475a",
            "#bd93f9",
            "#ff79c6",
            "#f8f8f2",
            "#6272a4",
            "#6272a4",
        ),
        theme_def(
            "midnight-blue",
            "Midnight Commander Blue",
            "#000044",
            "#000088",
            "#0000aa",
            "#00ffff",
            "#ffffff",
            "#ffffff",
            "#a0a0ff",
            "#00aaff",
        ),
        theme_def(
            "skumring",
            "Larvikite Skumring",
            "#0a0e14",
            "#111822",
            "#1e2c3d",
            "#38bdf8",
            "#7dd3fc",
            "#e6edf3",
            "#8b9bb4",
            "#243347",
        ),
        theme_def(
            "demring",
            "Larvikite Demring",
            "#eef2f6",
            "#f7fafc",
            "#cbd5e1",
            "#0e7490",
            "#155e75",
            "#0f172a",
            "#475569",
            "#cbd5e1",
        ),
        theme_def(
            "trollnatt",
            "Larvikite Trollnatt",
            "#0b100d",
            "#121914",
            "#222f26",
            "#4ade80",
            "#86efac",
            "#edf4ee",
            "#93a797",
            "#25342a",
        ),
        theme_def(
            "myrtaake",
            "Larvikite Myrtåke",
            "#edf2ee",
            "#f5f9f6",
            "#cad5cc",
            "#15803d",
            "#166534",
            "#0f1712",
            "#49594d",
            "#cbd7cd",
        ),
        theme_def(
            "bergtatt",
            "Kittelsen Bergtatt",
            "#0a0c0f",
            "#11141a",
            "#222935",
            "#d9a042",
            "#f1b759",
            "#e8e2d8",
            "#8e8d89",
            "#262e3d",
        ),
        theme_def(
            "soria-moria",
            "Kittelsen Soria Moria",
            "#ebe5dc",
            "#f5f0e6",
            "#cbbead",
            "#b87a1f",
            "#8f5a0e",
            "#1c1815",
            "#5d554a",
            "#c6bbaa",
        ),
        theme_def(
            "pestanatt",
            "Kittelsen Pestanatt",
            "#0b090a",
            "#141011",
            "#261e20",
            "#dc2626",
            "#ef4444",
            "#e6dede",
            "#948285",
            "#2b2023",
        ),
        theme_def(
            "sotslette",
            "Kittelsen Sotslette",
            "#ece6dc",
            "#f5f0e6",
            "#cec3b2",
            "#991b1b",
            "#b91c1c",
            "#1c1517",
            "#5c4f52",
            "#c7bcab",
        ),
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
    #[serde(default = "default_true")]
    pub windows_native_file_ops: bool,
    #[serde(default = "default_true")]
    pub detect_locking_processes: bool,
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
            windows_native_file_ops: true,
            detect_locking_processes: true,
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
    #[serde(default)]
    pub window_title: String,   // Custom document / window title (empty = default "Brum - Multi-Pane Web Environment")
    #[serde(default)]
    pub document_title_template: Option<String>,
    #[serde(default)]
    pub panel_gap_x: Option<String>,
    #[serde(default)]
    pub panel_gap_y: Option<String>,
    #[serde(default)]
    pub app_margin: Option<String>,
    #[serde(default)]
    pub corner_radius: Option<String>,
    #[serde(default)]
    pub folder_color: Option<String>,
    #[serde(default)]
    pub login_title: Option<String>,
    #[serde(default)]
    pub login_subtitle_template: Option<String>,
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
            hostname_color: "slate".to_string(),
            hostname_style: "text".to_string(),
            hostname_icon: "none".to_string(),
            hostname_size: "md".to_string(),
            window_title: String::new(),
            document_title_template: None,
            panel_gap_x: None,
            panel_gap_y: None,
            app_margin: None,
            corner_radius: None,
            folder_color: None,
            login_title: None,
            login_subtitle_template: None,
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
pub struct TerminalConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_terminal_allow_roles")]
    pub allow_roles: Vec<String>,
    #[serde(default = "default_terminal_allow_virtual_users")]
    pub allow_virtual_users: bool,
    #[serde(default = "default_terminal_drop_privileges")]
    pub drop_privileges: bool,
    #[serde(default)]
    pub default_shell: Option<String>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_roles: default_terminal_allow_roles(),
            allow_virtual_users: false,
            drop_privileges: true,
            default_shell: None,
        }
    }
}

fn default_terminal_allow_roles() -> Vec<String> {
    vec!["admin".to_string(), "root".to_string()]
}
fn default_terminal_allow_virtual_users() -> bool {
    false
}
fn default_terminal_drop_privileges() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_plugins_system_dir")]
    pub directory: String,
    #[serde(default = "default_plugins_user_dir")]
    pub user_directory: String,
    #[serde(default = "default_false")]
    pub allow_user_installs: bool,
    #[serde(default = "default_plugin_policy")]
    pub default_policy: String, // "allow_all", "whitelist", "blacklist"
    #[serde(default = "default_global_whitelist")]
    pub global_whitelist: Vec<String>,
    #[serde(default = "default_global_blacklist")]
    pub global_blacklist: Vec<String>,
}

fn default_plugins_system_dir() -> String {
    #[cfg(windows)]
    {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let p = parent.join("plugins");
                if p.is_dir() {
                    return p.to_string_lossy().to_string();
                }
            }
        }
        if std::path::Path::new("plugins").exists() {
            return "plugins".to_string();
        }
        if let Ok(app_data) = std::env::var("PROGRAMDATA") {
            return format!("{}\\Brum\\plugins", app_data);
        }
        "C:\\ProgramData\\Brum\\plugins".to_string()
    }
    #[cfg(not(windows))]
    {
        if std::path::Path::new("plugins").exists() {
            return "plugins".to_string();
        }
        if std::path::Path::new("/usr/share/brum/plugins").exists() {
            return "/usr/share/brum/plugins".to_string();
        }
        "/etc/brum/plugins".to_string()
    }
}

fn default_plugins_user_dir() -> String {
    #[cfg(windows)]
    {
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local_appdata).join("brum").join("plugins").to_string_lossy().to_string();
        }
        if let Some(local_dir) = dirs::data_local_dir() {
            return local_dir.join("brum").join("plugins").to_string_lossy().to_string();
        }
        if let Some(app_data) = dirs::config_dir() {
            return app_data.join("brum").join("plugins").to_string_lossy().to_string();
        }
        "C:\\Users\\Default\\AppData\\Local\\brum\\plugins".to_string()
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(".config/brum/plugins").to_string_lossy().to_string();
        }
        "/data/plugins".to_string()
    }
}

fn default_plugin_policy() -> String {
    "allow_all".to_string()
}
fn default_global_whitelist() -> Vec<String> {
    vec!["*".to_string()]
}
fn default_global_blacklist() -> Vec<String> {
    vec![]
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            directory: default_plugins_system_dir(),
            user_directory: default_plugins_user_dir(),
            allow_user_installs: false,
            default_policy: default_plugin_policy(),
            global_whitelist: default_global_whitelist(),
            global_blacklist: default_global_blacklist(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub nodes: Vec<FleetNodeConfig>,
}

impl Default for FleetConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            nodes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FleetNodeConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub start_path: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub color_accent: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
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

/// Preprocesses raw TOML text to automatically repair unescaped Windows backslashes in double-quoted strings.
pub fn sanitize_toml_content(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 64);

    for line in input.lines() {
        let trimmed = line.trim();
        // Skip comment lines or empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            output.push_str(line);
            output.push('\n');
            continue;
        }

        let mut repaired_line = String::with_capacity(line.len() + 16);
        let mut in_double_quote = false;
        let mut in_single_quote = false;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        let len = chars.len();

        while i < len {
            let c = chars[i];

            if in_single_quote {
                repaired_line.push(c);
                if c == '\'' {
                    in_single_quote = false;
                }
                i += 1;
                continue;
            }

            if !in_double_quote {
                if c == '#' {
                    // Comment until end of line
                    repaired_line.push_str(&chars[i..].iter().collect::<String>());
                    break;
                } else if c == '\'' {
                    in_single_quote = true;
                    repaired_line.push(c);
                    i += 1;
                } else if c == '"' {
                    // Check if triple quote
                    if i + 2 < len && chars[i + 1] == '"' && chars[i + 2] == '"' {
                        // Skip multi-line strings verbatim
                        repaired_line.push_str("\"\"\"");
                        i += 3;
                    } else {
                        in_double_quote = true;
                        repaired_line.push(c);
                        i += 1;
                    }
                } else {
                    repaired_line.push(c);
                    i += 1;
                }
                continue;
            }

            // We are inside a double-quoted string
            if c == '"' {
                in_double_quote = false;
                repaired_line.push(c);
                i += 1;
                continue;
            }

            if c == '\\' {
                if i + 1 < len {
                    let next_c = chars[i + 1];
                    if next_c == '"' {
                        // Check if this is a trailing backslash at the end of a Windows path e.g. "D:\"
                        // If there is no other quote later in the line before a comment, this quote is the closing quote!
                        let remaining = &chars[i + 2..];
                        let has_another_quote = remaining.iter().take_while(|&&ch| ch != '#').any(|&ch| ch == '"');
                        if !has_another_quote {
                            // This is a trailing backslash before closing quote! Repair: \\"
                            repaired_line.push_str("\\\\\"");
                            in_double_quote = false;
                            i += 2;
                            continue;
                        } else {
                            // Intended escaped quote \" inside string
                            repaired_line.push_str("\\\"");
                            i += 2;
                            continue;
                        }
                    } else if next_c == 'u' || next_c == 'U' {
                        // Check if valid unicode escape (4 hex for \u, 8 hex for \U)
                        let hex_len = if next_c == 'u' { 4 } else { 8 };
                        let is_hex = i + 1 + hex_len < len && chars[i + 2..=i + 1 + hex_len].iter().all(|ch| ch.is_ascii_hexdigit());
                        if is_hex {
                            repaired_line.push('\\');
                            repaired_line.push(next_c);
                            i += 2;
                            continue;
                        } else {
                            // Windows folder starting with \u... (e.g. \users) -> escape as \\
                            repaired_line.push_str("\\\\");
                            i += 1;
                            continue;
                        }
                    } else {
                        // Escape single backslash as \\ so TOML parser treats it as literal \
                        repaired_line.push_str("\\\\");
                        i += 1;
                        continue;
                    }
                } else {
                    // Backslash at end of line
                    repaired_line.push_str("\\\\");
                    i += 1;
                    continue;
                }
            } else {
                repaired_line.push(c);
                i += 1;
            }
        }

        output.push_str(&repaired_line);
        output.push('\n');
    }

    output
}

/// Config & External Theme Manager
pub struct ConfigManager;

impl ConfigManager {
    /// Parses and normalizes configuration string with smart backslash repair fallback
    pub fn parse_config_str(content: &str) -> Result<AppConfig, String> {
        match toml::from_str::<AppConfig>(content) {
            Ok(mut cfg) => {
                Self::normalize_config_paths(&mut cfg);
                Ok(cfg)
            }
            Err(e) => {
                let sanitized = sanitize_toml_content(content);
                match toml::from_str::<AppConfig>(&sanitized) {
                    Ok(mut cfg) => {
                        info!("Parsed configuration successfully after auto-repairing Windows path escape sequences");
                        Self::normalize_config_paths(&mut cfg);
                        Ok(cfg)
                    }
                    Err(sanitized_err) => {
                        Err(format!("Failed to parse config: {} (after pre-processing: {})", e, sanitized_err))
                    }
                }
            }
        }
    }

    /// Normalizes configured storage paths and expands environment variables
    pub fn normalize_config_paths(config: &mut AppConfig) {
        config.server.root_path = crate::vfs::local::expand_windows_env_vars(&config.server.root_path);
        config.storage.default_user_home_template = crate::vfs::local::expand_windows_env_vars(&config.storage.default_user_home_template);
        if let Some(ref mut trash) = config.paranoid.custom_trash_dir {
            *trash = crate::vfs::local::expand_windows_env_vars(trash);
        }

        for root in &mut config.storage.roots {
            root.path = crate::vfs::local::expand_windows_env_vars(&root.path);
            let trimmed = root.path.trim().to_string();
            if !trimmed.is_empty() {
                root.path = trimmed;
            }
            if root.id.trim().is_empty() {
                if !root.name.trim().is_empty() {
                    root.id = root.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-");
                } else if !root.path.trim().is_empty() {
                    root.id = Path::new(&root.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "storage".to_string());
                } else {
                    root.id = "storage".to_string();
                }
            }
            if root.name.trim().is_empty() {
                root.name = if !root.path.trim().is_empty() {
                    Path::new(&root.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| root.id.clone())
                } else {
                    root.id.clone()
                };
            }
        }
        config.storage.roots.retain(|r| !r.path.trim().is_empty());
    }

    /// Returns all candidate config paths in priority order:
    /// 1. Environment variable override: $BRUM_CONFIG, $CD_CONFIG, $CONFIG_PATH, $CONFIG_FILE
    /// 2. User Roaming AppData: %APPDATA%/brum/config.toml (or ~/.config/brum/config.toml)
    /// 3. User Local AppData: %LOCALAPPDATA%/brum/config.toml
    /// 4. User Roaming AppData (legacy): %APPDATA%/commanderdog/config.toml
    /// 5. User Local AppData (legacy): %LOCALAPPDATA%/commanderdog/config.toml
    /// 6. Executable directory: <exe_dir>/config.toml, <exe_dir>/brum.toml (portable mode)
    /// 7. Current working directory: ./brum.toml, ./config.toml
    /// 8. System-wide & container config: /data/config.toml, /etc/brum/config.toml, /etc/commanderdog/config.toml
    pub fn candidate_config_paths() -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        // 1. Environment Variable Overrides
        for env_var in &["BRUM_CONFIG", "CD_CONFIG", "CONFIG_PATH", "CONFIG_FILE"] {
            if let Ok(val) = std::env::var(env_var) {
                let trimmed = val.trim();
                if !trimmed.is_empty() {
                    candidates.push(PathBuf::from(trimmed));
                }
            }
        }

        // 2. User Config Dir (~/.config or %APPDATA%)
        if let Some(d) = dirs::config_dir() {
            candidates.push(d.join("brum").join("config.toml"));
            candidates.push(d.join("commanderdog").join("config.toml"));
        }

        // 3. User Local Data Dir (%LOCALAPPDATA% on Windows, ~/.local/share on Linux)
        if let Some(d) = dirs::data_local_dir() {
            candidates.push(d.join("brum").join("config.toml"));
            candidates.push(d.join("commanderdog").join("config.toml"));
        }

        // 4. Windows Explicit %LOCALAPPDATA%
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let p = PathBuf::from(local_appdata);
            candidates.push(p.join("brum").join("config.toml"));
            candidates.push(p.join("commanderdog").join("config.toml"));
        }

        // 5. Executable Directory (Portable installations)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                candidates.push(parent.join("config.toml"));
                candidates.push(parent.join("brum.toml"));
            }
        }

        // 6. Working Directory & Container Mounts
        candidates.push(PathBuf::from("./brum.toml"));
        candidates.push(PathBuf::from("./config.toml"));
        candidates.push(PathBuf::from("/data/config.toml"));
        candidates.push(PathBuf::from("/etc/brum/config.toml"));
        candidates.push(PathBuf::from("/etc/commanderdog/config.toml"));

        candidates
    }

    /// Resolves the currently active configuration path, or the preferred writable location if none exists
    pub fn active_config_path() -> PathBuf {
        for candidate in Self::candidate_config_paths() {
            if candidate.is_file() {
                return candidate;
            }
        }

        // Default write location if no existing config file was found
        if let Some(user_config) = dirs::config_dir().map(|d| d.join("brum").join("config.toml")) {
            user_config
        } else if let Some(local_config) = dirs::data_local_dir().map(|d| d.join("brum").join("config.toml")) {
            local_config
        } else {
            PathBuf::from("./config.toml")
        }
    }

    /// Loads configuration with sub-millisecond fast-path resolution across all standard paths.
    /// Also scans external theme directories (~/.config/brum/themes/*.toml, %LOCALAPPDATA%/brum/themes/*.toml).
    pub fn load_all() -> AppConfig {
        // Ensure user config and themes directories exist
        if let Some(user_config_dir) = dirs::config_dir().map(|d| d.join("brum")) {
            let _ = fs::create_dir_all(user_config_dir.join("themes"));
        }
        if let Some(local_data_dir) = dirs::data_local_dir().map(|d| d.join("brum")) {
            let _ = fs::create_dir_all(local_data_dir.join("themes"));
        }
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let _ = fs::create_dir_all(PathBuf::from(local_appdata).join("brum").join("themes"));
        }

        let mut config = AppConfig::default();

        for candidate in Self::candidate_config_paths() {
            if candidate.is_file() {
                info!("Loading master configuration: {}", candidate.display());
                match fs::read_to_string(&candidate) {
                    Ok(content) => match Self::parse_config_str(&content) {
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

        // OIDC / SSO Environment Variable Overrides
        if let Ok(v) = std::env::var("BRUM_OIDC_ENABLED").or_else(|_| std::env::var("OIDC_ENABLED")) {
            config.auth.oidc.enabled = v.eq_ignore_ascii_case("true") || v == "1";
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_ISSUER_URL").or_else(|_| std::env::var("OIDC_ISSUER_URL")) {
            if !v.trim().is_empty() { config.auth.oidc.issuer_url = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_CLIENT_ID").or_else(|_| std::env::var("OIDC_CLIENT_ID")) {
            if !v.trim().is_empty() { config.auth.oidc.client_id = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_CLIENT_SECRET").or_else(|_| std::env::var("OIDC_CLIENT_SECRET")) {
            if !v.trim().is_empty() { config.auth.oidc.client_secret = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_REDIRECT_URL").or_else(|_| std::env::var("OIDC_REDIRECT_URL")) {
            if !v.trim().is_empty() { config.auth.oidc.redirect_url = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_PROVIDER_NAME").or_else(|_| std::env::var("OIDC_PROVIDER_NAME")) {
            if !v.trim().is_empty() { config.auth.oidc.provider_name = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_ADMIN_GROUP").or_else(|_| std::env::var("OIDC_ADMIN_GROUP")) {
            if !v.trim().is_empty() { config.auth.oidc.admin_group = v.trim().to_string(); }
        }
        if let Ok(v) = std::env::var("BRUM_OIDC_FORCE_SSO").or_else(|_| std::env::var("OIDC_FORCE_SSO")) {
            config.auth.oidc.force_sso_only = v.eq_ignore_ascii_case("true") || v == "1";
        }

        config
    }

    fn collect_theme_files_sorted(dir: &Path, files: &mut Vec<PathBuf>) {
        if dir.exists() && dir.is_dir() {
            if let Ok(entries) = fs::read_dir(dir) {
                let mut dir_files: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension().map_or(false, |ext| {
                                ext.eq_ignore_ascii_case("toml") || ext.eq_ignore_ascii_case("conf")
                            })
                    })
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
        if let Some(d) = dirs::data_local_dir() {
            theme_dirs.push(d.join("brum").join("themes"));
            theme_dirs.push(d.join("commanderdog").join("themes"));
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            theme_dirs.push(PathBuf::from(&local_app_data).join("brum").join("themes"));
            theme_dirs.push(PathBuf::from(local_app_data).join("commanderdog").join("themes"));
        }
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                theme_dirs.push(parent.join("themes"));
            }
        }

        let mut theme_files = Vec::new();
        for dir in theme_dirs {
            Self::collect_theme_files_sorted(&dir, &mut theme_files);
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
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ThemeListOrNested {
            List(Vec<ThemeDefinition>),
            Nested { themes: Vec<ThemeDefinition> },
        }

        // Try parsing as multi-theme array struct: [[themes]], [themes] themes = [...], [dark], [light]
        #[derive(Deserialize)]
        struct MultiThemeContainer {
            themes: Option<ThemeListOrNested>,
            theme: Option<ThemeDefinition>,
            dark: Option<ThemeDefinition>,
            light: Option<ThemeDefinition>,
        }

        if let Ok(container) = toml::from_str::<MultiThemeContainer>(content) {
            let mut matched = false;
            if let Some(list_or_nested) = container.themes {
                let list = match list_or_nested {
                    ThemeListOrNested::List(l) => l,
                    ThemeListOrNested::Nested { themes } => themes,
                };
                for t in list {
                    Self::upsert_theme(&mut config.themes.themes, t);
                }
                matched = true;
            }
            if let Some(t) = container.theme {
                Self::upsert_theme(&mut config.themes.themes, t);
                matched = true;
            }
            if let Some(mut d) = container.dark {
                if d.id.is_empty() {
                    d.id = format!("{}-dark", file_stem);
                }
                Self::upsert_theme(&mut config.themes.themes, d);
                matched = true;
            }
            if let Some(mut l) = container.light {
                if l.id.is_empty() {
                    l.id = format!("{}-light", file_stem);
                }
                Self::upsert_theme(&mut config.themes.themes, l);
                matched = true;
            }
            if matched {
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
            folder_color: Option<String>,
            radius: Option<String>,
            panel_gap_x: Option<String>,
            panel_gap_y: Option<String>,
            app_margin: Option<String>,
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
                folder_color: flat.folder_color,
                radius: flat.radius,
                panel_gap_x: flat.panel_gap_x,
                panel_gap_y: flat.panel_gap_y,
                app_margin: flat.app_margin,
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
            home_fallback_scheme = "roots_only"
            auto_create_home_dirs = false

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
        assert_eq!(config.storage.home_fallback_scheme, "roots_only");
        assert_eq!(config.storage.auto_create_home_dirs, false);
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

    #[test]
    fn test_terminal_config_parsing() {
        let sample_toml = r#"
            [terminal]
            enabled = true
            allow_roles = ["admin", "operator"]
            allow_virtual_users = false
            drop_privileges = true
            default_shell = "/bin/bash"
        "#;

        let config: AppConfig = toml::from_str(sample_toml).unwrap();
        assert_eq!(config.terminal.enabled, true);
        assert_eq!(config.terminal.allow_roles, vec!["admin", "operator"]);
        assert_eq!(config.terminal.allow_virtual_users, false);
        assert_eq!(config.terminal.drop_privileges, true);
        assert_eq!(config.terminal.default_shell, Some("/bin/bash".to_string()));
    }

    #[test]
    fn test_windows_unescaped_backslashes_auto_repair() {
        let raw_toml = r#"
            [storage]
            allow_entire_system = true
            default_user_home_template = "C:\Users\{username}"

            [[storage.roots]]
            id = "d-drive"
            name = "D Drive"
            path = "D:\Storage\Media"
            read_only = false

            [[storage.roots]]
            id = "samba-share"
            name = "NAS Samba"
            path = "\\192.168.1.100\share\data"
            read_only = true

            [[storage.roots]]
            id = "d-root"
            name = "D Root"
            path = "D:\"
            read_only = false

            [[storage.roots]]
            id = "literal-single"
            name = "Single Quoted"
            path = 'C:\Users\Photos'
            read_only = false
        "#;

        let parsed = ConfigManager::parse_config_str(raw_toml).expect("Should parse despite unescaped backslashes");
        assert_eq!(parsed.storage.default_user_home_template, "C:\\Users\\{username}");
        assert_eq!(parsed.storage.roots.len(), 4);
        assert_eq!(parsed.storage.roots[0].path, "D:\\Storage\\Media");
        assert_eq!(parsed.storage.roots[1].path, "\\\\192.168.1.100\\share\\data");
        assert_eq!(parsed.storage.roots[2].path, "D:\\");
        assert_eq!(parsed.storage.roots[3].path, "C:\\Users\\Photos");
    }

    #[test]
    fn test_sftp_config_parsing() {
        // 1. Default fallback
        let cfg_default: AppConfig = toml::from_str("").unwrap();
        assert_eq!(cfg_default.get_sftp_config().host_key_checking, "tofu");
        assert_eq!(cfg_default.get_sftp_config().known_hosts_file, None);

        // 2. [storage.sftp] nested format
        let toml_storage = r#"
            [storage.sftp]
            host_key_checking = "strict"
            known_hosts_file = "/etc/ssh/ssh_known_hosts"
        "#;
        let cfg_storage: AppConfig = toml::from_str(toml_storage).unwrap();
        assert_eq!(cfg_storage.get_sftp_config().host_key_checking, "strict");
        assert_eq!(cfg_storage.get_sftp_config().known_hosts_file, Some("/etc/ssh/ssh_known_hosts".to_string()));

        // 3. Top-level [sftp] format
        let toml_toplevel = r#"
            [sftp]
            host_key_checking = "auto_accept"
            known_hosts_file = "~/.ssh/known_hosts"
        "#;
        let cfg_top: AppConfig = toml::from_str(toml_toplevel).unwrap();
        assert_eq!(cfg_top.get_sftp_config().host_key_checking, "auto_accept");
        assert_eq!(cfg_top.get_sftp_config().known_hosts_file, Some("~/.ssh/known_hosts".to_string()));
    }

    #[test]
    fn test_storage_roots_resilient_parsing() {
        let raw_toml = r#"
            [[storage.roots]]
            name = "Incomplete Root Without Path"

            [[storage.roots]]
            path = "/var/log"

            [[storage.roots]]
            id = "custom_id"
            name = "Custom Name"
            path = "/var/data"
        "#;

        let mut parsed = ConfigManager::parse_config_str(raw_toml).expect("Should parse even with missing fields");
        ConfigManager::normalize_config_paths(&mut parsed);
        assert_eq!(parsed.storage.roots.len(), 2);
        assert_eq!(parsed.storage.roots[0].path, "/var/log");
        assert_eq!(parsed.storage.roots[0].name, "log");
        assert_eq!(parsed.storage.roots[0].id, "log");
        assert_eq!(parsed.storage.roots[1].id, "custom_id");
        assert_eq!(parsed.storage.roots[1].name, "Custom Name");
        assert_eq!(parsed.storage.roots[1].path, "/var/data");
    }
}
