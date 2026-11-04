# Chewtoy & Plugin Development Guide (.grr)

> Complete manual for developing, styling, sandboxing, and distributing modular **Chewtoys** (extensions) for **Brum** using the **`.grr`** package standard.

---

## 1. Overview & Architecture

Brum's modular architecture cleanly separates user-facing utilities and backend execution:

```mermaid
flowchart TD
    subgraph Package [1. Package Bundle]
        GRR[".grr Archive (ZIP)"] --> TOML["plugin.toml (Manifest)"]
        GRR --> UI["index.html + style.css + main.js"]
        GRR --> BE["backend/ (Optional Shell / Wasm / Hooks)"]
    end

    subgraph Backend [2. Rust Backend Engine (Plugins)]
        PM["PluginManager (src/plugins/)"] --> RBAC["Admin RBAC & Policy Engine"]
        RBAC --> SEC["Whitelist / Blacklist / Permissions Filter"]
        PM --> SVR["Asset Server & Script Runner (/api/plugins/*)"]
    end

    subgraph Frontend [3. Frontend Presentation (Chewtoys)]
        UI_HOST["Chewtoy Host & window.Brum SDK"] --> DUAL["Dual-Mode (Floating & In-Pane Dock)"]
        UI_HOST --> SETTINGS["Settings (F10) ➔ Chewtoys & Extensions"]
    end

    Package --> Backend
    Backend --> Frontend
```

* **Backend / System Terminology**: Strictly termed **`Plugins`** (`src/plugins/`, `PluginManager`, `plugin.toml`, `/api/plugins`).
* **Frontend / User Terminology**: Presented to end users everywhere as **`Chewtoys`** (*"Chewtoys & Extensions"*, *"Install Chewtoy (.grr)"*).

---

## 2. The `.grr` Package Structure

A Chewtoy is distributed as a **`.grr`** file (a standard ZIP archive). You can inspect and unpack it with any standard ZIP tool:

```
my-custom-chewtoy.grr/
├── plugin.toml          # Required: Manifest, metadata, UI dimensions, permissions
├── assets/
│   └── icon.svg         # Required: 24x24 / 48x48 Chewtoy vector icon (or .webp / .png)
├── index.html           # Required: Main UI markup template
├── style.css            # Scoped CSS styling (inheriting Brum theme tokens)
├── main.js              # Client logic (using window.Brum SDK)
└── backend/             # (Optional) Server-side hooks or scripts
    └── run.sh           # Backend runner hook
```

---

## 3. The `plugin.toml` Manifest Specification

The `plugin.toml` manifest file defines everything Brum needs to load, render, and sandbox your Chewtoy:

```toml
[plugin]
id = "custom-calculator"
name = "Scientific Calculator"
version = "1.0.0"
author = "Bolt J Woofson <bolt@arf.ac>"
description = "Scientific calculator and unit conversion tool for engineering workflows."
homepage = "https://github.com/Woofson/calculator"
icon = "assets/icon.svg"
category = "utilities"   # utilities | media | development | games | system

[ui]
modes = ["floating", "docked"]
default_mode = "floating"
default_width = 860
default_height = 580
min_width = 420
min_height = 300

[integrations]
# Tools & Chewtoys Launchpad menu entry
launchpad = true
launchpad_label = "Calculator"

# File Context Menu integration in file panels (Optional)
file_extensions = ["*.calc", "*.math"]
context_menu_label = "Open in Calculator"

# Global Shortcut (Optional)
shortcut = "Ctrl+Shift+K"

[permissions]
# Sandboxed capabilities declared by the plugin
permissions = [
    "fs:read",       # Read files in active panels
    "fs:write",      # Save/modify files
    "ui:notify",     # Send toast notifications
    "pty:exec"       # Run backend scripts (Admin approval required)
]

[backend]
# Optional backend script or binary
entrypoint = "backend/run.sh"
script_type = "shell"
```

---

## 4. Designing Compliant Chewtoys (UI/UX Specification)

To ensure a seamless, native feel with Brum's orthodox commander interface, all Chewtoys should follow the window design specifications:

### 1. Primary Window Header (`42px` Min-Height)
* **Drag Handle**: The entire header bar must serve as a non-fiddly drag handle (`cursor: grab;` with `:active { cursor: grabbing; }`).
* **Branding**: Include the Chewtoy icon (`16x16`) and bold title in amber accent color (`var(--accent)`).

### 2. Standard Buttons & Controls (`28px` Height)
* All header buttons, selects, and action controls must share a uniform **`28px` height** with `border-radius: var(--radius)` (`6px`).
* Action buttons use `font-size: 12px; font-weight: 600;`.

### 3. Official Woofsons Amber Design Tokens
Always use CSS variables so your Chewtoy automatically adapts when the user switches themes:

```css
.my-chewtoy-panel {
  background: var(--bg-panel, #18181b);
  color: var(--text-main, #f4f4f5);
  border: 1px solid var(--border, #3f3f46);
  border-radius: var(--radius, 6px);
}

.my-chewtoy-btn-primary {
  background: var(--accent, #f59e0b);
  color: #121214;
}

.my-chewtoy-btn-primary:hover {
  background: var(--accent-hover, #fbbf24);
}
```

---

## 5. The `window.Brum` JavaScript SDK

### Sandbox and the message bridge
Chewtoys run in a sandboxed frame with no origin of their own. Inside it:

* `window.parent.Brum` and other properties of Brum's page cannot be read.
* `localStorage`, `sessionStorage`, `indexedDB` and cookies are unavailable; touching them throws and stops your script.
* The browser clipboard and the EyeDropper API are unavailable.

Everything goes through `postMessage`. Send `{ type: "BRUM_REQ", reqId, action, payload }` to `window.parent`; Brum answers with `{ type: "BRUM_RES", reqId, result, error }`. Shortly after your page loads, Brum sends `{ type: "BRUM_READY", context }`. The starter template (section 8) wraps this in a small `window.Brum` client, and the examples below use that client.

`context` includes `activePath`, `selectedFiles`, `theme`, `isDocked`, `viewport`, `storage` (your saved values, see E), `capabilities` (`{ eyedropper, clipboard }`) and `region` (the user's formats, below).

`context.region` holds the user's choices from **Settings → General → Region & Formats**, already resolved (never "follow my browser"):

| Key | Values |
| --- | --- |
| `time` | `"24"` or `"12"` |
| `date` | `"iso"` (2026-11-04), `"dmy_dot"` (04.11.2026), `"dmy_slash"` (04/11/2026), `"mdy"` (11/04/2026), `"d_mon_y"` (4 Nov 2026), `"auto"` (the browser's style) |
| `week_start` | `"mon"`, `"sun"` or `"sat"` |
| `number` | `"auto"`, `"space_comma"` (1 234,5), `"dot_comma"` (1.234,5), `"comma_dot"` (1,234.5), `"space_dot"` (1 234.5), `"apostrophe_dot"` (1’234.5) |
| `units` | `"metric"` or `"imperial"` |
| `temperature` | `"c"` or `"f"` |
| `sizes` | `"classic"` (KB, 1024), `"iec"` (KiB, 1024), `"si"` (kB, 1000) |
| `language` | the browser's language, e.g. `"nb-NO"` |
| `sample_time`, `sample_date` | ready-made examples ("14:30", "2026-11-04") showing the chosen style |

A weather Chewtoy, for example, shows °C and km/h when `units` is `"metric"`.

### A. Lifecycle & Context
```javascript
window.Brum.onReady((context) => {
  console.log("Chewtoy initialized!");
  console.log("Current active path:", context.activePath);
  console.log("Selected files in panel:", context.selectedFiles);
  console.log("Panel ID:", context.panelId); // 1 or 2
});
```

### B. Filesystem API (`Brum.fs`)
```javascript
// Read text file
const text = await Brum.fs.readFile("/home/user/document.txt");

// Read binary file (returns ArrayBuffer or Base64)
const binaryData = await Brum.fs.readFile("/home/user/firmware.bin", { binary: true });

// Write file
await Brum.fs.writeFile("/home/user/output.txt", "Hello from Chewtoy!");

// List directory
const listing = await Brum.fs.listDir("/home/user/projects");
```

### C. UI & Notifications (`Brum.ui`)
```javascript
// Send toast notifications
Brum.ui.notify("Saved 1,024 bytes!", { type: "success" }); // "info" | "success" | "warning" | "error"

// Query current theme
const theme = Brum.ui.getTheme(); // "amber-charcoal" | "zink" | "skumring" ...
```

### D. Window & Dock Controls (`Brum.window`)
```javascript
// Dock to Left Panel (Panel 1) or Right Panel (Panel 2)
Brum.window.dockTo(1);

// Float as draggable window
Brum.window.float();

// Close window
Brum.window.close();
```

### E. Storage (`storage.*`, no permission needed)
Each Chewtoy gets its own small key/value store (256 KB, strings only), kept by Brum in the user's browser. No Chewtoy can read another's store. Saved values arrive in `context.storage` with `BRUM_READY`, so read them there at start-up and send changes with:

| Action | Payload | Result |
| :--- | :--- | :--- |
| `storage.set` | `{ key, value }` | `true` |
| `storage.get` | `{ key }` | the value or `null` |
| `storage.remove` | `{ key }` | `true` |
| `storage.getAll` | none | object with all values |
| `storage.clear` | none | `true` |

### F. Clipboard, eyedropper and theme colours
| Action | Permission | Payload | Result |
| :--- | :--- | :--- | :--- |
| `clipboard.writeText` | `clipboard:write` | `{ text }` | `true`, or an error if the browser refused |
| `ui.pickColor` | `ui:eyedropper` | none | `"#rrggbb"`, or `null` if cancelled. Check `context.capabilities.eyedropper` first; only Chromium-based browsers have one |
| `ui.getTheme` | none | none | `{ id, isDark, accent, palette }`, where `palette` maps theme variables such as `--accent` to colours |

### G. Leaderboards (`scores` permission)
`scores.get` (`{ mode, userOnly, limit }`), `scores.submit` (`{ score, lines_cleared, level, duration_seconds, mode, player_name }`) and `scores.clear` use the signed-in user's account. They currently serve the Tetrion leaderboard.

---

## 6. Admin Governance, RBAC & Whitelisting

Admins have complete operational control over which Chewtoys standard users can see, install, or run.

### A. Server Configuration (`config.toml`)
```toml
[plugins]
enabled = true
directory = "/etc/brum/plugins"       # Global system plugin directory
user_directory = "/data/plugins"      # User-installed plugin directory
allow_user_installs = false           # If false, only Admins can install .grr packages
default_policy = "allow_all"          # "allow_all" | "whitelist" | "blacklist"
global_whitelist = ["*"]              # Global allowed plugin IDs
global_blacklist = ["unapproved-app"] # Global blocked plugin IDs
```

### B. User-Level Whitelist & Blacklist Overrides
In **Settings (<kbd>F10</kbd>) ➔ Users Tab**, administrators can configure per-user overrides:
* **`can_install_plugins`**: Enable/disable personal `.grr` installation for this user.
* **`allowed_plugins`**: List of whitelisted plugin IDs (e.g. `["calculator", "tetrion"]` or `["*"]`).
* **`blocked_plugins`**: List of blacklisted plugin IDs (e.g. `["games-*"]`).

---

## 7. Packaging Your Chewtoy into `.grr`

To package your Chewtoy directory for distribution:

### Using Standard ZIP CLI
Inside your Chewtoy source directory:
```bash
# Compress all files into .grr archive
zip -r ../my-chewtoy.grr plugin.toml index.html style.css main.js assets/
```

### Installing Your `.grr` Package
1. Open Brum in your browser.
2. Open **Settings (<kbd>F10</kbd>) ➔ "Chewtoys & Extensions"** tab.
3. Drag and drop `my-chewtoy.grr` into the installer card.
4. Your Chewtoy is instantly available in the Tools Launchpad and file context menus!

---

## 8. Starter Boilerplate Template

A complete, working starter template is provided in the repository:
📁 [**`examples/starter-chewtoy/`**](../examples/starter-chewtoy/)

```bash
# Clone or copy the starter template:
cp -r examples/starter-chewtoy my-chewtoy
cd my-chewtoy
# Edit plugin.toml, index.html, main.js, and package into .grr!
```

---

## License & Community
MIT License © [Bolt J Woofson](https://www.arf.ac) @ Woofsons Lab.
