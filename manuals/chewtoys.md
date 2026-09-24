# Brum Native Core Functions & ChewToys Manual

**Brum** replaces a fragmented collection of separate desktop and command-line utilities with a unified, high-performance web and native desktop interface. These integrated tools are divided into **Native Core Functions** (embedded directly into the core DOM and Rust backend with zero iframe overhead) and **Chewtoys** (isolated, sandboxed modular `.grr` extension packages).

```
┌────────────────────────────────────────────────────────────────────────────────────────────┐
│ The Brum Core Function & Chewtoy Replacement Matrix                                        │
├────────────────────────┬─────────────────────────────┬─────────────────────────────────────┤
│ Legacy / External App  │ Native Brum Core Function   │ Replaced Capabilities               │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ FileZilla & Mountain D │ Native Multi-Protocol VFS   │ SFTP/SSH, SMB/CIFS, NFS, WebDAV,    │
│                        │ & Client Local VFS          │ Hetzner Storage Box, Proton Drive,  │
│                        │ (client://)                 │ Google Drive & S3 Object Storage    │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ rclone & rsync         │ DeltaCopy / RoboCopy        │ Differential delta streaming,       │
│                        │ Engine & Background Tasks   │ bandwidth throttling & auto-retry   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Bvckup 2 & SyncToy     │ Sync & Replication Studio   │ 4 replication profiles, in-place    │
│                        │                             │ block deltas, snapshots & scheduler │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Syncthing              │ Syncthing Dashboard         │ Peer status, throughput charts,     │
│                        │ & Direct Local/LAN Sync     │ folder scan triggers, P2P sync      │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ PuTTY & OpenSSH SCP    │ Bite! Terminal (PTY)        │ Embedded WebSocket pseudo-terminal  │
│                        │                             │ (fish/zsh/bash/powershell) in path  │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Total / Multi / MC /   │ 1-to-4 Multi-Tab Dynamic    │ Orthodox keyboard shortcuts, dual-  │
│ XYplorer / Directory O │ Panes & Orthodox Suite      │ pane power diff, batch rename,      │
│                        │                             │ branch view, rich MIME icon suite   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Blender / MeshLab /    │ 3D CAD Studio               │ Interactive WebGL CAD & mesh viewer │
│ FreeCAD                │                             │ (STL, OBJ, DXF, PLY, STEP, slicing) │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ HJSplit / GSplit       │ File Splitter & Combiner    │ Multi-part chunk splitting, MD5/    │
│                        │                             │ SHA-256 integrity check & combine   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ PDFsam / Acrobat Split │ PDF Studio                  │ Pure-Rust visual merge, split,      │
│                        │                             │ page reordering & rotation grid     │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Nextcloud / Dropbox    │ Sharing Center              │ Public links, passcodes, expiration,│
│                        │                             │ access logs, client upload dropzone │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ WinDirStat / Baobab    │ Disk Usage Analyzer         │ Interactive storage visualizer,     │
│                        │                             │ size bar charts & top file explorer │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Cryptomator / VeraCrypt│ AES-256-GCM Vaults          │ Zero-leakage in-memory containers   │
│                        │ (.cdvault)                  │ (Argon2id + AES-GCM RAM-only VFS)   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ VS Code / Sublime Text │ Text & Code Editor          │ Multi-tab syntax editor, live split │
│                        │                             │ preview, template generator & diff  │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ HandBrake / FFmpeg GUI │ Format Converter            │ Browser-native image/audio/video/   │
│                        │                             │ document conversion engine          │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ FastStone / Feh Viewer │ High-DPI Image Viewer       │ Mouse wheel browse, focal zoom,     │
│                        │                             │ slideshow, format conversion        │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Obsidian / Joplin      │ Notes Studio                │ Hierarchical Markdown notebook,     │
│                        │                             │ checklists, revision diff history   │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Winamp / Foobar2000 /  │ Audio Player & Media        │ Windowshade mode, mini-pills,       │
│ Audacious / XMPlay     │                             │ 10-band EQ, 60fps spectrum, .m3u PL │
├────────────────────────┼─────────────────────────────┼─────────────────────────────────────┤
│ Classic Arcade Tetris  │ Tetrion (Chewtoy)           │ Authentic classic Tetris clone,     │
│ & Desktop Distraction  │                             │ 60 FPS canvas engine, SRS/NES modes,│
│                        │                             │ DAS/ARR tuning, high scores & audio │
└────────────────────────┴─────────────────────────────┴─────────────────────────────────────┘
```

---

## 1. Dual-Mode Architecture (Floating Windows & In-Pane Docking)

Every Core Function and power tool in Brum supports dual operational modes conforming to **Rule 9**:
1. **Floating Window Mode**: Freely draggable via its header drag handle, resizable with corner handles, minimizable to interactive task mini-pills, and maximizable.
2. **In-Pane Docking Mode**: Dockable directly into Panel 1, Panel 2, Panel 3, or Panel 4 via the pane selector or window dock button (`panel-left-close`), maintaining state with zero DOM rebuild overhead.

---

## 2. 3D CAD Studio & Mesh Viewer
* **Format Support**: Direct visualization for Stereolithography (`.stl` binary and ASCII), Wavefront (`.obj`), AutoCAD (`.dxf`), Polygon File Format (`.ply`), and STEP / STP / IGES / 3MF wireframe models.
* **Viewport Presets & Shading**:
  * Camera views: Isometric (`ISO`), Top, Front, Right.
  * Shading shaders: Solid PBR, Smooth Shaded, Wireframe, Normal Map, Matcap Clay.
  * Color presets: Woofsons Amber Charcoal, Amber Gold, Polished Steel, Industrial Bronze, Cyber Cyan, Studio White.
* **Cross-Section Slicing Plane**: Dynamic $X$, $Y$, and $Z$ cross-section clipping plane with real-time offset slider.
* **Telemetry HUD**: Real-time bounding box dimensions ($W \times H \times D$ mm), mesh volume ($\text{cm}^3$), surface area ($\text{cm}^2$), triangle and vertex counts, and estimated 3D print weight (grams).
* **Snapshot Export**: High-resolution PNG rendering export directly to active directory.

---

## 3. File Splitter & Combiner
* **Chunk Splitting**: Split oversized files into fixed-size chunks with preset targets (1.44MB Floppy, 700MB CD, 4.7GB DVD, 4GB FAT32, or Custom MB/GB chunk sizes).
* **Drag-and-Drop Ingestion**: Drop files directly onto the Split or Combine tab dropzones.
* **Integrity Verification**: Automatic MD5 and SHA-256 checksum generation during splitting and verification upon recombination.
* **Part Auto-Discovery**: Dropping or selecting `.001` or `.part1` automatically discovers and validates all sequential companion chunks in the directory.

---

## 4. PDF Studio
* **Pure-Rust Engine (`lopdf`)**: Zero external dependencies, running entirely inside Brum's Rust backend.
* **Visual Page Organizer**: Interactive thumbnail grid showing all pages with 90° clockwise/counter-clockwise rotation.
* **Page Operations**: Drag-and-drop page reordering, page deletion, single-page extraction, and bundle splitting.
* **PDF Merger**: Combine multiple source PDFs into a single unified document with preserved bookmarks and vector fidelity.

---

## 5. Sharing Center & Public Cloud Portals
* **Public Link Management**: Centralized dashboard to view, edit, and revoke all active public share tokens (`/share/:token`).
* **Access Rules & Security**: Optional passcodes, expiration timestamps (1 hour, 1 day, 7 days, custom date), and max download limits.
* **Upload Dropzones**: Enable public client file uploads directly into specific folders with optional size quotas.
* **Guest Access Logs**: View real-time visitor IP addresses, download counts, and access timestamps.

---

## 6. Disk Usage Analyzer
* **Interactive Breakdown**: Directory tree distribution charts displaying largest space-consuming folders and files.
* **Top Files Inspector**: Identifies large files across directories with 1-click jump-to-location and cleanup actions.
* **Fast Multi-Threaded Scanning**: Background asynchronous scan engine that avoids blocking UI file browsing.

---

## 7. Notes Studio
* **Tree Hierarchy**: SQLite-backed Markdown notebook with folder trees, drag-and-drop organization, and full-text search indexing.
* **Interactive Checklists**: Live `- [ ]` / `- [x]` interactive checklists with instant state persistence.
* **Revision Snapshots**: Automated snapshot history in `.notedog_versions/` with side-by-side diff previews and 1-click restore.
* **Templates**: Built-in templates for Project Plans, SOPs, Daily Logs, and Meeting Minutes.
* **Encrypted Notes**: Seamless encryption and decryption support for `.md.enc` files.

---

## 8. Audio Player & Media Player
* **Modular Winamp-Style & Windowshade**: Compact player, 10-band studio graphic equalizer, and playlist editor.
* **Floating Mini-Pill Badges**: Minimize playback into an interactive floating pill badge (`#audio-player-pill`) with live elapsed progress, track ticker, and quick play/pause.
* **Zero-Copy Byte-Range Streaming**: Real-time seeking and scrubbing for multi-gigabyte `.mp3`, `.flac`, `.wav`, `.mp4`, `.webm`, and `.mkv` files.
* **10-Band EQ & Spectrum Visualizer**: 60 FPS frequency analyzer with 8 acoustic presets (*Flat, Bass Boost, Rock, Synthwave, Acoustic/Vocal, Jazz, Classical, Pop*).

---

## 9. Bite! Slide-Up PTY Terminal
* **Native WebSocket PTY**: Full pseudo-terminal session attached directly to the active pane's directory.
* **Shell Support**: Automatically detects and loads your default shell (`fish`, `zsh`, `bash`, `powershell`, `cmd.exe`).
* **Bundled Nerd Fonts**: Embedded `JetBrainsMono Nerd Font` provides out-of-the-box support for `eza --icons`, `starship`, and powerline glyphs without missing boxes or tofu characters.
* **Dynamic Geometry**: Automatic `ResizeObserver` recalculates dimensions and synchronizes `{ cols, rows }` across pane and window resizes.

---

## 10. Delta Backup & Sync Studio
* **4 Replication Profiles**:
  1. `synchronize`: Bidirectional 2-way sync with newer-file conflict resolution.
  2. `echo`: 1-way mirror making destination an exact replica (prunes destination orphans).
  3. `contribute`: Additive replication copying additions & modifications (preserves destination orphans).
  4. `subscribe`: Historical versioning archiving changes into timestamped `_archive/` snapshot trees.
* **In-Place Block Delta Copy**: 64KB CRC32 chunk hashing skips unmodified blocks and patches modified byte blocks in-place directly on disk.
* **Scheduler & Webhooks**: Interval, daily (HH:MM), and continuous real-time triggers with Discord, Slack, and generic JSON webhook alerts.

---

## 11. Format Converter
* **Multi-Format Conversion Matrix**:
  * **Video**: Transcode legacy containers and codecs (`.avi`, `.mkv`, `.mov`, `.flv`, `.wmv`, `.mpg`, `.m4v`, `.ts`) to web-optimized formats (`.mp4` with H.264 + AAC, `.webm` with VP9 + Opus).
  * **Audio**: Convert lossy and lossless audio files (`.flac`, `.wav`, `.aac`, `.m4a`, `.ogg`, `.wma`, `.opus`) to `.mp3`, `.ogg`, `.flac`, or `.wav`.
  * **Images & Raster Graphics**: Convert and optimize `.png`, `.jpg`, `.jpeg`, `.webp`, `.avif`, `.gif`, `.bmp`, `.ico`, `.tiff` with custom quality, dimension, or color palette adjustments.
* **Background Task Mini-Pills**: Collapse long running conversions into interactive task mini-pills with live progress metrics.

---

## 12. Batch Renamer
* **Advanced Patterns**: Find & replace, Regex capture groups, sequential numbering (`001`, `002`), prefix/suffix injection, and case transformations (`UPPER`, `lower`, `Title`, `snake_case`, `kebab-case`).
* **Dry-Run Simulation**: Real-time side-by-side preview table highlighting collisions, identical names, and validation warnings before executing.

---

## 13. Hex Editor
* **Byte-Level Precision**: Hexadecimal and ASCII dual-column editor with offset address indicators.
* **Navigation & Search**: Search hexadecimal byte sequences or ASCII strings with instant jump to byte offset.
* **In-Place Modification**: Direct byte overwriting and safe file saving.

---

## 14. Transparent Encrypted Vaults (.cdvault)
* **Zero-Knowledge Security**: AES-256-GCM authenticated encryption + Argon2id key derivation.
* **In-Memory Streaming**: RAM-only virtual filesystem mount with zero plaintext leakage to disk.
* **Inactivity Lock**: Configurable automatic lock timer (default: 15 minutes) and 1-click breadcrumb lock.

---

## 15. Tetrion (Modular Chewtoy)
* **60 FPS Arcade Engine**: HTML5 canvas rendering driven by a fixed-timestep physics loop.
* **DAS & ARR Tuning**: Configurable Delayed Auto Shift and Auto Repeat Rate for competitive, instant piece movement.
* **Guideline Mechanics**: 7-bag randomizer, Super Rotation System (SRS) with wall kicks, ghost piece projection, hold queue, and synchronized leaderboard.
