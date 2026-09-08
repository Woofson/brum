# 📜 Changelog

All notable changes to **CommanderDog** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.0-rc6] - 2026-09-08

### Responsive 60/40 File Table Columns & Touch-Friendly Resizing
- **Proportional 60/40 Column Allocation on Phone & Tablet**: Allocated ~60% horizontal priority to the `Name` column on Phone (<600px) and Tablet/Foldable (601px–1024px) viewports, distributing remaining space proportionally across secondary columns (Size, Modified, Mode, Owner) to prevent long filenames from being squished.
- **Touch-Friendly Pointer Event Column Resizers**: Migrated column resize handlers to Pointer Events (`onpointerdown` with `setPointerCapture`) and expanded touch hitboxes (`30px` on coarse pointers), enabling effortless finger/stylus column dragging on touchscreens.
- **Semantic File Table Cell Classes**: Added dedicated semantic CSS classes (`file-cell-ext`, `file-cell-size`, `file-cell-modified`, `file-cell-created`, `file-cell-mode`, `file-cell-owner`, `file-cell-group`, `file-cell-hash`, `file-cell-tags`) across all table rows for precise viewport-level styling.

## [0.8.0-rc5] - 2026-09-08

### Single-Line File Listing Optimization & Customizable Display Preferences
- **Single-Line File Listing Default**: Re-established clean, space-optimized single-line rows across all viewports (Phone, Tablet, Desktop) with standardized 32px height, prominent MIME icons, and clear `filename.ext` alignment.
- **Compact Date Formatting**: Introduced short, legible timestamp formatting (`MM-DD HH:mm` for current-year files, `YYYY-MM-DD` for older entries) enabled by default to conserve horizontal screen space.
- **Togglable `<DIR>` Directory Indicators**: Directory sizes now default to clean blank entries, with an optional toggle to show classic `<DIR>` tags in the size column.
- **Optional Phone Multi-Line Subtext**: Retained rich multi-line metadata subtext (permissions, owner, size, date) as a togglable user preference (`cd_file_list_multiline`) configurable in Settings and the column header context menu.
- **Context Menu & Settings Integration**: Added display options toggles directly inside the table column chooser context menu and General Settings modal.

## [0.8.0-rc4] - 2026-09-08

### Responsive Viewports, Mobile Permissions Subtext & Standardized Form Controls
- **Phone Viewport Permissions & Ownership Subtext**: Added mobile metadata subtext (`.file-subtext-mobile`) displaying file size, UNIX permissions (`rwxr-xr-x`), and owner/group (`user:group`) directly below file names on Phone viewports (<600px) (#8).
- **Adaptive Micro-Text Typography Scaling**: Tuned font hierarchies, line heights, tag badges, breadcrumbs, and bottom action bar buttons for Phone touch viewports (<600px) and Tablet/Foldable screens (600px–1024px) (#5, #6).
- **Theme Engine Standardized Form Controls**: Harmonized theme-aware form styling across *Woofsons Amber Charcoal* (Dark) and *Woofsons Amber Zink* (Light), featuring custom amber-accent tickboxes, radio buttons, focus rings, and custom SVG chevron dropdown selects (#10).
- **Token-Efficient GitHub Backlog Triage Helper**: Added [`scripts/issues.sh`](file:///home/bolt/projects/commanderdog/scripts/issues.sh) CLI utility for compact, token-efficient issue queries and category digests.

## [0.8.0-rc3] - 2026-09-08

### Integrated Terminal WebSocket Safety, Prompt Redraw & Viewport Fit
- **WebSocket Connection Race Conditions**: Resolved race condition where rapid socket reconnection or drawer toggles caused `[WebSocket connection error]` by properly detaching previous event listeners before disposal and guarding connection states (#16).
- **Prompt Duplication Fix**: Removed forced backend `PS1` and `PROMPT_COMMAND` overrides, calibrated initial PTY dimension exchange, debounced frontend terminal resize signals, and added backend size deduplication to prevent spurious `SIGWINCH` duplicate prompt redraws on launch.
- **Docked Viewport Layout & Overflow**: Fixed bottom line cutoff and vertical overflow in docked pane mode by setting strict box-sizing, flex column layouts, and container constraints for `.terminal-body` (#16).

## [0.8.0-rc2] - 2026-09-08

### NoteDog Docked Preview & Split Markdown Engine
- **Full Markdown Preview in Docked Mode**: Unified the Markdown parsing engine across floating and docked NoteDog modes; docked preview now renders rich Markdown headings (`#`, `##`, `###`), fenced code blocks with syntax highlight and Mermaid diagrams, interactive task checkboxes (`- [ ]`, `- [x]`), blockquotes, bullet lists, tables, and color spans (#2).
- **Split View Layout Alignment**: Ensured side-by-side split view mode (edit on left, live rendered Markdown preview on right) expands with flex container column and full width alignment across all docked panes (#2).
- **Live Bi-Directional Input & View Mode Sync**: Synchronized real-time typing, view mode switching (`edit`, `split`, `preview`), and interactive checkbox clicks between floating and docked NoteDog instances.

## [0.8.0-rc1] - 2026-09-08

### Header Layout, Icons & Responsive Viewport Enhancements
- **Header Tools & Task Manager Order**: Swapped the header layout sequence so the ChewToys & Tools Launchpad menu precedes the Background Tasks & Transfers pill (#11).
- **Modernized Upload Icon**: Replaced legacy cloud-upload icon (`upload-cloud`) with clean, streamlined tray upload indicator (`upload`) across desktop headers, context menus, pane toolbars, and mobile bottom action bars (#12).
- **Responsive Branding Badge Visibility**: Refined header logo text visibility rules to automatically hide on Phone screens (<600px) to maximize toolbar space while displaying clearly on Tablet/Foldable screens (600px–1024px) and Desktop viewports (#7).

### Mobile Bookmarks & NoteDog Docking Fixes
- **Mobile Phone Bookmarks Manager Trigger**: Resolved issue where tapping Bookmarks in the mobile pane tools drawer failed due to hidden desktop button anchoring; implemented fixed popup viewport positioning, direct Bookmarks Manager trigger, and outside-touch auto-dismissal (#9).
- **NoteDog Docked Selection Lag & Stale Content**: Fixed selection delay and stale content rendering in docked mode by making notebook/section/note selection handlers async and dynamically synchronizing all active docked pane textareas, previews, and header titles (#1).

## [0.7.9] - 2026-09-07

### ⚡ HTTP Range Request & Media Streaming Engine Fix
- **HTTP `Range` Header Streaming (`206 Partial Content` & `416 Range Not Satisfiable`)**:
  - **Continuous Media Playback Engine Fix**:
    - Resolved the audio playback stalling issue occurring at ~20 seconds where HTML5 `<audio>`/`<video>` elements pause initial buffer chunks and request subsequent byte slices via `Range: bytes=<offset>-`.
    - Fully implemented RFC 7233 / 9110 Range parsing with asynchronous chunk slicing (`build_local_file_range_response` and `build_bytes_range_response`) for local files, `vault://`, `smb://`, and `sftp://` media streams.

### 🏷️ ARFAMP Official Rebranding
- **Winamp / SoundDog ChewToy Rebranded to `ARFAMP`**:
  - Rebranded the audio player & Winamp 2.x clone chewtoy to **`ARFAMP`** across the entire UI and documentation.
  - Updated main player window titlebar (`ARFAMP 2.91`), equalizer title (`ARFAMP EQUALIZER`), and playlist title (`ARFAMP PLAYLIST`).
  - Updated context menu actions to "Play in ARFAMP ▶", "Add to ARFAMP Queue ➕", and "Play Folder in ARFAMP".
  - Updated Spotlight Search descriptors and Tools menu registry to `ARFAMP`.
  - Updated default marquee scrolling ticker to `*** 1. Woofsons Lab - ARFAMP 2.91 Jukebox ***`.
  - Updated minimized floating pill badge and title to `ARFAMP`.
  - Updated playlist export default filename to `ARFAMP_Playlist_YYYY-MM-DD.m3u`.

## [0.7.8] - 2026-09-07

### 📻 ARFAMP: Authentic Winamp 2.x Clone & Studio Equalizer ChewToy
- **18th Built-in Power ChewToy (`ARFAMP`)**:
  - **Modular Winamp 2.x Multi-Window Architecture**:
    - **Classic 3-Module Layout**: Main Player Window, 10-Band Graphic Equalizer, and Playlist Editor.
    - **Window Shade Mode**: Individual windowshade collapsing for Main Window, EQ, and Playlist via titlebar button `▲`/`▼`, titlebar double-click, and global shortcut <kbd>Alt+W</kbd>.
    - **Green/Amber Fluorescent LED 7-Segment Digital Timer**: Real-time timer display with interactive toggle between *Time Elapsed* and *Time Remaining* (`-MM:SS`).
    - **Marquee Scrolling Track Ticker**: Amber LED track title ticker with bitrate (`KBPS`), sample rate (`KHZ`), and `STEREO`/`MONO` active channel indicators.
    - **Real-Time 60 FPS Winamp Visualizer**: Canvas visualizer featuring 18-band segmented green/amber/red LED spectrum analyzer with falling peak caps, CRT phosphor oscilloscope waveform, and ambient glow.
    - **10-Band Graphic Equalizer (EQ)**: Authentic Winamp center frequencies (`60Hz`, `170Hz`, `310Hz`, `600Hz`, `1kHz`, `3kHz`, `6kHz`, `12kHz`, `14kHz`, `16kHz`), Preamp fader (-6dB to +6dB), ON/AUTO switches, and presets (*Flat, Bass Boost, Rock, Synthwave, Acoustic/Vocal, Jazz, Classical, Pop*).
    - **Playlist Editor**: Monospace numbered track list (`1. Artist - Title (MM:SS)`), filter search bar, selection tracking, drag & drop track loading, and action buttons (`+FILE`, `+DIR`, `-FILE`, `-ALL`, `SHUF`, `LIST` .m3u export).
    - **Authentic Winamp Keyboard Shortcuts**:
      - <kbd>Z</kbd> (Previous Track), <kbd>X</kbd> (Play), <kbd>C</kbd> (Pause / Unpause), <kbd>V</kbd> (Stop), <kbd>B</kbd> (Next Track), <kbd>L</kbd> (Open Files).
      - <kbd>Alt+W</kbd> (Windowshade Mode), <kbd>Alt+G</kbd> (Toggle Equalizer), <kbd>Alt+E</kbd> (Toggle Playlist).
      - <kbd>S</kbd> (Shuffle), <kbd>R</kbd> (Repeat), <kbd>←</kbd>/<kbd>→</kbd> (Seek ±5s), <kbd>↑</kbd>/<kbd>↓</kbd> (Volume ±5%), <kbd>Delete</kbd> (Remove Selected Track).
  - **Full-Spectrum Background Audio Engine**: Continuous uninterrupted audio streaming and queue playback that persists across directory navigation, pane transitions, and background workflows.
  - **Dual-Mode Operational Architecture**: Draggable & resizable floating window (`42px` grab handle header with standard `28px` action controls) and in-pane docking into Panels 1–4 with automatic panel state persistence.
  - **Mini-Player Pill & Media Session API**: Minimized floating pill (`#sounddog-pill`) with animated soundwave indicator, and full integration with OS lock screen controls and hardware media keys (`navigator.mediaSession`).
  - **File Commander & Context Menu Integration**: "Play in ARFAMP ▶" and "Add to ARFAMP Queue ➕" context menu options on audio files and directories, and double-click handler for all audio file extensions (`.mp3`, `.flac`, `.wav`, `.ogg`, `.m4a`, `.aac`, `.opus`, `.webm`, `.weba`).

## [0.7.7-rc7] - 2026-09-07

### 🐛 TetraDog Input Lifecycle & Window Visibility Specificity Fix
- **Floating Window Display & Close Specificity Fix**:
  - Removed aggressive `display: flex !important;` rule on `.floating-tetradog-window` that was overriding inline `style="display: none;"` states on mobile.
  - Fixed issue where the window would render in an uninitialized state while `win.style.display` remained `'none'`, causing keyboard and touch inputs to be rejected and preventing `closeTetraDog()` from hiding the window.
  - Refined mobile modal header cleanup selectors to ensure `.modal-close-btn` and buttons with `data-lucide="x"` always maintain full clickability (`display: inline-flex !important; pointer-events: auto !important;`).
  - Added explicit `setTetraDogView('game')` in `openTetraDog()` ensuring game canvas views and event handlers are actively mounted on launch.

## [0.7.7-rc6] - 2026-09-07

### 🌐 Mobile Viewport & Vivaldi / Android Bottom Address Bar Collision Fix
- **Dynamic Viewport Height & Bottom Chrome Synchronization**:
  - Resolved mobile browser address bar collisions (specifically in Vivaldi for Android, Chrome, and Samsung Internet with bottom address/navigation bars):
    - Replaced static `100vh` rules on mobile floating windows with dynamic `height: 100dvh` and CSS variable `var(--viewport-height, 100dvh)`.
    - Bound `window.visualViewport` height and offset tracking with dynamic `--mobile-bottom-offset` calculations and `max(env(safe-area-inset-bottom), var(--mobile-bottom-offset))` padding.
    - Updated TetraDog arena and canvas container to dynamically scale (`max-height: calc(var(--viewport-height, 100dvh) - 200px - ...)`), fully preventing bottom status bars and NES gamepad thumb controls from being obscured behind bottom browser address bars.

## [0.7.7-rc5] - 2026-09-07

### 📱 ChewToy & Modal Mobile Header Refinement: Clean Window Controls
- **Mobile Header Clutter Elimination**:
  - Automatically hidden redundant desktop window controls on Phone viewports (`@media (max-width: 640px)`):
    - **Dock Button** (`panel-left-close` / `dock...`), **Minimize Button** (`minus` / `minimize...`), **Maximize/Fullscreen Button** (`maximize-2` / `fullscreen...`), and **Float Button** (`picture-in-picture` / `external-link`).
  - Standardized phone header actions across all ChewToys (EditorDog, NoteDog, Calculator, TetraDog, Task Manager, Universal Document/PDF Viewer, Image Viewer, Git Manager, Disk Usage Analyzer, Bite! Terminal Console) to display strictly the essential ChewToy brand / view switcher and the **Close Button** (`x`).
  - Added universal CSS and layout cleanup rules ensuring zero horizontal header squishing or accidental mis-taps on phone screens.

## [0.7.7-rc4] - 2026-09-07

### 🎮 TetraDog: NES Retro Controller & Ultra-Compact Mobile Viewport Optimization
- **Tactile Virtual NES-Style Thumb Controller**:
  - Implemented an authentic, thumb-friendly NES retro layout at the bottom of the screen designed specifically for single-handed and dual-thumb mobile play:
    - **Left Thumb Zone**: 4-Way tactile cross D-Pad (<kbd>↑</kbd> Hard Drop / Slam, <kbd>←</kbd> Move Left, <kbd>→</kbd> Move Right, <kbd>↓</kbd> Soft Drop) with instant tactile touch feedback.
    - **Center Zone**: Rubber-style pill buttons for <kbd>HOLD</kbd> (Select) and <kbd>PAUSE</kbd> (Start).
    - **Right Thumb Zone**: Angled <kbd>B</kbd> (Rotate Counter-Clockwise `↺`) and <kbd>A</kbd> (Rotate Clockwise `↻`) round action buttons with ruby and amber gradients for intuitive thumb rolling.
  - Full multi-touch support (`{ passive: false }` with `preventDefault`) preventing browser viewport scrolling, zoom, or gesture conflicts while holding directional buttons and tapping rotation.
- **Ultra-Compact Mobile Display Layout (Z Fold & Slim Screen Optimization)**:
  - Engineered compact layout specifically tailored for narrow displays (such as Galaxy Z Fold outer screens ~374–412px wide and compact phones):
    - **Mini Side Cards**: Scaled Hold card (`44×44px`) and Next preview card (`44×96px`) flanking the main board.
    - **Symmetrical Mobile Top HUD**: Centered Mode badge and Top Score HUD directly above the board canvas.
    - **Adaptive Auto-Scaling Arena**: Board canvas dynamically maintains exact 1:2 aspect ratio (`aspect-ratio: 1 / 2; max-height: calc(100vh - 210px)`) fitting the entire board, sidecards, HUD, NES controller, and status bar with zero horizontal or vertical overflow.

## [0.7.7-rc3] - 2026-09-07

### 🕹️ TetraDog: Classic Arcade Tetris ChewToy & Synchronized Leaderboard
- **Full-Spectrum Arcade ChewToy (`TetraDog`)**:
  - Implemented 17th built-in ChewToy with dual operational modes: freely draggable/resizable floating window (`42px` header with grab handles) and in-pane docked mode across Panes 1–4.
  - **Status-Bar & Toolbar Optimization**:
    - Relocated live **Score**, **Level**, and **Lines** counters down to a dedicated bottom status bar (`.tetradog-statusbar`).
    - Positioned mode badge (`MARATHON` / `SPRINT` / `ULTRA`) and **Top Score** prominently inside the main game arena stats card.
    - Added inverted high-contrast sound toggle trigger in the status bar with live mute indicator.
    - Cleaned top window header bar to strictly standard Orthodox actions (view switchers, dock, minimize, maximize, close) with theme selection consolidated exclusively under TetraDog Settings.
  - **Mobile Touch & Tablet Viewport Optimization**:
    - Compact 3-column responsive layout for Phone and Tablet viewports fitting game board, hold canvas, next preview, and enlarged touch virtual D-pad without vertical viewport clipping.
  - **Grid & Coordinate Alignment**: Standardized active board to 20 visible rows (`10×20`), eliminated top-buffer clipping gaps, and enabled immediate piece visibility upon spawn.
  - **Instant Play Lifecycle**: Auto-starts falling tetrominoes on window open or keypress without requiring extra modal clicks.
  - **Frame-Accurate 60 FPS HTML5 Canvas Engine**: Zero-lag fixed-timestep game loop (`requestAnimationFrame`) with sub-pixel crisp rendering and master gravity scaling (Levels 1–20+ / 20G instant drop).
  - **Authentic Mechanics & Guideline Parity**:
    - **Fair 7-Bag Randomizer**: 7-bag piece generation eliminating piece droughts.
    - **Super Rotation System (SRS)**: Complete 4-state rotation with 5-point wall kick tests for standard pieces (`J`, `L`, `S`, `T`, `Z`) and `I` tetromino, plus optional classic NES single-rotation toggle.
    - **Ghost Piece Projection**: Accurate translucent landing shadow preview.
    - **Hold Queue & Next Preview**: Instant 1-swap hold slot (<kbd>C</kbd> / <kbd>Shift</kbd>) and 3-piece next queue.
    - **Lock Delay & Scoring**: 500ms lock delay with 15-reset safety limit, Back-to-Back Tetris multipliers (`1.5×`), and T-Spin detection.
    - **3 Game Modes**: Marathon, Sprint (40 lines time attack), and Ultra (3-minute score attack).
  - **Competitive Controls & Tuning**:
    - Customizable **DAS** (Delayed Auto Shift, 60–250ms) and **ARR** (Auto Repeat Rate, 0–50ms / instant shift).
    - Responsive mobile / tablet on-screen touch virtual D-pad controls.
  - **Zero-Dependency 8-Bit Web Audio Synthesizer**:
    - Pure browser Web Audio API synthesized retro sound effects (move clicks, rotate chirps, hard drop slam, lock sound, line clear chords, Tetris fanfare, level-up arpeggios, and game over chimes) with master volume slider and 1-click mute.
  - **Synchronized SQLite Multi-User Leaderboards**:
    - Server-side SQLite `tetradog_scores` table and REST API endpoints (`/api/chewtoys/tetradog/scores`).
    - Synchronized instance-wide leaderboards displaying global Top 50, user rankings, personal bests, player aliases, line clears, and timestamps across all CommanderDog user accounts.
  - **Retro Theme Palettes**:
    - Instant 1-click theme switching between *Woofsons Amber Charcoal*, *Game Boy Monochrome Green*, *NES 8-Bit Vibrant*, and *Arcade Cyberpunk*.

## [0.7.6] - 2026-09-07

### 🔄 Complete Cross-Device User Preference Sync & Docker Persistence Engine
- **Full-Spectrum User Preference Synchronization**:
  - Cloud preferences engine persists and synchronizes all 15+ UI/UX customization dimensions across web sessions, mobile viewports, and desktop clients:
    - **Themes & Custom Palettes**: Active theme (`cd_theme`), custom themes (`cd_custom_themes`).
    - **Typography & Sizing**: Base UI font size (`cd_font_size`).
    - **Border & Active Pane Styles**: Pane border width (`cd_border_width`), active ring style (`cd_ring_style`).
    - **Custom Hostname Badging**: Hostname badge toggle, custom label, color palette, style, icon, and font size.
    - **Directory Table Customization**: Column widths (`cd_col_widths`) and column visibility toggles (`cd_col_visibility`).
    - **View Modes & Hierarchy**: Per-pane view modes (details, grid, compact), grid thumbnail sizes, and directory tree toggles.
    - **Navigation & Safety Toggles**: Dotfiles / hidden files toggle, F-keys bottom bar, parent directory `..` row, double-click to navigate parent directory, and auto-open task manager.
    - **Drag & Drop Behavior**: Default DnD action, prompt mode, and paranoid safety confirmation toggle.
    - **Icon Themes & Nerd Fonts**: Global icon theme, global folder icon, custom filetype icon rules, and preset suites (Nerd Fonts & Emojis).
    - **New File Templates Suite**: Custom file creation templates and boilerplate generators.
    - **Editor & Diff Configurations**: Code editor theme, font size, word wrap, minimap, diff whitespace ignore, and split/unified diff view.
    - **Workspaces & Panes**: Custom pane names, custom border tints, default layout, and initial start paths.
- **Native Vector Nerd Font Glyphs Everywhere**:
  - Fully transitioned all folder and directory representations across file listings (Table Details, Grid Gallery, Compact List) and utility modals to crisp, scalable vector Nerd Font glyphs (`` / ``).
- **Instantaneous Real-Time Synchronization**:
  - Attached real-time debounced updates (`queueSaveUserPreferencesToServer()`) across all preference modifiers in Settings Modal, view switchers, grid size controls, tree toggles, DnD configurations, and custom icon creators.
  - Implemented sync-guarding (`skipSync`) during startup hydration to eliminate redundant round-trips and UI flickering.
- **Persistent Across Container Updates**:
  - Preferences persist directly in `commanderdog.db` SQLite database volume, ensuring preferences survive Docker container updates, recreation, and multi-user deployments.

## [0.7.6-rc7] - 2026-09-07

### 🔄 Real-Time Pane Color Cross-Device Synchronization Fix
- **Direct Palette & Swatch Server Sync**:
  - Connected `queueSaveUserPreferencesToServer()` directly to [`setPaneColorPref()`](file:///home/bolt/projects/commanderdog/frontend/app.js#L13142) and [`switchLayout()`](file:///home/bolt/projects/commanderdog/frontend/app.js#L11542), guaranteeing instant server-side persistence whenever a user selects a color swatch, hex code, cycles colors, or toggles layouts.
- **Unrestricted Web / Remote Client Sync**:
  - Decoupled cloud workspace loading and saving from desktop standalone mode guard so remote browser sessions, local networks, and mobile phones seamlessly exchange preference payloads over `/api/user/preferences`.
  - Fixed `SystemStatusResponse` standalone flag ([`src/server/mod.rs`](file:///home/bolt/projects/commanderdog/src/server/mod.rs#L290)) to strictly reflect the server binary's CLI mode.

## [0.7.6-rc6] - 2026-09-07

### 🌐 Cross-Device Tab Naming, Coloring & Workspace Persistence (Web/Server Mode)
- **Server-Backed User Preferences Store**:
  - Implemented `user_preferences` SQLite storage ([`src/auth/mod.rs`](file:///home/bolt/projects/commanderdog/src/auth/mod.rs#L160)) and REST endpoints (`GET`, `POST`, `DELETE` [`/api/user/preferences`](file:///home/bolt/projects/commanderdog/src/server/mod.rs#L80)) allowing per-user persistence of pane configurations across all client devices.
  - Automatically serializes and syncs:
    - Custom pane names / labels (`pane_names`)
    - Custom pane border & badge color themes (`pane_colors`)
    - Default workspace pane layout (`default_layout`, e.g. `layout-dual-vertical`, `layout-quad`)
    - Per-pane initial start directories (`pane_start_paths`)
- **Seamless Frontend Hydration & Live Cloud Sync**:
  - Automatically loads user preferences on startup and authentication ([`loadUserPreferencesFromServer()`](file:///home/bolt/projects/commanderdog/frontend/app.js#L101)).
  - Real-time debounced background sync (`queueSaveUserPreferencesToServer()`) whenever panes are renamed, colors cycled, or layouts switched.
  - Fallback local cache (`localStorage`) ensures instantaneous page rendering without layout shift or UI flickering.
- **Dedicated UI Management & Isolation**:
  - Added dedicated **"Cross-Device Workspace Defaults (Server Mode)"** card in Settings Modal (F10 -> General), enabling 1-click workspace saving or server defaults reset.
  - Added Start Directory configuration and "Save All to Cloud" quick action directly in Pane Settings popover ([`openPaneSettingsMenu()`](file:///home/bolt/projects/commanderdog/frontend/app.js#L13320)).
  - Strict feature isolation (`.web-only-setting`, `.server-only-setting` in [`frontend/app.css`](file:///home/bolt/projects/commanderdog/frontend/app.css#L10013)) hides cloud sync controls in local standalone desktop sessions.

## [0.7.6-rc5] - 2026-09-07

### 👻 Phone Viewport Ambient Pane Ghost Watermark Indicator
- **Ambient Pane Number / Custom Name Watermark**:
  - Added `.pane-ghost-watermark` in the background of active directory panels (`.pane-main-view`) and docked ChewToys on phone viewports (`@media (max-width: 600px)`), rendering the active pane's number (e.g. `1`, `2`) or custom user-assigned name (e.g. `DOWNLOADS`, `SERVER`) in large, subtle, low-opacity typography (`0.055`).
  - Completely non-interactive and transparent to touch gestures (`pointer-events: none; user-select: none; z-index: 0;`), displaying cleanly behind directory file rows without obstructing readability or tap actions.
- **Dynamic Sizing & Accent Color Synchronization**:
  - Clamps typography between huge monospace numerals (`min(44vw, 190px)`) and adaptive letter-spaced custom strings (`.pane-ghost-long`, `min(13vw, 52px)`).
  - Dynamically synchronizes text and color accents with pane rename events (`updatePaneTitles()`) and custom pane color picks (`applyPaneColors()`).
  - Added smooth transition fade-in (`@keyframes ghostFadeIn 0.2s`) when toggling between panes.

## [0.7.6-rc4] - 2026-09-07

### 📱 Responsive ChewToys & Floating Utilities Architecture (Phone & Tablet)
- **Non-Floating Presentation Across Phone & Tablet (`max-width: 1024px`)**:
  - Unified all ChewToys and floating tools ([`NoteDog`](file:///home/bolt/projects/commanderdog/frontend/index.html#L2281), [`EditorDog`](file:///home/bolt/projects/commanderdog/frontend/index.html#L1983), [`Calculator`](file:///home/bolt/projects/commanderdog/frontend/index.html#L2180), Document Viewer, Image Viewer, Task Manager, Disk Usage, DiffDog) into fixed, fullscreen modals (`100vw x 100dvh`) without awkward offsets, dragging overflows, or desktop 2D resize handles.
  - Tablet viewports (`601px - 1024px`) present centered modals or side-by-side splits with clamped dimensions, eliminating accidental off-screen displacement.
- **Icon-Only Branding on Phone Screens (`max-width: 600px`)**:
  - Added `.chewtoy-brand-text` across all utility headers, automatically hiding long branding strings (e.g. "NoteDog", "EditorDog", "Calculator", "Stats: Disk Usage & Storage Treemap Analyzer", "Comparison & Diff Engine") on phone viewports to preserve maximum horizontal real estate for search inputs, actions, and close buttons.
- **NoteDog Mobile Master-Detail Navigation**:
  - Implemented 1-tap master-detail flow on phone screens (`<= 600px`): full-width notebooks/sections/notes explorer transitions directly into the full-width note editor upon selection, with a dedicated back button ([`#btn-notedog-mobile-back`](file:///home/bolt/projects/commanderdog/frontend/index.html#L2376)) in the workspace header to return to the notes list.
  - Optimized NoteDog's formatting bar with smooth horizontal swipe scrolling and clamped action buttons without wrapping or viewport overflow.

## [0.7.6-rc3] - 2026-09-07

### 📱 Streamlined Phone Viewport: Redundant Switcher Removal & Unified Badge Switching
- **Removed Redundant Mobile Pane Switcher Bar**:
  - Removed the top `.mobile-pane-switcher-bar` and `.mobile-pane-tab` full-width buttons on mobile viewports, relying on the unified pane number/color badge ([`.pane-badge-btn`](file:///home/bolt/projects/commanderdog/frontend/app.js#L843)) in the pane toolbar.
- **Direct Tap-to-Switch on Phone Screens**:
  - Tapping [`.pane-badge-btn`](file:///home/bolt/projects/commanderdog/frontend/app.js#L843) on narrow phone screens (`<= 600px`) seamlessly cycles between active panes, while long-press or right-click opens the full pane renaming, color, and border settings.
- **Embedded Pane Switcher in Settings Popover**:
  - Added a quick "Switch Active Pane" button bar directly into the pane settings popover ([`openPaneSettingsMenu()`](file:///home/bolt/projects/commanderdog/frontend/app.js#L13088)), providing 1-tap pane jumping from any device.

## [0.7.6-rc2] - 2026-09-07

### 📱 Mobile Phone Viewport Touch Pass-Through & Status-Bar Touch Targets
- **Task Drawer Peek Bar Pointer Event Pass-Through**:
  - Configured `pointer-events: none` on `.mobile-task-peek-bar` and `pointer-events: auto` exclusively on `.mobile-peek-pill` in [`frontend/app.css`](file:///home/bolt/projects/commanderdog/frontend/app.css#L4753). This eliminates transparent overlay dead zones across the bottom of the screen, allowing touch events to reach the Details button and status-bar controls unhindered.
- **Enhanced Mobile Status-Bar Touch Ergonomics**:
  - Expanded `.pane-footer-view-btn` touch hit targets to `22px x 22px` with `13px` icons and `touch-action: manipulation` on mobile/phone screens (`<= 768px`).
  - Elevated status-bar controls to `z-index: 105` and streamlined the peek handle pill height (`12px`) for optimal tap precision.

## [0.7.6-rc1] - 2026-09-07

### 🧹 Panel Status-Bar View Modes & Table Header Column Config Integration
- **View Mode Buttons Moved to Status-Bar**:
  - Relocated the Details, Thumbnail Gallery, and Compact Multi-Column list buttons from the pane top header down into the panel's bottom status-bar (`.pane-footer` in [`frontend/app.js`](file:///home/bolt/projects/commanderdog/frontend/app.js#L993)), placed rightmost after the file size indicators.
  - Added a subtle vertical divider (`.pane-footer-sep`) between the size stats and view-mode button group.
  - Implemented an inverted button aesthetic (`.pane-footer-view-btn` in [`frontend/app.css`](file:///home/bolt/projects/commanderdog/frontend/app.css#L1037)) with recessed dark background and high-contrast amber active state.
  - Kept panel status-bar height strictly constrained to `24px` to preserve compact orthodox density.
- **Table Header Column Config Button**:
  - Added an inverted column configuration button (`.pane-col-config-btn`, `sliders-horizontal`) inside the leftmost icon cell header (`<th class="col-header col-icon">`) directly preceding the "Name" column.
  - Opens the table columns chooser popover (`openColumnHeaderContextMenu`) for column visibility and auto-fit adjustments.
- **Cleaned Up Pane Header Bar**:
  - Removed `.viewmode-btn-group` from `.pane-header` to maximize path bar and breadcrumbs visibility.

## [0.7.5-rc9] - 2026-09-07

### 🌿 Flat Branch View Relocated to Directory Pane Toolbars
- **Per-Pane Branch View Button**:
  - Relocated the Flat Branch View button (`.pane-branch-btn`, `#btn-branch-${index}`) from the top application header directly into each directory panel's navigation toolbar (`.pane-nav-btns` in [`frontend/app.js`](file:///home/bolt/projects/commanderdog/frontend/app.js#L885)), positioned right next to the pane's folder tree toggle (`#btn-tree-${index}`).
  - Removed `#btn-toggle-branch` from the top main header toolbar in [`frontend/index.html`](file:///home/bolt/projects/commanderdog/frontend/index.html).
- **Per-Pane State Synchronization & Visual Styling**:
  - Enhanced `updateBranchToggleState()` to dynamically synchronize `.active` accent highlighting across each pane's individual `#btn-branch-${index}` button based on `pane.isBranchView`.
  - Added CSS rule support for `.pane-branch-btn.active` and `.pane-tree-btn.active` in [`frontend/app.css`](file:///home/bolt/projects/commanderdog/frontend/app.css#L780) adhering to the standard amber accent aesthetic.

## [0.7.5-rc8] - 2026-09-07

### 🌿 Flat Branch View Header Toolbar Integration & ChewToys Clean-Up
- **Header Toolbar Button Addition**:
  - Added a dedicated Flat Branch View toggle button ([`#btn-toggle-branch`](file:///home/bolt/projects/commanderdog/frontend/index.html#L67), `Ctrl+B`) directly next to the Tree sidebar button in the primary header toolbar.
  - Linked active state highlighting (`.active`) to dynamically reflect the active pane's flat branch view mode.
- **ChewToys Menu Clean-Up**:
  - Removed "Flat" branch view from the ChewToys / Tools launchpad dropdown menu (`#tools-dropdown-menu` and `DEFAULT_TOOLS_MENU`), matching the "Tree" streamlining.

## [0.7.5-rc7] - 2026-09-07

### 🧹 ChewToys Launchpad Streamlining
- **ChewToys Menu Cleanup**:
  - Removed duplicate "Tree" item from the ChewToys / Tools launchpad dropdown menu (`#tools-dropdown-menu` and `DEFAULT_TOOLS_MENU`), consolidating directory tree toggling to the primary header button (`#btn-toggle-tree`, `Ctrl+T`).

## [0.7.5-rc6] - 2026-09-07

### 🖥️ Native Desktop External Program Integration & Web Feature Isolation
- **External Programs Definition (Desktop Standalone Only)**:
  - Added dedicated **"External Programs"** configuration tab in Settings (F10) for standalone desktop sessions (`App.isStandalone === true`).
  - Configurable commands for **External Text & Code Editor (F4)**, **External Document / Media Viewer (F3)**, and **External Terminal Emulator** with quick 1-click presets (VS Code, VSCodium, Gedit, Kate, Sublime, Notepad++, Neovim GUI, VLC, MPV, EOG, Feh, Zathura, Alacritty, Kitty, GNOME Terminal, Konsole, WezTerm).
  - Checkboxes to seamlessly override F4 (Edit) and F3 (Quick View) keyboard shortcuts and context clicks to directly launch user-defined desktop applications.
  - Included dynamic command token interpolation (`%1` / `{file}`, `{dir}`, `{selection}`, `{target_pane}`) and test launch buttons for each program.
- **Strict Web vs Desktop Feature Isolation**:
  - Implemented `.desktop-only-setting` / `.standalone-only` CSS and JS isolation rules ensuring external program settings, "Open With" handlers, desktop windowing/tray preferences, and native terminal launchers are **strictly hidden in the web version** to eliminate out-of-context host executions for remote browser clients.
- **Backend Working Directory & Command Parameter Handling**:
  - Enhanced `/api/system/open-with` with automatic `{dir}` replacement and `current_dir` process spawning so launched external programs immediately open in the target item's folder.

## [0.7.5-rc5] - 2026-09-07

### 📱 Responsive Phone & Tablet Menu Clamping & Fullscreen Modal Adaptations
- **Phone Viewport Fullscreen Modals & Scrollable Pill Navigation**:
  - Implemented responsive fullscreen viewports for Settings and Admin Control Panel (`width: 100vw; height: 100dvh; border-radius: 0;`) on mobile screens (`<= 640px`).
  - Converted the vertical sidebar on Phone viewports into a single-row, horizontally scrollable chip carousel (`.settings-sidebar` with `overflow-x: auto; scrollbar-width: none; border-radius: 20px;`) to preserve 100% vertical space for content.
- **Tablet & Foldable Modal Optimization**:
  - Configured compact 190px sidebar layout and 95vw width for tablets and foldables (`641px - 1024px`), preventing text clipping.
- **Dropdown Viewport Bounding Clamps**:
  - Added fixed viewport-clamped positioning for Tools launchpad (`#tools-dropdown-menu`), profile menu (`#profile-dropdown-menu`), and pane favorites/color dropdowns on Phone viewports, preventing menus from bleeding past screen edges or escaping below bottom bars.

## [0.7.5-rc4] - 2026-09-07

### 🐛 Fix DOM Modal Nesting Between Settings and Admin Control Panel
- **DOM Hierarchy & Modal Isolation**:
  - Fixed an unclosed `<div>` container in `#settings-modal` that caused `#admin-panel-modal` to nest as an internal child element of the Settings modal.
  - Resolved the bug where opening the Admin Control Panel from the user profile dropdown failed to display and subsequently popped up both modals upon triggering F10 Settings.

## [0.7.5-rc3] - 2026-09-07

### 🗂️ Unified Vertical Sidebar Navigation for Settings & Admin Control Panel
- **F10 Settings Modal Sidebar Architecture**:
  - Replaced cramped horizontal tabs with a modern, scrollable two-column layout featuring an amber-accented vertical sidebar navigation list (`.settings-sidebar` + `.settings-content-area`).
  - Categorized all 9 settings tabs into structured groups: `Appearance & UI`, `Workflow & ChewToys`, and `System & Shortcuts`.
  - Fully eliminates tab crowding, multi-row wrapping, and out-of-frame tab clipping across all desktop and laptop resolutions.
- **Administrator Control Panel Sidebar Unification**:
  - Unified the dedicated Administrator Control Panel modal with the identical sleek vertical sidebar layout.
  - Grouped administration categories into `Server Administration` and `Security & Config`.
- **Responsive Mobile & Tablet Viewport Adaptations**:
  - Added responsive media queries for screens under 720px to fluidly transition sidebar items into compact horizontal chips without layout breakage.

## [0.7.5-rc2] - 2026-09-07

### ⚡ Comprehensive Engine Performance & Concurrency Acceleration
- **In-Memory TTL Caching for User & Group Resolution**:
  - Implemented a 60-second thread-safe in-memory cache (`USER_GROUP_CACHE` with `parking_lot::RwLock`) for `/etc/passwd` and `/etc/group` in `LocalFs`.
  - Eliminates synchronous disk reads and string parsing on repeated directory listings and panel navigation.
- **HTTP Payload Compression Layer (Gzip / Brotli / Zstd)**:
  - Integrated `tower_http::compression::CompressionLayer` across the Axum router.
  - Automatically compresses large JSON responses (directory listings with 5,000+ items, git commit logs, search results) by 80–90%, dramatically reducing latency over WiFi/WAN.
- **Embedded Static Asset Caching & ETag Validation (`rust-embed`)**:
  - Implemented strong SHA-256 `ETag` generation, `If-None-Match` -> `304 Not Modified` conditional responses, and explicit `Content-Length` headers in `handle_static_asset`.
  - Added `Cache-Control: public, max-age=31536000, immutable` for static fonts and icons.
- **SQLite WAL Mode & Concurrency Pragmas**:
  - Configured SQLite connections with `PRAGMA journal_mode = WAL`, `PRAGMA synchronous = NORMAL`, `PRAGMA temp_store = MEMORY`, `PRAGMA cache_size = -16000` (16 MB cache), and `PRAGMA busy_timeout = 5000`.
  - Unlocks concurrent non-blocking reads while writing, eliminates disk synchronization bottlenecks, and prevents lock contention.
- **DeltaCopy & VFS Streaming Buffer Tuning**:
  - Upgraded block-delta and streaming chunk buffers from 64 KB to 256 KB, cutting system call context switches by 4x.

## [0.7.5-rc1] - 2026-09-07

### 🚀 High-Speed In-Memory Blob URL LRU Cache & Standalone WebKitGTK Acceleration
- **In-Memory Blob URL LRU Cache (`imageViewerBlobCache`)**:
  - Implemented client-side in-memory Blob URL caching with LRU eviction (up to 80 decoded images in RAM).
  - Subsequent image visits and adjacent browsing hits display in **0.0ms** instantly with zero network/disk latency.
- **Asynchronous Blob Streaming & Single-Thread Stall Prevention**:
  - Fetches image buffers via native background async `fetch()` rather than synchronous WebKitGTK `<img>` network stalls, eliminating UI thread lockups on large WebP and high-resolution photos in standalone desktop mode.
- **Enhanced Multi-Step Adjacent Prefetching**:
  - `preloadAdjacentImages` now pre-fetches 3 images ahead and 3 images behind directly into RAM Blobs, enabling 60+ FPS smooth slideshow browsing via mouse wheel and arrow keys.
- **Explicit Content-Length Headers**:
  - Added `Content-Length` headers across all backend download and streaming responses (`src/server/mod.rs`), preventing chunked re-allocation bottlenecks in WebKitGTK/libsoup.

## [0.7.4-rc9] - 2026-09-05

### ⚡ Ultra-Fast WebP & Media Viewer Streaming, HTTP Caching & Async I/O
- **MIME Auto-Inline Streaming**:
  - Automatically serves `Content-Disposition: inline` on all media MIME types (`image/*`, `video/*`, `audio/*`, `application/pdf`, `text/*`) across local filesystem, Vault, SMB, and SFTP endpoints.
  - Allows browser progressive decoders and hardware accelerators (WebP VP8/VP8L chunks, AVIF, PNG) to decode frames on the fly without buffering into download sandboxes.
- **HTTP Caching & 304 Not Modified Support**:
  - Implemented strong `ETag` validation and `Cache-Control: public, max-age=86400, must-revalidate` on file download & streaming routes.
  - Validates `If-None-Match` requests to return lightweight `304 Not Modified` responses, eliminating redundant data transfer during image flipping.
- **Async Tokio Disk I/O**:
  - Replaced blocking synchronous file reads (`std::fs::read`) with non-blocking async `tokio::fs::read` and `tokio::fs::metadata`.
- **Dual-Layer Media Token & Cookie Authentication**:
  - Implemented automatic `cd_token` cookie authentication and `?token=` query fallback in `extract_claims_or_local` and `handle_download`.
  - Added centralized `getDownloadUrl()` in `frontend/app.js` with remote URI credential resolution (SFTP/SMB/Vault) and session token injection across all image, audio, video, PDF, and inspector views.

## [0.7.4-rc8] - 2026-09-05

### 🎨 Frameless Icon Suite & 128px Amber Asset Integration
- **New Frameless & 128px Amber Icon Suite**:
  - Integrated full set of 128x128 lossless `.webp` icons across both root `assets/` and embedded `frontend/assets/128/`.
  - **Frameless Apps / ChewToys Launchpad**: Updated main header launchpad trigger (`#btn-tools-menu`), Settings Tools tab, and context menus with modern frameless apps icon (`amber-frameless-apps.webp`).
  - **Frameless Settings & Config**: Updated Settings modal header, profile menu item, and context menu template actions with crisp frameless settings icon (`amber-frameless-settings.webp`).
  - **Frameless Admin & Security**: Integrated frameless administrator shield icon into Admin Control Panel modal header and profile dropdown (`amber-frameless-admin.webp`).
  - **Frameless Search & Quick-Switcher**: Replaced mobile Spotlight trigger with frameless search icon (`amber-frameless-search.webp`).
  - **About & Info**: Added dedicated info badge icon to About CommanderDog modal header, profile menu, and Spotlight actions (`amber-info.webp`).
  - **Folder Tree & Flat Branch Views**: Integrated dedicated amber folder tree (`amber-folder-tree.webp`) and Git branch (`amber-git-branch.webp`) icons into Tools dropdown, tree sidebar header, branch banner, and Spotlight.
  - **Stats & Disk Usage Treemap**: Integrated dedicated amber piechart icon (`amber-piechart.webp`) into Disk Usage ChewToy modal header, Tools launchpad menu, and context menus.

## [0.7.4-rc7] - 2026-09-05

### 🖼️ Enhanced Photo/Image Viewer Mouse Scroll & In-Viewer File Operations
- **Tri-Mode Mouse Wheel Interaction**:
  - **Image Cycling at 1:1 Scale**: Scrolling wheel down/up browses to next/previous image with smooth 160ms debounce.
  - **Pan on Zoom (`zoom > 100%`)**: Natural vertical wheel scrolling and horizontal Shift-wheel panning across zoomed images.
  - **Zoom on Key/Mode**: Holding <kbd>Ctrl</kbd>/<kbd>Cmd</kbd> or enabling Direct Zoom mode seamlessly zooms in/out at the mouse cursor position.
- **In-Viewer File Operations Suite**:
  - Added dedicated toolbar action buttons & global shortcuts to both the **Image Viewer** and **Universal Document Viewer**:
    - **Rename (<kbd>F2</kbd>)**: Rename current image or document in-place with instant preview reload.
    - **Copy to Opposite Panel (<kbd>F5</kbd>)**: Direct transfer to opposite pane destination.
    - **Move to Opposite Panel (<kbd>F6</kbd>)**: Moves file to opposite pane and automatically progresses to the next image/closes viewer.
    - **Delete / Trash (<kbd>F8</kbd> / <kbd>Del</kbd>)**: Trashes or permanently deletes file with confirm dialog and auto-advances.
- **Top-Level Z-Index Layering for Confirmation Modals**:
  - Elevated `#app-dialog-modal` to `z-index: 9999` and transfer modals to `z-index: 3200`, completely preventing confirmation dialogs from rendering beneath active floating or full-screen viewers.

## [0.7.4-rc6] - 2026-09-05

### 🪟 Multi-Panel Real-Time Synchronization Across All Mutations
- **Universal Multi-Panel Directory Refresh**:
  - Whenever a file or folder is deleted (via F8, context menu, or keyboard), created, renamed, uploaded, extracted, or saved, all active visible panels (`refreshAllPanes()`) are automatically updated immediately.
  - Fixes synchronization issue where opening the same folder in two panels left deleted/modified files stale in the second panel.
- **Stale Selection Pruning Across Panels**:
  - Automatically clears deleted and moved items from selection state across all open panels, preventing ghost selections.
- **Full Mutation Coverage**:
  - Audited and updated all filesystem mutation endpoints in frontend: `triggerDelete`, `executeMkdir`, `executeRename`, `executeBulkRename`, `executeCompressArchive`, `executeExtractArchive`, `handleDirectFileUpload`, drag-and-drop upload, editor tab saves, security permissions, and NoteDog deletions.

## [0.7.4-rc5] - 2026-09-05

### 🔄 Guaranteed Real-Time Source & Destination Directory Refresh
- **Active Task Completion Tracking**:
  - Implemented `trackTransferTask` in frontend to actively monitor background transfer lifecycle via `/api/tasks/:id`.
  - Guaranteed automatic universal pane refresh (`refreshAllPanes()`) as soon as operations complete or stream progress across all visible panes.
  - Added dedicated `/api/tasks/:id` endpoint in backend Task Manager for sub-millisecond task status resolution.
- **Robust Transfer Execution & Error Resiliency**:
  - Hardened `refreshPane` with boundary index checks preventing unhandled index errors.
  - Immediately clears selection on source panels upon executing moves.
  - Ensured source and destination panes are reliably refreshed on F5/F6 orthodox keys, drag-and-drop, context menus, and clipboard paste.

## [0.7.4-rc4] - 2026-09-05

### 🛑 Global Clean Quit / Exit Shortcut (<kbd>Ctrl+Q</kbd> / <kbd>Cmd+Q</kbd>)
- **Global Clean Termination**:
  - Wired <kbd>Ctrl+Q</kbd> / <kbd>Cmd+Q</kbd> to initiate clean application shutdown across native desktop standalone mode and web sessions.
  - Features an elegant confirmation modal (`showConfirmDialog`) before sending graceful `POST /api/system/exit` termination.
  - Added <kbd>Ctrl+Q</kbd> to Keybindings reference table in Settings (<kbd>F10</kbd>) and `README.md`.

## [0.7.4-rc3] - 2026-09-05

### 🎯 Strict Drag-and-Drop & Drag-and-Select Separation
- **Strict Empty-Space Starting Requirement for Marquee Selection**:
  - Drag-and-select (rubberband marquee) now exclusively starts when mouse interaction originates on **empty space** (blank background, margins, or space below file rows/cards).
  - Dragging starting on any file row, card, or item seamlessly triggers **HTML5 Drag-and-Drop** transfers to target folders and panes without collision.
- **Unified Drag-and-Drop Across All View Modes**:
  - Added full HTML5 drag-and-drop support (`dragstart`, `dragover`, `dragleave`, `drop`) and visual drag-hover cues (`.drag-over-card`, `.drag-over-item`) to Grid Gallery and Compact List views.

## [0.7.4-rc2] - 2026-09-05

### 🖱️ Mouse Deselection & Dedicated Installation Guide
- **Mouse Deselection on Empty Space & Single Clicks**:
  - Clicking on empty space (outside files or below lists) now cleanly deselects all selected items in table, grid, and compact list views.
  - Clicking a single row without modifier keys deselects all other selected items and focuses the target item.
  - Pressing <kbd>Escape</kbd> when no modal/menu is active now instantly deselects all selected items in the active pane.
  - Fixed grid card and compact list item propagation on double click and context menus.
- **Dedicated Comprehensive `INSTALL.md`**:
  - Documented native desktop builds (`cargo build --release --features gui`), tiling window manager borderless execution (`commanderdog -s --frameless`), Hyprland window rules, Arch AUR packages, Windows installers, and headless server mode.
  - Indexed across `README.md` and `manuals/README.md`.

## [0.7.4-rc1] - 2026-09-05

### 🚀 Marquee Drag-Selection, Shift Multi-Select & Configurable Drag-and-Drop
- **Marquee / Rubberband Drag-and-Select**:
  - Implemented smooth visual rubberband drag-selection with amber highlight rectangle on mouse input across table, grid, and compact view modes.
  - Real-time bounding box intersection detection dynamically selects files and updates pane footer badges.
  - Full modifier key support (<kbd>Shift</kbd> to add to selection, <kbd>Ctrl</kbd>/<kbd>Cmd</kbd> to toggle individual items).
- **Continuous Row Range Selection (<kbd>Shift</kbd> + Click / Arrow Keys)**:
  - Added Windows File Explorer / Total Commander standard range selection preserving anchor index for intuitive range expansion and shrinking.
  - Keyboard range selection support via <kbd>Shift</kbd> + <kbd>ArrowUp</kbd> / <kbd>Shift</kbd> + <kbd>ArrowDown</kbd>.
- **Configurable Drag-and-Drop Defaults & "No Asking" Direct Mode**:
  - Added Drag-and-Drop & Transfer Defaults section in Settings (<kbd>F10</kbd>).
  - Selectable default action: `Ask Every Time`, `Move`, or `Copy`.
  - Confirmation prompt modes: `Always Ask / Show Confirmation Dialog` or `Direct Execution (No Asking / Silent Action)`.
  - Configurable paranoid SHA-256 prompt behavior on drag-and-drop.
- **TetraDog (Arcade Tetris ChewToy) Roadmap Integration**:
  - Added full technical specifications for `TetraDog` arcade Tetris ChewToy to `ROADMAP.md`, `manuals/chewtoys.md`, and `manuals/wishlist.md`.

## [0.7.3] - 2026-09-04

### 🚀 Milestone 0.7.3 Release: WebP Power Suite, High-DPI ChewToys & Visual Disk Analyzer
- **Lossless WebP Asset Engine & Ultra-Lightweight Footprint**:
  - Migrated 100% of internal ChewToy and UI icon assets to crystal-clear, lossless `.webp` format.
  - Reduced total embedded asset footprint by **98%**, delivering instantaneous sub-50ms page and view transitions even over remote NetBird/VPN reverse proxy tunnels.
  - Properly aligned 128×128 canvas dimensions across all 22 custom ChewToy icons.
- **Smart Responsive Header & Mobile Hostname Switcher**:
  - Dynamic responsive header optimization: automatically hides brand text on mobile viewports when the host environment badge is active, preventing header button overflow while preserving full branding when disabled.
- **Visual Disk Usage Treemap & Storage Inspector**:
  - Interactive proportional storage treemap with direct folder drill-down, multi-color proportional capacity bars, and top-20 subtree heavy-file analyzer.
  - Integrated 1-click "Open in Pane" jump navigation directly into CommanderDog dual panes.
- **NoteDog Cryptographic Studio (ChaCha20-Poly1305 / Argon2id)**:
  - Full binary and cryptographic interoperability with NoteDog TUI client (`NOTEDOG_ENC_V1` container format).
  - In-workspace decryption unlock cards, transparent on-save re-encryption, and 1-click note conversion (<kbd>Ctrl+E</kbd>).
- **Streamlined Modal Ergonomics (<kbd>F7</kbd> Mkdir & <kbd>F2</kbd> Rename)**:
  - Instant autofocus, intelligent base-name selection excluding extensions, and fast keyboard workflows (<kbd>Enter</kbd> to confirm, <kbd>Esc</kbd> to dismiss).
- **Native Multi-Resolution Platform Packaging**:
  - 7-layer native Windows `.ico` generation (16×16 to 256×256) and complete Windows Store / Tauri tile suites.

- **Uniform Woofsons Amber Icon Suite Integration**:
  - Integrated full suite of 14 uniform amber PNG icons across all application surfaces:
    - `amber-chewtoy-2.png`: Dedicated top header launchpad trigger button (`#btn-tools-menu`).
    - `amber-chewtoy.png`: Settings "Tools Menu" tab and Launchpad customizer header.
    - `amber-pdftool.png`: PDFDog power studio modal header, Launchpad dropdown, Spotlight action, and context menu.
    - `amber-diff.png`: Compare & Diff engine modal header, Launchpad, Spotlight (`F9`), and context menus.
    - `amber-terminal.png`: Terminal console drawer header, Launchpad, Spotlight, and docked pane header.
    - `amber-sync.png`: Backup & Sync studio modal header, backup profile editor, Launchpad, and context menus.
    - `amber-spot.png`: Spotlight search bar icon, search palette footer branding, and Launchpad menu item.
    - `amber-calc.png`: Floating calculator header, minimized floating pill, Launchpad, and Spotlight.
    - `amber-note.png`: NoteDog floating window, revision history modal, note creation modal, Launchpad, and Spotlight.
    - `amber-settings.png` / `conf.png`: User profile dropdown (`F10`), `#settings-modal` header, `#confd-assembler-modal` header, and context menus.
    - `amber-task.png`: Top header active transfer badge (`#header-task-icon`), floating transfer manager header, and docked queue.
    - `amber-sharemgr.png`: Active Shares & Dropboxes modal header, share creation dialog, Launchpad, and file context menu.
    - `amber-docs.png`: Universal Document & Text Viewer header (`#doc-viewer-modal`).
    - `amber-media.png`: Rich Media EXIF GPS Inspector and Audio/Video player modal headers.
    - `amber-syntaxedit.png` / `edit.png`: EditorDog floating header, docked editor header, Launchpad, Spotlight (`F4`), and context menus.
- **HTML DOM Hierarchy & Loading State Repair**:
  - Closed orphaned tags in `#notedog-versions-modal` inside `frontend/index.html`, eliminating an issue where subsequent DOM elements were trapped in a hidden modal.
- **Embedded Asset Synchronization**:
  - Synchronized new graphical assets directly into `frontend/assets/` to ensure `RustEmbed` serves all ChewToy, NoteDog, and Settings icons without 404 fallbacks.
- **Modal Layer Stacking & Context Resolution**:
  - Elevated `.modal-overlay` base layer to `z-index: 2800` across `app.css` and dialog markup to prevent modals from rendering beneath floating utilities or backdrops.
  - Hardened item and pane resolution in `triggerRename()` and `triggerMkdir()` with proper context resets.

## [0.7.3-rc9] - 2026-09-04

### 📁 Streamlined New Folder Creation Modal (F7)
- **Ergonomic Quick-Mkdir Dialog**:
  - **ChewToy Standards & Visual Hierarchy**: Upgraded `#mkdir-modal` with 420px width, `folder-plus` Lucide icon in amber accent, clean monospace input field, and a dedicated confirmation button.
  - **Instant Autofocus & Selection**: Opening the modal via <kbd>F7</kbd>, bottom toolbar "F7 Mkdir", context menu "New Folder...", or Spotlight command immediately focuses the input field with text selected for instant typing.
  - **Keyboard Workflow**: Pressing <kbd>Enter</kbd> submits and creates the folder via `POST /api/fs/mkdir`; pressing <kbd>Esc</kbd> cancels and dismisses the dialog immediately.
  - **Context-Aware Pane Resolution**: Accurate directory targeting when triggered from pane context menus or right-click actions (`App.contextPaneIndex`).
  - **Clear User Feedback**: Displays real-time toast feedback on creation success or sanitized error feedback on failure.

## [0.7.3-rc8] - 2026-09-04

### 🔒 NoteDog Cross-TUI Encryption Suite & Streamlined Rename Modal
- **Streamlined Rename Modal (`F2`)**:
  - **Instant Autofocus & Smart Selection**: Pressing <kbd>F2</kbd>, clicking the bottom toolbar "F2 Rename", or choosing context menu "Rename" opens a clean, focused modal. The filename is automatically pre-selected excluding its extension (e.g. `report` in `report.pdf`, or entire name for folders/dotfiles).
  - **Keyboard Ergonomics**: <kbd>Enter</kbd> confirms and executes rename via `/api/fs/rename`; <kbd>Esc</kbd> immediately cancels and closes the modal.
- **NoteDog Encrypted Notes & Cross-TUI Compatibility**:
  - **100% NoteDog TUI Cryptographic Interoperability**: Implemented symmetric authenticated encryption using **ChaCha20-Poly1305** and **Argon2id** key derivation, providing identical binary format (`NOTEDOG_ENC_V1` header + 16-byte salt + 12-byte nonce + ciphertext + Poly1305 tag) to the NoteDog TUI terminal client.
  - **In-Workspace Decryption Unlock Card**: Selecting a `.md.enc` note presents an in-place unlock card with password field, Enter-to-unlock, and session passphrase caching.
  - **Transparent Encrypted Editing & Saving**: Editing decrypted notes in NoteDog automatically re-encrypts the payload upon saving (<kbd>Ctrl+S</kbd> or auto-save) without ever touching disk in plaintext.
  - **1-Click Note Encryption Toggle (<kbd>Ctrl+E</kbd>)**: Convert any unencrypted note `.md` to `.md.enc` with a passphrase, or decrypt back to plain `.md`.
  - **Batch Section & Notebook Encryption / Decryption**: Added dedicated 🔒 actions to encrypt/decrypt entire sections or notebooks with directory `.encrypted` markers.
  - **New Note Creation Modal**: Direct modal to create plain or encrypted notes with optional custom passphrases.

## [0.7.3-rc7] - 2026-09-04

### 🏷️ ChewToy Unified Shortened Nomenclature & Dedicated Icon Suite
- **16 Standardized ChewToy Power Tool Names**:
  - Harmonized the entire built-in tool suite across the Launchpad dropdown menu, Settings customization tab, Spotlight Quick-Switcher (`Ctrl+K`), and panel context menus:
    1. **`Spot!`** — Spotlight Quick-Shifter & Command Palette (`Ctrl+K`)
    2. **`Task Manager`** — Background Transfers & Job Queue
    3. **`Terminal`** — Slide-Up PTY Web Terminal Console (`'`)
    4. **`EditorDog`** — Multi-Tab Code & Text Editor (`F4`)
    5. **`Calculator`** — Floating Calculator & Unit Converter
    6. **`Tree`** — Folder Hierarchy Tree (`Ctrl+T`)
    7. **`Flat`** — Flat / Branch Recursive View (`Ctrl+B`)
    8. **`NoteDog`** — Notes & Markdown Studio
    9. **`Compare`** — Side-by-Side Diff Engine (`F9`)
    10. **`Search`** — Deep File Search (`Ctrl+F`)
    11. **`Share Manager`** — Active Shares & Dropboxes
    12. **`Backup`** — Delta Backup & Sync Studio (SyncToy / Bvckup2)
    13. **`Stats`** — Disk Usage & Treemap Space Analyzer
    14. **`Syncthing`** — Live Continuous P2P Syncthing Dashboard
    15. **`ConvertX`** — Universal Transcoder & File Converters
    16. **`PDFDog`** — PDF Power Studio (Merge, Split, Rotate, Extract)
- **New Graphical Assets & Visual Identity Integration**:
  - **ChewToys Suite Launcher**: Main header suite button now displays the dedicated `assets/chewtoy.png` icon.
  - **NoteDog Studio Identity**: NoteDog floating window header, Launchpad menu, Spotlight search, and minimized pill now render `assets/note.png`.
  - **Settings & Config Identity**: Settings (<kbd>F10</kbd>) modal header, profile dropdown item, and Spotlight entries now render `assets/conf.png`.
- **Dynamic Config Upgrades**:
  - Updated `getToolsMenuConfig()` to seamlessly upgrade labels and icons while preserving any custom tool ordering or visibility preferences in `localStorage`.

## [0.7.3-rc6] - 2026-09-04

### 🌲 Flat / Branch View Performance Overhaul & UX Streamlining
- **High-Performance Bounded Backend Lister**:
  - **Pruned Directory Traversal**: Configured `WalkDir` with iterator-level `filter_entry` pruning that stops descent into hidden directories (`.git`, `.cache`, `.cargo`, `node_modules`) when `show_hidden=false`, avoiding scanning millions of irrelevant files.
  - **Entry Cap & Safety Limit**: Implemented a configurable entry cap (`MAX_BRANCH_ENTRIES = 5,000`, max 25,000) with a `is_truncated` response flag, eliminating browser freezing, memory blowups, and massive JSON payloads on root/system folders.
  - **Non-Blocking Async Execution**: Offloaded `LocalFs::list_branch_view` to `tokio::task::spawn_blocking` to prevent blocking the axum event loop and terminal/WebSocket traffic.
  - **Lightweight Metadata Resolution**: Eliminated redundant per-directory `read_dir` empty checks during recursive scans for 10x-50x speedups on large filesystems.
- **Persistent Sticky Branch View Banner (`.pane-branch-banner`)**:
  - Added a dedicated, amber-accented banner pinned above the file table/grid/compact lists with item count, truncation indicator badge, and a prominent **`[ ✕ Exit Branch View (Ctrl+B) ]`** button.
- **Cancelable Scan Engine & AbortController**:
  - Integrated `AbortController` into `loadPaneDirectory` allowing users to cleanly cancel in-flight recursive branch scans via a "✕ Cancel Scan" button without leaving the UI in a hanging state.
- **Intuitive Multi-Trigger Exit Pathways**:
  - Seamlessly exit flat branch view back to normal folder view via `Ctrl+B`, the sticky banner exit button, breadcrumb badge exit button, double clicking `..` (parent dir), double clicking any directory in the branch list, clicking any ancestor directory in the breadcrumb bar, or selecting bookmarks/tree nodes.
- **Clickable Ancestor Breadcrumb Navigation**:
  - Breadcrumbs in Branch View now render full clickable directory paths, allowing users to jump directly to any ancestor folder in standard view with a single click.

## [0.7.3-rc5] - 2026-09-04

### 🎨 Dedicated ChewToy Icon Integrations & Customizable Tools Launchpad
- **Terminal (Bite!) Visual Identity**:
  - Integrated dedicated high-resolution graphical asset `assets/term.png` across the slide-up Terminal drawer header, in-pane docked terminal headers, panel right-click context menu ("Open in Terminal"), Tools dropdown, and Spotlight quick-actions.
- **Sleek & Larger Icons in Tools & Launchpad Dropdown**:
  - Standardized Tools & Utilities dropdown menu icons to 18px with crisp alignment and balanced vertical spacing.
- **Customizable & Rearrangable Tools Launchpad**:
  - **Native Drag-and-Drop Reordering**: Rearrange the order of ChewToys and tools in the Launchpad dropdown menu with real-time drag-and-drop handles (`⋮⋮`).
  - **1-Click Direction Controls**: Fast Move to Top (⤒), Move Up (▲), Move Down (▼), and Move to Bottom (⤓) ChewToy buttons.
  - **Item Visibility Toggles**: Checkboxes to show or hide any tool from the dropdown suite with active visible counter (`X / 16 Visible`).
  - **Dedicated Settings Tab & Dropdown Quick-Action**: Added "Tools Menu" manager tab in Settings Modal and an instant "Customize Tools Menu..." footer shortcut in the dropdown.
  - **State Persistence & Reset**: User preferences persist across sessions in `localStorage` with a 1-click "Reset Defaults" action.
- **EditorDog, Calculator & Task Manager Visual Identity**:
  - Integrated dedicated high-resolution graphical assets `assets/edit.png`, `assets/calc.png`, and `assets/task.png` across the entire application interface.
  - Upgraded the main header Task activity button (`#btn-header-tasks`), Tools & Launchpad dropdown items, floating window headers, minimized floating pills (`#tasks-pill`, `#editor-pill`, `#calc-pill`), and in-pane docked ChewToy titles to render their respective dedicated graphical icons.
  - Added full image icon support in the Spotlight Quick-Switcher (Ctrl+K).

## [0.7.2] - 2026-09-04

### 🪟 ChewToy Design System, Delta Sync Templates, Hostname Badges & Polish
- **Delta Backup & Sync Studio Templates & Visual Exclusion Builder**:
  - **1-Click Profile Templates**: Added quick replication presets (`🪞 NAS Mirror`, `💻 Codebase Sync`, `📦 Snapshot Vault`, `📸 Media Backup`) for both live diff studio and scheduled job creator.
  - **Visual Exclusion Builder**: Interactive preset exclusion chips (`node_modules`, `.git`, `target/`, `.cache/`, `tmp/`, `*.tmp`, `.DS_Store`, `Thumbs.db`, `*.log`, `dist/`, `*.bak`) with dynamic custom pattern tag input.
  - **Pattern Engine & SQLite Migration**: Integrated wildcard and substring path matching into sync scanning/execution and persisted exclusions per backup profile in SQLite.
- **ChewToy Design & Layout Language Specification**:
  - Enforced 42px header bar standard with full drag handle (`cursor: grab;`), uniform `28px` square buttons, and right-aligned icon-only layout switches.
  - Standardized flat stealthy window control buttons (Minimize, Maximize, Dock, Float, Close) across all ChewToys.
  - Unified sub-headers and inner workspace toolbars to `26px x 26px` buttons across NoteDog, Git Manager, Diff Engine, Disk Usage, and Sync Studio.
  - Docked in-pane ChewToys now use icon-only float buttons (`external-link`).
- **Terminal Console (Bite!) Lifecycle**:
  - Added clean handling for `Ctrl+D` (EOF), shell `logout`, and `exit` commands to automatically close the docked drawer/window and reset PTY state.
- **Chewtoy & Tools Launchpad Icon**:
  - Replaced generic grid icon with dedicated `assets/tool.png` icon on the main header Tools & Launchpad suite.
- **EditorDog Compact Canvas & Interactive Status Bar Selector**:
  - Removed redundant inner document pane sub-headers; tabs now connect directly to the editor canvas for maximum vertical space.
  - Interactive status bar language selector (`RUST ▾`, `JS / TS ▾`, etc.) with auto-detection and 1-click syntax mode switching.
- **Compact Breadcrumbs & Sleek Panel Git Badges**:
  - Tightened breadcrumb spacing (`gap: 2px;`), streamlined separator chips, and compacted storage root dropdown buttons.
  - Minified panel git branch badge (9.5px, 17px height, 10px icons) for an uncluttered path navigation bar.
- **Universal "New ▶" Context Menu & File Template Engine**:
  - Accessible anywhere across all file rows, cards, compact items, and empty background space.
  - Built-in rich templates for Plain Text (`.txt`), Markdown (`.md`), HTML5 (`.html`), CSS (`.css`), JavaScript (`.js`), TypeScript (`.ts`), Python (`.py`), Rust (`.rs`), Bash (`.sh`), JSON (`.json`), and YAML (`.yaml`).
  - Dynamic variable expansion: `{{TITLE}}` (humanized title), `{{FILENAME}}`, `{{NAME}}`, `{{DATE}}` (`YYYY-MM-DD`), `{{TIME}}`, `{{USER}}`, `{{ISO_DATE}}`, `{{YEAR}}`, `{{MONTH}}`, `{{DAY}}`, `{{AUTHOR}}`.
  - Interactive "Create from Template" dialog with real-time live preview code editor and instant EditorDog opening.
  - Full "Templates" manager tab in Settings to create, edit, duplicate, test, or delete custom file templates, plus 1-click "Save Selected File as Template" action.
- **Task Manager & Activity Polish**:
  - Balanced 32px height for main activity button, resolved stuck UI pill states, and added automated completed tasks pruning.
- **Documentation Reorganization**:
  - Reorganized root documentation into a structured [`manuals/`](manuals/README.md) hierarchy and synchronized `ROADMAP.md`.

---

## [0.7.1] - 2026-09-03

### 🎨 Universal Floating Viewers, Live Tail Streaming, Bundled Fonts & Themes
- **Universal Floating & Resizable Viewers**:
  - Detachable floating window mode for Universal Document/Text viewer and Image viewer with window geometry persistence in `localStorage`.
  - Non-fiddly, full title bar drag handle and desktop edge/corner resizing.
  - Live log follow mode (`tail -f`) with custom line count filtering (`-n 50`, `100`, `250`, `500`, `1000`, `All`), glowing live indicator, and word-wrap toggle.
  - Unified right-aligned header layout with uniform `28px` control buttons.
- **Offline Embedded JetBrainsMono Nerd Fonts**:
  - Embedded local `JetBrainsMono Nerd Font` and `Symbols Nerd Font` into the binary via `rust_embed` for native out-of-the-box `eza --icons`, `starship`, and `fish` powerline glyphs without requiring client OS font installations.
- **Official Woofsons Amber Palette & Auto-Merge Engine**:
  - Implemented official **`Woofsons Amber Charcoal`** (Dark) and **`Woofsons Amber Zink`** (Light) themes.
  - Automatic configuration merge ensuring new built-in themes are dynamically available even on existing installations with prior `config.toml` files.
  - Fixed white-on-white text contrast bug across all `<select>` and `<option>` elements globally.
- **Procedural Randomized Login Background & Version Sync**:
  - Dynamically computes random focal positions, radii, and amber opacities on each reload so no two logins look identical.
  - Centralized compile-time versioning in `Cargo.toml` with early unauthenticated API sync updating all `.login-version-badge` and UI badges dynamically.
- **Chewtoy Enhancements & Direct Downloads**:
  - **Right-Click Context Menu**: Added **"Save / Download File"** action directly in the file context menu for 1-click downloads.
  - **Notes (NoteDog Chewtoy)**: Fixed note loading authorization header resolution and added NoteDog TUI `.md.enc` encryption detection.
  - **Bite! Terminal Console**: Added dynamic `ResizeObserver` to docked terminal host automatically updating xterm dimensions and dispatching PTY WebSocket resize frames.
  - **Background Transfers & Task Manager**: Fixed stuck "Uploading..." floating pill by enforcing `.active` CSS class toggling in `finally` blocks.

---

## [0.7.0] - 2026-09-01

### 📝 NoteDog Notes Studio & Orthodox Power Tools Suite
- **NoteDog Hierarchical Markdown Notes Studio**:
  - **Tree Organizer & Notebook Structure**: Seamless creation, nesting, categorization, and editing of markdown documents with folder hierarchies.
  - **Interactive Checklists & Tasks**: Live interactive checklist parsing (`[ ]` / `[x]`) with instant status persistence.
  - **Template Engine**: Instant note creation from built-in templates (*Meeting Notes*, *Project Plan*, *Checklist / SOP*, *Daily Journal*).
  - **Automatic Snapshot Revision History**: In-place version snapshots stored under `.notedog_versions/` with 1-click preview and rollback.
  - **Instant Search & Tag Filtering**: Fast real-time keyword indexing and tag filtering across all user notes.
- **Enhanced Color Labels & Custom Tagging System**:
  - Direct metadata tagging with persistent SQLite indexes and file table badges.
  - Extended color preset palette (`🔴 🟠 🟡 🟢 🔵 🟣`) and instant filter shortcuts.
- **Multi-Platform Release & Packaging Updates**:
  - Linux packaging: Arch Linux (`PKGBUILD`, `commanderdog-bin.PKGBUILD`), Alpine Linux (`APKBUILD`), Debian/Ubuntu binaries, and multi-arch Docker images.
  - Windows packaging: Tauri v2 standalone native executable, MSI installer, NSIS setup, Scoop manifest, and Winget manifests.

---

## [0.6.9] - 2026-08-31

### ⚡ Bvckup 2 & SyncToy Delta Backup Engine, Background Daemons & Folder Watchers
- **4 Core Replication Profiles (SyncToy & Bvckup 2)**:
  - **🔄 Synchronize (Two-Way Sync)**: Bi-directional replication with collision and conflict detection, syncing newer files across both left and right panes.
  - **🪞 Echo / Mirror (One-Way Backup)**: Makes Destination an exact 1:1 replica of Source (left-to-right), updating modified files and automatically deleting destination orphans.
  - **➕ Contribute / Additive Backup**: Replicates additions and modifications from Source to Destination without deleting any destination files.
  - **🕰️ Subscribe / Historical Versioning**: Retains older versions of modified and deleted files in timestamped `_archive/YYYY-MM-DD_HHmmss/` snapshot trees with automatic retention pruning (e.g. 30 days).
- **Block-Level In-Place Binary Delta Copy Engine (Bvckup 2 Speed)**:
  - **Fine-Grained 64KB Chunk Hashing**: Compares source and destination blocks using CRC32 checksums, skipping identical blocks and writing *only* modified byte blocks in-place directly on disk.
  - **High-Speed Differential Patching**: Massive speedup for large databases, virtual disk images, video projects, and log files with zero redundant transfers.
  - **Cryptographic Verification**: Post-transfer bit-perfect integrity validation supporting CRC32 and SHA-256 signatures.
- **Automated Backup Profiles & Background Scheduler**:
  - **Persistent SQLite Backup Profile Manager**: Create, edit, toggle, and delete named backup jobs stored securely in the database.
  - **Background Tokio Scheduler Daemon**: Periodic automated execution for intervals (every X minutes/hours) and daily scheduled times (e.g. `02:00`).
  - **⚡ Real-Time Continuous Watchers**: Automated folder monitoring triggers syncing changes as soon as files are modified.
  - **Webhook Notifications**: Automated JSON POST webhook alerts on backup completion/failure for Discord, Slack, Telegram, Matrix, or custom servers.
  - **Execution History & Audit Log**: Audit log tracking timestamp, profile name, status (Success/Failed), files copied/updated/deleted/archived, and elapsed duration.
- **Upgraded Sync & Backup Studio UI**:
  - **3-Tab Navigation**: Seamless switching between *Live Diff & Replication*, *Backup Jobs & Scheduler*, and *Run History & Logs*.
  - **Interactive Profile Selection Cards**: Visual cards with descriptive summaries for Synchronize, Echo, Contribute, and Subscribe.
  - **Rich Action Badges & Filters**: Clear visual status indicators (`➔ Copy Right`, `⬅ Copy Left`, `⚡ Delta Patch`, `📦 Archive Old`, `🗑 Delete`, `✔ Equal`).
  - **1-Click Profile Creation**: Pre-fill and save any active diff comparison into a reusable automated backup job.

---

## [0.6.8] - 2026-08-31

### 📐 Resizable Columns, Custom Column Chooser, Multi-Size Grid & Folder Tree Sidebar
- **Interactive Drag-to-Resize Table Columns**:
  - **Dynamic Column Resizers**: Smooth grab-and-drag dividers on all table headers (`Name`, `Ext`, `Size`, `Modified`, `Created`, `Mode`, `Owner`, `Group`, `SHA-256`, `Tags`).
  - **Double-Click Auto-Fit (`autoFitColumn`)**: Double-clicking any column divider automatically scans DOM content widths and fits the column perfectly (with min 45px / max 600px limits).
  - **Auto-Fit All Columns**: One-click action to auto-fit all visible columns simultaneously.
  - **Sticky LocalStorage Persistence**: Column widths save to `localStorage.cd_col_widths` and seamlessly restore across sessions and panes.
- **Custom Column Chooser & Header Settings Popover**:
  - **Right-Click Table Header Chooser**: Right-clicking any column header opens a sleek popover with checkboxes to toggle any column on or off on the fly.
  - **Quick Header Slider Button**: Added `sliders-horizontal` tool button to pane top bar for instant column visibility customization.
  - **Extended Columns**: Added support for *Date Created*, *SHA-256 Checksum Preview*, and *Color Labels & Tags* directly in the file table.
  - **Reset to Defaults**: 1-click restore to standard orthodox column widths and visibility.
- **Thumbnail Gallery Multi-Size View Modes**:
  - **4 Card Preview Sizes**: Seamless switching between Small (`90px`), Medium (`130px`), Large (`180px`), and Extra Large (`260px`) grid cards.
  - **Rich Card Previews**: High-DPI thumbnails for images, videos, documents, audio discs, and folders with item counts.
- **Collapsible Directory Tree Sidebar Per Pane**:
  - **Pane Folder Tree (`[ 🌳 ]` Toggle)**: Added collapsible sidebar on the left side of any pane header.
  - **Expandable Hierarchy**: Live dynamic asynchronous subfolder expansion fetching subdirectories on demand.
  - **Active Node Sync**: Automatically highlights and navigates the tree node when browsing directories in the active pane.
  - **Draggable Sidebar Resizer**: Smooth drag divider to resize tree width between 120px and 450px with `localStorage` persistence.

---

## [0.6.7] - 2026-08-31

### 🖱️ Orthodox Context Menu & Submenus, Windows Properties Dialog & GitHub Markdown Engine
- **Orthodox Right-Click Context Menu & Submenus**:
  - **Full-Width Viewport & Touch Submenus Fix**: Eliminated media query pointer restrictions and parent clipping (`overflow: visible`), enabling multi-level flyout submenus across all display resolutions.
  - **Dynamic Collision Avoidance**: Automatically flips submenus to the left if near the right edge of the screen and offsets upwards if near the bottom viewport edge.
  - **Tiered Action Hierarchy**: Cleanly structured into *Open with...*, *Quick view (F3)*, *Edit (F4)*, *Properties (Alt+Enter)*, *Copy to...*, *Move to...*, *Clipboard Actions (Copy/Cut/Paste/Rename/Delete)*, *Archive Submenu*, and *Tools Submenu*.
- **Windows-Style Properties Dialog (`Alt+Enter`)**:
  - **Tab 1: General & Media/EXIF**: Detailed file metadata, dimensions, camera model, lens, exposure, audio/video stream codecs, and bitrate.
  - **Tab 2: Security & Permissions**: Interactive POSIX 3x3 permission matrix (`rwx` for User, Group, Others), octal display/input (`0755`), recursive permission applicator, and Owner/Group selectors.
  - **Tab 3: Checksums & Hashes**: On-demand calculation of SHA-256, MD5, and SHA-1 cryptographic hashes with 1-click clipboard copy.
  - **Tab 4: Color Labels & Tags**: Full palette selector and tag chip editor with instant SQLite persistence.
- **GitHub-Flavored Markdown (GFM) & HTML Rendering**:
  - **Raw HTML Tag Support**: Full support for `<details>`, `<summary>`, `<kbd>`, `<div>`, `<center>`, `<img>`, `<table>`, `<sup>`, `<sub>`, `<del>`, and `<br>` in EditorDog preview and Quick View.
  - **GitHub Alerts**: Dedicated callouts for `> [!NOTE]`, `> [!TIP]`, `> [!IMPORTANT]`, `> [!WARNING]`, and `> [!CAUTION]`.
  - **Code Blocks & Mermaid**: PrismJS syntax highlighting with 1-click *"Copy Code"* buttons and live Mermaid diagram rendering.
  - **Dual-Engine Architecture**: Integrated `marked.js` with zero-dependency offline fallback parser for airgapped environments.
- **Refined Header Color Dot & Custom Tags**:
  - Top context header featuring a crisp circular dot with expandable mini color palette (`[ 🔴 🟠 🟡 🟢 🔵 🟣 ✕ ] [More...]`).
  - Dynamic tag icon glowing emerald green (`#22c55e`) when tags are present on the file, and neutral gray when unassigned.
  - Dedicated simple Custom Tags modal with chip pill management and instant add/remove.
  - Fixed database persistence bug where clearing color labels was only visual; now explicitly updates and clears SQLite records.

---

## [0.6.6] - 2026-08-31

### ⚙️ Windows NT Service, Autostart & Rich Filetype Icon Suite
- **Windows NT Service & Autostart Management**:
  - Headless background service management via `commanderdog service [install|uninstall|start|stop|status]`.
  - Windows registry Run key autostart and Linux desktop integration.
  - Minimization to system tray via `--minimized` / `--tray-only`.
- **Extended Rich MIME Icon Suite**:
  - Over 100+ specialized high-DPI vector icons for programming languages, 3D models, databases, media codecs, and system directories.
  - Multi-resolution Windows `.ico` bundle for Windows Taskbar and Start Menu.
- **Intelligent User Home Directory Fallback**:
  - Automatic resolution of PAM / LDAP / DB home directory on fresh login sessions, preventing access errors on restricted roots (`/`).

---

## [0.6.0] - 2026-08-30

### 🪟 Windows Native Desktop, Installers & Package Distribution
- **Microsoft WebView2 Native Desktop App (`CommanderDog.exe`)**:
  - Tauri v2 standalone desktop architecture targeting `x86_64-pc-windows-msvc` utilizing built-in Microsoft WebView2 runtime without Electron overhead.
  - Interactive Windows System Tray icon with quick Summon/Hide and Minimize-to-Tray on close.
- **Windows Packaging & Package Managers**:
  - **Setup Installers (`.msi` / `.exe`)**: Automated NSIS setup installer and WiX `.msi` bundle with Desktop and Start Menu shortcut generation.
  - **Portable `.zip` Distribution**: Zero-install standalone archive with in-place database (`commanderdog.db`) and configuration persistence.
  - **Windows Package Managers**: Automated **Winget** (`winget install Woofson.CommanderDog`) and **Scoop** bucket manifests in `packaging/windows/`.
  - **Windows Explorer Context Menu**: Added 1-click registration scripts (`register-context-menu.reg`) integrating *"Open in CommanderDog"* into directory and background right-click context menus.
- **Windows Filesystem & UNC Path Engine**:
  - Full drive letter breadcrumb navigation (`C:\`, `D:\`, `Z:\`).
  - Universal Windows environment variable resolution (`%USERPROFILE%`, `%APPDATA%`, `%LOCALAPPDATA%`, `%TEMP%`).
  - Windows UNC network share access (`\\server\share\path`).
  - Slide-up web terminal defaulting to Windows PowerShell / `COMSPEC` (`cmd.exe`).
- **Release Matrix CI/CD Pipeline**:
  - Created `.github/workflows/release.yml` for automated multi-target builds across `windows-latest` (MSVC) and `ubuntu-22.04` (Debian/Tarball) with automatic SHA-256 integrity checksum generation.
- **Documentation**:
  - Added comprehensive [**`WINDOWS.md`**](WINDOWS.md) installation, deployment, and packaging guide.

---

## [0.5.5] - 2026-08-29

### 🔒 Transparent Encrypted Vaults (AES-256-GCM / Argon2id) & Subsystem Deletions
- **Transparent Encrypted Vaults (`.cdvault` / `.cdv`)**:
  - Zero-knowledge, self-contained password-protected virtual filesystem containers.
  - Authenticated encryption powered by **AES-256-GCM** (96-bit random nonce + 128-bit authentication tag per data block).
  - Password key derivation via **Argon2id** with a 16-byte cryptographically secure salt.
  - Zero plaintext disk leakage: on-the-fly decryption and streaming directly in volatile RAM buffers.
  - Auto-lock inactivity timers (5m, 15m, 30m, 1h, 4h, session) with immediate memory purging upon lock.
  - 1-Click breadcrumb lock chip and full in-memory live editing with **EditorDog**.
  - Comprehensive documentation in [**`VAULT.md`**](VAULT.md).
- **Subsystem & Cross-Mount Deletion Engine**:
  - Implemented cross-device copy+delete fallback for trash operations overcoming Linux `EXDEV` limitations across separate mounts and subsystem partitions.
  - Added `force_remove_entry` with automatic `0777` permission correction and system `rm -rf --` CLI fallback for root/subsystem mounted folders.
- **Context Menu UX Hardening**:
  - Capture-phase global dismiss listener ensuring all context menu action clicks immediately close the menu.

---

## [0.5.0] - 2026-08-29

### 🔒 Security, UI Modernization & Streamlined UX Release
- **Comprehensive Credential Leakage Elimination & URI Sanitization**:
  - Audited and secured all frontend components and backend endpoints against credential exposure.
  - Sftp/Smb VFS backends (`src/vfs/sftp.rs`, `src/vfs/mod.rs`) strictly serialize clean `sftp://user@host:port/path` into `FileEntry` and `DirectoryListing` without embedded passwords.
  - Deep File Search, Spotlight, Disk Usage, File Operations, and Breadcrumbs automatically sanitize all displayed paths.
  - Ephemeral in-memory authentication router (`resolveAuthUri`) transparently handles session credentials over the wire while keeping DOM and storage 100% credential-free.
- **Leftmost Unified Pane Button & Customization Popover**:
  - Replaced scattered palette icons and rename badges with a single, leftmost `[ 🟡 1 ]` button on each pane header across Desktop, Laptop, Tablet, Foldable, and Mobile.
  - Integrated 1-click customization popover: Rename label, 9-preset color swatches + custom hex color picker, border width (`1px`–`4px`), and active ring style.
  - Streamlined default pane naming to clean, non-redundant numbers (`1`, `2`, `3`, `4`).
- **Streamlined Top Navbar & App Launcher**:
  - Swapped Tools menu icon with modern **`layout-grid`** app launcher grid icon.
  - Moved Settings into the User Profile main menu and preserved quick-access global shortcuts (<kbd>F10</kbd> / F-key bar).
  - Cleaned redundant Lock and Terminal buttons from the top bar.
- **Touch & Click Dropdown Auto-Dismiss**:
  - Launching tools (EditorDog, Calculator, Terminal, Git, Sync, Search) from menus immediately dismisses the parent dropdown.
- **1-Click Remote Disconnect & Close Archive**:
  - Added instant `[ 🔌 ]` Unplug and `[ ✕ ]` Close Archive chips directly in breadcrumbs at index 0 for mobile, tablet, and desktop.
- **SFTP Remote `$HOME` Resolution**:
  - Empty path or `~` now opens the remote user's home folder directly using `sftp.realpath(".")`.

---

## [0.4.2] - 2026-08-29

### 🔒 Fixed & Enhanced — SSH/SFTP Client & Multi-Tier Authentication
- **Multi-Tier SSH Authentication Engine**:
  - Fixed `"no auth socket"` / agent connection errors by isolating SSH-Agent queries to only occur when an agent is actually reachable and responsive.
  - Implemented multi-tier authentication cascade:
    1. **Password Authentication**: Standard password verification.
    2. **Keyboard-Interactive Fallback**: Automatic keyboard-interactive prompt handler for servers requiring interactive challenge-response or 2FA.
    3. **Explicit & User SSH Keys**: Automatic discovery of user identity keys (`~/.ssh/id_ed25519`, `~/.ssh/id_rsa`, `~/.ssh/id_ecdsa`, `~/.ssh/id_dsa`).
    4. **SSH Agent Discovery**: Graceful connection to `SSH_AUTH_SOCK` identities without failing hard if the socket is absent.
- **Robust SFTP URI Encoding & Credential Handling**:
  - Implemented standard percent-encoding and decoding for usernames and passwords containing special characters (e.g. `@`, `:`, `/`, `%`).
  - Fixed SFTP credential propagation in Frontend modals (`openRemoteModal`, `saveNewGlobalMount`, and `connectRemoteToActivePane`).
- **Zero-Dependency Runtime Dynamic PAM Engine**:
  - Replaced legacy `pam-auth` crate and `-lpam_misc` compile-time linking with native runtime dynamic loading (`dlopen("libpam.so.0")`).
  - Completely eliminates linker errors (`rust-lld: error: unable to find library -lpam_misc` / `-lpam`) across Arch Linux, CachyOS, Debian, Ubuntu, and Fedora.
  - `cargo run` now builds and runs immediately out-of-the-box on Linux while maintaining seamless local Linux user PAM authentication.
- **Complete SFTP VFS Operations**:
  - Added direct SFTP support to `handle_read_file`, `handle_write_file`, `handle_download`, `handle_upload`, and remote connection tester (`handle_test_remote`).

---

## [0.4.1] - 2026-08-28

### 🛡️ Added — Configurable Storage Roots & Sandboxed User RBAC
- **Configurable Storage Roots (`[[storage.roots]]`)**:
  - Define any number of storage roots in `config.toml` (e.g. Mass Storage `/mnt/storage`, Application Data `/data`, Read-Only Backups `/mnt/backups`).
  - Each root supports `id`, `name`, `path`, `read_only: bool`, and `allowed_roles`.
- **System-Wide Sandboxing Switch (`allow_entire_system = false`)**:
  - Prevents CommanderDog from accessing arbitrary host files outside allowed storage roots and personal user homes.
  - When disabled, non-admin users and sandboxed accounts cannot traverse to `/etc`, `/sys`, or other unauthorized host paths.
- **Unified Path Validation Engine (`validate_path_access`)**:
  - Enforces path normalization (resolving `..`, `.`, traversal attacks) across all filesystem endpoints: listing, read, write, upload, download, move, copy, deltacopy, chmod, chown, delete, archive, diff, sync, and disk usage.
- **Per-User Allowed Roots RBAC (`allowed_roots`)**:
  - User accounts can be assigned specific allowed roots (e.g. `["storage", "data"]`) or `["*"]` for all roots.
  - Automatic SQLite migration: `ALTER TABLE users ADD COLUMN allowed_roots TEXT DEFAULT '["*"]'`.
- **Dynamic Home Directory Resolution**:
  - Linux PAM users dynamically resolve authentic `$HOME` from `/etc/passwd` (e.g. `/home/bolt`).
  - Virtual/Database users use configured `home_dir` with template fallback (`default_user_home_template = "/home/{username}"`).
- **Admin UI & Favorites Integration**:
  - Admin User Management table features interactive **Allowed Storage Roots** checkboxes alongside Protocol Services.
  - Pane **⭐ Quick Favorites** dropdown dynamically loads and renders authorized storage roots from `GET /api/storage/roots`.

### 🐳 Added — Multi-Arch Alpine & Debian Containers (GHCR)
- **Multi-Arch Docker Images (`linux/amd64`, `linux/arm64`)**:
  - `ghcr.io/woofson/commanderdog:alpine`: Ultra-lightweight static musl container (~18 MB).
  - `ghcr.io/woofson/commanderdog:latest` & `:bookworm`: Debian 12 Bookworm slim container (~35 MB).
- **Automated CI/CD Publishing**: GitHub Actions workflow (`.github/workflows/docker-publish.yml`) with automated QEMU multi-arch cross-compilation and GHCR publishing.

---

## [0.4.0] - 2026-08-28

### 🪓 Added — File Splitter, Git Client & User Home Startup
- **Multi-Part File Splitter & Combiner**:
  - Split large archives into sized chunks (`.001`, `.002`, ...) with `.sha256` integrity manifest generation.
  - 1-Click synchronous file combiner with automated SHA-256 checksum verification.
- **Integrated Git Client**:
  - Real-time Git status indicator for repository directories.
  - Side-by-side git diff viewer with staged vs unstaged file tracking.
  - Commit composer, Git log history, and branch push/pull operations.
- **Universal Tilde Expansion & User `$HOME` Startup**:
  - Panes automatically launch in `/home/$USER` on session start.
  - Universal path expansion for `~` and `~/...` across all directory and API handlers.

---

## [0.3.6] - 2026-08-28

### 🎨 Added — XDG Configuration, External Themes & Custom Palette Builder
- **Unified Fast-Path XDG Configuration**:
  - Sub-millisecond single-pass configuration loading from `~/.config/commanderdog/config.toml` (and `/etc/commanderdog/config.toml`).
- **External TOML Themes Discovery**:
  - Automatically loads custom `.toml` themes dropped into `~/.config/commanderdog/themes/` or `/etc/commanderdog/themes/`.
  - Supports single-theme flat files and multi-theme array files (`[[themes]]`).
- **In-Browser Web Custom Theme Creator & Exporter**:
  - Interactive theme designer in Settings (<kbd>F10</kbd>) with live color pickers and 1-click TOML download.
- **Tiling WM Borderless Mode**:
  - CLI flags `--no-decorations` / `--frameless` and configuration setting `window_decorations = false` for seamless Hyprland/Sway integration.

---

## [0.3.5] - 2026-08-26

### 🖥️ Added — Native Standalone Window & Wayland Explicit Sync
- **Standalone Native Desktop App (`commanderdog --standalone`)**:
  - Runs with native WebKitGTK / WebView2 engine without Electron overhead (~25–35 MB RAM footprint).
- **Wayland / Hyprland GDK Error 71 Resolution**:
  - Explicit synchronization handling to eliminate `wl_surface` protocol crashes on Wayland compositors.
- **Automated AUR Packaging**:
  - Arch Linux AUR packages: `commanderdog` (source) and `commanderdog-bin` (pre-compiled binary).

---

## [0.3.0] - 2026-08-24

### 🪟 Added — In-Pane Tool Docking & Power Tools
- **In-Pane Docking Engine (`⇲ Dock / ⇱ Float`)**:
  - Dock EditorDog, Terminal Console, Byte Calculator, Background Transfers, and Git Client directly into any active directory pane.
- **Dual-Pane Text & Markdown Editor**:
  - Syntax highlighting for 12+ programming languages, Find & Replace engine, and live Markdown preview with Mermaid.js diagram support.
- **ConvertX Universal File Converter**:
  - In-browser conversion for images (WebP, PNG, JPG, AVIF), audio (MP3, WAV, FLAC, OGG), video (MP4, WebM, MKV, GIF), and documents (PDF, TXT, HTML).

---

## [0.2.0] - 2026-08-22

### ⚡ Added — DeltaCopy Engine & Cloud Storage Integrations
- **DeltaCopy / RoboCopy / TeraCopy Engine**:
  - Delta-skip unchanged files, post-transfer cryptographic SHA-256 verification, and exponential backoff auto-retries.
- **Multi-Cloud & Remote Protocols**:
  - Samba/CIFS (`smb://`), NFSv3/v4 (`nfs://`), AWS S3/MinIO/R2, SFTP/SSH, WebDAV, Proton Drive E2EE, and Syncthing dashboard.
- **Visual POSIX Permissions ($3\times 3$ Matrix)**:
  - Interactive `chmod`/`chown` matrix with real-time octal calculation and system user/group synchronization.

---

## [0.1.0] - 2026-08-20

### 🚀 Initial Release
- Multi-Tab orthodox 1-to-4 toggleable file panes (`Alt+1`–`4`).
- Orthodox keyboard shortcuts (`Tab`, `F1`–`F10`, `Insert`, `Space`).
- Slide-up native Web Terminal (PTY over WebSockets).
- Real-time background task manager and floating progress indicator.
