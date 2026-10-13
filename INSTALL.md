# Brum Installation & Build Guide

Welcome to the installation and compilation manual for **Brum** ("Multi-Pane Web Environment (File Commander/Manager) - By Woofson").

---

## Quick Selection: How Do You Want to Run Brum?

| Deployment Target | Build / Command | Output Mode |
| :--- | :--- | :--- |
| **Standalone Desktop (Linux / macOS)** | `cargo build --release`<br>`./target/release/brum -s` | Standalone browser/PWA window with local OS auto-auth |
| **Tiling WMs (Hyprland / Sway / i3)** | `chromium --app=http://127.0.0.1:3140` | Dedicated borderless, isolated desktop window |
| **Arch Linux / CachyOS (AUR)** | `yay -S brum` (or `yay -S brum-bin`) | Single binary package (`/usr/bin/brum`) |
| **Windows Desktop & CLI** | `winget install Woofson.Brum`<br>or `scoop install brum` | Universal `brum.exe` binary with Windows NT service runner |
| **Headless Server (Web Environment)** | `cargo build --release`<br>`./target/release/brum --server` | Background web service on `http://0.0.0.0:3140` |
| **Docker / Proxmox Container** | `docker compose up -d` | Minimal container on Alpine Linux (`ghcr.io/woofson/brum:latest`) |

---

## 1. Standalone Desktop & Tiling Window Managers

Brum is built from the ground up as a pure web environment. It compiles as a single, universal Rust binary with zero C graphical dependencies (`wry`, `tao`, `gtk3`, `webkit2gtk`), allowing it to build and run seamlessly across desktop workstations and headless servers alike.

### Compiling from Source
```bash
# Build optimized release binary (Linux / macOS / Windows)
cargo build --release

# Binary is located at:
./target/release/brum
```

### Launching Standalone Desktop Mode
Launch with `-s` (standalone local user) to automatically authorize your current local operating system account and open the interface:

```bash
# Direct run (opens in your default browser or PWA)
./target/release/brum -s

# Or install globally into ~/.cargo/bin/
cargo install --path .
brum -s
```

### Frameless Window Mode (Tiling WMs & Purist Browsers)
For a completely borderless, isolated desktop window without browser tabs or URL bars:

* **Chromium / Brave / Chrome / Edge**:
  ```bash
  chromium --app=http://127.0.0.1:3140
  ```
* **Hyprland Window Rule Example (`hyprland.conf`)**:
  ```ini
  windowrulev2 = opacity 0.96 0.96, class:^(chrome-127.0.0.1.*)$
  bind = $mainMod, E, exec, brum -s && chromium --app=http://127.0.0.1:3140
  ```
* **Qutebrowser / Udo / Surf**:
  ```bash
  qutebrowser http://127.0.0.1:3140
  ```

---

## 2. Arch Linux AUR Installation

Brum is available in the Arch User Repository:

```bash
# Using yay (source build)
yay -S brum

# Using paru
paru -S brum

# Binary pre-compiled release (instant install)
yay -S brum-bin
```

---

## 3. Windows Desktop & CLI Standalone

Windows users can install Brum via package managers or standalone portable ZIP:

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
Download standalone portable `brum-v<VERSION>-windows-x86_64.zip` from [GitHub Releases](https://github.com/Woofson/brum/releases). For more details, see [**`manuals/windows.md`**](manuals/windows.md).

---

## 4. Headless Web Server Mode (Linux / Servers / NAS)

If deploying as a headless network storage server or remote commander:

```bash
# Build headless binary (no GTK/WebKit dependencies required)
cargo build --release

# Run web service
./target/release/brum --server --port 3140 --host 0.0.0.0
```

---

## 5. Docker & Homelab Deployment

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

## 6. Optional System Dependencies (Format Converter)

Brum is distributed as a single static binary with embedded web assets. Core file management, PTY terminal emulation, encrypted `.cdvault` vaults, and note management require **zero external runtime dependencies**.

To unlock high-performance audio/video and image transcoding in **Format Converter (ConvertX)**, install `ffmpeg` and `ImageMagick` on your host, container, or desktop:

| Operating System | Package Manager / Install Command |
| :--- | :--- |
| **Debian / Ubuntu / Proxmox LXC** | `sudo apt-get install -y ffmpeg imagemagick` |
| **Arch Linux / CachyOS / Manjaro** | `sudo pacman -S ffmpeg imagemagick` |
| **Fedora / RHEL / Rocky Linux** | `sudo dnf install -y ffmpeg imagemagick` |
| **Alpine Linux (Docker)** | `apk add --no-cache ffmpeg imagemagick` |
| **Windows 10 / 11** | `winget install Gyan.FFmpeg ImageMagick.ImageMagick`<br>or `scoop install ffmpeg imagemagick` |
| **macOS (Homebrew)** | `brew install ffmpeg imagemagick` |

> [!NOTE]
> When transcoding files on remote nodes (SFTP, SMB, SSH Fleet Nodes), Brum executes the conversion server-side on the host where the media resides, provided `ffmpeg` is available on that remote host.

---

## CLI Flags & Commands Reference

```bash
brum [OPTIONS]
brum service [install|uninstall|start|stop|restart|status|run]
brum shortcut [install|uninstall] [--desktop]
```

### Options

| Flag | Description |
| :--- | :--- |
| `-s`, `--standalone` | Runs in standalone desktop mode (auto-authenticates as local user `$USER`, opens default browser/window) |
| `-c`, `--connect <URL>` | Connects desktop GUI/browser directly to an existing local or remote Brum server |
| `-o`, `--open` | Automatically opens Brum in the default web browser or desktop webview |
| `--frameless`, `--no-decorations` | Launches without window titlebars/frame (ideal for Hyprland, Sway, i3, and tiling WMs) |
| `--decorations` | Explicitly forces window titlebar and borders |
| `--minimized` | Launches minimized in system tray / background without opening window immediately |
| `--server`, `--headless` | Runs in headless background server mode |
| `-p`, `--port <PORT>` | Overrides web server port (default: `3140` or `config.toml` setting) |
| `--host <HOST>` | Overrides bind host address (default: `0.0.0.0`) |
| `--no-auth` | Disables authentication globally and runs with local permissions |
| `-v`, `--version` | Displays current version and build number |
| `-h`, `--help` | Displays help message |

### Service Commands (`brum service <cmd>`)

Manages background services (systemd user service on Linux, Windows NT Service on Windows):

| Command | Action |
| :--- | :--- |
| `install` | Registers Brum as a systemd user service (`~/.config/systemd/user/brum.service`) or Windows NT Service |
| `uninstall` | Removes the registered background service unit |
| `start` | Starts the background service |
| `stop` | Stops the running background service |
| `restart` | Restarts the background service |
| `status` | Queries service execution status |
| `run` | Executes the service dispatcher directly |

### Shortcut Commands (`brum shortcut <cmd>`)

| Command | Action |
| :--- | :--- |
| `install [--desktop]` | Installs desktop application menu shortcut (and optional Desktop shortcut) |
| `uninstall` | Removes application menu and desktop shortcuts |
