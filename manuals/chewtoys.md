# Brum Power Tools & ChewToys Manual

**Brum** replaces a fragmented collection of separate desktop and command-line utilities with a unified, high-performance web and native interface. These integrated extensions are known as **"ChewToys"**.

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

## 1. NoteDog Notes Studio
* **Tree Hierarchy**: Markdown notebook with folder trees, drag-and-drop organization, and full-text search indexing.
* **Interactive Checklists**: Live `- [ ]` / `- [x]` interactive checklists with instant state persistence.
* **Revision Snapshots**: Automated snapshot history in `.notedog_versions/` with side-by-side diff previews and 1-click restore.
* **Templates**: Built-in templates for Project Plans, SOPs, Daily Logs, and Meeting Minutes.
* **Encrypted Notes**: Seamless encryption and decryption support for `.md.enc` files.

---

## 2. Bite! Slide-Up PTY Terminal
* **Native WebSocket PTY**: Full pseudo-terminal session attached directly to the active pane's directory.
* **Shell Support**: Automatically detects and loads your default shell (`fish`, `zsh`, `bash`, `powershell`, `cmd.exe`).
* **Bundled Nerd Fonts**: Embedded `JetBrainsMono Nerd Font` provides out-of-the-box support for `eza --icons`, `starship`, and powerline glyphs without missing boxes or tofu characters.
* **Dynamic Geometry**: Automatic `ResizeObserver` recalculates dimensions and synchronizes `{ cols, rows }` across pane and window resizes.

---

## 3. Delta Backup & Sync Studio (SyncToy & Bvckup 2)
* **4 Replication Profiles**:
  1. `synchronize`: Bidirectional 2-way sync with newer-file conflict resolution.
  2. `echo`: 1-way mirror making destination an exact replica (prunes destination orphans).
  3. `contribute`: Additive replication copying additions & modifications (preserves destination orphans).
  4. `subscribe`: Historical versioning archiving changes into timestamped `_archive/` snapshot trees.
* **In-Place Block Delta Copy**: 64KB CRC32 chunk hashing skips unmodified blocks and patches modified byte blocks in-place directly on disk.
* **Scheduler & Webhooks**: Interval, daily (HH:MM), and continuous real-time triggers with Discord, Slack, and generic JSON webhook alerts.

---

## 4. PDF Power Studio (PDFDog)
* **Pure-Rust Engine (`lopdf`)**:
  * Visual drag-and-drop page reordering and bookmark outlines.
  * Split by custom page ranges, extract single pages, or split into $N$-page bundles.
  * Visual page organizer grid with 90° clockwise/counter-clockwise rotation controls.

---

## 5. ConvertX Transcoder
ConvertX is Brum's integrated media conversion and audio/video transcoding suite. It allows users to convert video, audio, image, and document formats directly inside the browser or native desktop interface, without needing to install or manage separate command-line or GUI converter applications.

* **Multi-Format Conversion Matrix**:
  * **Video**: Transcode legacy, high-bitrate, or non-browser containers and codecs (e.g. `.avi`, `.mkv`, `.mov`, `.flv`, `.wmv`, `.mpg`, `.m4v`, `.ts`) to web-optimized formats (`.mp4` with H.264 + AAC, `.webm` with VP9 + Opus).
  * **Audio**: Convert lossy and lossless audio files (`.flac`, `.wav`, `.aac`, `.m4a`, `.ogg`, `.wma`, `.opus`) to `.mp3`, `.ogg`, `.flac`, or `.wav`.
  * **Images & Raster Graphics**: Convert and optimize `.png`, `.jpg`, `.jpeg`, `.webp`, `.avif`, `.gif`, `.bmp`, `.ico`, `.tiff` with custom quality, dimension, or color palette adjustments.
  * **Documents & Text**: Batch convert markdown, text, HTML, and structured document formats.

* **Non-Blocking Background Processing & Task Pill**:
  * **Minimize to Task Pill**: When initiating a long video/audio transcode, click the minimize icon (<kbd>−</kbd>) or **"Run in Background"** to collapse the modal into a floating amber task pill (`#convertx-pill`) in the bottom-right corner.
  * **Live Feedback**: The floating pill displays real-time execution animation and progress indicators while allowing you to freely browse files, edit notes, or play music simultaneously.
  * **1-Click Restore**: Click the floating task pill at any time to reopen the full ConvertX modal and monitor live stdout/stderr metrics.

* **Smart Destination & Collision Protection**:
  * **Automatic Suffixing**: When converting to the same format or extension (e.g. converting `11052009.mp4` to a browser-compatible `.mp4`), ConvertX automatically appends `_converted` to avoid overwriting the source file.
  * **User-Editable Filenames**: Complete freedom to rename the target file directly in the modal before starting.
  * **Destination Preview**: Live path calculation showing exactly where the converted artifact will be saved.
  * **Post-Conversion Summary**: Upon successful completion, the interface presents a clear summary box detailing source file, target file, and execution time, swapping action buttons to a clean `[OK]` dismissal.

* **Remote Host & Fleet Node Server-Side Execution**:
  * When browsing remote hosts (SFTP / SMB / NFS / SSH nodes), ConvertX executes conversion pipelines directly on the remote server where the data resides.
  * Eliminates the need to download large multi-gigabyte video or archive files across the WAN just to transcode them.

* **System Dependencies**:
  * Video and audio transcoding rely on `ffmpeg` in the system `$PATH` (or `%PATH%` on Windows).
  * Image conversions utilize pure-Rust decoders alongside `ImageMagick` (`magick`) for advanced formats and high-speed processing.

---

## 6. Transparent Encrypted Vaults (.cdvault)
* **Zero-Knowledge Security**: AES-256-GCM authenticated encryption + Argon2id key derivation.
* **In-Memory Streaming**: RAM-only virtual filesystem mount with zero plaintext leakage to disk.
* **Inactivity Lock**: Configurable automatic lock timer (default: 15 minutes) and 1-click breadcrumb lock.

---

## 7. Multi-Part File Splitter & Combiner
* **Chunk Splitting**: Split oversized files into fixed-size chunks (e.g. for FAT32 filesystem limits, email attachments, or multi-part uploads).
* **Integrity Combining**: 1-click SHA-256 verified re-assembly back to the original file.

---

## 8. TetraDog (Arcade Tetris Studio & Leaderboard)
* **Ultra-Responsive 60 FPS Engine**: HTML5 canvas rendering driven by a fixed-timestep physics loop for lag-free performance at Level 15+ / 20G gravity.
* **DAS & ARR Tuning**: Configurable Delayed Auto Shift and Auto Repeat Rate for competitive, instant piece movement without OS keyboard lag.
* **Full Guideline Mechanics**: Fair 7-bag randomizer, Super Rotation System (SRS) with wall kicks, ghost piece projection, hold queue, and lock delay.
* **High Scores & Leaderboard**: Local and server-synchronized high score tracking (lines, scores, levels).
* **Web Audio Synthesis**: Integrated 8-bit retro sound effect synthesizer with zero external audio assets.

---

## 9. ARFAMP (Authentic Winamp 2.x Clone & 10-Band EQ ChewToy)
* **Modular Winamp 2.x Architecture**: Authentic 3-module snappable layout consisting of **Main Player Window**, **10-Band Graphic Equalizer**, and **Playlist Editor**.
* **Windowshade Mode (Alt+W)**: Collapses windows into ultra-compact titlebar shade mode (via `▲`/`▼` button, double-clicking titlebar, or <kbd>Alt+W</kbd>) with live mini-scrubber, mini-timer, and quick controls.
* **Green/Amber Fluorescent LED 7-Segment Timer**: Real-time LED display with click-to-toggle between *Time Elapsed* and *Time Remaining* (`-MM:SS`).
* **Scrolling Marquee Ticker & HUD**: Marquee track title ticker with `KBPS` bitrate, `KHZ` sample rate, and `STEREO`/`MONO` indicator lights.
* **Real-Time 60 FPS Winamp Visualizer**: 18-band segmented green/amber/red LED spectrum analyzer with falling peak caps, CRT phosphor oscilloscope waveform, and ambient glow modes.
* **10-Band Studio Graphic Equalizer**: Authentic Winamp center frequencies (`60Hz`, `170Hz`, `310Hz`, `600Hz`, `1kHz`, `3kHz`, `6kHz`, `12kHz`, `14kHz`, `16kHz`), Preamp fader (-6dB to +6dB), ON/AUTO switches, and 8 acoustic presets (*Flat, Bass Boost, Rock, Synthwave, Acoustic/Vocal, Jazz, Classical, Pop*).
* **Full Playlist Editor**: Monospace green-on-black track list (`1. Artist - Title (MM:SS)`), filter search input, drag & drop track enqueueing, resize handle, and action buttons (`+FILE`, `+DIR`, `-FILE`, `-ALL`, `SHUF`, `LIST` .m3u export).

---

## 10. DiffDog (File & Directory Comparison)
* **Side-by-Side & Unified Diff**: Syntax-highlighted line diffs with character-level inline changes.
* **Directory Comparison Matrix**: Fast file size & mtime checks, cryptographic SHA-256 deep hash comparisons, and 1-click sync actions.

---

## 11. Disk Usage & Storage Treemap
* **Visual Treemap**: Interactive zoomable nested block treemap visualizing folder consumption.
* **Fast Scan Engine**: Multi-threaded traversal with instant top disk hogs identification and direct cleanup actions.

---

## 12. EditorDog (Code & Markdown Studio)
* **Syntax Highlighting**: 15+ programming and config languages (Rust, JS, TS, Python, Go, Bash, HTML, CSS, Markdown, JSON, YAML, TOML, SQL, Dockerfile).
* **Live Markdown & Mermaid Preview**: Real-time markdown rendering with interactive Mermaid.js diagrams, sequence charts, and flowchart generation.
* **Find & Replace**: Regex and case-sensitive batch token replacement.

---

## 13. High-DPI Media & Image Viewer & Audio/Video Player
* **Wide Format Support**: `.png`, `.jpg`, `.jpeg`, `.webp`, `.svg`, `.gif`, `.bmp`, `.ico`, `.avif`, `.tiff`.
* **Focal Zoom & Navigation**: Mouse-wheel folder cycling, `Ctrl`+wheel zoom, 90° rotation, slideshow mode, and video/audio playback.
* **Zero-Copy Byte-Range Media Streaming**:
  * Built-in HTTP/1.1 byte-range streaming (`206 Partial Content`, `Content-Range`, `Content-Encoding: identity`) enables instantaneous seek-and-scrub responsiveness across multi-gigabyte `.mp4`, `.webm`, `.mkv`, `.mp3`, `.flac`, and `.wav` media without waiting for full file downloads.
* **Intelligent Codec Diagnostics**:
  * Automatically detects container formats (e.g. `.mp4`, `.avi`, `.mpg`) containing legacy or unsupported video streams (such as MPEG-4 Part 2, MPEG-1/2, DivX/Xvid, WMV3) where only audio would otherwise be decoded by the browser.
  * Proactively presents 1-click **"Convert with ConvertX"**, **"Open With..."**, or **"Download"** options, seamlessly closing the media player window to prevent UI clutter.

---

## 14. Multi-File Bulk Renamer
* **Advanced Patterns**: Find & replace, Regex capture groups, sequential numbering (`001`, `002`), prefix/suffix injection, and case conversions (`UPPER`, `lower`, `Title`, `snake_case`, `kebab-case`).
* **Conflict Prevention**: Live side-by-side preview table highlighting collisions prior to execution.
