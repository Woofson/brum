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
# Brum Master Configuration (v1.12.4)
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
server_name = ""                 # Friendly node name; also matches WebSocket/CORS origins with/without http:// or https://
allow_web_config_edit = true     # Web Configuration Editor toggle in Settings (sensitive secrets masked as ********)
# jwt_secret = "..."             # Master cluster signing key (prefer injecting via BRUM_JWT_SECRET in /etc/brum/brum.env)

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
default_admin_pass = "brum"      # Initial password for fresh DB only (generates random 20-char password if left as "brum" or blank)

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
# token = ""                       # Leave empty for shared BRUM_JWT_SECRET cluster federation; or provide a static API token
# start_path = "/mnt/storage"      # Initial path (auto-syncs from config unless overridden by user in Fleet Manager)
# read_only = false
# color_accent = "#f59e0b"
# tags = ["homelab", "nas", "tailscale"]

# --- Modular Chewtoys & Plugins ---
[plugins]
enabled = true
directory = "/etc/brum/plugins"
user_directory = "~/.config/brum/plugins" # Auto-expands leading tilde (~) to user's home directory
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
  * Example output: `Brum v1.12.4 (build #419 · commit 8f42092 · 2026-09-30 14:00:00 UTC · x86_64-pc-windows-gnu)`
* Query status and build info via REST API: `GET /api/system/status` or `GET /api/health`
* Inspect build metadata in the UI: Open **Settings (Tab 10: About)** or the standalone **About Brum** modal to view the build number (`#419`), git commit hash, and UTC compilation timestamp.

---

## Instant 0ms Startup & Cache Optimizations

Brum achieves instantaneous UI rendering and directory browsing through:
1. **Optimistic Local VFS Pre-Rendering**: Panes render immediately using fast cache metadata while asynchronous re-validation occurs in the background.
2. **State Persistence**: When `startup_mode = "last_state"`, active panel tabs and directory paths restore seamlessly without UI flicker.
3. **Remote Fallback Protection**: Setting `remote_startup_fallback = "home"` ensures that if a pane was previously connected to an unavailable remote node or SFTP server, Brum falls back to `$HOME` instantly rather than blocking on network timeouts.

---

## Web Configuration Editor & Sensitive Secrets Masking

Brum includes a built-in Web Configuration Editor accessible in **Settings -> Raw Config**:

* **Sensitive Secrets Masking**: When viewing the configuration in the Web UI or querying `/api/system/raw-config`, sensitive keys (`jwt_secret`, `default_admin_pass`, `client_secret`, etc.) are automatically redacted as `********`.
* **Safe Secret Preservation**: When saving edits in the Web UI, Brum merges the updated TOML with the on-disk file. Any key left as `********` automatically preserves its existing secret value on disk, allowing administrators to edit unrelated settings without accidentally wiping secrets.
* **Disabling Web Editing**: To enforce strict SSH/CLI-only administration, set `allow_web_config_edit = false` in `config.toml` or set the environment variable:
  ```bash
  BRUM_ALLOW_WEB_CONFIG_EDIT=false
  ```
  When disabled, the Web UI displays the configuration in read-only mode and rejects write attempts with `403 Forbidden`.

---

## Initial Administrative Credentials & Seeding Protocol

`default_admin_pass` in `config.toml` operates under specific security rules:

1. **One-Time Database Seeding Only**:
   The `default_admin_pass` setting is evaluated **only** on the very first startup when the SQLite database (`brum.db`) contains zero users. Once seeded, user credentials are stored as Argon2 hashes in the database. Editing `default_admin_pass` in `config.toml` afterwards has **no effect** on existing accounts. Passwords must be updated via the UI (**Settings -> Security**) or by modifying the database.
2. **Insecure Default Guard (`"brum"` vs Custom Password)**:
   * **If left as `"brum"` (or blank)**: Brum deliberately rejects `"brum"` as an insecure default and generates a cryptographically random 20-character password on startup. It logs the credentials to stdout and writes them to `initial_admin_credentials.txt` in the configured log directory. Logging in with `admin` / `brum` will fail.
   * **If set to a custom password before first boot**: The configured custom password is used verbatim for the initial account.

---

## Fleet Federation, Defaults & User Overrides

When connecting multiple Brum instances across local subnets or overlay mesh networks (Tailscale, NetBird, WireGuard):

### 1. Shared Cluster Secret (`BRUM_JWT_SECRET`)
To federate nodes without managing static per-node API tokens:
* Inject the identical `BRUM_JWT_SECRET` string on all nodes via `/etc/brum/brum.env` (`chmod 600`).
* In `[[fleet.nodes]]`, set `token = ""` (or omit `token`).
* When `token` is empty, Brum's Gateway Hub automatically forwards the caller's active session JWT. Because both nodes share the master signing secret, the remote node validates the signature and authenticates the user instantly.
* **Static API Tokens**: If a remote node does *not* share the same secret key, create a static API Token on that node (**Settings -> Security -> API Tokens**) and set `token = "<your_token>"` in `config.toml`.

### 2. Config Defaults vs Fleet Manager Overrides
* **Automatic Ingestion**: Statically configured nodes in `config.toml` are automatically synced into the browser's UI (appearing in **Fleet Control** and the **Paw Menu**).
* **Auto-Syncing Updates**: Modifying a node in `config.toml` (e.g. changing `start_path` or `url`) automatically updates the browser on next load, avoiding stale cached paths.
* **User Overrides**: Users can customize a static node in the **Fleet Manager** (changing its start directory, display name, color accent, or routing mode) and click **Save Profile**. This tags the node with `user_override: true` and preserves personal preferences over `config.toml`.
* **One-Click Reset**: To discard custom overrides and revert to `config.toml` settings, click the amber **Reset** button (`rotate-ccw`) on the node card in Fleet Manager.

### 3. Zero CORS Requirements on Gateway Proxy
When routed through the Fleet Gateway Hub (`/api/fleet/proxy/:node_id`), all communication is proxied server-to-server. Remote nodes **do not** require the coordinator's domain in their `allowed_origins` whitelist.

---

## Environment Variable Overrides Matrix

Brum supports seamless runtime overrides across containerized, systemd, and homelab environments:

| Environment Variable | Overrides Key | Description | Example |
| :--- | :--- | :--- | :--- |
| `BRUM_PORT` / `CD_PORT` / `PORT` | `server.port` | HTTP listening port | `3140` |
| `BRUM_BIND` / `BRUM_HOST` / `HOST` | `server.host` | Network interface binding | `0.0.0.0` or `127.0.0.1` |
| `BRUM_DATABASE_PATH` | `server.database_path` | SQLite database file location | `/var/lib/brum/brum.db` |
| `BRUM_JWT_SECRET` | `server.jwt_secret` | Master cluster JWT signing key | `64-hex-char-secret` |
| `BRUM_ALLOW_WEB_CONFIG_EDIT` | `server.allow_web_config_edit` | Enable/disable Web Config Editor | `true` or `false` |
| `BRUM_OIDC_ENABLED` | `auth.oidc.enabled` | Enable OpenID Connect SSO | `true` or `false` |
| `BRUM_OIDC_ISSUER_URL` | `auth.oidc.issuer_url` | OIDC IdP Discovery Issuer URL | `https://auth.domain.com/` |
| `BRUM_OIDC_CLIENT_ID` | `auth.oidc.client_id` | OAuth2 Client ID | `brum-app` |
| `BRUM_OIDC_CLIENT_SECRET` | `auth.oidc.client_secret` | OAuth2 Client Secret | `secret-string` |
| `BRUM_OIDC_REDIRECT_URL` | `auth.oidc.redirect_url` | OAuth2 Callback Redirect URL | `https://domain.com/api/auth/oidc/callback` |
| `BRUM_OIDC_ADMIN_GROUP` | `auth.oidc.admin_group` | IdP claim group mapped to admin | `brum-admins` |
| `BRUM_OIDC_FORCE_SSO` | `auth.oidc.force_sso_only` | Disable local password login form | `true` or `false` |


