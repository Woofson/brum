# Brum Native Core Functions & Chewtoys Manual

Brum's tools come in two kinds:

* **Built-in tools** are part of Brum itself and open as floating windows or docked into a pane.
* **Chewtoys** are add-ons packaged as `.grr` files. They run in a sandbox with no access to your login or to Brum's pages.

| Area | Tools in this manual |
| :--- | :--- |
| Files & folders | Compare (folders, files, duplicates), Splitter & Combiner, Disk Usage, Batch Renamer, Hex Editor, Encrypted Vaults, Trash, Transfer hub |
| Sharing & backup | Sharing Center, Backup & Sync |
| Media & documents | 3D Model Viewer (CAD Studio), PDF Studio, Notes, Media Player, Format Converter |
| Server | Terminal, Fleet Manager |
| Chewtoys | Tetrion (block-puzzle game), Calculator, Colours (colour picker), Subnet Calculator, Workbench; optional: Tailspin (wheel, yes/no, dice) |

For remote storage (SFTP, SMB, NFS, WebDAV, S3, Proton Drive), see the [Remote Protocols guide](protocols.md).

---

## 1. Dual-Mode Architecture (Floating Windows & In-Pane Docking)

Every Core Function and power tool in Brum supports dual operational modes conforming to **Rule 9**:
1. **Floating Window Mode**: Freely draggable via its header drag handle, resizable with corner handles, minimizable to interactive task mini-pills, and maximizable.
2. **In-Pane Docking Mode**: Dockable directly into Panel 1, Panel 2, Panel 3, or Panel 4 via the pane selector or window dock button (`panel-left-close`), maintaining state with zero DOM rebuild overhead.

---

## 2. 3D CAD Studio & Mesh Viewer
* **Viewer Only**: CAD Studio displays and measures models; it does not create or edit them.
* **Format Support**: Stereolithography (`.stl` binary and ASCII), Wavefront (`.obj`), AutoCAD (`.dxf`) and Polygon File Format (`.ply`). STEP and IGES files (`.step`, `.stp`, `.iges`, `.igs`) show as a rough outline built from the file's points, not as full solids.
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
* **Revision Snapshots**: Automated snapshot history in `.notes_versions/` (named `.notedog_versions/` before Brum 1.17; renamed automatically) with side-by-side diff previews and 1-click restore.
* **Templates**: Built-in templates for Project Plans, SOPs, Daily Logs, and Meeting Minutes.
* **Encrypted Notes**: Seamless encryption and decryption support for `.md.enc` files.

---

## 8. Media Player & Synthesizer Studio
* **Unified Audio & Video Engine**: Comprehensive playback for HTML5 video/audio alongside full chiptune, tracker, and Meridian synthesizer playback (`.mp3`, `.flac`, `.wav`, `.ogg`, `.m4a`, `.aac`, `.opus`, `.wma`, `.mid`, `.midi`, `.mod`, `.xm`, `.it`, `.s3m`, `.sid`, `.nsf`, `.vgm`, `.spc`, `.gbs`, `.gym`, `.rgm`, `.rsn`, `.frm`).
* **Floating Mini-Pill Badges**: Minimize playback into an interactive floating pill badge (`#mediaplayer-pill`) with live elapsed progress, track ticker, and quick play/pause controls.
* **Zero-Copy Byte-Range Streaming**: Real-time seeking and scrubbing for multi-gigabyte audio and video files.
* **10-Band EQ & Real-Time Visualizer**: 60 FPS spectrum and waveform visualizers with 8 acoustic presets (*Flat, Bass Boost, Rock, Synthwave, Acoustic/Vocal, Jazz, Classical, Pop*).
* **Playlists & Aspect Ratio**: Full queue management (`.m3u` export/import), video scaling (16:9, 4:3, Cover, Contain), and automatic subtitle detection.

---

## 9. Bite! Slide-Up PTY Terminal
* **Native WebSocket PTY**: Full pseudo-terminal session attached directly to the active pane's directory.
* **Persistent Drawer Sessions**: Toggling or hiding the terminal drawer keeps running processes and active shells alive in the background without restarting sessions.
* **Terminal Tabs**: Run up to 8 shells side by side. **+** opens a new one in the active pane's folder, **×** or a middle-click closes one, and typing `exit` closes its tab. "Open in Terminal" adds a tab for that folder instead of replacing what is running. Tabs on Fleet nodes are labelled with the node's name, and all tabs move along when the terminal is docked into a pane.
* **Clipboard**: Copying in tmux, vim and other programs that use OSC 52 lands on your computer's clipboard (HTTPS or localhost). Programs can only write to the clipboard, never read it.
* **Shell Support**: Automatically detects and loads your default shell (`fish`, `zsh`, `bash`, `powershell`, `cmd.exe`).
* **Bundled Nerd Fonts**: Embedded `JetBrainsMono Nerd Font` provides out-of-the-box support for `eza --icons`, `starship`, and powerline glyphs without missing boxes or tofu characters.
* **Dynamic Geometry**: Automatic `ResizeObserver` recalculates dimensions and synchronizes `{ cols, rows }` across pane and window resizes.

---

## 10. Backup & Sync
Open it from **Tools → Backup & Sync**. It floats like other tools and can be docked into a Panel.

### My backups
The main screen lists your backups. Each card shows what is backed up and where, a plain status line ("Up to date · backed up 10 min ago", or why the last run failed), when it runs next, and **Back up now**, **Restore** and **History**. The pencil edits a backup, pause stops its schedule, and the bin removes the backup from Brum (the backed-up files are kept).

### Making a backup
1. **What to back up**: one or more folders. With several folders, each gets its own subfolder in the backup, named after it ("Pictures", "Pictures (2)" for a second folder with the same name).
2. **Where to keep it**: a folder on this machine, another drive, or a mounted NAS share; or a remote place (see below). The folder is created if needed.
3. **How**:
   * **Copy**: an exact copy. Files you delete are removed from the backup too.
   * **Copy and keep old versions** (default): an exact copy, and replaced or deleted files are kept in a hidden `.brum-versions` folder, in dated folders, for 30 days (adjustable; 0 keeps them forever).
   * **Keep two folders the same**: changes on either side are copied to the other, deletions too. Brum remembers the last sync, so it can tell a deleted file from a new one. A file changed on both sides keeps the newer version; the other is kept as an old version.
   * **Keep a history in git** (a *collection*): gathers files and folders from anywhere (configs, `/etc/hosts`, KiCad projects, PDFs) into one git repository, without moving them. Every run saves a version (a commit) you can go back to. See *Git collections* below.
4. **When**: only when you click, every few minutes, every few hours, or every day at a set time. Only changes are copied, so frequent checks are cheap. If Brum was off at the planned time, the backup runs when Brum starts. A backup never runs twice at the same time.

Saving a new backup shows a **preview** first ("Will copy 1,204 new files…"), with the full list, before anything is copied.

**More options**: files and folders to skip (a plain name such as `node_modules` skips anything with exactly that name, `*.tmp` uses a wildcard, `Photos/Raw` counts from the top of the backup); check every copy after writing it; update only the changed parts of large files; never remove files from the backup; how long to keep old versions and logs; a web address to notify when a backup finishes or fails; and whether Brum or rsync does the copying.

### Remote places
| Place | How to write it | Notes |
| --- | --- | --- |
| SFTP server (e.g. Hetzner Storage Box) | `sftp://user@server:port/folder` | Logs in with a password (kept with the backup, never shown again), a key file, the SSH agent or the keys in `~/.ssh`. Host keys are checked as set in `[storage.sftp]`. |
| rclone remote | `rclone://remote/folder` | Any remote set up with `rclone config` by the user Brum runs as (Samba, S3, cloud drives…). Logins stay in rclone's own config. The form lists your remotes. |
| Another Brum in the Fleet | `fleet://machine/folder` | Machines from `[[fleet.nodes]]` or saved in Fleet. The receiving Brum must be this version or newer; it writes as the user its token belongs to. The form lists your Fleet machines. |
| Proton Drive | `proton:///my-files/Backups/Photos` | Uses the backup owner's own Proton login (Settings → Proton Drive). **Copy** and **Copy and keep old versions** only: Proton keeps replaced files in its own version history and removed files in its trash. See the [Proton Drive guide](proton-drive.md#4-backups-to-proton-drive). |

Remote places are for admins and users explicitly allowed to use them. **rsync** can do the copying for Copy backups to a folder or an SSH server; it logs in with SSH keys only.

### Safety
* Files are copied under a temporary name, checked, and only then swapped in: an interrupted run never leaves half-written files in the backup.
* If what you back up is suddenly empty or missing (for example a NAS that is not mounted) while the backup has files, the run stops with a message instead of emptying the backup. With several folders this is checked for each one.

### Git collections
* **What to track**: files as well as folders (type a file's path, or browse for a folder). Inside the repository they keep their real paths: `home/…` for your home folder, `root/…` for everything else, so `~/.bashrc` is `home/.bashrc` and `/etc/hosts` is `root/etc/hosts`. A `brum-collection.toml` file in the repository lists what is tracked and from where.
* **Where**: a folder on this machine; Brum makes it a git repository. Copying never touches its `.git` folder.
* **Versions**: each run that changed something saves a version with a short note ("Updated 2, 4 KB"). Git keeps the history, so there is no `.brum-versions` folder.
* **Pushing**: set a remote (`git@github.com:you/dotfiles.git`, `ssh://…`, `https://…`, or a folder on this machine) and press **Push** on the card, or tick **Push after every saved version**. It logs in with the SSH keys or saved git login of the user Brum runs as. Other kinds of remote addresses are refused.
* **Before it goes anywhere**: the preview flags files that look like passwords or keys (`id_rsa`, `.env`, `*.key`, …) when the collection is pushed, and files over 50 MB, since every version of them stays in the repository.
* **Restore**: pick a saved version; the files come back exactly as they were then, into a separate folder or, after confirming, their original places.

### Restore
Choose **Latest backup** or a dated set of old versions, tick files or folders (nothing ticked restores everything), and where to put them:
* **A new, separate folder** (default), next to the original, with the date in its name. Nothing existing is touched.
* **Another folder** of your choice.
* **The original folder**: Brum first says how many files would be replaced and asks you to confirm. Files that are already exactly as in the backup are skipped.

### History and logs
Every backup and restore is listed with a one-line result. Open one to see every file that was copied, updated, removed, kept as an old version, skipped or failed (with the reason), filter to **Problems only**, or **Download log** as a text file. Logs are kept for 90 days by default.

---

## 11. Format Converter
* **Requirements**: Video and audio conversion uses FFmpeg; image conversion uses ImageMagick (or FFmpeg). These must be installed on the server. Data files (JSON, TOML, YAML) convert without extra programs.
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

### Bundled Chewtoys
Calculator, Colours, Subnet Calculator, Timestamp Converter, Workbench and Tetrion ship inside Brum as ordinary Chewtoys:
* **Installed on first start** into the Chewtoy folder (`[plugins] user_directory`), like any `.grr` package.
* **Kept up to date**: when a newer Brum brings a newer version, the installed copy is updated on start.
* **Uninstall freely**: an uninstalled bundled Chewtoy stays uninstalled; Brum remembers this in `.bundled.json` in the Chewtoy folder.
* **Settings → Chewtoys → Bundled with Brum** lists each one with the bundled and installed version, and offers **Install**, **Update** or **Reinstall** and a **Download** of the `.grr` package.

### Colours (Modular Chewtoy)
* **Five colour systems**: OKLCH, HWB, HSL, HSV and RGB, each with sliders, exact number boxes and transparency. In OKLCH, slider gaps show colours a normal screen cannot display.
* **Any colour in, every format out**: Type or paste hex, `rgb()`, `hsl()`, `hwb()`, `oklch()` or a colour name; copy HEX, RGB, HSL, HWB, OKLCH or HSV with one click.
* **Tools**: Tints and shades, colour harmonies (complementary, analogous, triadic, split complementary, tetradic), a WCAG contrast check, saved and recently copied colours, and the current Brum theme's colours with "copy as theme variable".
* **Eyedropper**: Pick a colour from anywhere on screen (Chrome and Edge).

### Subnet Calculator (Modular Chewtoy)
* **IPv4**: Type `192.168.1.10/24` or an address with a netmask (`10.0.0.1 255.255.0.0`): network, netmask, wildcard mask, broadcast, first and last usable address, host counts, the address type (private, CGNAT/Tailscale, link-local, public…) and a binary view of network and host bits. `/31` and `/32` are handled as point-to-point and single-host networks. Check whether another address is inside the network.
* **Split & combine**: Split a network into a number of parts, into networks of a given prefix, or into networks with room for a number of hosts. Combine a list of networks into the fewest networks with exactly the same addresses, and see the single network covering them all.
* **IPv6**: Network, first and last address, number of /64 networks, short and full notation, address type and the reverse DNS (`ip6.arpa`) zone.
* **Reference**: The special IPv4 and IPv6 ranges with what they are for; click one to open it in the calculator.
* Click any result to copy it. Recent entries are remembered, and numbers follow **Region & Formats**.

### Timestamp Converter (Modular Chewtoy)
* **Convert**: Paste Unix time (seconds, milliseconds, microseconds or nanoseconds, guessed from the number of digits or chosen), an ISO 8601 / RFC 3339 or RFC 2822 date, or a whole log line (web server, system log, Brum's own log): the date is found and shown as your time, UTC, all Unix forms, ISO 8601, RFC 2822, HTTP date and web server log format, with how long ago it was, the ISO week and the day of the year. Dates without a time zone are read as your time, or as a zone you pick ("what is 15:00 in New York for me?").
* **Time zones**: The same moment side by side in your favourite zones, with the difference from you and "yesterday"/"tomorrow" where the date differs. Add zones by city or name (Tokyo, America/New_York).
* **Durations**: The time between two moments, in days, hours and minutes, in calendar terms and as totals; add or subtract a period (`90d`, `2h 30m`, `1 year 2 months`, `P1DT12H`). Days and months follow the calendar, so adding one day across a clock change keeps the same clock time.
* A live clock with the current Unix time sits at the top. Click any result to copy it. Recent entries are remembered, and dates and times follow **Region & Formats**.

### Workbench (Modular Chewtoy)
The conversions and calculators you would otherwise search the web for in the garage or workshop. Works offline. Type in the search box ("awg", "m6", "1/4-20", "psi", "ohm") and press <kbd>Enter</kbd> to jump straight to a tool; a thread or wire size opens its row. Pin tools with ☆ to keep them at the top. Click any result to copy it. Inputs are remembered per tool, and numbers and metric/imperial defaults follow **Region & Formats**.

* **Units**: length, area, volume, weight, pressure, torque, temperature, speed, flow, power and energy, shown in every unit at once; mm ↔ inch with the nearest 1/16", 1/32" and 1/64" fraction (type inches as `0.375`, `3/8` or `1 3/8`).
* **Electrical**: AWG table with diameter, mm², resistance per km and per 1000 ft and typical current (chassis wiring and power transmission), with the next metric cable size up; Ohm's law and power from any two of V, I, R, P; resistor colour codes both ways for 4, 5 and 6 bands with the nearest E12/E24/E96 values (values like `4k7`, `0R47`, `2M2`); LED series resistor and its power rating; voltage divider, including the resistor for a wanted voltage; cable voltage drop for copper or aluminium; battery energy, usable capacity and runtime.
* **Threads & drills**: tap drills for metric coarse and fine (as the standard charts give them), UNC/UNF, and BSP/NPT pipe threads; ISO 273 clearance holes (close, medium, loose); number, letter and fractional drills ↔ mm, with the nearest drill to any size.
* **Shop**: weight of a plate, bar, tube or hex in 22 materials; area and volume of common shapes; spindle speed for drilling, turning and milling (HSS or carbide); gear and pulley ratio; lever; slope in %, degrees and 1:x.

Current ratings and cutting speeds are typical starting points; follow your local electrical code and the tool maker's figures.

### Tailspin (Optional Chewtoy)
Tailspin is maintained with Brum but not built in. Install `plugins/tailspin.grr` from the [Brum repository](https://github.com/Woofson/brum/tree/main/plugins) through **Settings → Chewtoys → Install**.
* **Wheel**: Type or paste entries, one per line (up to 500), and spin. Keep several named lists; optionally remove the winner after each spin to draw names until the list is empty. The result is shown as one plain line.
* **Yes / No**: One button, one answer.
* **Dice**: One die: tap it or press Roll for a short tumble and the result. D&D: d4, d6, d8, d10, d12, d20 and d100, rolls like `2d6+3` or `1d8+1d6+2` (up to 100 dice), and d20 with advantage or disadvantage. Yatzy: five dice, click to hold, three rolls per turn.
* **Fair results**: Every spin, answer and roll uses the browser's secure random generator.
* **Quiet by default**: Sound is off until you turn it on. With "reduce motion" set on your computer, the wheel stops at once instead of spinning.
* **Keys**: Space or Enter spins, asks or rolls on the open tab.

---

## 16. XDG Trash Suite & File Restoration
* **Linux XDG Compliance**: Implements the FreeDesktop.org Trash specification (`~/.local/share/Trash/files` and `~/.local/share/Trash/info`).
* **`.trashinfo` Metadata**: Generates standard metadata containing exact original file paths and RFC 3339 deletion timestamps.
* **Safe 1-Click Restoration**: Restores files or folders back into their exact original filesystem directory with path recreation and collision protection.
* **Places Hub Integration**: Accessible directly from the Favorites/Places bar (`btn-favorites`) with real-time item counter badges and instant navigation.
* **Deletion Safeguards**: Dual-mode protection with visual confirmation dialogs for safe Trash movements (<kbd>F8</kbd> / `Delete`) and irreversible permanent purge (<kbd>Shift+F8</kbd> / <kbd>Shift+Delete</kbd>).

---

## 17. Consolidated Transfer & Ingest Split-Button Hub
* **Unified Panel Header Split-Button**: Replaces cluttered action buttons with an ergonomic split button on panel headers.
* **Customizable 1-Click Action**: Main click executes user-configured action (defaults to `Copy to Other Pane`, configurable to `Move`, `Clone Path`, `Upload Files`, `Upload Folder`, or `Download Zip`).
* **Categorized Ingest Hub**: Chevron trigger reveals 3 structured sections:
  1. *Dual-Pane Transfers*: Cross-pane copy (<kbd>F5</kbd>), move (<kbd>F6</kbd>), duplicate, and path cloning.
  2. *Device Ingest & Egress*: File and folder uploads, drag-and-drop ingestion, and zip download packaging.
  3. *Sharing & Links*: Direct public link creation, QR codes, and client upload dropzones.

---

## 18. Distributed Fleet Manager & Node Diagnostics
* **Remote Fleet Administration**: Centralized hub in Settings (and the Paw header menu) to connect, monitor, and manage remote Brum instances across LAN, WAN, or private mesh networks.
* **Declarative Config Defaults (`[[fleet.nodes]]`)**: Fleet nodes defined in `/etc/brum/config.toml` are automatically ingested into the Fleet Manager with `Config Default` badges. If updated in `config.toml`, changes automatically propagate into the UI upon service restart or reload.
* **Granular User Overrides**: Editing node properties (such as display name, endpoint URL, start path, or auth credentials) in the Fleet Manager UI flags the profile with an `Override` badge and persists local customizations without mutating the master configuration file on disk.
* **1-Click Reset to Config Defaults**: A dedicated reset button (`rotate-ccw`) in the Fleet Manager allows users to instantly clear local overrides and restore the pristine settings defined in `config.toml`.
* **Zero-CORS Gateway Hub Relaying**: Nodes configured with `auto` or `proxy` routing route all REST API calls and WebSocket terminal sessions through the hub's `/api/fleet/proxy/:node_id` endpoint, preventing browser cross-origin CORS blocks and mixed-content HTTPS warnings.
* **Shared Cluster Secret Federation**: With `BRUM_JWT_SECRET` configured in `/etc/brum/brum.env` across the cluster, nodes operate with `token = ""` for unified single sign-on without manual per-node token provisioning.
* **Node Home Auto-Discovery & Custom Paths**: Automatically fetches the user's remote home directory or opens configured custom start paths per node.
* **Node-Aware Places & Breadcrumbs**: Seamlessly switch active panel context to remote nodes; Places drawer automatically loads remote node bookmarks and favorites.
* **Live Health & Version Drift Diagnostics**: Instant connection testing reporting real-time ping latency, remote OS/architecture, build number (`#<N>`), git commit, and UTC compile timestamp to quickly pinpoint outdated nodes across your fleet.

---

## 19. Compare (two folders, two files, duplicates)
Open it with <kbd>F9</kbd> / <kbd>Ctrl+D</kbd> or **Tools → Compare**; **Find Duplicates** in the Tools menu opens it on Duplicates. It floats like other tools and can be docked into a Panel. At the top, choose what to compare:

### Two folders
* The two folders open in your panels are filled in; change either one, browse for another, or swap them. **Use my panels** fills them in again.
* **Compare** looks at sizes and dates, including subfolders (untick **Include subfolders** for just the top level). **Compare contents** also compares every file's contents with checksums: slower, but certain.
* A one-line summary ("12 differ, 5 only on the left, 2 only on the right, 312 the same") sits above the list. A folder that exists on both sides counts as different when anything inside it differs.
* Filter the list (Modified, Left only, Right only, Identical), search it, and sort it. Each row can open a side-by-side view or be copied across.
* **Copy across**: **Left to right**, **Right to left** or **Both ways** copies files that are missing or newer on the other side into the same subfolders. Brum says how many files it will copy and asks first; nothing is deleted.

### Two files
Choose two files, or press **Use selected files** (one file selected in each panel, or two in one panel). Text files are shown side by side or inline with the changes marked; images are shown next to each other or with a slider. <kbd>F9</kbd> with one file selected in each panel opens this view directly.

### Duplicates
Choose a folder, the smallest size to look at and a file type, then **Find Duplicates**. Files are compared by their contents (size first, then a quick fingerprint, then SHA-256), so names do not matter. Pick which copies to remove by hand or with **Keep Newest**, **Keep Oldest** or **Keep Shortest Path**, then **Clean Selected**: files go to the trash by default, and Brum asks before anything happens.
