# 📋 Brum Factory Acceptance Test (FAT) & Verification Matrix

> **Document Version**: `2.1.0`  
> **Target Release**: `Brum v0.7.0 (Desktop & Web)`  
> **Author**: Bolt J. Woofson <bolt@boop.no>  
> **Last Updated**: 2026-09-01  

---

## 🎯 Matrix Overview: Viewports & Input Modalities

Brum is architected to deliver an orthodox two-pane desktop file manager experience on wide displays with physical mouse and keyboard, while seamlessly transforming into an adaptive touch-first interface on phones and foldables.

```
┌────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Brum Viewport & Form-Factor Taxonomy                                                          │
├──────────────────────┬─────────────────────────────┬───────────────────────────┬───────────────────────┤
│ Form Factor Category │ Target Viewport Width       │ Primary Input Modalities  │ Layout & Architecture │
├──────────────────────┼─────────────────────────────┼───────────────────────────┼───────────────────────┤
│ 📱 Phone             │ ≤ 480px (Portrait Mobile)   │ Touch, Gestures, On-Screen │ Single-pane auto-fit, │
│                      │                             │ Keyboard, Tap/Long-Press  │ bottom action bar,    │
│                      │                             │                           │ slide-up bottom sheet │
├──────────────────────┼─────────────────────────────┼───────────────────────────┼───────────────────────┤
│ 📖 Foldable / Tablet │ 601px – 1024px              │ Touch, Stylus, Bluetooth  │ Adaptive dual-pane,   │
│                      │ (Fold Unfolded, Dual-Screen)│ Keyboard, Trackpad        │ CSS viewport segments │
│                      │                             │                           │ (hinge seam avoidance)│
├──────────────────────┼─────────────────────────────┼───────────────────────────┼───────────────────────┤
│ 💻 Desktop / Laptop  │ ≥ 1025px                    │ Physical Keyboard (F-Keys)│ Multi-pane (1 to 4),  │
│    & Ultrawide       │ (Standard, 1440p, 4K, Ultra)│ Mouse (Wheel, Drag, R-Clk)│ drag-to-resize cols,  │
│                      │                             │                           │ floating window dock  │
└──────────────────────┴─────────────────────────────┴───────────────────────────┴───────────────────────┘
```

---

## 📱 SECTION 1: Phone Verification Protocol ($\le 480\text{px}$ Touch)

*Tested on mobile viewports ($\le 480\text{px}$) with pure touch interaction (Vivaldi Mobile, Firefox Mobile, Chrome/Kiwi).*

| Test ID | Test Scenario | Step-by-Step Procedure & Acceptance Criteria | Vivaldi | Firefox | Status |
| :--- | :--- | :--- | :---: | :---: | :---: |
| **MOB-01** | **Single-Pane Auto-Fit** | 1. Open app on phone viewport ($\le 480\text{px}$).<br>2. Verify single active pane renders without horizontal body scroll.<br>3. Path breadcrumb truncates neatly with `...` without wrapping. | [ ] | [ ] | [ ] |
| **MOB-02** | **Mobile Bottom Action Bar** | 1. Check bottom fixed action bar (`⚡ Actions`, `✂️ Cut`, `📋 Copy`, `🗑️ Delete`, `+ Select`).<br>2. Tap `+ Select` button $\rightarrow$ selection checkboxes appear next to all items.<br>3. Tap items to check/uncheck; counter on bar updates live (`Selected: 3 (12.4 MB)`). | [ ] | [ ] | [ ] |
| **MOB-03** | **Touch Long-Press Selection** | 1. Long-press any file row for 500ms.<br>2. Item highlights with accent border, haptic pulse triggers, and item is selected.<br>3. Short tap on folder navigates into folder; short tap on file opens preview. | [ ] | [ ] | [ ] |
| **MOB-04** | **Slide-Up Bottom Sheet Context Menu** | 1. Select items and tap `⚡ Actions` on bottom bar.<br>2. Context menu slides up smoothly from bottom (`width: calc(100vw - 20px)`).<br>3. Verify all options fit on screen with no horizontal clipping or off-screen overflow. | [ ] | [ ] | [ ] |
| **MOB-05** | **Accordion Nested Submenus** | 1. In mobile context menu, tap **`Quick Copy to...`**, **`Archive Submenu`**, or **`Tools`**.<br>2. Submenu expands **vertically inline (accordion style)** with animated chevron.<br>3. Sub-items are directly tappable with 44px minimum touch targets. | [ ] | [ ] | [ ] |
| **MOB-06** | **Color Labels & Quick Tagging** | 1. In context menu, tap red/green/blue color swatch.<br>2. Target file instantly receives tinted background and colored indicator badge.<br>3. Color tag persists across directory reloads. | [ ] | [ ] | [ ] |
| **MOB-07** | **Dynamic Viewport Height & Address Bar Sync** | 1. Scroll file table up and down causing browser address bar to expand and collapse.<br>2. Verify `--viewport-height` and `--mobile-bottom-offset` resize dynamically.<br>3. Bottom action bar and table footer remain fully visible and clickable (never hidden under browser UI). | [ ] | [ ] | [ ] |
| **MOB-08** | **EditorDog Mobile Mode** | 1. Open a text file in EditorDog on mobile.<br>2. Top toolbar consolidates actions into a clean `⚙️ More ▾` menu.<br>3. Virtual keyboard appears: editor area scrolls without breaking toolbar docking. | [ ] | [ ] | [ ] |
| **MOB-09** | **Media Quick View Touch Gestures** | 1. Tap an image to open High-DPI Media Viewer.<br>2. Swipe left/right to browse previous/next image in folder.<br>3. Double-tap to zoom 200%; pinch to zoom; tap `[✕]` or swipe down to dismiss. | [ ] | [ ] | [ ] |
| **MOB-10** | **Docked Floating Calculator** | 1. Open Calculator from top menu on phone.<br>2. Sits cleanly as a docked bottom card with large touch keys.<br>3. Storage unit keys (`KB`, `MB`, `GB`, `TB`) compute live on tap. | [ ] | [ ] | [ ] |
| **MOB-11** | **Sync & Backup Studio on Mobile** | 1. Open Sync & Backup Studio modal.<br>2. Sub-tabs (`Live Diff`, `Backup Jobs`, `Run History`) stack neatly with horizontal scroll.<br>3. Replication mode cards wrap cleanly into a 2x2 grid. | [ ] | [ ] | [ ] |
| **MOB-12** | **Touch Dialogs & Prompt Confirmations** | 1. Trigger a delete or overwrite prompt.<br>2. Custom dialog renders centered with full-width primary `Confirm` button (min 48px height) and secondary `Cancel` button. | [ ] | [ ] | [ ] |

---

## 📖 SECTION 2: Foldable & Tablet Protocol ($601\text{px} - 1024\text{px}$)

*Tested on foldable devices (Galaxy Z Fold unfolded, Pixel Fold, Surface Duo) and tablets ($601\text{px}$ – $1024\text{px}$) with Touch, Stylus, and Keyboard Cover.*

| Test ID | Test Scenario | Step-by-Step Procedure & Acceptance Criteria | Vivaldi | Firefox | Status |
| :--- | :--- | :--- | :---: | :---: | :---: |
| **FOLD-01** | **Adaptive Unfold Breakpoint Transition** | 1. Start with phone folded ($\le 600\text{px}$, Single Pane).<br>2. Unfold device ($\ge 601\text{px}$).<br>3. Interface dynamically transitions to side-by-side Dual Pane without page reload or state loss. | [ ] | [ ] | [ ] |
| **FOLD-02** | **CSS Viewport Segments (Hinge API)** | 1. On dual-screen foldables with center hinge (`horizontal-viewport-segments: 2`).<br>2. Pane 1 docks to Left screen segment; Pane 2 docks to Right screen segment.<br>3. Center divider expands to align with the physical hinge gap preventing text occlusion. | [ ] | [ ] | [ ] |
| **FOLD-03** | **Side-by-Side Live Diff on Foldable** | 1. Open Directory Diff tool (<kbd>F9</kbd> or menu).<br>2. Left directory and Right directory compare side-by-side across the fold.<br>3. Differences highlight with aligned action arrows in the center column. | [ ] | [ ] | [ ] |
| **FOLD-04** | **Dual-Spread Markdown & Mermaid Editor** | 1. Open EditorDog and select Markdown / Mermaid view.<br>2. Code/Markdown input occupies left screen half; live rendered HTML/Mermaid diagram occupies right screen half. | [ ] | [ ] | [ ] |
| **FOLD-05** | **Orientation Shift (Portrait $\leftrightarrow$ Landscape)** | 1. Rotate device 90 degrees.<br>2. In landscape: Dual vertical panes side-by-side.<br>3. In portrait: Dual stacked horizontal panes or wide dual panes with auto-adjusted column widths. | [ ] | [ ] | [ ] |
| **FOLD-06** | **Comic / EPUB Dual-Page Spread** | 1. Open `.cbz` / `.epub` document in viewer.<br>2. Displays book spread across both screens with swipe gesture page turning. | [ ] | [ ] | [ ] |
| **FOLD-07** | **Stylus & Touch Drag & Drop between Panes** | 1. Select files on Left Pane using stylus or finger.<br>2. Drag across center fold and drop into Right Pane.<br>3. Drop confirmation modal appears with **Copy** / **Move** options. | [ ] | [ ] | [ ] |
| **FOLD-08** | **Tablet Directory Tree Sidebar** | 1. Click `[ 🌳 ]` toggle on Left Pane header.<br>2. Collapsible tree expands on the left (200px width).<br>3. Tap subfolders to expand hierarchy asynchronously; active table syncs to selected folder. | [ ] | [ ] | [ ] |

---

## 💻 SECTION 3: Desktop & Ultrawide Protocol ($\ge 1025\text{px}$ Mouse & Keyboard)

*Tested on desktop and laptop displays ($\ge 1025\text{px}$, 1080p, 1440p, 4K, Ultrawide) with physical mouse and keyboard.*

| Test ID | Test Scenario | Step-by-Step Procedure & Acceptance Criteria | Vivaldi | Firefox | Status |
| :--- | :--- | :--- | :---: | :---: | :---: |
| **DSK-01** | **Multi-Pane Layout Grid** | 1. Use top layout switcher to cycle through: Dual Vertical (`2V`), Dual Horizontal (`2H`), Quad (`2x2`), and 3-Pane (`3P`).<br>2. All panes render independently with separate path histories, active focus ring, and status bars. | [ ] | [ ] | [ ] |
| **DSK-02** | **Full Orthodox Keyboard Navigation** | 1. <kbd>Tab</kbd>: Switch active pane focus.<br>2. <kbd>↑</kbd>/<kbd>↓</kbd>: Move cursor selection.<br>3. <kbd>Space</kbd> / <kbd>Insert</kbd>: Toggle item selection and advance cursor.<br>4. <kbd>Enter</kbd>: Open folder / file.<br>5. <kbd>F2</kbd>: Rename.<br>6. <kbd>F3</kbd>: Quick View.<br>7. <kbd>F4</kbd>: Edit file.<br>8. <kbd>F5</kbd>: Copy to opposite pane.<br>9. <kbd>F6</kbd>: Move to opposite pane.<br>10. <kbd>F7</kbd>: New Directory.<br>11. <kbd>F8</kbd> / <kbd>Delete</kbd>: Delete files.<br>12. <kbd>F9</kbd>: Compare / Diff.<br>13. <kbd>F10</kbd>: Settings Hub.<br>14. <kbd>F12</kbd> / <kbd>Ctrl+Alt+L</kbd>: Lock session. | [ ] | [ ] | [ ] |
| **DSK-03** | **Interactive Drag-to-Resize Table Columns** | 1. Hover mouse over header column dividers (Name, Ext, Size, Modified, Created, Mode, Tags).<br>2. Cursor changes to `col-resize`.<br>3. Click and drag left/right $\rightarrow$ column smoothly resizes live.<br>4. Double-click divider $\rightarrow$ executes `autoFitColumn` fitting longest text.<br>5. Refresh browser $\rightarrow$ widths persist from `localStorage.cd_col_widths`. | [ ] | [ ] | [ ] |
| **DSK-04** | **Custom Column Chooser Popover** | 1. Right-click any table column header (or click `sliders-horizontal` button).<br>2. Popover opens with checkboxes for all extended columns (`SHA-256`, `Date Created`, `Permissions Octal`, `Owner`, `Group`, `Tags`).<br>3. Toggle checkboxes on/off $\rightarrow$ table columns show/hide instantly.<br>4. Click `Reset to Default` $\rightarrow$ restores standard orthodox widths. | [ ] | [ ] | [ ] |
| **DSK-05** | **Thumbnail Gallery Multi-Size View** | 1. Switch active pane view to Thumbnail Gallery (`grid` icon).<br>2. Use size selector to test: Small (`90px`), Medium (`130px`), Large (`180px`), Extra Large (`260px`).<br>3. High-DPI thumbnails load asynchronously for images, videos, audio discs, and folders. | [ ] | [ ] | [ ] |
| **DSK-06** | **Pane Directory Tree Sidebar** | 1. Click `[ 🌳 ]` button on pane top bar.<br>2. Collapsible tree expands with draggable right-edge resizer.<br>3. Subdirectories expand on demand (`/api/fs/list?dirs_only=true`).<br>4. Click tree node $\rightarrow$ pane navigates immediately to path. | [ ] | [ ] | [ ] |
| **DSK-07** | **SyncToy & Bvckup 2 Delta Backup Studio** | 1. Open Sync Studio (<kbd>F9</kbd> $\rightarrow$ Sync or menu).<br>2. Tab 1 (**Live Diff**): Choose mode (Synchronize, Echo, Contribute, Subscribe); toggle `⚡ Block-Level Delta` and `🔒 Verify Checksums`; run `Compare & Analyze`; click action pills to toggle actions.<br>3. Tab 2 (**Backup Jobs & Scheduler**): Click `+ New Backup Job`; set name, source, dest, trigger (`Interval 30m`, `Daily @ 02:00`, `Real-Time Watcher`), webhook URL; save profile.<br>4. Tab 3 (**Run History**): View audit log with duration, copied, archived, and deleted stats. | [ ] | [ ] | [ ] |
| **DSK-08** | **Spotlight Quick-Switcher (<kbd>Ctrl+K</kbd>)** | 1. Press <kbd>Ctrl+K</kbd> / <kbd>Cmd+K</kbd>.<br>2. Type fuzzy query (e.g. `calc`, `sync`, `vault`, `diff`, `/var/log`).<br>3. Navigate with <kbd>↑</kbd>/<kbd>↓</kbd> and press <kbd>Enter</kbd> $\rightarrow$ action or navigation executes immediately. | [ ] | [ ] | [ ] |
| **DSK-09** | **Flat / Branch View (<kbd>Ctrl+B</kbd>)** | 1. In deep nested folder hierarchy, press <kbd>Ctrl+B</kbd>.<br>2. All nested subfiles flatten into a single unified list.<br>3. Sort by **Size** descending $\rightarrow$ instantly identifies largest storage hogs across all subfolders.<br>4. Click `[✕ Exit Branch View]` to return to normal directory view. | [ ] | [ ] | [ ] |
| **DSK-10** | **Deep Content Grep & "Feed to Pane"** | 1. Press <kbd>Ctrl+F</kbd> $\rightarrow$ enter regex content query.<br>2. Search runs recursively across files.<br>3. Click **`📥 Feed to Active Pane`** $\rightarrow$ virtual search results populate active pane for batch copying/moving/tagging. | [ ] | [ ] | [ ] |
| **DSK-11** | **Inter-Pane Mouse Drag & Drop** | 1. Click and drag selected items from Pane 1 to a directory in Pane 2.<br>2. Drag ghost indicator displays file count badge.<br>3. Dropping opens confirmation dialog prompting **Copy**, **Move**, or **Cancel**. | [ ] | [ ] | [ ] |
| **DSK-12** | **Context Menu Collision Avoidance** | 1. Right-click near rightmost edge of screen ($x > \text{innerWidth} - 350$).<br>2. Flyout submenus automatically flip to the left.<br>3. Right-click near bottom viewport edge $\rightarrow$ menu offsets upward to prevent clipping. | [ ] | [ ] | [ ] |
| **DSK-13** | **Floating Multi-Window Window Manager** | 1. Open Floating Editor (<kbd>F4</kbd>), Calculator, Task Manager.<br>2. Drag title bars to position freely; drag 4 edges + corner handle to resize.<br>3. Click minimize button $\rightarrow$ docks to taskbar pill; click pill to restore. | [ ] | [ ] | [ ] |
| **DSK-14** | **Interactive WebSocket PTY Terminal** | 1. Press <kbd>\`</kbd> or click Terminal button.<br>2. Terminal console slides up with interactive PTY (`bash`/`zsh`/`powershell`).<br>3. Runs commands (`htop`, `git log`, `vim`) with full ANSI colors and resize support. | [ ] | [ ] | [ ] |
| **DSK-15** | **Transparent Encrypted Vaults (.cdvault)** | 1. Create new `.cdvault` container with master password.<br>2. Unlock vault $\rightarrow$ mounts virtual in-memory VFS (Argon2id + AES-256-GCM).<br>3. Create/edit files inside vault; lock vault $\rightarrow$ RAM is wiped with zero plaintext on disk. | [ ] | [ ] | [ ] |
| **DSK-16** | **PDF Power Studio** | 1. Open PDF file $\rightarrow$ select PDF Power Studio.<br>2. Visual grid renders page thumbnails.<br>3. Drag to reorder pages; rotate 90°; select page ranges to split; merge multiple PDFs into one. | [ ] | [ ] | [ ] |
| **DSK-17** | **ConvertX & File Splitter / Combiner** | 1. Right-click image/video/audio $\rightarrow$ ConvertX batch convert.<br>2. Right-click large file $\rightarrow$ Split into 100MB chunks with SHA-256 manifest.<br>3. Right-click `.001` part file $\rightarrow$ Combine & verify SHA-256 checksum. | [ ] | [ ] | [ ] |
| **DSK-18** | **Public Shares & Guest Dropboxes** | 1. Right-click folder $\rightarrow$ Create Public Share link with password & expiry.<br>2. Open share URL in incognito tab $\rightarrow$ test guest file download and upload dropzone. | [ ] | [ ] | [ ] |
| **DSK-19** | **Session Lock & Inactivity Lock** | 1. Press <kbd>Ctrl+Alt+L</kbd> $\rightarrow$ lock screen appears with user avatar.<br>2. Enter password/PIN to unlock.<br>3. Verify 15-minute inactivity timer locks session automatically. | [ ] | [ ] | [ ] |

---

## 🖱️ SECTION 4: Input Modality Comparison Matrix (Mouse/Keeb vs Touch)

| Functional Action | 🖱️ Mouse & Keyboard Interaction | 👆 Touch & Stylus Interaction | Verified Behavior |
| :--- | :--- | :--- | :--- |
| **Select Single Item** | Left-Click row | Tap row (in select mode) or Tap checkbox | Item highlights, selection counter updates |
| **Range Multi-Select** | <kbd>Shift</kbd> + Click or <kbd>Shift</kbd> + <kbd>↑</kbd>/<kbd>↓</kbd> | Tap `+ Select` on bar $\rightarrow$ Tap start & end items | Continuous block of files selected |
| **Discrete Multi-Select** | <kbd>Ctrl</kbd> + Click or <kbd>Space</kbd> / <kbd>Insert</kbd> | Tap checkboxes on individual rows | Multiple individual files selected |
| **Open Folder / File** | Double-Click or <kbd>Enter</kbd> | Short Tap on row name | Folder navigates; file opens viewer |
| **Context Menu** | Right-Click row | Long-Press (500ms) or Tap `⚡ Actions` bar | Desktop: Flyout menu<br>Mobile: Bottom sheet |
| **Submenu Navigation** | Hover mouse / Arrow right | Tap item to expand vertical accordion | Sub-items reveal without horizontal scroll |
| **Pane Focus Switch** | Click inside pane or press <kbd>Tab</kbd> | Tap header or swipe horizontal indicator | Active pane indicator and border shift |
| **Copy / Move to Target** | Press <kbd>F5</kbd>/<kbd>F6</kbd> or Drag & Drop | Tap `📋 Copy` on bar $\rightarrow$ switch pane $\rightarrow$ `Paste` | Operation queues in Task Manager |
| **Zoom Media View** | Mouse scroll wheel or <kbd>+</kbd>/<kbd>-</kbd> | Pinch-to-zoom or double-tap | Focal zoom centered on cursor / pinch |
| **Browse Media Folder** | Scroll wheel up/down | Swipe left / right | Preloads adjacent images smoothly |
| **Resize Columns** | Click & drag column header divider | Auto-fitted to mobile screen width | Desktop: Custom width<br>Mobile: Auto-fit |

---

## 🚦 Acceptance Sign-off Matrix

| Platform / Form Factor | Browser Tested | Tests Passed | Tester Initials | Date Completed | Release Decision |
| :--- | :--- | :---: | :---: | :---: | :---: |
| 📱 **Phone ($\le 480\text{px}$)** | **Vivaldi Mobile** | _____ / 12 | | | `[ ] PASS` / `[ ] FAIL` |
| 📱 **Phone ($\le 480\text{px}$)** | **Firefox Mobile** | _____ / 12 | | | `[ ] PASS` / `[ ] FAIL` |
| 📖 **Foldable / Tablet ($601-1024\text{px}$)** | **Vivaldi (Unfolded)** | _____ / 8 | | | `[ ] PASS` / `[ ] FAIL` |
| 📖 **Foldable / Tablet ($601-1024\text{px}$)** | **Firefox (Unfolded)** | _____ / 8 | | | `[ ] PASS` / `[ ] FAIL` |
| 💻 **Desktop ($\ge 1025\text{px}$)** | **Vivaldi Desktop** | _____ / 19 | | | `[ ] PASS` / `[ ] FAIL` |
| 💻 **Desktop ($\ge 1025\text{px}$)** | **Firefox Desktop** | _____ / 19 | | | `[ ] PASS` / `[ ] FAIL` |

### 🏁 Final Release Decision:
- [x] RELEASE CANDIDATE APPROVED (v0.7.0 Ready for General Availability)
- [ ] BLOCKED (Remediation Required for Reported Issues)
