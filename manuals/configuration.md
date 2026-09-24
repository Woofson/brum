# Configuration Guide (`config.toml`)

Brum loads configuration in a single sub-millisecond pass without directory fragmentation.

---

## Configuration File Locations

Brum discovers its configuration in the following order of precedence:
1. **User Dotfiles**: `~/.config/brum/config.toml` *(Highest priority)*
2. **System-Wide Fallback**: `/etc/brum/config.toml`
3. **Custom Themes**: `~/.config/brum/themes/*.toml` (or `/etc/brum/themes/*.toml`)
4. **Environment Variables**: Overrides prefixed with `CD_` or `BRUM_` (e.g. `CD_PORT=3140`, `BRUM_OIDC_ENABLED=true`)

---

## Complete `config.toml` Reference

```toml
# ==============================================================================
# Brum Master Configuration (v1.0.0)
# ==============================================================================

[server]
# Bind address and listening port
host = "0.0.0.0"
port = 3140

# Server display identifier (shown in header badge when enabled)
server_name = "Brum Host"

# Public URL for reverse proxy / tunnel setups (optional)
# public_url = "https://files.example.com"

# Session timeout in minutes (default: 1440 = 24h, 0 for infinite)
session_timeout = 1440

[storage]
# When true, users can navigate the root filesystem (subject to OS permissions).
# When false, users are sandboxed strictly to their $HOME and [[storage.roots]].
allow_entire_system = false

# Default starting directory upon login (empty defaults to user $HOME)
default_path = ""

# Storage Roots: Explicit allowed volumes and mountpoints
[[storage.roots]]
id = "storage"
name = "Mass Storage"
path = "/mnt/storage"
read_only = false
# Optional: restrict to specific usernames (empty means accessible to all)
# allowed_users = ["admin", "bolt"]

[[storage.roots]]
id = "backups"
name = "Backup Volume"
path = "/mnt/backups"
read_only = true

[ui]
# Default number of panels on startup (1, 2, 3, 4)
default_pane_count = 2

# Default panel layout: "single", "dual-vertical", "dual-horizontal", "triple", "quad"
default_layout = "dual-vertical"

# Show hidden files (dotfiles) by default
show_hidden_files = true

# Fast optimistic cache for 0ms sub-millisecond panel directory rendering
fast_cache = true

# Startup directory restoration behavior:
# - "last_state" : Restore the exact paths open in each panel when last closed
# - "home"       : Open user $HOME in all panels
# - "custom"     : Open explicit paths defined in startup_path_left / startup_path_right
startup_mode = "last_state"
startup_path_left = ""
startup_path_right = ""

# When startup_mode is "last_state", fall back to $HOME if the previous path was a remote host
remote_startup_fallback = "home"

# Window decorations (set to false for frameless mode on Hyprland/Sway/i3)
window_decorations = true

# Custom window title
window_title = "Brum"

# Show hostname badge in top header
show_hostname_badge = true
hostname_badge = "" # Custom override text (leave empty for auto OS/host detection)

# Date display format: "iso" (YYYY-MM-DD), "us" (MM/DD/YYYY), "eu" (DD.MM.YYYY)
date_format = "iso"

[desktop]
# Minimize application to system tray on window close (Native desktop mode)
minimize_to_tray = true

# Enable desktop system tray icon
enable_tray = true

# Global desktop hotkey to summon/hide Brum
global_summon_hotkey = "Super+C"

[themes]
# Active default theme:
# - "amber-charcoal" (Woofsons Amber Charcoal - Dark / Default)
# - "zink" (Woofsons Amber Zink - Light)
# - "skumring", "demring", "trollnatt", "myrtaake" (Larvikite series)
# - "bergtatt", "soria-moria", "pestanatt", "sotslette" (Kittelsen series)
# - "gruvbox", "catppuccin-mocha", "tokyo-night", "nord", "dracula", etc.
default_theme = "amber-charcoal"

[auth]
# Authentication backend: "pam" (Linux system users) or "internal" (SQLite) or "mixed"
backend = "pam"

# Allow guest / anonymous read-only browsing (default: false)
allow_guest = false

# Session inactivity lock timeout in minutes (0 to disable)
inactivity_timeout = 15

[auth.oidc]
# OpenID Connect / Authentik / Keycloak / Authelia SSO
enabled = false
provider_name = "Authentik"
issuer_url = "https://auth.example.com/application/o/brum/"
client_id = "brum-sso"
client_secret = "your-sso-client-secret"
redirect_url = "https://files.example.com/api/auth/oidc/callback"
scopes = ["openid", "profile", "email", "groups"]
auto_provision = true
admin_group = "brum-admins"
default_user_role = "user"
default_home_template = "/home/{username}"
force_sso_only = false
button_icon = "shield-check"

[plugins]
# Modular Chewtoy plugins configuration
enabled = true
directory = "/etc/brum/plugins"
user_directory = "/data/plugins"
allow_user_installs = true
default_policy = "allow_all" # "allow_all" | "whitelist" | "blacklist"
global_whitelist = ["*"]
global_blacklist = []
```

---

## Filesystem Sandboxing & Storage Roots

By setting `allow_entire_system = false`, Brum enforces strict sandboxing:
* Users cannot navigate outside their configured storage roots or personal `$HOME`.
* Directory traversal attacks (`../`) are safely rejected and sanitized at the kernel VFS layer.
* Read-only flags (`read_only = true`) prevent accidental deletions, writes, or moves.

---

## Instant 0ms Startup & Cache Optimizations

Brum achieves instantaneous UI rendering and directory browsing through:
1. **Optimistic Local VFS Pre-Rendering**: Panes render immediately using fast cache metadata while asynchronous re-validation occurs in the background.
2. **State Persistence**: When `startup_mode = "last_state"`, active panel tabs and directory paths restore seamlessly without UI flicker.
3. **Remote Fallback Protection**: Setting `remote_startup_fallback = "home"` ensures that if a pane was previously connected to an unavailable remote node or SFTP server, Brum falls back to `$HOME` instantly rather than blocking on network timeouts.
