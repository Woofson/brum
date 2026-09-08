# <img src="assets/logo.png" alt="CommanderDog Logo" height="36" style="vertical-align: -6px; margin-right: 8px;" /> CommanderDog Wishlist & ChewToys Vision

> *"One Commander to replace them all."*  
> The ultimate goal of CommanderDog is to provide a single, unified, ultra-fast, cross-platform file manager that seamlessly replaces a fragmented suite of 10+ standalone utilities with built-in power-tools (**"ChewToys"**).

---

## 🧸 The "ChewToys" Replacement Suite

| Target App to Replace | Built-in ChewToy Replacement | Status / Implementation |
| :--- | :--- | :--- |
| **FileZilla & Mountain Duck** | **Multi-Protocol Remote VFS** | ✅ SFTP/SSH, SMB/CIFS, WebDAV, Proton Drive, Hetzner Storage Box, Google Drive, S3, NFS |
| **rclone & rsync** | **DeltaCopy Engine & Background Tasks** | 🔄 Fast streaming, bandwidth throttling, retry logic, background worker pool |
| **Bvckup 2 & SyncToy** | **Delta Backup & Sync Studio** | ✅ 4 replication profiles, in-place block deltas, snapshot archive, background daemon & webhooks |
| **Syncthing** | **Live Syncthing Controller** | 📋 P2P folder sync triggers, peer throughput monitor, conflict resolver |
| **PuTTY & OpenSSH SCP** | **Slide-Up PTY Web Terminal** | ✅ Embedded WebSocket pseudo-terminal (`bash`/`zsh`/`powershell`/`cmd.exe`) in active folder |
| **Total Commander / MC** | **Orthodox Multi-Pane Commander** | ✅ 1-to-4 panes, orthodox keyboard shortcuts (<kbd>F3</kbd>-<kbd>F10</kbd>), batch operations |
| **XYplorer & Directory Opus** | **Power Explorer Suite** | ✅ Resizable columns, custom column chooser, thumbnail gallery (4 sizes), directory tree sidebar, branch view |
| **Cryptomator / VeraCrypt** | **Transparent Encrypted Vaults** | ✅ AES-256-GCM + Argon2id RAM-only `.cdvault` virtual filesystem containers |
| **PDFsam / Acrobat Split** | **PDFDog Power Studio** | ✅ Pure-Rust visual PDF merge, split by page range, 90° rotation, and page organizer |
| **FastStone / Feh Viewer** | **High-DPI Media Quick View** | ✅ Mouse wheel folder browsing, focal zoom, slideshow, audio/video playback |
| **Beyond Compare / WinMerge** | **DiffDog File & Directory Compare** | ✅ Side-by-side text/code diffs with unified diff viewer; visual directory diff table |
| **HandBrake / FFmpeg GUI** | **ConvertX Media Transcoder** | ✅ In-browser media conversion (images, audio, video, documents) |
| **Classic Arcade Tetris** | **TetraDog (Arcade Tetris ChewToy)** | ✅ 60 FPS canvas clone, SRS/NES rotation, DAS/ARR tuning, high scores & synth audio |
| **Winamp / XMPlay / Foobar** | **ARFAMP (Winamp 2.x Jukebox & 10-Band EQ)** | ✅ 3-module snappable layout, windowshade mode, 10-band EQ, 60fps spectrum, .m3u PL |

---

## 🌟 Feature Wishlist & Ideas

### 1. View & UI Enhancements
- [x] **Interactive Drag-to-Resize Columns**: Drag column dividers in pane headers to adjust widths smoothly with localStorage persistence.
- [x] **Custom Column Chooser**: Toggle visible columns per pane (Permissions, Octal, Owner, Group, SHA-256 hash, Git status, Color tags).
- [x] **Thumbnail Gallery / Card View**: Adjustable preview grid (Small, Medium, Large, Extra Large) with live image/video/PDF thumbnails.
- [x] **Togglable Folder Tree Sidebar**: Classic collapsible directory tree on the left side of any pane with live active path sync.
- [x] **Flat / Branch View**: Show all files within subfolders recursively in a single flat list.
- [ ] **Synchronized Dual Scrolling**: Lock vertical scroll positions across adjacent panes for side-by-side visual folder comparisons.
- [ ] **Adaptive Viewport Typography & Form Controls**: Standardized inputs, tickboxes, and dropdowns across themes with mobile micro-text scaling.

### 2. Backup, Sync & Cloud Pipelines
- [x] **Bvckup 2 Style Delta Backup**:
  - [x] Block-level binary delta copying (transfer only modified 64KB byte blocks of large files with CRC32 chunk hashing).
  - [x] 4 core replication profiles: `synchronize` (2-way), `echo` (mirror), `contribute` (additive), `subscribe` (snapshot versioning).
  - [x] Historical snapshots & retention rules in `_archive/YYYY-MM-DD_HHmmss/`.
- [x] **Automated Backup Profiles & Background Scheduler**:
  - [x] Interval triggers (every X mins), daily scheduled time triggers (HH:MM), and continuous real-time mode.
  - [x] Automated webhook notifications (Discord/Slack/Telegram/JSON POST).
  - [x] Run history and audit logging.
- [ ] **Cloud Drive Sync Pipelines**: Background sync tasks for Google Drive, Proton Drive, and S3 buckets.

### 3. File Operations & Power Tools
- [ ] **Advanced Multi-Rename Tool (Total Commander Style)**:
  - Token replacer: `[N]`, `[E]`, `[C#]`, `[YMD]`, `[EXIF_Date]`, `[ID3_Artist]`, regex capture groups.
  - Live side-by-side preview with 1-click Undo.
- [ ] **Virtual Archive VFS**: Browse inside `.zip`, `.tar.gz`, `.7z`, `.rar`, `.iso` without full extraction.
- [ ] **Multi-Tab Workspaces**: Save and restore multi-pane arrangements and open directories as named workspace profiles.
- [ ] **Type-Ahead Quick Filter**: Instant keyboard search filtering visible rows with wildcard and regex support.
- [x] **TetraDog (Arcade Tetris Studio & Leaderboard)** (Shipped in `v0.7.7`):
  - 60 FPS HTML5 canvas engine with fixed-timestep game loop for ultra-responsiveness at master speeds.
  - DAS (Delayed Auto Shift) & ARR (Auto Repeat Rate) calibration with customizable keybindings.
  - 7-Bag randomizer, SRS wall kicks, ghost piece, hold queue, lock delay, and authentic scoring formulas.
  - Local and server-wide persistent Top 10 High Score Leaderboards.
  - Synthesized 8-bit retro Web Audio effects and multiple color palettes (Amber Charcoal, Game Boy, NES, Cyberpunk).
- [ ] **ChewToy Extensible Plugin Architecture (`.arf` / `.woof`)**:
  - External plugin packaging, manifest spec, Shell/Bash scripting bridge, and sandboxed execution.

### 4. Security & Enterprise
- [ ] **Enterprise OIDC SSO**: Authentik, Authelia, Keycloak, Okta, and Google Workspace integration alongside PAM.
- [ ] **Collaborative Office (WOPI)**: Live in-browser editing of `.docx`, `.xlsx`, `.pptx` via ONLYOFFICE / Collabora Online.
- [ ] **Cross-Platform User Profile Photos**: Local and OS user avatar auto-discovery (Linux AccountsService / `~/.face`, Windows AccountPictures).
