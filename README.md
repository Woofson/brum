# <img src="assets/logo.png" alt="CommanderDog Logo" height="36" style="vertical-align: -6px; margin-right: 8px;" /> CommanderDog

<div align="center">
  <img src="assets/CommanderDog3.png" alt="CommanderDog Desktop Edition" width="800" />
  <p><em>Multi-Tab File Commander for Web & Native Desktop (Linux & Windows) — By Woofson</em></p>
  
  <p>
    <a href="https://github.com/Woofson/commanderdog/releases/latest"><img src="https://img.shields.io/badge/version-v0.8.0-amber?style=flat-square&color=f59e0b" alt="Version" /></a>
    <img src="https://img.shields.io/badge/rust-2021_edition-orange?style=flat-square" alt="Rust 2021" />
    <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License MIT" />
    <a href="https://aur.archlinux.org/packages/commanderdog"><img src="https://img.shields.io/badge/arch_aur-commanderdog-blue?style=flat-square" alt="Arch AUR" /></a>
    <a href="https://ghcr.io/woofson/commanderdog"><img src="https://img.shields.io/badge/docker_ghcr-linux%2Famd64-cyan?style=flat-square" alt="Docker GHCR" /></a>
  </p>
</div>

---

## Why CommanderDog?

* **Blazing Fast Orthodox File Manager**: 1-to-4 dynamic panels (`Alt+1`–`4`), orthodox keyboard shortcuts (<kbd>F1</kbd>–<kbd>F10</kbd>), fast branch view, and sub-millisecond path traversal.
* **18+ Built-in Power ChewToys**: Integrated utilities replacing 10+ standalone apps (NoteDog Notes, ARFAMP Winamp 2.x Jukebox, TetraDog Arcade, PDFDog Studio, ConvertX, Slide-Up PTY Terminal, and Delta Backup).
* **Universal Remote VFS**: Direct zero-leakage client for SFTP/SSH, SMB/Windows Shares, NFS, S3 Cloud Storage, WebDAV, Proton Drive, and Hetzner Storage Box.
* **Zero-Knowledge Encrypted Vaults**: Password-protected `.cdvault` containers with Argon2id + AES-256-GCM RAM-only virtual streaming (no plaintext ever touches disk).
* **Dual Mode**: Run as a standalone native desktop app (Windows & Linux with tiling WM support) or as a headless web server.

---

## Quick Start

### Docker
```bash
docker run -d \
  --name commanderdog \
  -p 3140:3140 \
  -v ./data:/data \
  -v /home:/mnt/home:rw \
  --restart unless-stopped \
  ghcr.io/woofson/commanderdog:latest
```

### Arch Linux / CachyOS (AUR)
```bash
yay -S commanderdog        # or: paru -S commanderdog
```

### Cargo / Local Build
```bash
cargo run --release        # Open http://localhost:3140 in your browser
```

> **Default Login**: Username `admin`, Password `commanderdog` *(or log in directly with any Linux host system account via PAM)*.

---

## Built-in "ChewToys" Suite

| ChewToy | Description | Replaced Utility |
| :--- | :--- | :--- |
| **NoteDog** | Markdown notebook, interactive checklists, version snapshots, and encrypted notes | Obsidian / Joplin |
| **ARFAMP** | Authentic Winamp 2.x clone, windowshade mode, 10-band EQ, 60 FPS visualizer, .m3u PL | Winamp / XMPlay |
| **Bite! Terminal** | Slide-Up WebSocket PTY terminal with bundled Nerd Fonts in active directory | PuTTY / Web SSH |
| **Sync Studio** | Block-level binary delta replication (4 profiles), scheduler, and webhooks | Bvckup 2 / SyncToy |
| **PDFDog** | Pure-Rust visual PDF page reordering, splitting, 90° rotation, and merger | PDFsam / Acrobat |
| **ConvertX** | Browser-native image, audio, video, and document format transcoding | HandBrake / CloudConvert |
| **Vaults** | Zero-leakage AES-256-GCM in-memory encrypted virtual filesystem containers | Cryptomator / VeraCrypt |
| **TetraDog** | 60 FPS arcade canvas game, SRS rotation, DAS/ARR tuning & sync leaderboard | Desktop Distractions |
| **DiffDog** | Side-by-side text/code diffs and cryptographic directory comparison matrix | Beyond Compare / WinMerge |

---

## Documentation & User Manuals

All operational runbooks, platform guides, and security manuals are organized in [**`manuals/`**](manuals/README.md):

* [**Power Tools & ChewToys Manual**](manuals/chewtoys.md) — Notes Studio, ARFAMP, Terminal, PDF Toolkit, Vaults.
* [**Keyboard Shortcuts & Navigation**](manuals/shortcuts.md) — Orthodox <kbd>F1</kbd>–<kbd>F10</kbd> keys, ARFAMP keys, touch gestures.
* [**Configuration Guide (`config.toml`)**](manuals/configuration.md) — Master config options, storage roots, sandboxing.
* [**Remote Protocols & VFS Guide**](manuals/protocols.md) — SFTP, SMB, NFS, WebDAV, S3, Proton Drive, Hetzner.
* [**Transparent Encrypted Vaults Guide**](manuals/vaults.md) — Argon2id + AES-256-GCM in-memory containers.
* [**Windows Desktop & Packaging**](manuals/windows.md) — Winget, Scoop, NSIS Setup, MSI, and Portable ZIP.
* [**Docker Deployment Guide**](manuals/docker.md) — Compose, Portainer, and volume persistence.
* [**Proxmox VE & LXC Containers**](manuals/lxc-proxmox.md) — 1-click Debian LXC container setup.
* [**Reverse Proxy & Mesh VPN Guide**](manuals/reverse-proxy.md) — Tailscale, NetBird, Caddy 2, Nginx, Traefik, Cloudflare.
* [**Themes & Palette Guide**](manuals/themes-and-palette.md) — Woofsons Amber design tokens and custom themes.
* [**Product Roadmap**](ROADMAP.md) — Active milestones, sprint backlog, and architecture plans.
* [**Changelog**](CHANGELOG.md) — Release notes and version history.

---

## License

MIT License © [Bolt J Woofson](https://www.arf.ac) @ Woofsons Lab
