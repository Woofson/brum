# Keyboard Shortcuts & Navigation Guide

Brum provides orthodox keyboard-driven power navigation paired with intuitive mouse and touch gestures.

---

## Primary Function Keys (<kbd>F1</kbd> – <kbd>F12</kbd>)

| Key | Action | Description |
| :--- | :--- | :--- |
| **`Tab`** / **`Shift+Tab`** | **Switch Active Pane** | Cycle keyboard focus between open panels |
| **`F1`** | **Help & Shortcuts** | Open interactive keyboard shortcut reference modal |
| **`F2`** | **Rename Entry** | Quick inline or modal rename for the selected file or folder |
| **`F3`** | **Quick View / Media Viewer** | Open High-DPI Image Viewer, Video/Audio Player, 3D CAD Studio, or Document Viewer |
| **`F4`** | **Dual-Pane Editor** | Open built-in code and Markdown editor |
| **`F5`** | **Copy / DeltaCopy** | Transfer selected items to the destination panel with delta skipping |
| **`F6`** | **Move File(s)** | Move selected items to the destination panel |
| **`Shift+F6`** / **`Ctrl+M`** | **Bulk Renamer** | Open advanced regex, numbering, case-conversion bulk renamer |
| **`F7`** | **New Folder** | Create a new directory in the active path |
| **`Shift+F7`** | **Calculate Checksums** | Calculate and verify MD5, SHA-1, and SHA-256 hashes |
| **`F8`** / **`Delete`** | **Move to Trash** | Safely move selected items to Linux XDG Trash |
| **`Shift+F8`** / **`Shift+Delete`** | **Permanent Delete** | Irreversibly delete selected files/folders immediately (bypassing Trash) |
| **`F9`** / **`Ctrl+D`** | **Compare** | Compare the selected files, or the folders in your two panels |
| **`F10`** | **Settings Hub** | Open Settings modal (General, Desktop, Themes, Bookmarks, Users, Chewtoys, Keybinds) |
| **`F11`** | **Fullscreen Toggle** | Toggle browser fullscreen workspace |
| **`Alt+Enter`** | **Properties Dialog** | Open comprehensive 7-tab file/folder properties, permissions, and checksums dialog |
| **`Escape`** | **Close Modal / Window** | Dismiss active modal dialog, spotlight palette, or floating window |

---

## Navigation, Layouts & Selection

| Shortcut | Action |
| :--- | :--- |
| **`Alt+1`** .. **`Alt+4`** | Jump focus directly to Panel 1, 2, 3, or 4 |
| **`Ctrl+Alt+1`** | Single Panel Layout |
| **`Ctrl+Alt+2`** | Dual Vertical Split Layout (Orthodox Commander) |
| **`Ctrl+Alt+3`** | Dual Horizontal Split Layout |
| **`Ctrl+Alt+4`** | Quad 2x2 Grid Layout |
| **`Ctrl+K`** / **`Ctrl+F`** | Spotlight Global Search & Action Palette |
| **`Ctrl+B`** | Toggle Flat / Branch View (recursive directory flattening) |
| **`Ctrl+T`** | Toggle Sidebar Directory Tree |
| **<kbd>Ctrl+`</kbd>** | Toggle Slide-Up Native Web Terminal (PTY) |
| **`Space`** / **`Insert`** | Toggle item selection and move cursor down |
| **`Ctrl+A`** | Select all items in active panel |
| **`Ctrl+Shift+A`** | Invert selection |
| **`Ctrl+C`** / **`Ctrl+X`** / **`Ctrl+V`** | Standard Clipboard Copy, Cut, and Paste across panels |
| **`Ctrl+P`** | Toggle Paranoid File Handling Mode (checksum verification) |
| **`Ctrl+Alt+L`** | Lock Session (Enter PIN/Password to restore) |
| **`Ctrl+Q`** / **`Cmd+Q`** | Clean Exit / Quit Brum |

---

## Core Functions & Power Tools

| Shortcut | Action |
| :--- | :--- |
| **`Ctrl+Shift+H`** | Open Hex Editor |
| **`Ctrl+Shift+N`** | Open Notes Studio |
| **`Ctrl+Shift+D`** | Open Disk Usage Analyzer |
| **`Ctrl+Shift+P`** | Open PDF Studio |
| **`Ctrl+Shift+C`** | Open Format Converter (ConvertX) |

---

## Media Player & Synthesizer Shortcuts

| Key | Action |
| :--- | :--- |
| **`Z`** / **`B`** | Previous Track / Next Track |
| **`X`** / **`C`** / **`V`** | Play / Pause / Stop |
| **`L`** | Open Audio Files / Playlist File Picker |
| **`Alt+W`** | Windowshade Mode (collapses to mini-bar) |
| **`Alt+G`** | Toggle 10-Band Equalizer Window |
| **`Alt+E`** | Toggle Playlist Editor Window |
| **`S`** / **`R`** | Toggle Shuffle / Toggle Repeat |
| **`←`** / **`→`** | Seek Backward / Forward ±5 seconds |
| **`↑`** / **`↓`** | Volume ±5% |

---

## Mouse & Touch Navigation

* **Double-Click Empty Space**: Double-click anywhere on empty panel background space to immediately jump up one directory level (`..`).
* **Right-Click Menu**: Right-clicking a folder shows Open, Open in New Tab, the other pane and the terminal first; a file shows Open With, Quick View and Edit first, plus tools for its kind (PDF Studio, media player, 3D CAD Studio, browsing inside archives and disk images, vaults). Then come Cut, Copy, Paste, Copy To, Move To and Duplicate; Rename and Delete; Download, Share and Archives; Tools, Custom Actions and New; and Properties. Paste only shows when something was copied. **Settings → Menus & Shortcuts → Right-Click Menu** switches items on or off and changes their order for folders and for files; with several items selected, the file list's switches apply.
* **Duplicate (Right-Click)**: Right-click a file or folder and choose **Duplicate** to make a copy next to it, named like `report (copy).pdf` (`report (copy 2).pdf` when that name is taken). For one item the copy is selected with its name ready to edit; press Enter to keep a new name or Esc to keep the suggested one. With several items selected, each gets its own copy. Available for files and folders on the machine's own disks; for cloud storage, SFTP, SMB and other machines use Copy and Paste.
* **Empty Space Right-Click**: Context menu to create files from templates, create directories, paste clipboard items, open terminal, or view disk usage.
* **Synchronized Scrolling & Directory Navigation**: Accessible directly from individual panel dropdown menus to mirror scrolling offsets and directory navigation across opposite panes.
* **Phones and Tablets**: Long-press a file or folder, or tap **Actions** in the bottom bar, for the same menu as a right-click (with nothing selected, Actions shows the menu for the folder itself). Touch tablets get the same bottom bar as phones instead of the F1–F10 keys; keyboard hints are left out of menus on touch screens. Swipe between panes.
