# 📦 Brum Installation & Build Guide

Welcome to the comprehensive installation and compilation manual for **Brum** ("Multi-Pane Web Environment (File Commander/Manager) - By Woofson").

---

## 📋 Quick Selection: How Do You Want to Run Brum?

| Deployment Target | Build / Command | Output Mode |
| :--- | :--- | :--- |
| **Native Desktop (Tiling WM / Linux)** | `cargo build --release --features gui`<br>`./target/release/brum -s --frameless` | Standalone native WebKit window (No browser, borderless) |
| **Native Desktop (Standard Linux / Windowed)** | `cargo build --release --features gui`<br>`./target/release/brum -s` | Standalone native desktop window with titlebar |
| **Arch Linux / CachyOS (AUR)** | `yay -S brum` | Pre-configured native desktop + CLI package |
| **Windows Desktop** | `winget install Woofson.Brum`<br>or `scoop install brum` | Native Windows app (`Brum.exe` with System Tray) |
| **Headless Server (Web Environment)** | `cargo build --release`<br>`./target/release/brum --server` | Pure background web service on `http://0.0.0.0:3140` |
| **Docker / Proxmox Container** | `docker compose up -d` | Minimal container on Alpine Linux (`ghcr.io/woofson/brum`) |

---

## 1. 🖥️ Native Desktop Standalone Mode (Linux)

### Build Dependencies (Debian / Ubuntu / Arch)
Before compiling with the native GUI windowing engine (`wry` + `tao`), ensure GTK3 and WebKitGTK development headers are installed:

```bash
# Arch Linux / CachyOS / Manjaro
sudo pacman -S webkit2gtk-4.1 gtk3 pkg-config openssl libssh2 sqlite

# Ubuntu 22.04+ / Debian 12+ / Linux Mint / Pop!_OS
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev pkg-config libssl-dev libssh2-1-dev libsqlite3-dev
```

### Compiling with Native Desktop GUI
To compile the standalone binary with embedded WebKitGTK desktop windowing:

```bash
# Build optimized release binary
cargo build --release --features gui

# Binary is located at:
./target/release/brum
```

### Launching on Tiling Window Managers (Hyprland, Sway, i3, bspwm)
For tiling window managers, launch with `-s` (standalone local user) and `--frameless` (borderless mode without window titlebar):

```bash
# Direct run
./target/release/brum -s --frameless

# Or install globally into ~/.cargo/bin/
cargo install --path . --features gui
brum -s --frameless
```

#### Hyprland Window Rule Example (`hyprland.conf`)
```ini
# Floating or tiled window rules
windowrulev2 = opacity 0.96 0.96, class:^(Brum)$
bind = $mainMod, E, exec, brum -s --frameless
```

---

## 2. 🐧 Arch Linux AUR Installation

Brum is available in the Arch User Repository with automated compilation of the native desktop GUI:

```bash
# Using yay
yay -S brum

# Using paru
paru -S brum

# Binary pre-compiled release (instant install)
yay -S brum-bin
```

---

## 3. 🪟 Windows Desktop Standalone

Windows users can install Brum via package managers or native installers:

### Via Winget
```powershell
winget install Woofson.Brum
```

### Via Scoop
```powershell
scoop bucket add woofson https://github.com/Woofson/scoop-bucket.git
scoop install brum
```

### Manual Release Installers
Download standalone portable ZIP, NSIS setup `.exe`, or `.msi` from [GitHub Releases](https://github.com/Woofson/brum/releases). For more details, see [**`manuals/windows.md`**](manuals/windows.md).

---

## 4. 🌐 Headless Web Server Mode (Linux / Servers / NAS)

If deploying as a headless network storage server or remote commander:

```bash
# Build headless binary (no GTK/WebKit dependencies required)
cargo build --release

# Run web service
./target/release/brum --server --port 3140 --host 0.0.0.0
```

---

## 5. 🐳 Docker & Homelab Deployment

See the comprehensive [**`manuals/docker.md`**](manuals/docker.md) and [**`manuals/lxc-proxmox.md`**](manuals/lxc-proxmox.md) guides:

```bash
# Quick Docker run
docker run -d \
  --name brum \
  -p 3140:3140 \
  -v /home:/mnt/home:rw \
  -v /mnt/storage:/mnt/storage:rw \
  --restart unless-stopped \
  ghcr.io/woofson/brum:latest
```

---

## ⚙️ CLI Flags Reference

| Flag | Description |
| :--- | :--- |
| `-s`, `--standalone` | Runs in standalone desktop mode (auto-authenticates as `$USER`, bypasses login) |
| `--frameless`, `--no-decorations` | Launches without window titlebars/frame (ideal for tiling WMs) |
| `--decorations` | Explicitly forces window titlebar and borders |
| `--server`, `--headless` | Runs in headless background server mode |
| `-p`, `--port <PORT>` | Overrides server bind port (default: `3140` or `config.toml`) |
| `--host <HOST>` | Overrides bind address (default: `0.0.0.0`) |
| `--no-auth` | Disables authentication globally |
| `-v`, `--version` | Displays current version (`0.8.2`) |

