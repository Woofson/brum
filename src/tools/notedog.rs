use argon2::{Argon2, Params};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use chrono::{DateTime, Local};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const NOTEDOG_MAGIC_HEADER: &[u8] = b"NOTEDOG_ENC_V1";
pub const NOTEDOG_SALT_LEN: usize = 16;
pub const NOTEDOG_NONCE_LEN: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDogFile {
    pub name: String,
    pub filename: String,
    pub path: String,
    pub relative_path: String,
    pub is_encrypted: bool,
    pub modified_sec: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDogSection {
    pub name: String,
    pub path: String,
    pub is_encrypted: bool,
    pub notes: Vec<NoteDogFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDogNotebook {
    pub name: String,
    pub path: String,
    pub is_encrypted: bool,
    pub sections: Vec<NoteDogSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDogInfo {
    pub root_folder: String,
    pub notebooks: Vec<NoteDogNotebook>,
    pub total_notes: usize,
    pub total_notebooks: usize,
    pub total_sections: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteVersionItem {
    pub filename: String,
    pub path: String,
    pub timestamp_sec: u64,
    pub formatted_time: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteTemplate {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub content: String,
}

/// Resolve the Notes root directory.
/// 1. If `override_path` is provided, expand and use that.
/// 2. If `config_folder` is configured in Brum (e.g. `[notedog] notes_folder = "..."`), use that.
/// 3. If `~/.config/notedog/notedog.toml` exists with `note_folder = "..."`, use that.
/// 4. Default to `~/Notes`.
pub fn get_notedog_root_dir(override_path: Option<&str>, config_folder: Option<&str>) -> PathBuf {
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

    // 1. Override path if supplied
    if let Some(ov) = override_path {
        let trimmed = ov.trim();
        if !trimmed.is_empty() {
            let mut folder = trimmed.to_string();
            if folder.starts_with("~/") {
                folder = folder.replacen("~", &home_dir.to_string_lossy(), 1);
            }
            let p = PathBuf::from(folder);
            if !p.exists() {
                let _ = fs::create_dir_all(&p);
            }
            return p;
        }
    }

    // 2. Check Brum config.toml [notedog] notes_folder
    if let Some(cf) = config_folder {
        let trimmed = cf.trim();
        if !trimmed.is_empty() && trimmed != "~/Notes" {
            let mut folder = trimmed.to_string();
            if folder.starts_with("~/") {
                folder = folder.replacen("~", &home_dir.to_string_lossy(), 1);
            }
            let p = PathBuf::from(folder);
            if !p.exists() {
                let _ = fs::create_dir_all(&p);
            }
            return p;
        }
    }

    // 3. Fallback to ~/.config/notedog/notedog.toml
    if let Some(config_dir) = dirs::config_dir() {
        let conf_file = config_dir.join("notedog/notedog.toml");
        if conf_file.exists() {
            if let Ok(c) = fs::read_to_string(conf_file) {
                if let Ok(v) = c.parse::<toml::Value>() {
                    if let Some(nf) = v.get("note_folder").and_then(|x| x.as_str()) {
                        let mut folder = nf.trim().to_string();
                        if folder.starts_with("~/") {
                            folder = folder.replacen("~", &home_dir.to_string_lossy(), 1);
                        }
                        let p = PathBuf::from(folder);
                        if !p.exists() {
                            let _ = fs::create_dir_all(&p);
                        }
                        return p;
                    }
                }
            }
        }
    }

    // 4. Default: ~/Notes
    let default_notes = home_dir.join("Notes");
    if !default_notes.exists() {
        let _ = fs::create_dir_all(&default_notes);
        // Create initial default welcome structure: Notes/Welcome/Getting-Started.md
        let welcome_dir = default_notes.join("Welcome");
        let _ = fs::create_dir_all(&welcome_dir);
        let welcome_path = welcome_dir.join("Getting-Started.md");
        if !welcome_path.exists() {
            let welcome_text = r##"# 🐶 Welcome to Notes Studio!

Notes Studio is a vibrant, modern **Notes & Markdown Studio** chewtoy seamlessly integrated into Brum.

---

## 🎨 Color-Supported Markdown
Notes supports rich inline color tags that render beautifully in live preview **and** stay 100% compatible with Obsidian, VSCode, and GitHub:

- Use standard HTML spans: <span style="color:#FF8C00">Warm Dark Orange</span> or <span style="color:#FFD700">Gold Accent</span>!
- Or HTML font tags: <font color="#FFA500">Bright Amber Text</font>
- Or quick shorthand: {[#E67E22]Warm Border Color}

### ⚡ Feature Checklist
- [x] 3-Tier Hierarchy: `Notebook` > `Section` > `Note`
- [x] Dockable in any pane or float as a desktop window
- [x] Interactive task tickboxes (click to check/uncheck in preview!)
- [x] Mermaid & Flowchart diagram rendering
- [x] Endless revision history with live diffs and instant rollback
- [x] Shared themes and colors with Brum

---

> *"Organization is the key to clarity."*
"##;
            let _ = fs::write(welcome_path, welcome_text);
        }
    }
    default_notes
}

/// Scan the NoteDog hierarchy
pub fn scan_notedog_hierarchy(override_path: Option<&str>, config_folder: Option<&str>) -> NoteDogInfo {
    let root = get_notedog_root_dir(override_path, config_folder);
    let mut notebooks = Vec::new();
    let mut total_notes = 0;
    let mut total_sections = 0;

    if let Ok(entries) = fs::read_dir(&root) {
        let mut nb_dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && !p.file_name().unwrap_or_default().to_string_lossy().starts_with('.')
            })
            .collect();
        nb_dirs.sort();

        for nb_path in nb_dirs {
            let nb_name = nb_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let mut sections = Vec::new();
            if let Ok(sec_entries) = fs::read_dir(&nb_path) {
                let mut sec_dirs: Vec<PathBuf> = sec_entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_dir()
                            && !p.file_name().unwrap_or_default().to_string_lossy().starts_with('.')
                    })
                    .collect();
                sec_dirs.sort();

                for sec_path in sec_dirs {
                    let sec_name = sec_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let mut notes = Vec::new();
                    if let Ok(note_entries) = fs::read_dir(&sec_path) {
                        let mut note_files: Vec<PathBuf> = note_entries
                            .filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .filter(|p| {
                                if p.is_file() {
                                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                                    (name.ends_with(".md") || name.ends_with(".md.enc") || name.ends_with(".txt"))
                                        && !name.starts_with('.')
                                } else {
                                    false
                                }
                            })
                            .collect();
                        note_files.sort();

                        for note_path in note_files {
                            let filename = note_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            let clean_name = filename
                                .strip_suffix(".md.enc")
                                .or_else(|| filename.strip_suffix(".md"))
                                .or_else(|| filename.strip_suffix(".txt"))
                                .unwrap_or(&filename)
                                .to_string();
                            let is_encrypted = filename.ends_with(".md.enc");

                            let meta = fs::metadata(&note_path);
                            let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                            let modified_sec = meta
                                .as_ref()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);

                            let relative_path = format!("{}/{}/{}", nb_name, sec_name, filename);

                            notes.push(NoteDogFile {
                                name: clean_name,
                                filename,
                                path: note_path.to_string_lossy().to_string(),
                                relative_path,
                                is_encrypted,
                                modified_sec,
                                size_bytes,
                            });
                            total_notes += 1;
                        }
                    }

                    let sec_is_encrypted = sec_path.join(".encrypted").exists()
                        || (!notes.is_empty() && notes.iter().all(|n| n.is_encrypted));

                    total_sections += 1;
                    sections.push(NoteDogSection {
                        name: sec_name,
                        path: sec_path.to_string_lossy().to_string(),
                        is_encrypted: sec_is_encrypted,
                        notes,
                    });
                }
            }

            let nb_is_encrypted = nb_path.join(".encrypted").exists()
                || (!sections.is_empty() && sections.iter().all(|s| s.is_encrypted));

            notebooks.push(NoteDogNotebook {
                name: nb_name,
                path: nb_path.to_string_lossy().to_string(),
                is_encrypted: nb_is_encrypted,
                sections,
            });
        }
    }

    NoteDogInfo {
        root_folder: root.to_string_lossy().to_string(),
        total_notebooks: notebooks.len(),
        total_sections,
        total_notes,
        notebooks,
    }
}

/// Helper to get the version history directory for a note
fn get_version_dir_for_note(note_path: &Path) -> PathBuf {
    let root = if let Some(parent3) = note_path.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
        parent3.to_path_buf()
    } else {
        get_notedog_root_dir(None, None)
    };
    let versions_root = root.join(".notedog_versions");
    let filename = note_path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let parent = note_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string());
    let grand = note_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string());

    versions_root.join(grand).join(parent).join(filename)
}

/// Create a snapshot revision when saving a note
pub fn create_note_snapshot(note_path: &Path, content: &[u8]) -> io::Result<()> {
    let version_dir = get_version_dir_for_note(note_path);
    fs::create_dir_all(&version_dir)?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs();
    let nanos = now.subsec_nanos();

    // Check if identical to last version to avoid duplicate revisions
    if let Ok(entries) = fs::read_dir(&version_dir) {
        let mut rev_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().map_or(false, |ext| ext == "rev"))
            .collect();
        rev_files.sort();
        if let Some(latest) = rev_files.last() {
            if let Ok(latest_bytes) = fs::read(latest) {
                if latest_bytes == content {
                    return Ok(());
                }
            }
        }
    }

    let dt: DateTime<Local> = DateTime::from(SystemTime::now());
    let formatted = dt.format("%Y-%m-%d_%H-%M-%S").to_string();
    let rev_filename = format!("{}_{}_{}.rev", secs, nanos, formatted);
    let target = version_dir.join(rev_filename);

    fs::write(target, content)
}

/// List all version history snapshots for a note
pub fn list_note_versions(note_path: &Path) -> Vec<NoteVersionItem> {
    let version_dir = get_version_dir_for_note(note_path);
    let mut versions = Vec::new();

    if !version_dir.exists() {
        return versions;
    }

    if let Ok(entries) = fs::read_dir(&version_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "rev") {
                let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let parts: Vec<&str> = filename.split('_').collect();
                let ts_sec: u64 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                
                let dt: DateTime<Local> = if ts_sec > 0 {
                    DateTime::from(UNIX_EPOCH + std::time::Duration::from_secs(ts_sec))
                } else {
                    DateTime::from(SystemTime::now())
                };
                let formatted_time = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

                versions.push(NoteVersionItem {
                    filename,
                    path: path.to_string_lossy().to_string(),
                    timestamp_sec: ts_sec,
                    formatted_time,
                    size_bytes,
                });
            }
        }
    }

    versions.sort_by(|a, b| b.timestamp_sec.cmp(&a.timestamp_sec));
    versions
}

/// Available smart templates
pub fn get_builtin_templates() -> Vec<NoteTemplate> {
    let now: DateTime<Local> = DateTime::from(SystemTime::now());
    let date_str = now.format("%Y-%m-%d %H:%M").to_string();

    vec![
        NoteTemplate {
            id: "blank".to_string(),
            name: "Blank Note".to_string(),
            icon: "📄".to_string(),
            description: "Empty markdown note".to_string(),
            content: format!("# {{{{title}}}}\n\nWrite your notes here...\n"),
        },
        NoteTemplate {
            id: "todo".to_string(),
            name: "Todo & Checklist".to_string(),
            icon: "✅".to_string(),
            description: "Interactive task list with tickboxes".to_string(),
            content: format!(
                r#"# 📝 {{{{title}}}}

**Date**: {}

## ⚡ Priority Tasks
- [ ] 
- [ ] 
- [ ] 

## 📌 Secondary Items
- [ ] 

## 💡 Notes & Ideas
- 
"#,
                date_str
            ),
        },
        NoteTemplate {
            id: "shopping".to_string(),
            name: "Shopping & Grocery List".to_string(),
            icon: "🛒".to_string(),
            description: "Categorized produce, dairy, bakery & pantry".to_string(),
            content: format!(
                r#"# 🛒 {{{{title}}}}

**Date**: {}

## 🥦 Fresh Produce & Vegetables
- [ ] Apples / Fruit
- [ ] Greens / Salad

## 🥛 Dairy & Refrigerated
- [ ] Milk
- [ ] Cheese
- [ ] Eggs
- [ ] Butter

## 🍞 Bakery & Pantry
- [ ] Bread / Rolls
- [ ] Pasta / Rice
- [ ] Coffee / Tea

## 🧼 Household & Essentials
- [ ] Paper Towels
- [ ] Dish Soap
"#,
                date_str
            ),
        },
        NoteTemplate {
            id: "meeting".to_string(),
            name: "Meeting & Agenda".to_string(),
            icon: "📅".to_string(),
            description: "Structured agenda, discussion notes and action items".to_string(),
            content: format!(
                r#"# 📅 {{{{title}}}}

**Date & Time**: {}
**Participants**: 
**Location / Link**: 

## 🎯 Meeting Goals
1. 

## 💬 Discussion Notes
- 

## ⚡ Action Items
- [ ] **Who**: Task description
- [ ] **Who**: Follow-up item
"#,
                date_str
            ),
        },
        NoteTemplate {
            id: "retro".to_string(),
            name: "Sprint / Project Retro".to_string(),
            icon: "🚀".to_string(),
            description: "What went well, what needs improvement, and next steps".to_string(),
            content: format!(
                r#"# 🚀 {{{{title}}}}

**Date**: {}

## 🟢 What Went Well
- 

## 🔴 What Needs Improvement
- 

## 💡 Key Learnings
- 

## ⚡ Action Items
- [ ] 
"#,
                date_str
            ),
        },
        NoteTemplate {
            id: "mermaid".to_string(),
            name: "Mermaid Flowchart Diagram".to_string(),
            icon: "📊".to_string(),
            description: "Visual flowchart with decision trees and process blocks".to_string(),
            content: format!(
                r#"# 📊 {{{{title}}}}

**Created**: {}

## 🌲 Process Flowchart
```mermaid
graph TD
    A[Start Process] --> B{{{{Check Conditions}}}}
    B -->|Passed| C[Execute Next Step]
    B -->|Failed| D[Log Diagnostic Error]
    C --> E[Final Result]
    D --> E
```

### 📝 Notes & Architecture Details
- Block **A**: Entry point
- Block **B**: Validation logic
"#,
                date_str
            ),
        },
    ]
}

// ---------------- NOTEDOG CRYPTO ENGINE (ChaCha20-Poly1305 + Argon2id) ----------------

pub fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let params = Params::new(19456, 2, 1, Some(32))
        .map_err(|e| format!("Argon2id params error: {}", e))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| format!("Key derivation failed: {}", e))?;

    Ok(key)
}

pub fn encrypt_note(plaintext: &str, passphrase: &str) -> Result<Vec<u8>, String> {
    let mut salt = [0u8; NOTEDOG_SALT_LEN];
    let mut nonce_bytes = [0u8; NOTEDOG_NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt)?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)
        .map_err(|e| format!("Cipher init error: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("Encryption failed: {}", e))?;

    let mut result = Vec::with_capacity(NOTEDOG_MAGIC_HEADER.len() + NOTEDOG_SALT_LEN + NOTEDOG_NONCE_LEN + ciphertext.len());
    result.extend_from_slice(NOTEDOG_MAGIC_HEADER);
    result.extend_from_slice(&salt);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

pub fn decrypt_note(data: &[u8], passphrase: &str) -> Result<String, String> {
    let header_len = NOTEDOG_MAGIC_HEADER.len();
    if data.len() < header_len + NOTEDOG_SALT_LEN + NOTEDOG_NONCE_LEN {
        return Err("Encrypted payload too short".to_string());
    }

    if &data[..header_len] != NOTEDOG_MAGIC_HEADER {
        return Err("Invalid NoteDog file header".to_string());
    }

    let salt = &data[header_len..header_len + NOTEDOG_SALT_LEN];
    let nonce_bytes = &data[header_len + NOTEDOG_SALT_LEN..header_len + NOTEDOG_SALT_LEN + NOTEDOG_NONCE_LEN];
    let ciphertext = &data[header_len + NOTEDOG_SALT_LEN + NOTEDOG_NONCE_LEN..];

    let key = derive_key(passphrase, salt)?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)
        .map_err(|e| format!("Cipher init error: {}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext_bytes = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "Decryption failed (incorrect passphrase or corrupted note)".to_string())?;

    String::from_utf8(plaintext_bytes).map_err(|e| format!("UTF-8 decode error: {}", e))
}

pub fn is_encrypted_data(data: &[u8]) -> bool {
    data.len() >= NOTEDOG_MAGIC_HEADER.len() && &data[..NOTEDOG_MAGIC_HEADER.len()] == NOTEDOG_MAGIC_HEADER
}

pub fn decrypt_note_file(path: &Path, passphrase: &str) -> Result<String, String> {
    let raw = fs::read(path).map_err(|e| format!("Failed to read note file: {}", e))?;
    if !is_encrypted_data(&raw) {
        // Plaintext fallback
        return String::from_utf8(raw).map_err(|e| format!("UTF-8 error: {}", e));
    }

    match decrypt_note(&raw, passphrase) {
        Ok(text) => Ok(text),
        Err(err) => {
            if passphrase != "notedog" {
                if let Ok(text) = decrypt_note(&raw, "notedog") {
                    return Ok(text);
                }
            }
            Err(err)
        }
    }
}

pub fn encrypt_and_save_note(path: &Path, content: &str, passphrase: &str) -> Result<PathBuf, String> {
    let enc_bytes = encrypt_note(content, passphrase)?;
    let target_path = if path.to_string_lossy().ends_with(".md.enc") {
        path.to_path_buf()
    } else {
        path.with_extension("md.enc")
    };

    if let Some(parent) = target_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    fs::write(&target_path, &enc_bytes).map_err(|e| format!("Failed to save encrypted note: {}", e))?;

    if target_path != path && path.exists() {
        let _ = fs::remove_file(path);
    }

    let _ = create_note_snapshot(&target_path, &enc_bytes);
    Ok(target_path)
}

pub fn decrypt_and_save_plain_note(path: &Path, passphrase: &str) -> Result<PathBuf, String> {
    let content = decrypt_note_file(path, passphrase)?;
    let target_path = if path.to_string_lossy().ends_with(".md.enc") {
        path.with_extension("").with_extension("md")
    } else {
        path.to_path_buf()
    };

    fs::write(&target_path, content.as_bytes()).map_err(|e| format!("Failed to save decrypted note: {}", e))?;

    if target_path != path && path.exists() {
        let _ = fs::remove_file(path);
    }

    let _ = create_note_snapshot(&target_path, content.as_bytes());
    Ok(target_path)
}

pub fn encrypt_section_dir(sec_path: &Path, passphrase: &str, cached_pass: Option<&str>) -> Result<usize, String> {
    let mut count = 0;
    let entries = fs::read_dir(sec_path).map_err(|e| format!("Failed to read section directory: {}", e))?;

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if filename.ends_with(".md") && !filename.starts_with('.') {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(enc_bytes) = encrypt_note(&content, passphrase) {
                        let enc_path = path.with_extension("md.enc");
                        if fs::write(&enc_path, enc_bytes).is_ok() {
                            let _ = fs::remove_file(&path);
                            count += 1;
                        }
                    }
                }
            } else if filename.ends_with(".md.enc") && !filename.starts_with('.') {
                if let Ok(raw) = fs::read(&path) {
                    if decrypt_note(&raw, passphrase).is_ok() {
                        count += 1;
                    } else {
                        let mut decrypted_text = None;
                        if let Some(cp) = cached_pass {
                            if let Ok(text) = decrypt_note(&raw, cp) {
                                decrypted_text = Some(text);
                            }
                        }
                        if decrypted_text.is_none() {
                            if let Ok(text) = decrypt_note(&raw, "notedog") {
                                decrypted_text = Some(text);
                            }
                        }
                        if let Some(text) = decrypted_text {
                            if let Ok(enc_bytes) = encrypt_note(&text, passphrase) {
                                let _ = fs::write(&path, enc_bytes);
                                count += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    let marker = sec_path.join(".encrypted");
    let _ = fs::write(marker, b"NOTEDOG_SECTION_ENC");
    Ok(count)
}

pub fn decrypt_section_dir(sec_path: &Path, passphrase: &str) -> Result<usize, String> {
    let mut decrypted_count = 0;
    let mut failed_count = 0;
    let entries = fs::read_dir(sec_path).map_err(|e| format!("Failed to read section directory: {}", e))?;

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if filename.ends_with(".md.enc") && !filename.starts_with('.') {
                if let Ok(raw) = fs::read(&path) {
                    let mut decrypted = decrypt_note(&raw, passphrase);
                    if decrypted.is_err() && passphrase != "notedog" {
                        decrypted = decrypt_note(&raw, "notedog");
                    }

                    match decrypted {
                        Ok(plaintext) => {
                            let dec_path = path.with_extension("").with_extension("md");
                            if fs::write(&dec_path, plaintext).is_ok() {
                                let _ = fs::remove_file(&path);
                                decrypted_count += 1;
                            }
                        }
                        Err(_) => {
                            failed_count += 1;
                        }
                    }
                }
            }
        }
    }

    if failed_count == 0 {
        let marker = sec_path.join(".encrypted");
        if marker.exists() {
            let _ = fs::remove_file(marker);
        }
    }

    if decrypted_count == 0 && failed_count > 0 {
        return Err("Incorrect passphrase for encrypted notes in section".to_string());
    }

    Ok(decrypted_count)
}

pub fn encrypt_notebook_dir(nb_path: &Path, passphrase: &str, cached_pass: Option<&str>) -> Result<usize, String> {
    let mut total_count = 0;
    let entries = fs::read_dir(nb_path).map_err(|e| format!("Failed to read notebook directory: {}", e))?;

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() && !path.file_name().unwrap_or_default().to_string_lossy().starts_with('.') {
            if let Ok(cnt) = encrypt_section_dir(&path, passphrase, cached_pass) {
                total_count += cnt;
            }
        }
    }

    let marker = nb_path.join(".encrypted");
    let _ = fs::write(marker, b"NOTEDOG_NOTEBOOK_ENC");
    Ok(total_count)
}

pub fn decrypt_notebook_dir(nb_path: &Path, passphrase: &str) -> Result<usize, String> {
    let mut total_count = 0;
    let entries = fs::read_dir(nb_path).map_err(|e| format!("Failed to read notebook directory: {}", e))?;

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() && !path.file_name().unwrap_or_default().to_string_lossy().starts_with('.') {
            if let Ok(cnt) = decrypt_section_dir(&path, passphrase) {
                total_count += cnt;
            }
        }
    }

    let marker = nb_path.join(".encrypted");
    if marker.exists() {
        let _ = fs::remove_file(marker);
    }

    Ok(total_count)
}

pub fn create_new_note(
    sec_path: &Path,
    title: &str,
    is_encrypted: bool,
    passphrase: Option<&str>,
    initial_content: Option<&str>,
) -> Result<NoteDogFile, String> {
    if !sec_path.exists() {
        fs::create_dir_all(sec_path).map_err(|e| format!("Failed to create section dir: {}", e))?;
    }

    let clean_title = title.trim();
    if clean_title.is_empty() {
        return Err("Note title cannot be empty".to_string());
    }

    let sanitized_name: String = clean_title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect();
    let base_name = if sanitized_name.is_empty() { "Untitled_Note" } else { sanitized_name.trim() };

    let ext = if is_encrypted { "md.enc" } else { "md" };
    let mut note_filename = format!("{}.{}", base_name, ext);
    let mut target_path = sec_path.join(&note_filename);
    let mut counter = 1;

    while target_path.exists() {
        note_filename = format!("{}_{}.{}", base_name, counter, ext);
        target_path = sec_path.join(&note_filename);
        counter += 1;
    }

    let default_content = format!("# {}\n\n", clean_title);
    let body = initial_content.unwrap_or(&default_content);

    if is_encrypted {
        let pass = passphrase.unwrap_or("notedog");
        let enc_bytes = encrypt_note(body, pass)?;
        fs::write(&target_path, &enc_bytes).map_err(|e| format!("Failed to write encrypted note: {}", e))?;
        let _ = create_note_snapshot(&target_path, &enc_bytes);
    } else {
        fs::write(&target_path, body.as_bytes()).map_err(|e| format!("Failed to write note: {}", e))?;
        let _ = create_note_snapshot(&target_path, body.as_bytes());
    }

    let meta = fs::metadata(&target_path).map_err(|e| format!("Metadata error: {}", e))?;
    let size_bytes = meta.len();
    let modified_sec = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let nb_name = sec_path.parent().and_then(|p| p.file_name()).unwrap_or_default().to_string_lossy().to_string();
    let sec_name = sec_path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let relative_path = format!("{}/{}/{}", nb_name, sec_name, note_filename);

    Ok(NoteDogFile {
        name: clean_title.to_string(),
        filename: note_filename,
        path: target_path.to_string_lossy().to_string(),
        relative_path,
        is_encrypted,
        modified_sec,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notedog_encryption_cycle() {
        let text = "# Top Secret NoteDog Note\n\nConfidential content for CommanderDog.";
        let pass = "DogSecretPass123!";

        let encrypted = encrypt_note(text, pass).expect("Encryption failed");
        assert!(is_encrypted_data(&encrypted));

        let decrypted = decrypt_note(&encrypted, pass).expect("Decryption failed");
        assert_eq!(decrypted, text);
    }

    #[test]
    fn test_notedog_wrong_password() {
        let text = "Important Dog Data";
        let pass = "CorrectPass123";
        let wrong = "WrongPass";

        let encrypted = encrypt_note(text, pass).unwrap();
        assert!(decrypt_note(&encrypted, wrong).is_err());
    }
}

