# Configuration Guide (`config.toml`)

Brum loads configuration in a single sub-millisecond pass without directory fragmentation.

---

## Configuration File Locations

Brum discovers its configuration in the following order of precedence:
1. **Command-Line Option**: `--config /path/to/config.toml` *(Explicit override)*
2. **User Configuration**:
   - Linux/macOS: `~/.config/brum/config.toml`
   - Windows: `%APPDATA%\Brum\config.toml` *(e.g. `C:\Users\<User>\AppData\Roaming\Brum\config.toml`)*
3. **System-Wide Configuration**:
   - Linux/macOS: `/etc/brum/config.toml`
   - Windows: `%PROGRAMDATA%\Brum\config.toml` *(e.g. `C:\ProgramData\Brum\config.toml` for Windows Service)*
4. **Local Directory Fallback**: `./config.toml`
5. **Custom Themes**: `~/.config/brum/themes/*.toml`, `/etc/brum/themes/*.toml`, `%PROGRAMDATA%\Brum\themes\*.toml`
6. **Environment Variables**: Overrides prefixed with `CD_` or `BRUM_` (e.g. `CD_PORT=3140`, `BRUM_OIDC_ENABLED=true`)

---

## Complete `config.toml` Reference

```toml
# ==============================================================================
# Brum Master Configuration (v1.8.8)
# ==============================================================================

# --- Core Server Settings ---
[server]
host = "0.0.0.0"                 # "0.0.0.0" (all interfaces), "127.0.0.1" (local-only), or specific overlay IP (e.g. "100.64.0.1")
port = 3140                      # Brum default port
root_path = "/"                  # Root filesystem path ("/" on Linux, "C:\\" on Windows)
upload_max_size_mb = 20480       # Max file upload size in MB (20 GB)
enable_auth = true               # Enable authentication (automatically disabled in --standalone desktop mode)
session_duration_hours = 72      # JWT session expiry duration in hours
database_path = "brum.db"        # SQLite local database
server_name = ""                 # Optional friendly node name displayed in UI badges and fleet discovery

# --- Trusted Reverse Proxies ---
# CIDRs or IP addresses of reverse proxies (Caddy, Nginx, Traefik, HAProxy, Cloudflare Tunnel, Tailscale Funnel)
# Forwarded headers (X-Forwarded-For, X-Forwarded-Host, X-Real-IP) are only honored from these IPs.
trusted_proxies = [
    # "127.0.0.1",
    # "::1",
    # "10.0.0.0/8",
    # "172.16.0.0/12",
    # "192.168.0.0/16",
    # "100.64.0.0/10",           # RFC 6598 CGNAT / Tailscale / NetBird
]

# --- Allowed CORS & WebSocket Origins ---
# Origins permitted for the Web Terminal (Bite!), remote fleet tunnels, and API calls.
# Brum automatically permits loopback, *.local, *.lan, *.home.arpa, RFC 1918 LANs, and 100.64.0.0/10 overlay IPs.
# For custom public FQDNs or MagicDNS domains, explicitly add them:
allowed_origins = [
    # "https://brum.tailnet-name.ts.net",
    # "https://*.ts.net",
    # "https://*.netbird.cloud",
    # "https://files.homelab.internal",
    # "http://100.*:*",
]

# --- Authentication & Access Control ---
[auth]
mode = "mixed"                   # Options: "mixed" (PAM + Built-in), "builtin", "pam", "none"
pam_service = "login"            # Linux PAM service name
allow_guest = false              # Allow anonymous guest read-only access
default_admin_user = "admin"
default_admin_pass = "brum"

# --- OpenID Connect (OIDC / SSO) ---
[auth.oidc]
enabled = false
provider_name = "Authentik"      # Display name on login button
issuer_url = ""                  # e.g. "https://auth.homelab.lan/application/o/brum/"
client_id = ""
client_secret = ""
redirect_url = ""                # e.g. "https://brum.homelab.lan/api/auth/oidc/callback"
scopes = ["openid", "profile", "email", "groups"]
auto_provision = true
admin_group = "brum-admins"
default_user_role = "user"
default_home_template = "/home/{username}"
force_sso_only = false
button_icon = "shield-check"

# --- Storage Roots & Filesystem Sandboxing ---
[storage]
allow_entire_system = true
allow_remote_loopback = false
default_user_home_template = "/home/{username}"
home_fallback_scheme = "auto"    # "auto", "roots_only", "strict"
auto_create_home_dirs = true

# Storage Roots: Explicit allowed volumes and mountpoints
# [[storage.roots]]
# id = "storage"
# name = "Mass Storage"
# path = "/mnt/storage"
# read_only = false
# allowed_roles = ["admin", "user"]

# [[storage.roots]]
# id = "backups"
# name = "Backups (Read-Only)"
# path = "/mnt/backups"
# read_only = true

[storage.sftp]
host_key_checking = "tofu"       # "tofu" (Trust On First Use), "strict", "auto_accept"
# known_hosts_file = "~/.ssh/known_hosts"

# --- Multi-Node Fleet Management ---
[fleet]
enabled = true

# [[fleet.nodes]]
# id = "homelab-nas"
# name = "Homelab NAS (Tailscale)"
# url = "http://100.64.0.15:3140"
# token = "node_token_here"
# start_path = "/mnt/storage"
# read_only = false
# color_accent = "#f59e0b"
# tags = ["homelab", "nas", "tailscale"]

# --- Modular Chewtoys & Plugins ---
[plugins]
enabled = true
directory = "/etc/brum/plugins"
user_directory = "~/.config/brum/plugins"
allow_user_installs = false
default_policy = "allow_all"     # "allow_all", "whitelist", "blacklist"
global_whitelist = ["*"]
global_blacklist = []

# --- Logging & Diagnostics ---
[logging]
level = "info"                   # "trace", "debug", "info", "warn", "error"
file_enabled = true
directory = ""                   # Empty defaults to /var/log/brum or ~/.local/share/brum/logs
rotation = "daily"               # "daily", "hourly", "never"
max_files = 14
console_enabled = true

# --- User Interface & Panels ---
[ui]
default_pane_count = 2           # 1 to 4
default_layout = "dual-vertical" # "single", "dual-vertical", "dual-horizontal", "triple", "quad"
show_hidden_files = true
default_view_mode = "details"    # "details", "compact", "grid"
window_decorations = true        # Set false for borderless/frameless mode in tiling WMs
show_global_refresh = false
show_hostname_badge = true
hostname_badge = ""
hostname_color = "slate"
hostname_style = "text"
hostname_icon = "none"
hostname_size = "md"
window_title = ""

# --- Web Terminal (Bite!) & PTY Security ---
[terminal]
enabled = true
allow_roles = ["admin", "root"]
allow_virtual_users = false
drop_privileges = true
# default_shell = "/bin/bash"

# --- Standalone Desktop Window & System Tray ---
[desktop]
minimize_to_tray = true
enable_tray = true
global_summon_hotkey = "Super+C"
start_minimized = false

# --- Color Themes & Aesthetics ---
[themes]
default_theme = "amber-charcoal" # "amber-charcoal", "zink", "catppuccin-mocha", "nord", "gruvbox", etc.

# --- Paranoid File Integrity & Safety ---
[paranoid]
enabled = true
checksum_algorithm = "sha256"
verify_after_transfer = true
atomic_writes = true
trash_enabled = true
confirm_delete = true
confirm_overwrite = true
windows_native_file_ops = true
detect_locking_processes = true

# --- Syncthing P2P Integration ---
[syncthing]
enabled = true
url = "http://127.0.0.1:8384"
api_key = ""

# --- Notes & Markdown Studio ---
[notedog]
notes_folder = "~/Notes"
```

---

## Filesystem Sandboxing & Storage Roots

By setting `allow_entire_system = false`, Brum enforces strict sandboxing:
* Users cannot navigate outside their configured storage roots or personal `$HOME`.
* Directory traversal attacks (`../`) are safely rejected and sanitized at the kernel VFS layer.
* Read-only flags (`read_only = true`) prevent accidental deletions, writes, or moves.

---

## Automated SemVer Build Metadata

Brum automatically embeds full build metadata at compile time adhering to **SemVer 2.0.0 (Section 10)**:
* Query build details via CLI: `brum --version` or `brum.exe -v`
  * Example output: `Brum v1.8.8 (build #419 · commit 8f42092 · 2026-09-30 14:00:00 UTC · x86_64-pc-windows-gnu)`
* Query status and build info via REST API: `GET /api/system/status` or `GET /api/health`
* Inspect build metadata in the UI: Open **Settings (Tab 10: About)** or the standalone **About Brum** modal to view the build number (`#419`), git commit hash, and UTC compilation timestamp.

---

## Instant 0ms Startup & Cache Optimizations

Brum achieves instantaneous UI rendering and directory browsing through:
1. **Optimistic Local VFS Pre-Rendering**: Panes render immediately using fast cache metadata while asynchronous re-validation occurs in the background.
2. **State Persistence**: When `startup_mode = "last_state"`, active panel tabs and directory paths restore seamlessly without UI flicker.
3. **Remote Fallback Protection**: Setting `remote_startup_fallback = "home"` ensures that if a pane was previously connected to an unavailable remote node or SFTP server, Brum falls back to `$HOME` instantly rather than blocking on network timeouts.
