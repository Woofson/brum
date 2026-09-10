# 🐕 Woofson Design Specs — Golden Amber Palette

The official unified color palette and design tokens for the **Woofson** suite of applications (`Brum`, `NoteDog`, `DotDog`).

---

## 🎨 Color Palette Tokens

### Charcoal (Dark Theme)

| Token Name | Hex Code | RGB | Usage |
| :--- | :--- | :--- | :--- |
| **`bg-void`** | `#121214` | `18, 18, 20` | Main canvas background, deeply recessed areas |
| **`bg-panel`** | `#18181b` | `24, 24, 27` | Panes, cards, sidebars, modal surfaces |
| **`bg-header`** | `#202024` | `32, 32, 36` | Toolbar headers, breadcrumbs bar, status bars |
| **`bg-active`** | `#27272a` | `39, 39, 42` | Focused pane headers, active tabs |
| **`bg-hover`** | `#323238` | `50, 50, 56` | Interactive element hover states |
| **`bg-selected`** | `rgba(245, 158, 11, 0.18)` | — | Selected table rows, highlight overlays |
| **`accent-core`** | `#f59e0b` | `245, 158, 11` | Primary brand accent (Amber 500), focus rings, badges |
| **`accent-hover`** | `#fbbf24` | `251, 191, 36` | Button hover, bright amber highlights, folder icons |
| **`accent-dark`** | `#d97706` | `217, 119, 6` | Pressed states, active borders |
| **`border-subtle`**| `#3f3f46` | `63, 63, 70` | Default panel and table borders (Zinc 700) |
| **`border-focus`** | `#f59e0b` | `245, 158, 11` | Active focused pane border, input focus |
| **`text-main`** | `#f4f4f5` | `244, 244, 245` | High-contrast body text, file names, headings |
| **`text-muted`** | `#a1a1aa` | `161, 161, 170` | Secondary metadata (sizes, dates, perms) |
| **`text-dim`** | `#71717a` | `113, 113, 122` | Gutters, separators, placeholders |

### Zink (Light Theme)

| Token Name | Hex Code | RGB | Usage |
| :--- | :--- | :--- | :--- |
| **`bg-void`** | `#fafafa` | `250, 250, 250` | Main canvas background (Zinc 50) |
| **`bg-panel`** | `#ffffff` | `255, 255, 255` | Panes, cards, sidebars, modal surfaces |
| **`bg-header`** | `#f4f4f5` | `244, 244, 245` | Toolbar headers, breadcrumbs bar (Zinc 100) |
| **`bg-active`** | `#e4e4e7` | `228, 228, 231` | Focused pane headers, active tabs (Zinc 200) |
| **`bg-hover`** | `#ebecee` | `235, 236, 238` | Interactive row & button hover states |
| **`bg-selected`** | `rgba(245, 158, 11, 0.14)` | — | Selected table rows, highlight overlays |
| **`accent-core`** | `#d97706` | `217, 119, 6` | Primary brand accent (Amber 600) |
| **`accent-hover`** | `#b45309` | `180, 83, 9` | Button hover, pressed states (Amber 700) |
| **`accent-dark`** | `#92400e` | `146, 64, 14` | Deep contrast accent (Amber 800) |
| **`border-subtle`**| `#d4d4d8` | `212, 212, 216` | Default borders (Zinc 300) |
| **`border-focus`** | `#d97706` | `217, 119, 6` | Active pane border, input focus ring |
| **`text-main`** | `#18181b` | `24, 24, 27` | High-contrast body text (Zinc 900) |
| **`text-muted`** | `#52525b` | `82, 82, 91` | Secondary metadata (Zinc 600) |
| **`text-dim`** | `#71717a` | `113, 113, 122` | Gutters, separators, placeholders (Zinc 500) |

---

## 🚦 Functional Semantic Tokens

- **Success / Identical / Verified**: `#10b981` (Dark) / `#059669` (Zink)
- **Danger / Deleted / Overwrite**: `#ef4444` (Dark) / `#dc2626` (Zink)
- **Information / Remote SFTP / Symlink**: `#38bdf8` (Dark) / `#0284c7` (Zink)
- **Archive / Compressed**: `#f472b6` (Dark) / `#db2777` (Zink)
- **Directories**: `#fbbf24` (Amber 400)
- **Code / Executable**: `#34d399` (Emerald 400)

---

## 💻 CSS Variables Definition

### Amber Charcoal
:root[data-theme="charcoal"],
:root {
  --bg-dark: #121214;
  --bg-panel: #18181b;
  --bg-header: #202024;
  --bg-active: #27272a;
  --bg-hover: #323238;
  --bg-selected: rgba(245, 158, 11, 0.18);
  --border: #3f3f46;
  --border-focus: #f59e0b;
  --accent: #f59e0b;
  --accent-hover: #fbbf24;
  --accent-dark: #d97706;
  --accent-subtle: rgba(245, 158, 11, 0.12);
  --text-main: #f4f4f5;
  --text-muted: #a1a1aa;
  --text-dim: #71717a;
  --danger: #ef4444;
  --success: #10b981;              
  --info: #38bdf8;
  --archive: #f472b6;
  --font-mono: 'JetBrains Mono', 'Fira Code', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;               
  --font-sans: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
  --radius: 6px;
  --pane-border-width: 1px;
  --pane-active-ring-width: 1px;
  --shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
}

### Amber Zink
:root[data-theme="zink"] {
  --bg-dark: #fafafa;
  --bg-panel: #ffffff;
  --bg-header: #f4f4f5;
  --bg-active: #e4e4e7;
  --bg-hover: #ebecee;                                                                                                                     
  --bg-selected: rgba(245, 158, 11, 0.14);
  --border: #d4d4d8;
  --border-focus: #d97706;
  --accent: #d97706;
  --accent-hover: #b45309;
  --accent-dark: #92400e;
  --accent-subtle: rgba(245, 158, 11, 0.10);
  --text-main: #18181b;
  --text-muted: #52525b;
  --text-dim: #71717a;
  --danger: #dc2626;
  --success: #059669;              
  --info: #0284c7;
  --archive: #db2777;
  --font-mono: 'JetBrains Mono', 'Fira Code', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;               
  --font-sans: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
  --radius: 6px;
  --pane-border-width: 1px;
  --pane-active-ring-width: 1px;
  --shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
}
