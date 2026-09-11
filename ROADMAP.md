# <img src="assets/brum_commanderdog_legacy.webp" alt="Brum Logo" height="40" style="vertical-align: -6px; margin-right: 8px;" /> Brum Product Roadmap & Architecture

> **Creator & Lab**: Bolt J Woofson @ Woofsons Lab ([www.arf.ac](https://www.arf.ac))  
> **Official Web**: [www.arf.ac](https://www.arf.ac)  
> **Philosophy**: *No acronyms or slogans. Just a very good environment for file managing and orchestrating.*  
> **Publishing Prefix Rule**: All crates, binaries, and packages use the `arf-` or `arf_` prefix (e.g. `arf-cmdr`, `arf-remote`, `arf-desktop`).  
> **Single Source of Truth**: Active roadmap planning, sprint tasks, and bug triage are tracked exclusively on **[GitHub Issues](https://github.com/Woofson/brum/issues)** and **[Milestones](https://github.com/Woofson/brum/milestones)**.  
> **Release History & Notes**: For full release notes and changelogs, see [**`CHANGELOG.md`**](CHANGELOG.md) or run `./scripts/changelog.sh`.

---

## 1. 🌟 The Ultimate Replacement Vision: One Commander ("ChewToys")

> *"Replace a scattered suite of 10+ disconnected utilities with a single, ultra-fast, unified Commander and a suite of built-in power-tools ('ChewToys')."*

```
┌────────────────────────────────────────────────────────────────────────────────────────────┐
│ The Brum "ChewToy" Replacement Matrix                                                      │
├────────────────────────┬─────────────────────────────┬─────────────────────────────────────┤
│ Legacy / External App  │ Native Brum ChewToy         │ Replaced Capabilities               │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ FileZilla & Mountain D │ Native Multi-Protocol VFS   │ SFTP/SSH, SMB/CIFS, NFS, WebDAV,    │
│                        │                             │ Hetzner Storage Box, Proton Drive,  │
│                        │                             │ Google Drive & S3 Object Storage    │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ rclone & rsync         │ DeltaCopy / RoboCopy        │ Differential delta streaming,       │
│                        │ Engine & Background Tasks   │ bandwidth throttling & auto-retry   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Bvckup 2 & SyncToy     │ Backup (SyncToy Studio)     │ 4 replication profiles, in-place    │
│                        │                             │ block deltas, snapshots & scheduler │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Syncthing              │ Syncthing Dashboard         │ Peer status, throughput charts,     │
│                        │ & Direct Local/LAN Sync     │ folder scan triggers, P2P sync      │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ PuTTY & OpenSSH SCP    │ Terminal (PTY Web Console)  │ Embedded WebSocket pseudo-terminal  │
│                        │ (Bite!)                     │ (fish/zsh/bash/powershell) in path  │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Total / Multi / MC /   │ 1-to-4 Multi-Tab Dynamic    │ Orthodox keyboard shortcuts, dual-  │
│ XYplorer / Directory O │ Panes & Orthodox Suite      │ pane power diff, batch rename,      │
│                        │                             │ branch view, rich MIME icon suite   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Cryptomator / VeraCrypt│ AES-256-GCM Vaults          │ Zero-leakage in-memory containers   │
│                        │ (.cdvault)                  │ (Argon2id + AES-GCM RAM-only VFS)   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ VS Code / Sublime Text │ Edit (EditorDog)            │ Multi-tab syntax editor, live split │
│                        │                             │ preview, template generator & diff  │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ HandBrake / FFmpeg GUI │ ConvertX Transcoder         │ Browser-native image/audio/video/   │
│                        │                             │ document conversion engine          │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ PDFsam / Acrobat Split │ PDF Studio (PDFDog)         │ Pure-Rust visual merge, split,      │
│                        │                             │ page reordering & rotation grid     │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ FastStone / Feh Viewer │ High-DPI Image Viewer       │ Mouse wheel browse, focal zoom,     │
│                        │                             │ slideshow, format conversion        │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Obsidian / Joplin      │ Notes (NoteDog Studio)      │ Hierarchical Markdown notebook,     │
│                        │                             │ checklists, revision diff history   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Classic Arcade Tetris  │ Tetra (TetraDog Arcade)     │ Authentic classic Tetris clone,     │
│ & Desktop Distraction  │                             │ 60 FPS canvas engine, SRS/NES modes,│
│                        │                             │ DAS/ARR tuning, high scores & audio │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Apache Guacamole /     │ Remote (RemoteDog Gateway)  │ Sub-ms in-browser RDP (IronRDP/NLA),│
│ Remmina / mstsc / VNC  │                             │ VNC (RFB 3.8), SSH PTY, 1-4 grid,   │
│                        │                             │ clipboard auto-sync & file staging  │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Winamp / Foobar2000 /  │ AMP (ARFAMP Winamp Clone)   │ 3-modular layout, windowshade mode, │
│ Audacious / XMPlay     │                             │ 10-band EQ, 60fps spectrum, .m3u PL │
└────────────────────────┴─────────────────────────────┴─────────────────────────────────────┘
```

---

## 2. 🎨 Design Language & Viewport Standards

### Standardized Viewports
* **`Phone`** (`<600px`): Mobile touch screens. Single-pane focus, hidden branding badges, minimal micro-text.
* **`Tablet`** (`600px–1024px`): Foldables & tablet touch screens. Adaptive single/dual panel options, sliding note drawers, legible touch hierarchy.
* **`PC`** (`>1024px`): Desktop & laptop mouse & keyboard. Full multi-panel, resizable columns, dockable ChewToys, and frosted glass auth screens.

### UI & ChewToy Design Specification
* **Panels vs Tabs**: File browsing areas are strictly termed **"Panels"**; **"Tabs"** are strictly reserved for Settings modal tabs and Editor tabs.
* **Primary Window & Modal Headers (`42px` min-height)**:
  * Full-length drag handle (`cursor: grab;` / `:active { cursor: grabbing; }`).
  * Uniform **`28px x 28px`** buttons, selects, and icon triggers with `border-radius: var(--radius)` (`6px`) and `14px` icons.
  * Right-aligned icon-only layout switches with active amber accent glow (`rgba(245, 158, 11, 0.18)` + `var(--accent)`).
  * Flat, stealthy window control buttons (Minimize, Maximize, Dock/Float, Close).
* **Sub-Headers & Inner Workspace Toolbars (`26px` Standard)**:
  * Uniform **`26px x 26px`** buttons with `13px` icons across all secondary action rows.
* **Dual-Mode Architecture**:
  * All built-in ChewToys natively support dual modes: **Floating Draggable Window** and **In-Pane Docking** (Panel 1 or Panel 2) with state persistence.
* **Official Themes**:
  * **`Woofsons Amber Charcoal`** (Dark - Default)
  * **`Woofsons Amber Zink`** (Light)

---

## 3. 🧭 Active Backlog & GitHub Issues (Single Source of Truth)

To eliminate duplication, token waste, and sync drift, all active feature development, responsive refinements, and bug triage are managed directly through GitHub Issues and Milestones.

* **GitHub Issue Tracker**: [https://github.com/Woofson/brum/issues](https://github.com/Woofson/brum/issues)
* **GitHub Milestones**: [https://github.com/Woofson/brum/milestones](https://github.com/Woofson/brum/milestones)

### Developer CLI Workflow

Fast, token-efficient CLI scripts are provided for querying backlog and changelog state directly:

```bash
# Query active backlog and view issues
./scripts/issues.sh list               # List all open issues with labels
./scripts/issues.sh view <id>          # View issue description and requirements
./scripts/issues.sh bugs               # Filter open bug reports
./scripts/issues.sh viewports          # Filter phone/tablet/desktop responsive tasks
./scripts/issues.sh chewtoys           # Filter ChewToy issues

# Compile changelog and release notes to stdout
./scripts/changelog.sh                 # Output latest release / iteration notes
./scripts/changelog.sh -n 3            # Output last 3 releases
./scripts/changelog.sh git             # Compile conventional changelog directly from git commits
./scripts/changelog.sh releases        # List published GitHub releases via gh CLI
```

### Active Issue Categories

| Issue | Category | Summary |
| :--- | :--- | :--- |
| **[#26](https://github.com/Woofson/brum/issues/26)** | Navigation | Unified "Places" Hub (Merge Bookmarks, Storage Roots & Remote VFS shares) |
| **[#27](https://github.com/Woofson/brum/issues/27)** | UI & Ergonomics | Streamline Copy (F5), Move (F6), and Rename (F2) dialog ergonomics & button sizing |
| **[#21](https://github.com/Woofson/brum/issues/21)** | Security | Terminal PTY POSIX privilege dropping for authenticated non-root users in service mode |
| **[#23](https://github.com/Woofson/brum/issues/23)** | ChewToy | Multi-Drive Storage Overview & Mountpoint Disk Stats in Stats ChewToy |
| **[#24](https://github.com/Woofson/brum/issues/24)** | Feature & Mesh | Commander Fleet (Multi-Host Node Switcher, Cross-Pane Transfers & Hardened Remote Access) |
| **[#22](https://github.com/Woofson/brum/issues/22)** | Packaging | Modernize Debian `.deb` dependencies (`7zip`/`t64`) and standard FHS paths |
| **[#19](https://github.com/Woofson/brum/issues/19)** | Windows | Windows native platform capabilities and drive integrations |
| **[#15](https://github.com/Woofson/brum/issues/15)** | UI | Drag-and-drop rearrangement of dynamic panels |
| **[#13](https://github.com/Woofson/brum/issues/13)** | Plugins | Modular ChewToy plugin architecture & scripting engine (`.arf` / `.woof`) |
| **[#4](https://github.com/Woofson/brum/issues/4)** | Viewports | Optional single/dual panel mode toggle on Tablet & Foldables |
| **[#25](https://github.com/Woofson/brum/issues/25)** | NoteDog | Mobile & Foldable sliding collapsible drawer sidebar |

---

## 4. 🐕 ➔ ⚡ Brum Rebranding & Ecosystem Migration Master Plan

> **Goal**: Seamless, zero-downtime transition from **CommanderDog** to **Brum** across GitHub, package registries, binaries, Docker, and documentation ahead of `v1.0.0`.

### ChewToy Nomenclature Harmonization
* `NoteDog` ➔ **`Notes`** (Hierarchical Markdown notebook, task checklists, encrypted vaults)
* `EditorDog` ➔ **`Edit`** (Multi-tab syntax highlighter, code editor, live split preview)
* `ARFAMP` ➔ **`AMP`** (Winamp 2.x clone, 10-band equalizer, spectrum visualizer, .m3u playlists)
* `PDFDog` ➔ **`PDF Studio`** (Visual PDF merge, split, page reordering, rotation)
* `TetraDog` ➔ **`Tetra`** (Authentic classic 60 FPS arcade block puzzle & leaderboard)
* `Spot!` ➔ **`Spot!`** (Spotlight command palette & path quick-shifter `Ctrl+K`)
* `Bite! / Terminal` ➔ **`Terminal`** (Slide-up PTY terminal console `'`)
* `ConvertX` ➔ **`ConvertX`** (Universal browser-native media transcoder & converter)
* `Diff / Compare` ➔ **`Compare`** (Side-by-side visual diff engine `F9`)
* `Delta Backup` ➔ **`Backup`** (SyncToy / Bvckup 2 delta replication studio)
* `Disk Usage` ➔ **`Stats`** (Visual treemap & disk consumption analyzer)
* `Syncthing` ➔ **`Syncthing`** (Live Syncthing dashboard & LAN/P2P sync)
* `RemoteDog` ➔ **`Remote`** (In-browser sub-ms RDP, VNC, SSH remote gateway & multi-pane grid)

### Ecosystem Distribution Matrix

| Ecosystem / Channel | Legacy Target | New Target (`Brum`) | Migration Strategy & Transition Path |
| :--- | :--- | :--- | :--- |
| **GitHub Repository** | `Woofson/commanderdog` | `Woofson/brum` | GitHub repository rename with automatic URL and git clone redirects; preserve issues and pull requests. |
| **CLI / Server Binary** | `commanderdog` | `brum` | Primary binary renamed to `brum`; provide temporary symlink / transitional alias `commanderdog -> brum`. |
| **Windows Desktop** | `CommanderDog.exe` | `Brum.exe` | Standalone executable and launcher updated to `Brum.exe` / `brum.exe`. |
| **Docker / GHCR** | `ghcr.io/woofson/commanderdog` | `ghcr.io/woofson/brum` | Multi-arch Alpine image published to `ghcr.io/woofson/brum`; legacy repo redirected. |
| **Rust Crates.io** | `commanderdog` (`arf-cmdr`) | `brum` / `brum-cmdr` | Crates namespace reservation with `brum-` / `arf-` prefix rule; `brum-desktop` for Tauri wrapper. |
| **Arch Linux (AUR)** | `commanderdog`<br>`commanderdog-bin` | `brum`<br>`brum-bin` | AUR packages `brum` & `brum-bin` with `provides=('commanderdog')`, `conflicts=('commanderdog')`. |
| **Windows WinGet** | `Woofson.CommanderDog` | `Woofson.Brum` | New package manifest `Woofson.Brum` with upgrade path from `Woofson.CommanderDog`. |
| **Windows Scoop** | `commanderdog.json` | `brum.json` | Updated bucket manifest `brum.json` with fallback shim in `packaging/windows/scoop/`. |
| **Installers** | `CommanderDog_x64-setup.exe`<br>`CommanderDog_x64_en-US.msi` | `Brum_x64-setup.exe`<br>`Brum_x64_en-US.msi` | Updated NSIS setup and WiX MSI installers with `Brum` branding. |
| **Config & Data Paths**| `~/.config/commanderdog/`<br>`commanderdog.db` | `~/.config/brum/`<br>`brum.db` | Automatic migration check looking for legacy `~/.config/commanderdog/` and `commanderdog.db`. |

---

## 5. 🔮 Strategic Milestones

### Milestone 1: Multi-Cloud VFS, Remote Gateway ChewToy & High-Impact Extensions (`v0.8.0+`)
* **Remote Gateway ChewToy / Modular Plugin (`Remote` — incorporating RemoteDog)**:
  * Pure-Rust `IronRDP` with NLA/CredSSP, sub-rect dirty diffing, dynamic resizing (`MS-RDPEDISP`).
  * Full RFB VNC client (3.8), remote SSH PTY shell with SFTP, and 1-to-4 multi-pane grid (`Alt+1` to `Alt+4`).
  * Auto-clipboard synchronization and direct drag-and-drop file transfers between local panels and remote hosts.
* **Embedded WebDAV Server Mode**: Native WebDAV server daemon allowing external operating systems to mount storage as local network drives.
* **Multi-Cloud VFS Adapters**: Native connectors for Google Drive, Proton Drive, Hetzner Storage Box, and direct S3/MinIO browser streaming.
* **Modular Plugin Architecture (`.arf` / `.woof`)**: Dynamic external plugin packaging, manifest specification (`plugin.toml`), and sandboxed shell bridge.

### Milestone 2: Enterprise Identity, OIDC / SSO & Collaborative Office (`v0.9.0`)
* **Enterprise Identity Providers**: OpenID Connect (OIDC), OAuth2, SAML 2.0, Keycloak, Authentik, Authelia, Google, GitHub, Okta, Azure AD.
* **Collaborative Document Editing**: In-browser real-time collaborative editing for markdown, code, and Office documents (`.docx`, `.xlsx`, `.pptx` via Collabora / OnlyOffice WOPI).

### Milestone 3: Brum Full Release, High-Performance P2P Cluster & Distributed Storage (`v1.0.0`)
* **Ecosystem Migration Execution**: Finalize repository rename to `Woofson/brum`, publish `brum` binary and container packages to GHCR, crates.io, AUR, and WinGet.
* **Cluster Node Mesh**: Direct peer-to-peer authenticated node clustering with distributed metadata synchronization.
* **Distributed Virtual Storage**: Multi-host unified mountpoints and automated cross-node replication.

---

## 6. 📊 Release Version Matrix

| Version | Milestone Focus | Status | Changelog |
| :--- | :--- | :--- | :--- |
| **`v0.8.3`** | Places Hub, Dialog Ergonomics, Frosted Auth Glass, Security Hardening | **In Progress** | [View Notes](CHANGELOG.md#083-rc8---2026-09-11) |
| **`v0.8.2`** | Brum Rebranding, Bear Logo, Crate & AUR Distribution | **Released** | [View Notes](CHANGELOG.md#082---2026-09-08) |
| **`v0.8.1`** | ChewToy Nomenclature, Context Menu Auto-Dismiss, Windows Icon | **Released** | [View Notes](CHANGELOG.md#081---2026-09-08) |
| **`v0.8.0`** | Windows Navigation Fix, Touch Columns, Theme Form System | **Released** | [View Notes](CHANGELOG.md#080---2026-09-08) |
| **`v0.7.9`** | HTTP Range Audio Streaming & ARFAMP Rebrand | **Released** | [View Notes](CHANGELOG.md#079---2026-09-07) |
| **`v0.7.8`** | ARFAMP (Winamp 2.x Clone & 10-Band EQ ChewToy) | **Released** | [View Notes](CHANGELOG.md#078---2026-09-07) |
| **`v0.7.7`** | TetraDog (Classic Arcade Tetris ChewToy & Leaderboard) | **Released** | [View Notes](CHANGELOG.md#077---2026-09-07) |
| **`v0.7.3`** | Visual Disk Treemap, NoteDog Encryption & Release Automation | **Released** | [View Notes](CHANGELOG.md#073---2026-09-04) |
| **`v0.7.2`** | Documentation Reorganization & Chewtoy UI/UX Refinements | **Released** | [View Notes](CHANGELOG.md#072---2026-09-04) |
| **`v0.7.1`** | Universal Floating Viewers, Live Tail Follow, Bundled Fonts & Themes | **Released** | [View Notes](CHANGELOG.md#071---2026-09-03) |
| **`v0.7.0`** | NoteDog Notes Studio, Tags & Colors, Custom Workspaces | **Released** | [View Notes](CHANGELOG.md#070---2026-09-01) |
| **`v0.6.0`** | Windows Native Build & Release (MSI, ZIP, Winget, Scoop, WebView2) | **Released** | [View Notes](CHANGELOG.md#060---2026-09-30) |
| **`v0.5.0`** | Zero-Leakage Credentials, Leftmost Pane Customizer, Modern Navbar | **Released** | [View Notes](CHANGELOG.md#050---2026-08-28) |
| **`v0.4.0`** | Multi-Part Splitter, Integrated Git Client, Auto-$HOME Startup | **Released** | [View Notes](CHANGELOG.md#040---2026-08-22) |
| **`v0.3.0`** | In-Pane Tool Docking, ConvertX, Dual-Pane Editor | **Released** | [View Notes](CHANGELOG.md#030---2026-08-15) |
| **`v0.9.0`** | Enterprise OIDC / SSO, Collaborative Office (WOPI) & RBAC | *Planned* | — |
| **`v1.0.0`** | Brum Official Rebrand, High-Performance P2P Cluster & Distributed Storage | *Planned* | — |
