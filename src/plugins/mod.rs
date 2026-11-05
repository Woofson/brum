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

/// Chewtoys that ship inside Brum. Brum installs them like any Chewtoy, keeps them up to date
/// and leaves them out once an admin uninstalls them. The source folders are `plugins/<id>/`.
pub const BUNDLED_PLUGINS: &[(&str, &[u8])] = &[
    ("calculator", include_bytes!("../../plugins/calculator.grr")),
    ("colours", include_bytes!("../../plugins/colours.grr")),
    ("subnet", include_bytes!("../../plugins/subnet.grr")),
    ("tetrion", include_bytes!("../../plugins/tetrion.grr")),
];

/// A bundled Chewtoy and how it relates to what is installed
#[derive(Debug, Clone, Serialize)]
pub struct BundledPluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub installed_version: Option<String>,
    /// "user" (installed Chewtoy), "system" (admin-provided system folder) or none
    pub installed_location: Option<String>,
    pub update_available: bool,
    /// An admin uninstalled it; Brum does not put it back by itself
    pub removed: bool,
}

/// Compares dotted version numbers ("1.10.0" > "1.9.2"); text parts count as 0
pub fn version_is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.trim().trim_start_matches('v').split('.').map(|p| p.trim().parse().unwrap_or(0)).collect() };
    let (a, b) = (parse(candidate), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// Reads `plugin.toml` from a `.grr` package
pub fn read_grr_manifest(grr_data: &[u8]) -> Result<PluginManifest, String> {
    let mut archive = ZipArchive::new(Cursor::new(grr_data)).map_err(|e| format!("Invalid .grr archive format: {}", e))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().map_err(|e| e.to_string())?.to_string();
        if name == "plugin.toml" || name.ends_with("/plugin.toml") {
            let mut buf = String::new();
            file.read_to_string(&mut buf).map_err(|e| e.to_string())?;
            return toml::from_str(&buf).map_err(|e| format!("Invalid plugin.toml manifest: {}", e));
        }
    }
    Err("Package missing mandatory 'plugin.toml' manifest".to_string())
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
        let system_dir = crate::config::expand_tilde_path(system_dir);
        let user_dir = crate::config::expand_tilde_path(user_dir);
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
            let name = file.name().map_err(|e| e.to_string())?.to_string();
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

        const MAX_PLUGIN_BYTES: u64 = 100 * 1024 * 1024; // 100 MB
        const MAX_PLUGIN_ENTRIES: usize = 2_000;

        if archive.len() > MAX_PLUGIN_ENTRIES {
            return Err("Plugin package exceeded maximum entry limit (possible zip bomb)".to_string());
        }

        // 3. Extract to user_dir/plugin_id with Zip Slip protection
        let dest_dir = self.user_dir.join(plugin_id);
        let clean_dest = crate::vfs::local::clean_path_buf(&dest_dir);
        fs::create_dir_all(&dest_dir).map_err(|e| format!("Failed to create plugin directory: {}", e))?;

        let mut total_bytes: u64 = 0;

        let extract_result = (|| -> Result<(), String> {
            for i in 0..archive.len() {
                let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
                let enclosed = match file.enclosed_name() {
                    Some(p) => p.to_string_lossy().replace('\\', "/"),
                    None => {
                        return Err(format!("Security error: path traversal attempt in plugin entry '{}'", file.name().unwrap_or_default()));
                    }
                };

                // Strip top-level directory prefix if present
                let rel_name = if !root_prefix.is_empty() && enclosed.starts_with(&root_prefix) {
                    &enclosed[root_prefix.len()..]
                } else {
                    &enclosed
                };

                if rel_name.is_empty() {
                    continue;
                }

                // Reject any parent directory traversal
                for component in Path::new(&rel_name).components() {
                    if matches!(component, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(..)) {
                        return Err(format!("Security error: path traversal attempt in plugin entry '{}'", rel_name));
                    }
                }

                let outpath = dest_dir.join(rel_name);
                let clean_out = crate::vfs::local::clean_path_buf(&outpath);
                if !clean_out.starts_with(&clean_dest) {
                    return Err(format!("Security error: entry '{}' escapes plugin directory", rel_name));
                }

                if file.is_dir() || rel_name.ends_with('/') {
                    fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            fs::create_dir_all(p).map_err(|e| e.to_string())?;
                        }
                    }

                    total_bytes += file.size();
                    if total_bytes > MAX_PLUGIN_BYTES {
                        return Err("Plugin package exceeded maximum uncompressed size (possible zip bomb)".to_string());
                    }

                    let mut outfile = File::create(&outpath).map_err(|e| format!("Failed to write {}: {}", outpath.display(), e))?;
                    std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })();

        if let Err(e) = extract_result {
            let _ = fs::remove_dir_all(&dest_dir);
            return Err(e);
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

    // ---------------- Bundled Chewtoys ----------------

    fn bundled_state_path(&self) -> PathBuf {
        self.user_dir.join(".bundled.json")
    }

    /// Ids of bundled Chewtoys an admin uninstalled
    fn removed_bundled(&self) -> HashSet<String> {
        fs::read_to_string(self.bundled_state_path())
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("removed").cloned())
            .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default()
    }

    fn set_bundled_removed(&self, id: &str, removed: bool) {
        let mut set = self.removed_bundled();
        let changed = if removed { set.insert(id.to_string()) } else { set.remove(id) };
        if changed {
            let mut list: Vec<String> = set.into_iter().collect();
            list.sort();
            let _ = fs::create_dir_all(&self.user_dir);
            if let Err(e) = fs::write(self.bundled_state_path(), serde_json::json!({ "removed": list }).to_string()) {
                warn!("Could not save bundled Chewtoy state: {}", e);
            }
        }
    }

    pub fn bundled_package(id: &str) -> Option<&'static [u8]> {
        BUNDLED_PLUGINS.iter().find(|(bid, _)| *bid == id).map(|(_, bytes)| *bytes)
    }

    pub fn list_bundled(&self) -> Vec<BundledPluginInfo> {
        let installed = self.scan_plugins().unwrap_or_default();
        let removed = self.removed_bundled();
        BUNDLED_PLUGINS
            .iter()
            .filter_map(|(id, bytes)| {
                let m = read_grr_manifest(bytes).ok()?;
                let current = installed.iter().find(|p| p.id == *id);
                let installed_version = current.map(|p| p.version.clone());
                let installed_location = current.map(|p| if p.is_system { "system".to_string() } else { "user".to_string() });
                let update_available = current
                    .map(|p| !p.is_system && version_is_newer(&m.plugin.version, &p.version))
                    .unwrap_or(false);
                Some(BundledPluginInfo {
                    id: id.to_string(),
                    name: m.plugin.name,
                    version: m.plugin.version,
                    description: m.plugin.description,
                    installed_version,
                    installed_location,
                    update_available,
                    removed: current.is_none() && removed.contains(*id),
                })
            })
            .collect()
    }

    /// Installs or updates a bundled Chewtoy into the Chewtoy folder
    pub fn install_bundled(&self, id: &str, user: &str) -> Result<PluginInfo, String> {
        let bytes = Self::bundled_package(id).ok_or_else(|| format!("'{}' is not a bundled Chewtoy", id))?;
        if let Some(existing) = self.get_plugin(id) {
            if existing.is_system {
                return Err(format!("'{}' is provided by the system Chewtoy folder", id));
            }
            let path = PathBuf::from(&existing.path);
            if path.starts_with(&self.user_dir) {
                fs::remove_dir_all(&path).map_err(|e| format!("Failed to replace the installed copy: {}", e))?;
            }
        }
        let info = self.install_grr(bytes, user, true)?;
        self.set_bundled_removed(id, false);
        Ok(info)
    }

    /// At start-up: installs bundled Chewtoys that are missing (unless an admin uninstalled them)
    /// and updates installed copies that are older than the bundled one
    pub fn sync_bundled(&self) {
        for info in self.list_bundled() {
            let action = match (&info.installed_location, info.update_available, info.removed) {
                (None, _, false) => "Installed",
                (Some(loc), true, _) if loc == "user" => "Updated",
                _ => continue,
            };
            match self.install_bundled(&info.id, "brum") {
                Ok(p) => info!("{} bundled Chewtoy '{}' {}", action, p.id, p.version),
                Err(e) => warn!("Could not set up bundled Chewtoy '{}': {}", info.id, e),
            }
        }
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
            if Self::bundled_package(id).is_some() {
                self.set_bundled_removed(id, true);
            }
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

    /// Reads an asset file belonging to a plugin. Only files inside the plugin's own folder are
    /// served: parent, root and drive components are refused, and the real path (after
    /// following links) must stay inside the plugin's real folder.
    pub fn get_asset(&self, id: &str, subpath: &str) -> Result<(Vec<u8>, String), String> {
        const DENIED: &str = "Asset not found or access denied";
        let plugin = self.get_plugin(id).ok_or_else(|| format!("Plugin '{}' not found", id))?;
        let base_path = PathBuf::from(&plugin.path);

        let clean_subpath = subpath.replace('\\', "/");
        let clean_subpath = clean_subpath.trim_start_matches('/');
        let rel = Path::new(clean_subpath);
        if clean_subpath.is_empty()
            || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir))
        {
            return Err(DENIED.to_string());
        }

        let real_base = base_path.canonicalize().map_err(|_| DENIED.to_string())?;
        let real_target = base_path.join(rel).canonicalize().map_err(|_| DENIED.to_string())?;
        if !real_target.starts_with(&real_base) || !real_target.is_file() {
            return Err(DENIED.to_string());
        }

        let bytes = fs::read(&real_target).map_err(|e| format!("Failed to read asset: {}", e))?;
        let mime = mime_guess::from_path(&real_target).first_or_octet_stream().to_string();

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
    fn bundled_chewtoys_install_update_and_stay_uninstalled() {
        let temp = tempdir().unwrap();
        let user_dir = temp.path().join("user_plugins");
        let mgr = PluginManager::new(temp.path().join("system"), user_dir.clone(), false, "allow_all".to_string(), vec![], vec![]);
        let ids: Vec<&str> = BUNDLED_PLUGINS.iter().map(|(id, _)| *id).collect();

        // Fresh install: every bundled Chewtoy is installed and can be uninstalled
        mgr.sync_bundled();
        for id in &ids {
            let p = mgr.get_plugin(id).unwrap_or_else(|| panic!("{id} not installed"));
            assert!(!p.is_system && p.can_uninstall, "{id} must be an ordinary Chewtoy");
        }

        // Uninstalled stays uninstalled across restarts
        mgr.uninstall_plugin("tetrion", "admin", true).unwrap();
        mgr.sync_bundled();
        assert!(mgr.get_plugin("tetrion").is_none());
        let t = mgr.list_bundled().into_iter().find(|b| b.id == "tetrion").unwrap();
        assert!(t.removed && t.installed_version.is_none());

        // Installing on purpose brings it back
        mgr.install_bundled("tetrion", "admin").unwrap();
        assert!(mgr.get_plugin("tetrion").is_some());
        assert!(!mgr.list_bundled().into_iter().find(|b| b.id == "tetrion").unwrap().removed);

        // An older installed copy is updated on the next start, and stale files go
        let manifest = user_dir.join("colours/plugin.toml");
        let old = fs::read_to_string(&manifest).unwrap().replacen("version = \"", "version = \"0.0.1-old-", 1);
        fs::write(&manifest, old).unwrap();
        fs::write(user_dir.join("colours/stale.js"), "old").unwrap();
        assert!(mgr.list_bundled().into_iter().find(|b| b.id == "colours").unwrap().update_available);
        mgr.sync_bundled();
        let c = mgr.list_bundled().into_iter().find(|b| b.id == "colours").unwrap();
        assert!(!c.update_available && c.installed_version.as_deref() == Some(c.version.as_str()));
        assert!(!user_dir.join("colours/stale.js").exists());
    }

    /// Chewtoys kept and maintained in `plugins/` that are installed by hand, not built into Brum
    const OPTIONAL_PACKED_CHEWTOYS: &[&str] = &["tailspin"];

    #[test]
    fn every_packed_chewtoy_is_bundled_or_listed_as_optional() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins");
        for entry in fs::read_dir(&root).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("grr") {
                let id = path.file_stem().unwrap().to_string_lossy().to_string();
                let bytes = if OPTIONAL_PACKED_CHEWTOYS.contains(&id.as_str()) {
                    assert!(PluginManager::bundled_package(&id).is_none(), "{id} is optional and must not be in BUNDLED_PLUGINS");
                    fs::read(&path).unwrap()
                } else {
                    PluginManager::bundled_package(&id)
                        .unwrap_or_else(|| panic!("add plugins/{id}.grr to BUNDLED_PLUGINS, or to OPTIONAL_PACKED_CHEWTOYS"))
                        .to_vec()
                };
                let m = read_grr_manifest(&bytes).unwrap();
                assert_eq!(m.plugin.id, id, "package name and plugin id differ");
            }
        }
        for id in OPTIONAL_PACKED_CHEWTOYS {
            assert!(root.join(format!("{id}.grr")).is_file(), "optional Chewtoy plugins/{id}.grr is missing");
        }
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(version_is_newer("1.10.0", "1.9.2"));
        assert!(version_is_newer("1.1.0", "1.0.9"));
        assert!(!version_is_newer("1.0.0", "1.0.0"));
        assert!(!version_is_newer("0.9", "1.0.0"));
        assert!(version_is_newer("1.0.1", "1.0"));
    }

    #[test]
    fn assets_never_leave_the_plugin_folder() {
        let temp = tempdir().unwrap();
        let system_dir = temp.path().join("system_plugins");
        let plugin_dir = system_dir.join("demo");
        fs::create_dir_all(plugin_dir.join("assets")).unwrap();
        fs::write(plugin_dir.join("plugin.toml"), "[plugin]\nid = \"demo\"\nname = \"Demo\"\nversion = \"1.0.0\"\n").unwrap();
        fs::write(plugin_dir.join("index.html"), "<p>demo</p>").unwrap();
        fs::write(plugin_dir.join("assets/app.js"), "ok").unwrap();
        fs::write(temp.path().join("secret.txt"), "secret").unwrap();
        let mgr = PluginManager::new(system_dir.clone(), temp.path().join("user_plugins"), true, "allow".to_string(), vec![], vec![]);

        assert_eq!(mgr.get_asset("demo", "index.html").unwrap().0, b"<p>demo</p>");
        assert_eq!(mgr.get_asset("demo", "assets/app.js").unwrap().0, b"ok");
        assert_eq!(mgr.get_asset("demo", "./assets/app.js").unwrap().0, b"ok");

        let outside = temp.path().join("secret.txt");
        for bad in [
            "../../secret.txt".to_string(),
            "assets/../../../secret.txt".to_string(),
            "..\\..\\secret.txt".to_string(),
            outside.to_string_lossy().to_string(),
            format!("/{}", outside.to_string_lossy()),
            "assets".to_string(),
            "".to_string(),
        ] {
            assert!(mgr.get_asset("demo", &bad).is_err(), "served {bad}");
        }

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, plugin_dir.join("link.txt")).unwrap();
            assert!(mgr.get_asset("demo", "link.txt").is_err(), "followed a link out of the plugin folder");
        }
    }

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
file_extensions = ["*.bin", "*.dat", "*.so"]
context_menu_label = "Inspect in Hex Editor"
shortcut = "Ctrl+Shift+H"

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
        assert_eq!(installed.manifest.integrations.file_extensions, vec!["*.bin", "*.dat", "*.so"]);
        assert_eq!(installed.manifest.integrations.context_menu_label.as_deref(), Some("Inspect in Hex Editor"));
        assert_eq!(installed.manifest.integrations.shortcut.as_deref(), Some("Ctrl+Shift+H"));

        // Verify scan
        let plugins = mgr.scan_plugins().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "hexdog");
        assert_eq!(plugins[0].manifest.integrations.shortcut.as_deref(), Some("Ctrl+Shift+H"));

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
