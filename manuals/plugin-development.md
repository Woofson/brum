# 🧩 ChewToy & Plugin Development Guide (.grr)

> Complete manual for developing, styling, sandboxing, and distributing modular **ChewToys** (extensions) for **Brum** using the **`.grr`** package standard.

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

    subgraph Frontend [3. Frontend Presentation (ChewToys)]
        UI_HOST["ChewToy Host & window.Brum SDK"] --> DUAL["Dual-Mode (Floating & In-Pane Dock)"]
        UI_HOST --> SETTINGS["Settings (F10) ➔ ChewToys & Extensions"]
    end

    Package --> Backend
    Backend --> Frontend
```

* **Backend / System Terminology**: Strictly termed **`Plugins`** (`src/plugins/`, `PluginManager`, `plugin.toml`, `/api/plugins`).
* **Frontend / User Terminology**: Presented to end users everywhere as **`ChewToys`** (*"ChewToys & Extensions"*, *"Install ChewToy (.grr)"*).

---

## 2. The `.grr` Package Structure

A ChewToy is distributed as a **`.grr`** file (a standard ZIP archive). You can inspect and unpack it with any standard ZIP tool:

```
my-custom-chewtoy.grr/
├── plugin.toml          # Required: Manifest, metadata, UI dimensions, permissions
├── assets/
│   └── icon.svg         # Required: 24x24 / 48x48 ChewToy vector icon (or .webp / .png)
├── index.html           # Required: Main UI markup template
├── style.css            # Scoped CSS styling (inheriting Brum theme tokens)
├── main.js              # Client logic (using window.Brum SDK)
└── backend/             # (Optional) Server-side hooks or scripts
    └── run.sh           # Backend runner hook
```

---

## 3. The `plugin.toml` Manifest Specification

The `plugin.toml` manifest file defines everything Brum needs to load, render, and sandbox your ChewToy:

```toml
[plugin]
id = "hexdog"
name = "HexDog Hexadecimal Studio"
version = "1.0.0"
author = "Bolt J Woofson <bolt@arf.ac>"
description = "High-performance hex viewer, binary inspector, and byte patcher."
homepage = "https://github.com/Woofson/hexdog"
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
# Tools & ChewToys Launchpad menu entry
launchpad = true
launchpad_label = "Hex Studio"

# File Context Menu integration in file panels
file_extensions = ["*.bin", "*.dat", "*.so", "*.dll", "*.exe", "*.iso", "*.img", "*.rom"]
context_menu_label = "Open in HexDog"

# Global Shortcut (Optional)
shortcut = "Ctrl+Shift+H"

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

## 4. Designing Compliant ChewToys (UI/UX Specification)

To ensure a seamless, native feel with Brum's orthodox commander interface, all ChewToys should follow **Rule 9 (ChewToy Design Language)**:

### 1. Primary Window Header (`42px` Min-Height)
* **Drag Handle**: The entire header bar must serve as a non-fiddly drag handle (`cursor: grab;` with `:active { cursor: grabbing; }`).
* **Branding**: Include the ChewToy icon (`16x16`) and bold title in amber accent color (`var(--accent)`).

### 2. Standard Buttons & Controls (`28px` Height)
* All header buttons, selects, and action controls must share a uniform **`28px` height** with `border-radius: var(--radius)` (`6px`).
* Action buttons use `font-size: 12px; font-weight: 600;`.

### 3. Official Woofsons Amber Design Tokens
Always use CSS variables so your ChewToy automatically adapts when the user switches themes:

```css
.my-chewtoy-panel {
  background: var(--bg-panel, #1e1e24);
  color: var(--text-main, #e6dede);
  border: 1px solid var(--border, #2b2b36);
  border-radius: var(--radius, 6px);
}

.my-chewtoy-btn-primary {
  background: var(--accent, #f59e0b);
  color: #121214;
}

.my-chewtoy-btn-primary:hover {
  background: var(--accent-hover, #d97706);
}
```

---

## 5. The `window.Brum` JavaScript SDK

Brum automatically injects the `window.Brum` SDK into every loaded ChewToy:

### A. Lifecycle & Context
```javascript
window.Brum.onReady((context) => {
  console.log("ChewToy initialized!");
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
await Brum.fs.writeFile("/home/user/output.txt", "Hello from ChewToy!");

// List directory
const listing = await Brum.fs.listDir("/home/user/projects");
```

### C. UI & Notifications (`Brum.ui`)
```javascript
// Send toast notifications
Brum.ui.notify("Saved 1,024 bytes!", { type: "success" }); // "info" | "success" | "warning" | "error"

// Query current theme
const theme = Brum.ui.getTheme(); // "amber-charcoal" | "zink" | "emerald" ...
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

---

## 6. Admin Governance, RBAC & Whitelisting

Admins have complete operational control over which ChewToys standard users can see, install, or run.

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
* **`allowed_plugins`**: List of whitelisted plugin IDs (e.g. `["hexdog", "notedog"]` or `["*"]`).
* **`blocked_plugins`**: List of blacklisted plugin IDs (e.g. `["games-*"]`).

---

## 7. Packaging Your ChewToy into `.grr`

To package your ChewToy directory for distribution:

### Using Standard ZIP CLI
Inside your ChewToy source directory:
```bash
# Compress all files into .grr archive
zip -r ../my-chewtoy.grr plugin.toml index.html style.css main.js assets/
```

### Installing Your `.grr` Package
1. Open Brum in your browser.
2. Open **Settings (<kbd>F10</kbd>) ➔ "ChewToys & Extensions"** tab.
3. Drag and drop `my-chewtoy.grr` into the installer card.
4. Your ChewToy is instantly available in the Tools Launchpad and file context menus!

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
