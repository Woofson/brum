# <img src="../assets/logo.png" alt="CommanderDog Logo" height="36" style="vertical-align: -6px; margin-right: 8px;" /> Configuration Guide (`config.toml`)

CommanderDog loads configuration in a single sub-millisecond pass without directory fragmentation.

---

## Configuration File Locations

CommanderDog discovers its configuration in the following order of precedence:
1. **User Dotfiles**: `~/.config/commanderdog/config.toml` *(Highest priority)*
2. **System-Wide Fallback**: `/etc/commanderdog/config.toml`
3. **Custom Themes**: `~/.config/commanderdog/themes/*.toml` (or `/etc/commanderdog/themes/*.toml`)

---

## Complete `config.toml` Reference

```toml
# ==============================================================================
# CommanderDog Master Configuration
# ==============================================================================

[server]
# Bind address and listening port
host = "0.0.0.0"
port = 3140

# Server display identifier (shown in header badge when enabled)
server_name = "CommanderDog Host"

# Public URL for reverse proxy / tunnel setups (optional)
# public_url = "https://commander.example.com"

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

# Window decorations (set to false for frameless mode on Hyprland/Sway/i3)
window_decorations = true

# Show hostname badge in top header
show_hostname_badge = true
hostname_badge = "" # Custom override text (leave empty for auto OS/host detection)

# Date display format: "iso" (YYYY-MM-DD), "us" (MM/DD/YYYY), "eu" (DD.MM.YYYY)
date_format = "iso"

[desktop]
# Minimize application to system tray on window close
minimize_to_tray = true

# Enable desktop system tray icon
enable_tray = true

# Global desktop hotkey to summon/hide CommanderDog
global_summon_hotkey = "Super+C"

[themes]
# Active default theme:
# - "amber-charcoal" (Woofsons Amber Charcoal - Dark / Default)
# - "zink" (Woofsons Amber Zink - Light)
# - "gruvbox-dark", "catppuccin-mocha", "tokyo-night", "nord", "dracula", etc.
default_theme = "amber-charcoal"

[auth]
# Authentication backend: "pam" (Linux system users) or "internal" (SQLite)
backend = "pam"

# Allow guest / anonymous read-only browsing (default: false)
allow_guest = false
```

---

## Filesystem Sandboxing & Storage Roots

By setting `allow_entire_system = false`, CommanderDog enforces strict sandboxing:
* Users cannot navigate outside their configured storage roots or personal `$HOME`.
* Directory traversal attacks (`../`) are safely rejected and sanitized at the kernel VFS layer.
* Read-only flags (`read_only = true`) prevent accidental deletions, writes, or moves.
