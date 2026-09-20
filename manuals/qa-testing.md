# Brum QA Testing & Verification Manual

> **Document Version**: `3.0.0`  
> **Target Release**: `Brum v0.8.10+ / v1.0.0 Readiness`  
> **Maintainer**: Bolt J. Woofson <bolt@boop.no>  
> **Repository**: [Woofson/brum](https://github.com/Woofson/brum)

---

## 1. Overview & Testing Strategy

Brum combines automated backend test coverage with rigorous multi-viewport manual testing.

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                              Brum Quality Assurance Matrix                             │
├───────────────────────────────────────────┬────────────────────────────────────────────┤
│ 🤖 Automated Backend Testing              │ 🧪 Manual Viewport & Modality Testing      │
│ (Rust Cargo Test Runner, CI/CD)           │ (Phone, Tablet, PC, Browsers, Touch)       │
├───────────────────────────────────────────┼────────────────────────────────────────────┤
│ • Cryptographic engines & Argon2id vaults │ • Phone (< 600px): Touch-first single pane │
│ • VFS drivers (Local, SFTP, SMB, WebDAV)  │ • Tablet (600px–1024px): Adaptive panels   │
│ • Database integrity & SQLite WAL schema  │ • PC (> 1024px): Multi-panel & F-Keys      │
│ • Task scheduling & DeltaSync replication │ • Input modalities: Mouse, Touch, Stylus   │
│ • Token generation, PAM & OIDC parsing    │ • Browser rendering: Chrome, Firefox, WebKit│
└───────────────────────────────────────────┴────────────────────────────────────────────┘
```

---

## 2. Automated Test Suite (`cargo test`)

Brum includes **54 automated unit and integration test suites** in the Rust backend. These are run automatically on every build and pre-release check.

```bash
cargo test
```

### Automated Coverage Breakdown

| Subsystem | Automated Test Functions | What is Verified Automatically |
| :--- | :--- | :--- |
| **VFS & Filesystem** | `test_clean_path_buf_strips_unc_and_verbatim_prefix`<br>`test_copy_file_paranoid_prevents_recursive_loop`<br>`test_copy_file_paranoid_success`<br>`test_delete_and_rename_with_options`<br>`test_list_branch_view_hidden_filter`<br>`test_list_branch_view_max_entries_truncation`<br>`test_list_branch_view_recursive_flatten`<br>`test_resolve_local_path_windows_prefix`<br>`test_windows_native_helpers_and_lock_detection`<br>`test_sftp_parse_uri_variations`<br>`test_sanitize_uri` | • Path normalization & traversal prevention<br>• Recursive copy loop protection<br>• Branch/Flat view directory flattening<br>• SFTP URI parsing across IPv4/IPv6/ports<br>• Windows verbatim prefix (`\\?\`) handling |
| **Authentication & RBAC** | `test_auth_sqlite_wal_pragmas`<br>`test_verify_token_allow_expired_for_session_unlock`<br>`test_api_token_lifecycle_and_revocation`<br>`test_user_preferences_persistence`<br>`test_user_group_cache_ttl`<br>`test_is_role_permitted`<br>`test_extract_terminal_claims_standalone`<br>`test_extract_terminal_claims_with_token_and_query` | • SQLite WAL mode and schema initialization<br>• JWT token issuance, expiry & signature verification<br>• API token generation, prefix hashing & revocation<br>• Session unlock with expired token grace period<br>• Role-Based Access Control (Admin vs User vs Readonly)<br>• Terminal PTY claim extraction and auth guards |
| **Vaults & Encryption** | `test_vault_create_unlock_write_read_cycle`<br>`test_notedog_encryption_cycle`<br>`test_notedog_wrong_password` | • `.cdvault` Argon2id + AES-256-GCM RAM container<br>• Zero plaintext persistence on vault lock<br>• AES-256 encrypted database note attachments<br>• Rejection of invalid encryption passphrases |
| **Sync & Backup Studio** | `test_sync_replication_profiles`<br>`test_backup_manager_crud`<br>`test_splitter_and_combine_integrity` | • Replication modes (Mirror, Synchronize, Backup)<br>• Checksum calculation & delta block verification<br>• File splitting (`.001`, `.002`) and SHA-256 recombine |
| **Tools & Utilities** | `test_duplicate_scan_and_clean`<br>`test_duplicate_scan_with_filters`<br>`test_disk_usage_scan_and_report`<br>`test_disk_usage_single_file`<br>`test_disk_usage_nonexistent`<br>`test_get_system_disks_enumeration`<br>`test_pdf_merge`<br>`test_pdf_info_and_split`<br>`test_id3v1_metadata_cycle`<br>`test_log_viewer_tail_and_filters`<br>`test_tags_and_color_label_persistence_and_clearing`<br>`test_git_status_and_actions` | • Byte-exact duplicate file scanner & filters<br>• Disk usage recursive analysis<br>• PDF page splitting, rotation, and merging<br>• Audio ID3 tag parsing and embedding<br>• File color tagging and label persistence<br>• Git staging, diff, and status detection |
| **Sharing & Chewtoys** | `test_advanced_sharing_center_lifecycle`<br>`test_plugin_pack_and_install_grr`<br>`test_handle_install_and_list_plugins` | • Public share token generation, ACLs, and expiry<br>• `.grr` Chewtoy plugin extraction & manifest parsing<br>• Sandboxed plugin installation & uninstallation |
| **Server & HTTP** | `test_handle_health_endpoint`<br>`test_handle_static_asset_etags`<br>`test_http_range_parsing`<br>`test_normalize_path_resolution`<br>`test_path_starts_with_case_insensitive`<br>`test_master_config_parsing`<br>`test_external_theme_flat_parsing`<br>`test_external_theme_multi_parsing`<br>`test_terminal_config_parsing`<br>`test_storage_config_parsing` | • HTTP 206 Partial Content range requests (Media streaming)<br>• Static asset ETag caching & compression<br>• Master `config.toml` & external theme loader |

---

## 3. Manual Testing Protocol: Viewports & Modalities

Manual testing must be executed whenever UI layout, touch interactions, responsive styling, or frontend event forwarding are updated.

---

### 📱 SECTION 1: Phone Viewport (`< 600px`)

*Target devices: Mobile smartphones in portrait orientation (360px–480px) and folded screens of foldable devices. Primary input: Touch & On-Screen Keyboard.*

| Test ID | Test Scenario | Step-by-Step Procedure | Expected Result | Pass / Fail |
| :--- | :--- | :--- | :--- | :---: |
| **MOB-01** | **Single Panel Constraint** | 1. Resize browser or open on phone (`< 600px`).<br>2. Observe the main workspace. | • Single panel displays full width.<br>• No horizontal page scrolling.<br>• Breadcrumb path bar truncates cleanly with ellipsis (`...`). | `[ ]` |
| **MOB-02** | **Header Branding Minimization** | 1. Observe top application header on phone. | • Text logo ("Brum") hides gracefully.<br>• Logo icon and essential tools remain accessible.<br>• User avatar collapses to compact 32px circular icon. | `[ ]` |
| **MOB-03** | **Mobile Bottom Action Bar** | 1. Check bottom action bar.<br>2. Tap `+ Select` button.<br>3. Tap multiple files. | • Checkboxes appear next to table rows.<br>• Live counter updates (`Selected: 3 (1.2 MB)`).<br>• Action buttons (`Cut`, `Copy`, `Delete`, `Actions`) activate. | `[ ]` |
| **MOB-04** | **Slide-Up Context Menu** | 1. Select files and tap `⚡ Actions` on the bottom bar.<br>2. Scroll through context options. | • Menu slides up from the bottom as a modal bottom sheet.<br>• Touch targets are minimum 44px height.<br>• Submenus expand accordion-style without off-screen clipping. | `[ ]` |
| **MOB-05** | **Touch Long-Press Selection** | 1. Long-press any file row for 500ms.<br>2. Short-tap a folder.<br>3. Short-tap a file. | • Long-press selects row with haptic feedback.<br>• Short-tap navigates into folder.<br>• Short-tap on file opens previewer. | `[ ]` |
| **MOB-06** | **Media Player Interactive Pill** | 1. Start playing audio or video.<br>2. Tap minimize button on media player.<br>3. Tap play/pause or next track on floating pill.<br>4. Tap pill body to restore. | • Media player collapses into an interactive bottom-right pill.<br>• Controls operate directly on the pill without full window restoration.<br>• Clicking pill body restores window. | `[ ]` |
| **MOB-07** | **Virtual Keyboard & Form Offset** | 1. Open Renamer, Notes, or Search modal.<br>2. Tap inside an input field to raise on-screen keyboard. | • Viewport adjusts dynamically via `dvh` / `interactive-widget`.<br>• Input field remains centered above keyboard (not occluded). | `[ ]` |
| **MOB-08** | **High-DPI Touch Gestures** | 1. Tap an image to open viewer.<br>2. Swipe left/right.<br>3. Double-tap and pinch-to-zoom. | • Swipe navigates to adjacent photos in the directory.<br>• Double-tap zooms 200%; pinch gestures zoom smoothly.<br>• Swipe down or `[✕]` dismisses viewer. | `[ ]` |

---

### 📖 SECTION 2: Tablet & Foldable Viewport (`600px – 1024px`)

*Target devices: iPad, Android tablets, foldables in unfolded tablet mode (e.g. Galaxy Z Fold, Pixel Fold, Surface Duo). Primary input: Touch, Stylus, Bluetooth Keyboard & Trackpad.*

| Test ID | Test Scenario | Step-by-Step Procedure | Expected Result | Pass / Fail |
| :--- | :--- | :--- | :--- | :---: |
| **TAB-01** | **Adaptive Dual-Panel Transition** | 1. Start with phone viewport (`< 600px`).<br>2. Unfold or expand viewport width to `> 600px`. | • Workspace dynamically transitions from single-panel to side-by-side dual-panel without page reload or state loss. | `[ ]` |
| **TAB-02** | **Orientation Shift (Portrait ↔ Landscape)** | 1. In landscape orientation (`> 768px`), view dual vertical panels.<br>2. Rotate device to portrait mode. | • Dual panels adjust column widths proportionally.<br>• Panel headers, toolbars, and path bars remain legible with 26px standard buttons. | `[ ]` |
| **TAB-03** | **Touch & Stylus Drag-and-Drop** | 1. Select files on Panel 1 with stylus or finger.<br>2. Drag across the center divider and drop into Panel 2. | • Drag ghost indicator displays selected count.<br>• Target panel highlights with active drop border.<br>• Confirmation prompt appears with Copy / Move choices. | `[ ]` |
| **TAB-04** | **Notes Sliding Drawer Mode** | 1. Open Notes Core Function from tools launchpad.<br>2. Switch between floating window and in-pane docked mode. | • Notes docks cleanly into active panel without iframe overhead.<br>• Sliding note drawer allows note selection and instant markdown editing. | `[ ]` |
| **TAB-05** | **Hinge Seam Avoidance (Dual-Screen Foldables)** | 1. Open Brum on a dual-screen device with a physical hinge (`horizontal-viewport-segments: 2`). | • Panel 1 maps to Screen 1 (Left); Panel 2 maps to Screen 2 (Right).<br>• Center splitter aligns with physical hinge gap preventing text splitting. | `[ ]` |
| **TAB-06** | **Tablet Directory Tree Sidebar** | 1. Tap `[ 🌳 ]` folder tree button on panel header.<br>2. Expand nested subdirectories. | • Collapsible tree expands with smooth touch response.<br>• Selecting a tree node updates the panel file table immediately. | `[ ]` |

---

### 💻 SECTION 3: PC Desktop Viewport (`> 1024px`)

*Target devices: Desktop PCs, laptops, and ultrawide monitors (1080p, 1440p, 4K, Ultrawide). Primary input: Physical Mouse, Scroll Wheel, and Keyboard.*

| Test ID | Test Scenario | Step-by-Step Procedure | Expected Result | Pass / Fail |
| :--- | :--- | :--- | :--- | :---: |
| **PC-01** | **Header Margins & Alignment** | 1. View top header on wide PC screen (`1920x1080`+).<br>2. Check left logo position and right profile button. | • Header has comfortable 18px horizontal padding on both edges.<br>• Logo is not pressed against left monitor bezel.<br>• User profile button has proper right margin and does not clip outside viewport. | `[ ]` |
| **PC-02** | **Passive Mouseover Wheel Scrolling** | 1. Ensure Panel 2 is **inactive** (Panel 1 has active focus ring).<br>2. Hover mouse cursor over Panel 2's header, breadcrumbs, column headers, and status bar.<br>3. Scroll mouse wheel up and down. | • Inactive Panel 2 scrolls smoothly without needing to click or activate it first.<br>• Scrolling directly over table rows also scrolls smoothly. | `[ ]` |
| **PC-03** | **Focus Follows Mouse (Hover Activation)** | 1. Open **Settings > General > Layout & Viewport Defaults**.<br>2. Enable **"Activate Panel on Mouse Hover (Focus Follows Mouse)"**.<br>3. Move cursor back and forth between Panel 1 and Panel 2.<br>4. Use arrow keys (<kbd>↑</kbd>/<kbd>↓</kbd>) immediately after hovering. | • Active panel indicator and keyboard focus shift immediately to the hovered panel without clicking.<br>• Keyboard navigation operates on the hovered panel instantly. | `[ ]` |
| **PC-04** | **Multi-Panel Layout Switcher** | 1. Use header layout toggle group to switch layouts:<br>• Single (`1`)<br>• Dual Vertical (`2V`)<br>• Dual Horizontal (`2H`)<br>• Triple Columns (`3`)<br>• Triple Split 1+2 (`3S`)<br>• Quad 2x2 (`4`). | • Panes reconfigure instantly with preserved paths and selection state.<br>• Active layout button displays amber accent glow.<br>• Keyboard shortcuts (<kbd>Alt+1</kbd> to <kbd>Alt+4</kbd>) switch layouts. | `[ ]` |
| **PC-05** | **Orthodox Keyboard Navigation (<kbd>F1</kbd>–<kbd>F10</kbd>)** | Test standard keyboard shortcuts:<br>• <kbd>Tab</kbd>: Switch active panel.<br>• <kbd>↑</kbd>/<kbd>↓</kbd>: Move cursor selection.<br>• <kbd>Space</kbd> / <kbd>Insert</kbd>: Toggle selection and advance cursor.<br>• <kbd>Enter</kbd>: Open directory / execute.<br>• <kbd>F2</kbd>: Quick Rename.<br>• <kbd>F3</kbd>: Quick View / Document Viewer.<br>• <kbd>F4</kbd>: Multi-Tab Editor.<br>• <kbd>F5</kbd>: Copy to opposite panel.<br>• <kbd>F6</kbd>: Move to opposite panel.<br>• <kbd>F7</kbd>: New Directory.<br>• <kbd>F8</kbd> / <kbd>Delete</kbd>: Delete files.<br>• <kbd>F9</kbd>: Compare & Diff.<br>• <kbd>F10</kbd>: Settings Hub. | • Every shortcut performs its dedicated action without lag.<br>• Bottom F-Key bar reflects active keys and responds to mouse clicks. | `[ ]` |
| **PC-06** | **Drag-to-Resize Table Columns** | 1. Hover cursor over column divider lines (Name, Ext, Size, Modified, Created, Mode, Tags).<br>2. Click and drag left/right.<br>3. Double-click column divider.<br>4. Refresh browser (<kbd>Ctrl+F5</kbd>). | • Cursor changes to `col-resize` and column smoothly resizes live.<br>• Double-click auto-fits column to longest entry.<br>• Custom column widths persist after refresh. | `[ ]` |
| **PC-07** | **Branch / Flat View (<kbd>Ctrl+B</kbd>)** | 1. Navigate into a deep nested folder hierarchy.<br>2. Press <kbd>Ctrl+B</kbd>.<br>3. Sort by **Size** descending.<br>4. Click `[✕ Exit Branch View]`. | • Nested files flatten into a single unified list.<br>• Sorting identifies largest storage consumers across all subfolders.<br>• Exiting restores regular hierarchical view. | `[ ]` |
| **PC-08** | **Spotlight Quick-Switcher (<kbd>Ctrl+K</kbd>)** | 1. Press <kbd>Ctrl+K</kbd> / <kbd>Cmd+K</kbd>.<br>2. Type fuzzy query (e.g. `calc`, `sync`, `vault`, `diff`, `notes`, `/var/log`).<br>3. Use <kbd>↑</kbd>/<kbd>↓</kbd> and press <kbd>Enter</kbd>. | • Modal opens with instantaneous fuzzy search.<br>• Enter executes action or navigates to directory. | `[ ]` |
| **PC-09** | **Bite! Terminal & Nerd Fonts** | 1. Press <kbd>\`</kbd> (Backtick) or click Terminal.<br>2. Run `eza --icons`, `ls -la`, `git status`, or interactive CLI (`htop`, `vim`).<br>3. Press <kbd>Ctrl+D</kbd> or type `exit`. | • Terminal opens with full ANSI color support.<br>• JetBrainsMono Nerd Font glyphs render without tofu boxes.<br>• Exiting PTY automatically closes terminal drawer. | `[ ]` |
| **PC-10** | **3D CAD Studio & Model Viewer** | 1. Click `.stl`, `.obj`, or `.gltf` / `.glb` 3D model.<br>2. Use mouse to rotate, pan, and zoom.<br>3. Toggle wireframe mode, bounding box, and grid helpers. | • Three.js WebGL viewport renders 3D mesh smoothly.<br>• Mouse drag orbits camera, scroll zooms, right-drag pans.<br>• Mesh stats (poly count, vertex count, bounding dimensions) calculate accurately. | `[ ]` |

---

## 4. Authentication & Security Verification Matrix

| Test ID | Test Scenario | Step-by-Step Procedure | Expected Result | Pass / Fail |
| :--- | :--- | :--- | :--- | :---: |
| **AUTH-01** | **OpenID Connect (OIDC) / Authentik SSO Flow** | 1. Configure `[auth.oidc]` in `brum.toml` (or env vars).<br>2. Open login screen.<br>3. Click **"Sign in with Authentik"**.<br>4. Authenticate at Authentik portal.<br>5. Observe return to Brum dashboard. | • Login card displays high-contrast SSO button.<br>• Redirects to Authentik authorization endpoint with PKCE.<br>• Returns to `/` with valid session token and success toast.<br>• New user profile auto-provisions in database. | `[ ]` |
| **AUTH-02** | **SSO Admin Group Role Mapping** | 1. In Authentik, place user in `brum-admins` group.<br>2. Sign in with SSO.<br>3. In Authentik, remove user from admin group and re-login. | • User in `brum-admins` is automatically elevated to `admin` in Brum.<br>• User without admin group receives default `user` role. | `[ ]` |
| **AUTH-03** | **Direct SSO Bypass (`force_sso_only`)** | 1. Set `force_sso_only = true` in `brum.toml`.<br>2. Open unauthenticated browser session at `/`.<br>3. Open `/` with `?local=1` parameter. | • Visiting `/` immediately redirects to Authentik without showing login modal.<br>• Visiting `/?local=1` allows local admin password entry. | `[ ]` |
| **AUTH-04** | **Session Lock (<kbd>Ctrl+Alt+L</kbd>) & Inactivity Timeout** | 1. Press <kbd>Ctrl+Alt+L</kbd> or click Lock from profile menu.<br>2. Enter password/PIN to unlock.<br>3. Leave browser inactive for configured timeout (e.g. 15m). | • Lock screen obscures all workspace panes.<br>• Valid password restores session without page reload.<br>• Inactivity timer triggers lock screen automatically. | `[ ]` |
| **AUTH-05** | **Zero-Leakage Encrypted Vaults (.cdvault)** | 1. Click **Vaults > Create New Vault**.<br>2. Set master passphrase and container size.<br>3. Unlock vault, create sensitive files inside, and lock vault. | • Vault mounts to virtual in-memory VFS.<br>• Locking vault immediately zeroes RAM keys; container on disk remains ciphertext only. | `[ ]` |

---

## 5. Acceptance Sign-off Matrix

| Platform / Viewport | Browser Tested | Tests Passed | Tester | Date | Release Decision |
| :--- | :--- | :---: | :---: | :---: | :---: |
| 📱 **Phone (< 600px)** | **Chrome / Kiwi Mobile** | _____ / 8 | | | `[ ] PASS` / `[ ] FAIL` |
| 📱 **Phone (< 600px)** | **Firefox Mobile** | _____ / 8 | | | `[ ] PASS` / `[ ] FAIL` |
| 📱 **Phone (< 600px)** | **Vivaldi Mobile** | _____ / 8 | | | `[ ] PASS` / `[ ] FAIL` |
| 📖 **Tablet (600px–1024px)** | **iPad Safari** | _____ / 6 | | | `[ ] PASS` / `[ ] FAIL` |
| 📖 **Tablet (600px–1024px)** | **Android Tablet Chrome** | _____ / 6 | | | `[ ] PASS` / `[ ] FAIL` |
| 💻 **PC Desktop (> 1024px)** | **Chrome Desktop** | _____ / 10 | | | `[ ] PASS` / `[ ] FAIL` |
| 💻 **PC Desktop (> 1024px)** | **Firefox Desktop** | _____ / 10 | | | `[ ] PASS` / `[ ] FAIL` |
| 💻 **PC Desktop (> 1024px)** | **Vivaldi / Edge Desktop** | _____ / 10 | | | `[ ] PASS` / `[ ] FAIL` |
| 🔒 **Auth & Security** | **Authentik SSO / Local PAM** | _____ / 5 | | | `[ ] PASS` / `[ ] FAIL` |

### Final Release Decision
- [ ] **RELEASE CANDIDATE APPROVED** (All automated tests pass + manual sign-off complete)
- [ ] **BLOCKED** (Remediation required for reported regressions)
