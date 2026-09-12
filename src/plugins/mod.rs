use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSection {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default = "default_plugin_icon")]
    pub icon: String,
    #[serde(default = "default_plugin_category")]
    pub category: String,
}

fn default_plugin_icon() -> String {
    "icon.svg".to_string()
}

fn default_plugin_category() -> String {
    "utilities".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSection {
    #[serde(default = "default_modes")]
    pub modes: Vec<String>,
    #[serde(default = "default_mode")]
    pub default_mode: String,
    #[serde(default = "default_width")]
    pub default_width: u32,
    #[serde(default = "default_height")]
    pub default_height: u32,
    #[serde(default = "default_min_width")]
    pub min_width: u32,
    #[serde(default = "default_min_height")]
    pub min_height: u32,
}

fn default_modes() -> Vec<String> {
    vec!["floating".to_string(), "docked".to_string()]
}
fn default_mode() -> String {
    "floating".to_string()
}
fn default_width() -> u32 {
    800
}
fn default_height() -> u32 {
    550
}
fn default_min_width() -> u32 {
    400
}
fn default_min_height() -> u32 {
    300
}

impl Default for UiSection {
    fn default() -> Self {
        Self {
            modes: default_modes(),
            default_mode: default_mode(),
            default_width: default_width(),
            default_height: default_height(),
            min_width: default_min_width(),
            min_height: default_min_height(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationsSection {
    #[serde(default = "default_true")]
    pub launchpad: bool,
    #[serde(default)]
    pub launchpad_label: Option<String>,
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub context_menu_label: Option<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for IntegrationsSection {
    fn default() -> Self {
        Self {
            launchpad: true,
            launchpad_label: None,
            file_extensions: Vec::new(),
            context_menu_label: None,
            shortcut: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsSection {
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendSection {
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub script_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginSection,
    #[serde(default)]
    pub ui: UiSection,
    #[serde(default)]
    pub integrations: IntegrationsSection,
    #[serde(default)]
    pub permissions: PermissionsSection,
    #[serde(default)]
    pub backend: BackendSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub homepage: String,
    pub icon: String,
    pub category: String,
    pub manifest: PluginManifest,
    pub is_system: bool,
    pub is_enabled: bool,
    pub installed_by: String,
    pub created_at: String,
    pub can_uninstall: bool,
    pub path: String,
}

#[derive(Clone)]
pub struct PluginManager {
    pub system_dir: PathBuf,
    pub user_dir: PathBuf,
    pub allow_user_installs: bool,
    pub default_policy: String,
    pub global_whitelist: Vec<String>,
    pub global_blacklist: Vec<String>,
    disabled_plugins: Arc<Mutex<HashSet<String>>>,
}

impl PluginManager {
    pub fn new(
        system_dir: PathBuf,
        user_dir: PathBuf,
        allow_user_installs: bool,
        default_policy: String,
        global_whitelist: Vec<String>,
        global_blacklist: Vec<String>,
    ) -> Self {
        let mgr = Self {
            system_dir,
            user_dir,
            allow_user_installs,
            default_policy,
            global_whitelist,
            global_blacklist,
            disabled_plugins: Arc::new(Mutex::new(HashSet::new())),
        };
        mgr.init();
        mgr
    }

    pub fn init(&self) {
        if let Err(e) = fs::create_dir_all(&self.user_dir) {
            warn!("Failed to create user plugins directory {:?}: {}", self.user_dir, e);
        }
        if let Err(e) = fs::create_dir_all(&self.system_dir) {
            tracing::debug!("System plugins directory {:?} notice: {}", self.system_dir, e);
        }
    }

    /// Scans both system and user plugin directories and returns all discovered plugins
    pub fn scan_plugins(&self) -> Result<Vec<PluginInfo>, String> {
        let mut plugins = Vec::new();
        let mut seen_ids = HashSet::new();

        // 1. Scan System Plugins (read-only)
        if self.system_dir.exists() {
            if let Ok(entries) = fs::read_dir(&self.system_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(info) = self.load_plugin_dir(&path, true, "system") {
                            seen_ids.insert(info.id.clone());
                            plugins.push(info);
                        }
                    }
                }
            }
        }

        // 2. Scan User Plugins
        if self.user_dir.exists() {
            if let Ok(entries) = fs::read_dir(&self.user_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(info) = self.load_plugin_dir(&path, false, "admin") {
                            if !seen_ids.contains(&info.id) {
                                seen_ids.insert(info.id.clone());
                                plugins.push(info);
                            }
                        }
                    }
                }
            }
        }

        plugins.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(plugins)
    }

    fn load_plugin_dir(&self, dir: &Path, is_system: bool, default_installed_by: &str) -> Option<PluginInfo> {
        let manifest_path = dir.join("plugin.toml");
        let index_path = dir.join("index.html");

        if !manifest_path.exists() || !index_path.exists() {
            return None;
        }

        let content = fs::read_to_string(&manifest_path).ok()?;
        let manifest: PluginManifest = match toml::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                warn!("Invalid plugin.toml in {:?}: {}", dir, e);
                return None;
            }
        };

        let disabled = self.disabled_plugins.lock().unwrap().contains(&manifest.plugin.id);
        let icon_path = dir.join(&manifest.plugin.icon);
        let icon_rel = if icon_path.exists() {
            format!("/api/plugins/{}/assets/{}", manifest.plugin.id, manifest.plugin.icon)
        } else {
            "assets/amber-frameless-apps.webp".to_string()
        };

        Some(PluginInfo {
            id: manifest.plugin.id.clone(),
            name: manifest.plugin.name.clone(),
            version: manifest.plugin.version.clone(),
            author: manifest.plugin.author.clone(),
            description: manifest.plugin.description.clone(),
            homepage: manifest.plugin.homepage.clone(),
            icon: icon_rel,
            category: manifest.plugin.category.clone(),
            manifest: manifest.clone(),
            is_system,
            is_enabled: !disabled,
            installed_by: default_installed_by.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            can_uninstall: !is_system,
            path: dir.to_string_lossy().to_string(),
        })
    }

    /// Finds a specific plugin by ID
    pub fn get_plugin(&self, id: &str) -> Option<PluginInfo> {
        let all = self.scan_plugins().ok()?;
        all.into_iter().find(|p| p.id == id)
    }

    /// Installs a `.grr` package (ZIP archive containing plugin.toml, index.html, etc.)
    pub fn install_grr(&self, grr_data: &[u8], user: &str, is_admin: bool) -> Result<PluginInfo, String> {
        if !is_admin && !self.allow_user_installs {
            return Err("Permission denied: standard users cannot install plugins. Contact administrator.".to_string());
        }

        let cursor = Cursor::new(grr_data);
        let mut archive = ZipArchive::new(cursor).map_err(|e| format!("Invalid .grr archive format: {}", e))?;

        // 1. Locate and parse plugin.toml from root or single top-level folder
        let mut manifest_content = None;
        let mut root_prefix = String::new();

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = file.name().to_string();
            if name == "plugin.toml" || name.ends_with("/plugin.toml") {
                let mut buf = String::new();
                file.read_to_string(&mut buf).map_err(|e| e.to_string())?;
                manifest_content = Some(buf);
                if let Some(idx) = name.rfind("/plugin.toml") {
                    root_prefix = format!("{}/", &name[..idx]);
                }
                break;
            }
        }

        let manifest_str = manifest_content.ok_or_else(|| "Package missing mandatory 'plugin.toml' manifest".to_string())?;
        let manifest: PluginManifest = toml::from_str(&manifest_str).map_err(|e| format!("Invalid plugin.toml manifest: {}", e))?;

        // 2. Validate plugin ID (strict identifier format)
        let plugin_id = manifest.plugin.id.trim();
        if plugin_id.is_empty() || !plugin_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err("Plugin ID must only contain alphanumeric characters, hyphens, and underscores".to_string());
        }

        // 3. Extract to user_dir/plugin_id with Zip Slip protection
        let dest_dir = self.user_dir.join(plugin_id);
        fs::create_dir_all(&dest_dir).map_err(|e| format!("Failed to create plugin directory: {}", e))?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
            let raw_name = file.name().to_string();
            
            // Strip top-level directory prefix if present
            let rel_name = if !root_prefix.is_empty() && raw_name.starts_with(&root_prefix) {
                &raw_name[root_prefix.len()..]
            } else {
                &raw_name
            };

            if rel_name.is_empty() {
                continue;
            }

            // Security: Prevent Zip Slip path traversal
            let outpath = dest_dir.join(rel_name);
            if !outpath.starts_with(&dest_dir) {
                return Err(format!("Security error: path traversal attempt detected in entry '{}'", raw_name));
            }

            if file.is_dir() || rel_name.ends_with('/') {
                fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                }
                let mut outfile = File::create(&outpath).map_err(|e| format!("Failed to write {}: {}", outpath.display(), e))?;
                std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
            }
        }

        // Ensure index.html exists
        if !dest_dir.join("index.html").exists() {
            let _ = fs::remove_dir_all(&dest_dir);
            return Err("Plugin package must contain an 'index.html' entrypoint".to_string());
        }

        info!("Successfully installed ChewToy plugin '{}' ({}) by {}", manifest.plugin.name, plugin_id, user);

        let info = self.load_plugin_dir(&dest_dir, false, user)
            .ok_or_else(|| "Failed to load newly installed plugin".to_string())?;

        Ok(info)
    }

    /// Uninstalls a user plugin by ID
    pub fn uninstall_plugin(&self, id: &str, user: &str, is_admin: bool) -> Result<(), String> {
        let plugin = self.get_plugin(id).ok_or_else(|| format!("Plugin '{}' not found", id))?;
        if plugin.is_system {
            return Err(format!("Cannot uninstall system plugin '{}'", id));
        }

        if !is_admin && plugin.installed_by != user {
            return Err("Permission denied: cannot uninstall plugins created by other users".to_string());
        }

        let path = PathBuf::from(&plugin.path);
        if path.exists() && path.starts_with(&self.user_dir) {
            fs::remove_dir_all(&path).map_err(|e| format!("Failed to remove plugin directory: {}", e))?;
            info!("Uninstalled ChewToy plugin '{}' by {}", id, user);
            Ok(())
        } else {
            Err("Plugin path is invalid or outside user plugin directory".to_string())
        }
    }

    /// Toggles a plugin's enabled/disabled state
    pub fn toggle_plugin(&self, id: &str, enabled: bool) -> Result<bool, String> {
        let _ = self.get_plugin(id).ok_or_else(|| format!("Plugin '{}' not found", id))?;
        let mut disabled = self.disabled_plugins.lock().unwrap();
        if enabled {
            disabled.remove(id);
        } else {
            disabled.insert(id.to_string());
        }
        Ok(enabled)
    }

    /// Reads an asset file belonging to a plugin securely
    pub fn get_asset(&self, id: &str, subpath: &str) -> Result<(Vec<u8>, String), String> {
        let plugin = self.get_plugin(id).ok_or_else(|| format!("Plugin '{}' not found", id))?;
        let base_path = PathBuf::from(&plugin.path);

        // Sanitize subpath to prevent path traversal
        let clean_subpath = subpath.trim_start_matches('/');
        let target_path = base_path.join(clean_subpath);

        if !target_path.starts_with(&base_path) || !target_path.exists() || !target_path.is_file() {
            return Err("Asset not found or access denied".to_string());
        }

        let bytes = fs::read(&target_path).map_err(|e| format!("Failed to read asset: {}", e))?;
        let mime = mime_guess::from_path(&target_path).first_or_octet_stream().to_string();

        Ok((bytes, mime))
    }

    /// Evaluates if a given user is allowed to access and run a plugin based on Admin status and RBAC rules
    pub fn is_allowed_for_user(
        &self,
        plugin_id: &str,
        is_admin: bool,
        allowed_json: &str,
        blocked_json: &str,
    ) -> bool {
        // Admins bypass all blacklist and whitelist restrictions
        if is_admin {
            return true;
        }

        // 1. Check Global Blacklist
        if self.global_blacklist.iter().any(|b| b == "*" || b == plugin_id) {
            return false;
        }

        // 2. Check User-specific Blacklist
        if let Ok(blocked) = serde_json::from_str::<Vec<String>>(blocked_json) {
            if blocked.iter().any(|b| b == "*" || b == plugin_id) {
                return false;
            }
        }

        // 3. Check Global & User Whitelist
        let mut user_allowed = false;
        if let Ok(allowed) = serde_json::from_str::<Vec<String>>(allowed_json) {
            if allowed.iter().any(|a| a == "*" || a == plugin_id) {
                user_allowed = true;
            }
        } else {
            user_allowed = true; // Default fallback to allowed if unconfigured
        }

        if !user_allowed {
            return false;
        }

        // 4. Check Global Whitelist policy
        if self.default_policy == "whitelist" {
            self.global_whitelist.iter().any(|w| w == "*" || w == plugin_id)
        } else {
            true
        }
    }

    /// Packs a directory into a standard `.grr` package
    pub fn pack_grr(source_dir: &Path, output_path: &Path) -> Result<(), String> {
        let manifest_path = source_dir.join("plugin.toml");
        let index_path = source_dir.join("index.html");

        if !manifest_path.exists() {
            return Err("Cannot pack .grr: missing 'plugin.toml' in source directory".to_string());
        }
        if !index_path.exists() {
            return Err("Cannot pack .grr: missing 'index.html' in source directory".to_string());
        }

        // Validate manifest content before packing
        let manifest_str = fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
        let _manifest: PluginManifest = toml::from_str(&manifest_str).map_err(|e| format!("Invalid plugin.toml: {}", e))?;

        let file = File::create(output_path).map_err(|e| format!("Failed to create output file: {}", e))?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        fn add_dir_to_zip(
            zip: &mut ZipWriter<File>,
            base_dir: &Path,
            current_dir: &Path,
            options: SimpleFileOptions,
        ) -> Result<(), String> {
            let entries = fs::read_dir(current_dir).map_err(|e| e.to_string())?;
            for entry in entries.flatten() {
                let path = entry.path();
                let rel_path = path.strip_prefix(base_dir).map_err(|e| e.to_string())?;
                let rel_str = rel_path.to_string_lossy().replace('\\', "/");

                if path.is_dir() {
                    zip.add_directory(&format!("{}/", rel_str), options).map_err(|e| e.to_string())?;
                    add_dir_to_zip(zip, base_dir, &path, options)?;
                } else {
                    zip.start_file(&rel_str, options).map_err(|e| e.to_string())?;
                    let mut f = File::open(&path).map_err(|e| e.to_string())?;
                    let mut buffer = Vec::new();
                    f.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
                    zip.write_all(&buffer).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }

        add_dir_to_zip(&mut zip, source_dir, source_dir, options)?;
        zip.finish().map_err(|e| format!("Failed to finalize .grr archive: {}", e))?;

        info!("Packed ChewToy plugin into {:?}", output_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_plugin_pack_and_install_grr() {
        let temp = tempdir().unwrap();
        let src_dir = temp.path().join("hexdog_src");
        fs::create_dir_all(&src_dir).unwrap();

        let manifest_content = r#"
[plugin]
id = "hexdog"
name = "HexDog Hex Studio"
version = "1.0.0"
author = "Bolt J Woofson"
description = "Hex viewer chewtoy"
category = "utilities"

[ui]
default_mode = "floating"
default_width = 800
default_height = 500

[integrations]
launchpad = true
launchpad_label = "Hex Studio"

[permissions]
permissions = ["fs:read"]
"#;
        fs::write(src_dir.join("plugin.toml"), manifest_content).unwrap();
        fs::write(src_dir.join("index.html"), "<h1>HexDog</h1>").unwrap();
        fs::write(src_dir.join("style.css"), "body { color: var(--accent); }").unwrap();

        let grr_path = temp.path().join("hexdog.grr");
        PluginManager::pack_grr(&src_dir, &grr_path).unwrap();
        assert!(grr_path.exists());

        let system_dir = temp.path().join("system_plugins");
        let user_dir = temp.path().join("user_plugins");
        let mgr = PluginManager::new(
            system_dir,
            user_dir.clone(),
            true,
            "allow_all".to_string(),
            vec!["*".to_string()],
            vec![],
        );

        let grr_bytes = fs::read(&grr_path).unwrap();
        let installed = mgr.install_grr(&grr_bytes, "test_admin", true).unwrap();
        assert_eq!(installed.id, "hexdog");
        assert_eq!(installed.name, "HexDog Hex Studio");

        // Verify scan
        let plugins = mgr.scan_plugins().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "hexdog");

        // Verify asset retrieval
        let (html, mime) = mgr.get_asset("hexdog", "index.html").unwrap();
        assert_eq!(String::from_utf8(html).unwrap(), "<h1>HexDog</h1>");
        assert!(mime.contains("text/html"));

        // Verify RBAC
        assert!(mgr.is_allowed_for_user("hexdog", true, "[]", "[]"));
        assert!(mgr.is_allowed_for_user("hexdog", false, "[\"*\"]", "[]"));
        assert!(!mgr.is_allowed_for_user("hexdog", false, "[\"*\"]", "[\"hexdog\"]"));

        // Verify uninstall
        mgr.uninstall_plugin("hexdog", "test_admin", true).unwrap();
        assert_eq!(mgr.scan_plugins().unwrap().len(), 0);
    }
}
