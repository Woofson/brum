# 🦴 CommanderDog Power Tools & ChewToys Manual

**CommanderDog** replaces a fragmented collection of separate desktop and command-line utilities with a unified, high-performance web and native interface. These integrated extensions are affectionately known as **"ChewToys"**.

---

## 1. 📝 NoteDog Notes Studio
* **Hierarchy**: Tree-structured markdown notebook with subdirectories, drag-and-drop hierarchy, and full-text keyword indexing.
* **Interactive Checklists**: Live `- [ ]` / `- [x]` interactive checklists with instant DOM state persistence.
* **Revision Snapshots**: Automated version snapshot engine saved to `.notedog_versions/` with side-by-side diff previews and 1-click restore.
* **Templates**: Built-in templates for Project Plans, SOPs, Daily Logs, and Meeting Minutes.
* **Encryption Detection**: Seamless awareness for NoteDog TUI `.md.enc` encrypted notes.

---

## 2. ⚡ Bite! Slide-Up PTY Terminal
* **Native WebSocket PTY**: Full pseudo-terminal session attached directly to the active pane's directory.
* **Shell Support**: Automatically detects and loads your default shell (`fish`, `zsh`, `bash`, `powershell`, `cmd.exe`).
* **Offline Nerd Fonts**: Bundled `JetBrainsMono Nerd Font` provides out-of-the-box support for `eza --icons`, `starship`, and powerline glyphs without missing boxes or tofu characters.
* **Dynamic Geometry**: Automatic `ResizeObserver` recalculates dimensions and synchronizes `{ cols, rows }` across pane and window resizes.

---

## 3. 🔄 Delta Backup & Sync Studio (SyncToy & Bvckup 2)
* **4 Replication Profiles**:
  1. `synchronize`: Bidirectional 2-way sync with newer-file conflict resolution.
  2. `echo`: 1-way mirror making destination an exact replica (prunes destination orphans).
  3. `contribute`: Additive replication copying additions & modifications (preserves destination orphans).
  4. `subscribe`: Historical versioning archiving changes into timestamped `_archive/` snapshot trees.
* **In-Place Block Delta Copy**: 64KB CRC32 chunk hashing skips unmodified blocks and patches modified byte blocks in-place directly on disk.
* **Scheduler & Webhooks**: Interval, daily (HH:MM), and continuous real-time triggers with Discord, Slack, and generic JSON webhook alerts.

---

## 4. 📄 PDF Power Studio
* **Pure-Rust Engine (`lopdf`)**:
  * Visual drag-and-drop page reordering and bookmark outlines.
  * Split by custom page ranges, extract single pages, or split into $N$-page bundles.
  * Visual page organizer grid with 90° clockwise/counter-clockwise rotation controls.

---

## 5. 🎞️ ConvertX Transcoder
* **Multi-Format Media Engine**:
  * Browser-native and server-assisted image, audio, video, and document format conversion.
  * Batch processing across selected files in active panels.

---

## 6. 🗄️ Transparent Encrypted Vaults (.cdvault)
* **Zero-Knowledge Security**: AES-256-GCM authenticated encryption + Argon2id key derivation.
* **In-Memory Streaming**: RAM-only virtual filesystem mount with zero plaintext leakage to disk.
* **Inactivity Lock**: Configurable automatic lock timer (default: 15 minutes) and 1-click breadcrumb lock.

---

## 7. ✂️ Multi-Part File Splitter & Combiner
* **Chunk Splitting**: Split oversized files into fixed-size chunks (e.g. for FAT32 filesystem limits, email attachments, or multi-part uploads).
* **Integrity Combining**: 1-click SHA-256 verified re-assembly back to the original file.

---

## 8. 🕹️ TetraDog (Arcade Tetris Studio & Leaderboard)
* **Ultra-Responsive 60 FPS Engine**: HTML5 canvas rendering driven by a fixed-timestep physics loop for lag-free performance at Level 15+ / 20G gravity.
* **DAS & ARR Tuning**: Configurable Delayed Auto Shift and Auto Repeat Rate for competitive, instant piece movement without OS keyboard lag.
* **Full Guideline Mechanics**: Fair 7-bag randomizer, Super Rotation System (SRS) with wall kicks, ghost piece projection, hold queue, and lock delay.
* **High Scores & Leaderboard**: Local and server-synchronized high score tracking (lines, scores, levels).
* **Web Audio Synthesis**: Integrated 8-bit retro sound effect synthesizer with zero external audio assets.

---

## 9. 📻 ARFAMP (Authentic Winamp 2.x Clone & 10-Band EQ ChewToy)
* **Modular Winamp 2.x Architecture**: Authentic 3-module snappable layout consisting of **Main Player Window**, **10-Band Graphic Equalizer**, and **Playlist Editor**.
* **Windowshade Mode (Alt+W)**: Collapses windows into ultra-compact titlebar shade mode (via `▲`/`▼` button, double-clicking titlebar, or <kbd>Alt+W</kbd>) with live mini-scrubber, mini-timer, and quick controls.
* **Green/Amber Fluorescent LED 7-Segment Timer**: Real-time LED display with click-to-toggle between *Time Elapsed* and *Time Remaining* (`-MM:SS`).
* **Scrolling Marquee Ticker & HUD**: Marquee track title ticker with `KBPS` bitrate, `KHZ` sample rate, and `STEREO`/`MONO` indicator lights.
* **Real-Time 60 FPS Winamp Visualizer**: 18-band segmented green/amber/red LED spectrum analyzer with falling peak caps, CRT phosphor oscilloscope waveform, and ambient glow modes (click canvas or cycle modes).
* **10-Band Studio Graphic Equalizer**: Authentic Winamp center frequencies (`60Hz`, `170Hz`, `310Hz`, `600Hz`, `1kHz`, `3kHz`, `6kHz`, `12kHz`, `14kHz`, `16kHz`), Preamp fader (-6dB to +6dB), ON/AUTO switches, and 8 acoustic presets (*Flat, Bass Boost, Rock, Synthwave, Acoustic/Vocal, Jazz, Classical, Pop*).
* **Full Playlist Editor**: Monospace green-on-black track list (`1. Artist - Title (MM:SS)`), filter search input, drag & drop track enqueueing, resize handle, and action buttons (`+FILE`, `+DIR`, `-FILE`, `-ALL`, `SHUF`, `LIST` .m3u export).
* **Authentic Winamp Keyboard Shortcuts**:
  * <kbd>Z</kbd> (Previous Track), <kbd>X</kbd> (Play), <kbd>C</kbd> (Pause / Unpause), <kbd>V</kbd> (Stop), <kbd>B</kbd> (Next Track), <kbd>L</kbd> (Open Files).
  * <kbd>Alt+W</kbd> (Windowshade Mode), <kbd>Alt+G</kbd> (Toggle Equalizer), <kbd>Alt+E</kbd> (Toggle Playlist).
  * <kbd>S</kbd> (Shuffle), <kbd>R</kbd> (Repeat), <kbd>←</kbd>/<kbd>→</kbd> (Seek ±5s), <kbd>↑</kbd>/<kbd>↓</kbd> (Volume ±5%), <kbd>Delete</kbd> (Remove Selected Track).
* **Mini-Player Pill & Media Session API**: Minimized floating pill (`#sounddog-pill`) with animated soundwave indicator, and full integration with OS lock screen controls and hardware media keys (`navigator.mediaSession`).


