# CommanderDog Project Rules & Development Guidelines

Welcome to the **CommanderDog** project repository. All agents and pair-programming sessions must strictly adhere to the rules and conventions below.

---

## 1. Versioning & Iteration Rules

* **Iteration Release Candidates (`rc1-rc9`)**: Every time changes/iterations are made to the codebase between official releases, bump the release candidate postfix using SemVer `rc1` through `rc9` (e.g. `0.7.1-rc1`, `0.7.1-rc2`, `0.7.1-rc3`...) in `Cargo.toml`. When an official milestone release is finalized, drop the `-rc` suffix (e.g. `0.7.1`).
* **Centralized Single Source of Truth**:
  * The version number is defined exclusively in `Cargo.toml` (`[package] version = "..."`).
  * The Rust backend automatically embeds this compile-time via `env!("CARGO_PKG_VERSION")` and serves it via `/api/config` and `/api/system/status`.
  * The frontend dynamically populates all UI badges (`#about-version-badge`, `.login-version-badge`, `#sync-version-badge`) through `applyAppVersion()`.
  * **Token Efficiency**: Do NOT burn tokens editing hardcoded version strings across multiple HTML/JS files; updating `Cargo.toml` updates everything.

---

## 2. Rebuild vs Refresh Notifications

At the end of every response after making changes, **always clearly inform the user** whether:
1. **Backend Recompile Needed**: When Rust code (`src/**/*.rs`), dependencies, or assets embedded into the binary have changed. Instruct the user to restart/rebuild with `cargo run` or `cargo build`.
2. **Browser Refresh Only Needed**: When changes were purely frontend or configuration that does not require binary recompilation. Instruct the user to perform a hard refresh (Ctrl+F5 / Cmd+Shift+R).

---

## 3. Git Workflow, GitHub Issues & Roadmap Synchronization

* **GitHub Issues as Single Source of Truth for Roadmap**:
  * Treat GitHub Issues on `Woofson/brum` as the exclusive single source of truth for the roadmap, active backlog, and sprint tracking.
  * Regularly query, check, and update issues using `scripts/issues.sh` or the GitHub CLI (`gh issue list`, `gh issue view`, `gh issue create`, `gh issue comment`, `gh issue close`).
  * When implementing features or fixing bugs, cross-reference the corresponding issue number in commit messages (e.g. `fix(notedog): resolve dropdown lag in docked mode (fixes #1)`).
* **Documentation & Changelog CLI Tooling**:
  * Use `scripts/changelog.sh` to compile changelog notes directly to stdout (`./scripts/changelog.sh` or `./scripts/changelog.sh git`).
  * Before every git commit or push, ensure that `CHANGELOG.md` has the iteration entry and `Cargo.toml` is bumped.
* **Command: `"push to git"`**:
  * Commit modified files with a clean, conventional commit message and push to the remote Git repository.
  * **DO NOT trigger releases or publish packages.**
* **Command: `"push and release"`**:
  * Commit, tag, push, and execute release automation across **ALL** distribution channels via `scripts/release.sh`:
    1. **GitHub Release & Tags**: Tagged release on GitHub.
    2. **Arch Linux AUR**: `commanderdog` (source) and `commanderdog-bin` PKGBUILDs.
    3. **Docker / GHCR**: `ghcr.io/woofson/commanderdog`.
    4. **Crates.io**: `cargo publish` using the `arf-` prefix rule (`arf-cmdr`).
    5. **Windows**: Scoop bucket and Winget package manifests (`packaging/windows/`).

---

## 4. Professional Tone & Minimal Emoji Policy

* **Minimal Emotes**: Keep emojis and emotes to a strict minimum in all responses, documentation, commit messages, and UI text.
* **Professional Engineering Tone**: Maintain a sharp, direct, high-signal tone with subtle hints of brand identity where appropriate, avoiding over-the-top emoji spam in headings and bullet points.

---

## 5. Brand & Theme Nomenclature

* **Creator & Lab**: Bolt J Woofson @ Woofsons Lab ([www.arf.ac](https://www.arf.ac)).
* **Publishing Prefix Rule**: All publishing packages, binaries, and crates must use the `arf-` or `arf_` prefix (e.g., `arf-cmdr` for CommanderDog/Shunt, `arf-remote` for RemoteDog).
* **Official Theme Names**:
  * **Dark Theme**: Strictly named **`Woofsons Amber Charcoal`** (ID: `amber-charcoal` / `charcoal`).
  * **Light Theme**: Strictly named **`Woofsons Amber Zink`** (ID: `zink`).

---

## 6. Viewport & UI Terminology

* **Viewport Terms**:
  * `Phone`: Mobile touch screens (`<600px`). Requires single-pane focus, hidden branding badges, and minimal micro-text.
  * `Tablet`: Foldables and tablet touch screens (`600px–1024px`). Requires adaptive single/dual panel options, sliding note drawers, and legible touch hierarchy.
  * `PC`: Desktop & laptop mouse & keyboard (`>1024px`). Full multi-panel, resizable columns, and dockable ChewToys.
* **Panels vs Tabs**:
  * File browsing areas are strictly termed **"Panels"** (e.g., Left Panel, Right Panel, Pane 1, Pane 2).
  * The word **"Tabs"** is strictly reserved for Settings modal tabs and Editor tabs.

---

## 7. Universal Viewer & Terminal Standards

* **Universal Document & Text Viewer**:
  * Dynamic tools (`tail -f`, line count selector `-n`, word wrap, refresh, scroll to bottom) and window actions must be **right-aligned** in `.doc-viewer-header-right`.
  * All buttons, selects, and icon controls must share a uniform **`28px` height**.
  * The entire header bar must serve as a non-fiddly drag handle (`cursor: grab;`).
* **Bite! Terminal Console & Nerd Fonts**:
  * Terminal uses embedded **`JetBrainsMono Nerd Font`** (bundled locally in `/assets/fonts/` and served directly by the backend).
  * Must support `eza --icons`, `starship`, `fish`, and powerline glyphs without missing boxes or tofu characters.

---

## 8. ChewToy Design & Layout Language Specification

Whenever creating a new ChewToy (built-in power tool), modal, or floating utility, or modifying existing ones, strictly adhere to the following UI/UX architecture:

### 1. Primary Window & Modal Headers (`42px` min-height)
* **Full-Length Drag Handle**: The entire header bar must serve as an easy, non-fiddly drag handle (`cursor: grab;` with `:active { cursor: grabbing; }`).
* **Uniform 28px Sizing**: All primary header buttons, selects, and icon triggers must be uniform **`28px x 28px`** (or `28px` height) with `border-radius: var(--radius)` (`6px`) and `14px` icons.
* **Right-Aligned Layout Switchers**: Layout toggle groups (`.layout-btn-group`, view switches) must be right-aligned, **icon-only** (NO text labels), have descriptive tooltips (`title="..."`), and use the active amber accent glow (`rgba(245, 158, 11, 0.18)` + `var(--accent)`).
* **Flat & Stealthy Window Controls**: Window action buttons (Minimize `minus`, Maximize `maximize-2`, Dock `panel-left-close`, Float `external-link`/`picture-in-picture`, Close `x`) must share the flat, stealthy aesthetic:
  * Default: `border: 1px solid transparent; background: transparent; opacity: 0.7; border-radius: var(--radius);`
  * Hover: Soft white glow (`rgba(255, 255, 255, 0.08); opacity: 1;`)
  * Close Hover: Subtle calming red feedback (`rgba(239, 68, 68, 0.2); color: #ef4444;`)

### 2. Sub-Headers, Inner Toolbars & Workspace Actions (`26px` Standard)
* **Uniform 26px Sizing**: All secondary toolbars, workspace action bars (e.g. NoteDog workspace, Diff filters, Git staging bar, Disk Usage path row), and panel sub-headers must use **`26px x 26px`** buttons with `border-radius: var(--radius)` (`6px`) and `13px` icons.
* **Docked Tool Headers**: In-pane docked Chewtoys must use icon-only float/undock buttons (`external-link` / `26px`) without the text label `"Float"`.

### 3. Dual-Mode Architecture (Floating + In-Pane Docking)
* Every ChewToy must natively support dual operational modes:
  1. **Floating Window Mode**: Freely draggable, resizable, minimizable, and maximizable.
  2. **In-Pane Docking Mode**: Dockable directly into Panel 1 or Panel 2 with automatic panel state persistence.

### 4. Tool & Terminal Lifecycle Protocols
* Interactive CLI Chewtoys (e.g., Bite! Terminal) must cleanly handle EOF (`Ctrl+D`), `logout`, and `exit` to automatically close the docked drawer/window and reset PTY state.
* ChewToys must gracefully remember user preferences (active layouts, selected views, column sizes) via `localStorage` or backend configuration.
