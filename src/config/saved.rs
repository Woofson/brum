//! Server settings saved inside Brum (#195).
//!
//! Admins change server-wide settings in the web page; they are kept in the database
//! (`server_settings`, keys starting with `config:`), not in `config.toml`. At start-up Brum
//! takes its built-in defaults, puts the settings saved in Brum on top, and then `config.toml`
//! on top of that: anything written in the file wins, so a setup managed by hand keeps working
//! exactly as before. Environment variables still win over both.
//!
//! Only the settings listed in [`SPECS`] can be saved this way. Anything Brum needs before it
//! can open its database (where the database is, the signing key, the "settings only in the
//! file" lock) and anything that names a program to run stays in the file.

use super::AppConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;

/// Database key prefix for settings saved in Brum
pub const KEY_PREFIX: &str = "config:";

/// What a setting holds, and what values it accepts
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Bool,
    /// A whole number from..to
    Int(i64, i64),
    /// One line of text, at most this many characters
    Text(usize),
    /// Like Text; empty means "not set"
    OptText(usize),
    /// One of these words
    Choice(&'static [&'static str]),
    /// An address Brum listens on (an IP address)
    Host,
    /// A web address (http:// or https://), or empty
    Url,
    /// A folder that exists on this machine
    Folder,
    /// Like Folder; empty means "not set"
    OptFolder,
    /// A list of short texts
    List,
    /// A list of IP addresses or address ranges (10.0.0.0/8)
    Proxies,
    /// A list of web origins (https://files.example.com), or "*"
    Origins,
    /// A list of role names
    Roles,
    /// A password or key: stored encrypted, never sent back to the browser
    Secret,
    /// The shared folders list (`[[storage.roots]]`)
    Roots,
}

pub struct Spec {
    pub key: &'static str,
    pub kind: Kind,
}

const fn s(key: &'static str, kind: Kind) -> Spec {
    Spec { key, kind }
}

/// Every setting an admin may change from the web page
pub const SPECS: &[Spec] = &[
    // Who can reach this Brum
    s("server.host", Kind::Host),
    s("server.port", Kind::Int(1, 65535)),
    s("server.server_name", Kind::Text(64)),
    s("server.trusted_proxies", Kind::Proxies),
    s("server.allowed_origins", Kind::Origins),
    s("server.upload_max_size_mb", Kind::Int(1, 10_000_000)),
    s("server.session_duration_hours", Kind::Int(1, 8760)),
    s("server.root_path", Kind::Folder),
    // Who can log in
    s("auth.mode", Kind::Choice(&["mixed", "builtin", "pam"])),
    s("auth.pam_service", Kind::Text(64)),
    s("auth.allow_guest", Kind::Bool),
    s("auth.oidc.enabled", Kind::Bool),
    s("auth.oidc.provider_name", Kind::Text(64)),
    s("auth.oidc.issuer_url", Kind::Url),
    s("auth.oidc.client_id", Kind::Text(256)),
    s("auth.oidc.client_secret", Kind::Secret),
    s("auth.oidc.redirect_url", Kind::Url),
    s("auth.oidc.scopes", Kind::List),
    s("auth.oidc.auto_provision", Kind::Bool),
    s("auth.oidc.admin_group", Kind::Text(128)),
    s("auth.oidc.default_user_role", Kind::Choice(&["user", "readonly", "admin"])),
    s("auth.oidc.default_home_template", Kind::Text(512)),
    s("auth.oidc.force_sso_only", Kind::Bool),
    s("auth.oidc.button_icon", Kind::Text(64)),
    // Which folders people can see
    s("storage.allow_entire_system", Kind::Bool),
    s("storage.allow_remote_loopback", Kind::Bool),
    s("storage.default_user_home_template", Kind::Text(512)),
    s("storage.home_fallback_scheme", Kind::Choice(&["auto", "roots_only", "strict"])),
    s("storage.auto_create_home_dirs", Kind::Bool),
    s("storage.roots", Kind::Roots),
    s("storage.sftp.host_key_checking", Kind::Choice(&["tofu", "strict", "auto_accept"])),
    // Safety
    s("paranoid.enabled", Kind::Bool),
    s("paranoid.checksum_algorithm", Kind::Choice(&["sha256", "sha1", "md5"])),
    s("paranoid.verify_after_transfer", Kind::Bool),
    s("paranoid.atomic_writes", Kind::Bool),
    s("paranoid.trash_enabled", Kind::Bool),
    s("paranoid.custom_trash_dir", Kind::OptFolder),
    s("paranoid.confirm_delete", Kind::Bool),
    s("paranoid.confirm_overwrite", Kind::Bool),
    s("paranoid.windows_native_file_ops", Kind::Bool),
    s("paranoid.detect_locking_processes", Kind::Bool),
    // Several machines
    s("fleet.enabled", Kind::Bool),
    s("fleet.transfer_jobs", Kind::Int(1, 16)),
    // Terminal
    s("terminal.enabled", Kind::Bool),
    s("terminal.allow_roles", Kind::Roles),
    s("terminal.allow_virtual_users", Kind::Bool),
    s("terminal.drop_privileges", Kind::Bool),
    // Chewtoys
    s("plugins.enabled", Kind::Bool),
    s("plugins.allow_user_installs", Kind::Bool),
    s("plugins.default_policy", Kind::Choice(&["allow_all", "whitelist", "blacklist"])),
    s("plugins.global_whitelist", Kind::List),
    s("plugins.global_blacklist", Kind::List),
    // Logs and records
    s("logging.level", Kind::Choice(&["error", "warn", "info", "debug", "trace"])),
    s("logging.file_enabled", Kind::Bool),
    s("logging.rotation", Kind::Choice(&["daily", "hourly", "never"])),
    s("logging.max_files", Kind::Int(0, 3650)),
    s("logging.console_enabled", Kind::Bool),
    s("activity.enabled", Kind::Bool),
    s("activity.retention_days", Kind::Int(0, 36500)),
    // Other services
    s("syncthing.enabled", Kind::Bool),
    s("syncthing.url", Kind::Url),
    s("syncthing.api_key", Kind::Secret),
    s("proton.allow_download", Kind::Bool),
    s("notes.notes_folder", Kind::Text(512)),
    // What everyone sees first
    s("ui.default_pane_count", Kind::Int(1, 4)),
    s("ui.default_layout", Kind::Choice(&["quad", "triple", "dual-vertical", "dual-horizontal", "single"])),
    s("ui.default_view_mode", Kind::Choice(&["details", "compact", "grid"])),
    s("ui.show_hidden_files", Kind::Bool),
    s("ui.window_title", Kind::Text(128)),
    s("ui.login_title", Kind::OptText(128)),
    s("desktop.enable_tray", Kind::Bool),
];

/// Settings that can only ever be set in `config.toml` (or the environment), with the reason
pub const FILE_ONLY: &[(&str, &str)] = &[
    ("server.database_path", "Brum needs it to find its database, where the other settings are kept"),
    ("server.allow_web_config_edit", "it decides whether settings may be changed in the web page at all"),
    ("server.jwt_secret", "it is the shared Fleet signing key; keep it out of the database"),
    ("server.jwt_secret_file", "it is the shared Fleet signing key; keep it out of the database"),
    ("server.enable_auth", "switching logins off from a web page is too easy to do by mistake"),
    ("auth.default_admin_user", "it only matters the very first time Brum starts"),
    ("auth.default_admin_pass", "it only matters the very first time Brum starts"),
    ("terminal.default_shell", "it names a program to run"),
    ("terminal.user_shells", "it names programs to run"),
    ("proton.cli_path", "it names a program to run"),
    ("plugins.directory", "it decides where Chewtoys are loaded from"),
    ("plugins.user_directory", "it decides where Chewtoys are loaded from"),
    ("logging.directory", "Brum needs it before it starts"),
];

pub fn spec(key: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.key == key)
}

pub fn file_only_reason(key: &str) -> Option<&'static str> {
    FILE_ONLY.iter().find(|(k, _)| *k == key).map(|(_, r)| *r)
}

/// Environment variables Brum reads for a setting, most important first (matches `load_all`)
fn env_vars_for(key: &str) -> &'static [&'static str] {
    match key {
        "server.port" => &["BRUM_PORT", "CD_PORT", "PORT"],
        "server.host" => &["BRUM_BIND", "BRUM_HOST", "CD_BIND", "CD_HOST", "HOST"],
        "auth.oidc.enabled" => &["BRUM_OIDC_ENABLED", "OIDC_ENABLED"],
        "auth.oidc.issuer_url" => &["BRUM_OIDC_ISSUER_URL", "OIDC_ISSUER_URL"],
        "auth.oidc.client_id" => &["BRUM_OIDC_CLIENT_ID", "OIDC_CLIENT_ID"],
        "auth.oidc.client_secret" => &["BRUM_OIDC_CLIENT_SECRET", "OIDC_CLIENT_SECRET"],
        "auth.oidc.redirect_url" => &["BRUM_OIDC_REDIRECT_URL", "OIDC_REDIRECT_URL"],
        "auth.oidc.provider_name" => &["BRUM_OIDC_PROVIDER_NAME", "OIDC_PROVIDER_NAME"],
        "auth.oidc.admin_group" => &["BRUM_OIDC_ADMIN_GROUP", "OIDC_ADMIN_GROUP"],
        "auth.oidc.force_sso_only" => &["BRUM_OIDC_FORCE_SSO", "OIDC_FORCE_SSO"],
        _ => &[],
    }
}

/// The environment variable that currently decides this setting, if any
pub fn env_override(key: &str) -> Option<&'static str> {
    env_vars_for(key).iter().copied().find(|v| std::env::var(v).map(|x| !x.trim().is_empty()).unwrap_or(false))
}

/// Whether a dotted setting name is written in a parsed config file
pub fn table_has(table: &toml::Table, key: &str) -> bool {
    let mut cur = table;
    let parts: Vec<&str> = key.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        // `[notedog]` from older configs counts as `[notes]`
        let found = cur.get(*part).or_else(|| if i == 0 && *part == "notes" { cur.get("notedog") } else { None });
        match found {
            None => return false,
            Some(_) if i + 1 == parts.len() => return true,
            Some(toml::Value::Table(t)) => cur = t,
            Some(_) => return false,
        }
    }
    false
}

/// The value of a dotted setting in a parsed config file (`[notedog]` counts as `[notes]`)
pub fn table_get<'a>(table: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = table;
    for (i, part) in parts.iter().enumerate() {
        let found = cur.get(*part).or_else(|| if i == 0 && *part == "notes" { cur.get("notedog") } else { None })?;
        if i + 1 == parts.len() {
            return Some(found);
        }
        cur = found.as_table()?;
    }
    None
}

/// Removes a dotted setting from a parsed config file, and any section it leaves empty
pub fn table_remove(table: &mut toml::Table, key: &str) {
    fn go(t: &mut toml::Table, parts: &[&str], top: bool) {
        let name = if top && parts[0] == "notes" && !t.contains_key("notes") { "notedog" } else { parts[0] };
        if parts.len() == 1 {
            t.remove(name);
            return;
        }
        let mut now_empty = false;
        if let Some(toml::Value::Table(inner)) = t.get_mut(name) {
            go(inner, &parts[1..], false);
            now_empty = inner.is_empty();
        }
        if now_empty {
            t.remove(name);
        }
    }
    let parts: Vec<&str> = key.split('.').collect();
    go(table, &parts, true);
}

/// Puts `over` on top of `base`: sections are merged setting by setting, everything else replaced
pub fn deep_merge(base: &mut toml::Table, over: &toml::Table) {
    for (k, v) in over {
        match (base.get_mut(k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => deep_merge(b, o),
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Turns saved settings (dotted names) into the nested shape of a config file
pub fn to_table(saved: &BTreeMap<String, Value>) -> Result<toml::Table, String> {
    let mut root = toml::Table::new();
    for (key, value) in saved {
        if value.is_null() {
            continue;
        }
        let tv = toml::Value::try_from(value).map_err(|e| format!("{key}: {e}"))?;
        let parts: Vec<&str> = key.split('.').collect();
        let mut cur = &mut root;
        for part in &parts[..parts.len() - 1] {
            let entry = cur.entry(part.to_string()).or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if !entry.is_table() {
                *entry = toml::Value::Table(toml::Table::new());
            }
            cur = entry.as_table_mut().expect("just made a table");
        }
        cur.insert(parts[parts.len() - 1].to_string(), tv);
    }
    Ok(root)
}

/// The configuration Brum gets from its defaults, the settings saved in Brum and the config
/// file (in that order, the file winning). Environment variables are applied later by `load_all`.
pub fn compose(file: Option<&toml::Table>, saved: &BTreeMap<String, Value>) -> Result<AppConfig, String> {
    let mut table = to_table(saved)?;
    if let Some(f) = file {
        // `[notedog]` from older configs is the same section as `[notes]`
        if f.contains_key("notedog") && table.contains_key("notes") && !f.contains_key("notes") {
            if let Some(n) = table.remove("notes") {
                let mut f = f.clone();
                if let (Some(toml::Value::Table(old)), toml::Value::Table(ours)) = (f.get_mut("notedog"), n) {
                    let mut merged = ours;
                    deep_merge(&mut merged, old);
                    *old = merged;
                }
                deep_merge(&mut table, &f);
                return finish(table);
            }
        }
        deep_merge(&mut table, f);
    }
    finish(table)
}

fn finish(table: toml::Table) -> Result<AppConfig, String> {
    let mut cfg: AppConfig = toml::Value::Table(table).try_into().map_err(|e: toml::de::Error| e.to_string())?;
    super::ConfigManager::normalize_config_paths(&mut cfg);
    Ok(cfg)
}

/// Reads the settings saved in Brum from an open database (secrets decrypted)
pub fn read_from_conn(conn: &rusqlite::Connection) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT key, value FROM server_settings WHERE key LIKE 'config:%'") else { return out };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) else { return out };
    for (k, v) in rows.flatten() {
        let Some(key) = k.strip_prefix(KEY_PREFIX) else { continue };
        // Settings no longer offered (or never allowed) are ignored, never applied
        let Some(spec) = spec(key) else { continue };
        let Ok(mut value) = serde_json::from_str::<Value>(&v) else {
            tracing::warn!("Ignoring the saved setting {key}: it could not be read");
            continue;
        };
        if matches!(spec.kind, Kind::Secret) {
            if let Some(sealed) = value.as_str() {
                value = Value::String(crate::secrets::open(sealed));
            }
        }
        out.insert(key.to_string(), value);
    }
    out
}

/// Reads the settings saved in Brum from the database file, without changing it.
/// An empty list when there is no database yet.
pub fn read_from_db_file(db_path: &str) -> BTreeMap<String, Value> {
    if db_path.is_empty() || db_path == ":memory:" || !Path::new(db_path).is_file() {
        return BTreeMap::new();
    }
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(conn) = rusqlite::Connection::open_with_flags(db_path, flags) else { return BTreeMap::new() };
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    // Encrypted values need the key next to this database
    crate::secrets::init(db_path);
    read_from_conn(&conn)
}

/// The text stored in the database for a checked value (secrets encrypted)
pub fn stored_text(spec: &Spec, value: &Value) -> String {
    match (spec.kind, value) {
        (Kind::Secret, Value::String(s)) => Value::String(crate::secrets::seal(s)).to_string(),
        _ => value.to_string(),
    }
}

/// What the browser is told about a setting's accepted values
pub fn describe(kind: Kind) -> Value {
    use serde_json::json;
    match kind {
        Kind::Bool => json!({ "type": "bool" }),
        Kind::Int(min, max) => json!({ "type": "number", "min": min, "max": max }),
        Kind::Text(max) => json!({ "type": "text", "max_length": max }),
        Kind::OptText(max) => json!({ "type": "text", "max_length": max, "optional": true }),
        Kind::Choice(c) => json!({ "type": "choice", "choices": c }),
        Kind::Host => json!({ "type": "host" }),
        Kind::Url => json!({ "type": "url", "optional": true }),
        Kind::Folder => json!({ "type": "folder" }),
        Kind::OptFolder => json!({ "type": "folder", "optional": true }),
        Kind::List => json!({ "type": "list" }),
        Kind::Proxies => json!({ "type": "list", "of": "address-range" }),
        Kind::Origins => json!({ "type": "list", "of": "origin" }),
        Kind::Roles => json!({ "type": "list", "of": "role" }),
        Kind::Secret => json!({ "type": "secret" }),
        Kind::Roots => json!({ "type": "folders" }),
    }
}

fn one_line(v: &Value, max: usize) -> Result<String, String> {
    let s = v.as_str().ok_or("must be text")?.trim().to_string();
    if s.chars().count() > max {
        return Err(format!("must be at most {max} characters"));
    }
    if s.chars().any(|c| c.is_control()) {
        return Err("must be a single line of plain text".into());
    }
    Ok(s)
}

fn folder(v: &Value) -> Result<String, String> {
    let s = one_line(v, 1024)?;
    if s.is_empty() {
        return Err("needs a folder".into());
    }
    let expanded = crate::vfs::local::expand_windows_env_vars(&s);
    let expanded = super::expand_tilde_path(&expanded);
    if !expanded.is_dir() {
        return Err(format!("the folder {s} does not exist on this machine"));
    }
    Ok(s)
}

fn list(v: &Value, check: impl Fn(&str) -> Result<(), String>) -> Result<Value, String> {
    let items = v.as_array().ok_or("must be a list")?;
    if items.len() > 200 {
        return Err("has too many entries (at most 200)".into());
    }
    let mut out = Vec::new();
    for item in items {
        let s = one_line(item, 256)?;
        if s.is_empty() {
            continue;
        }
        check(&s).map_err(|e| format!("\"{s}\" {e}"))?;
        if !out.contains(&s) {
            out.push(s);
        }
    }
    Ok(Value::from(out))
}

fn check_role(s: &str) -> Result<(), String> {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        Ok(())
    } else {
        Err("is not a role name (letters, digits, - and _ only)".into())
    }
}

fn check_range(s: &str) -> Result<(), String> {
    let (ip, bits) = match s.split_once('/') {
        Some((ip, bits)) => (ip, Some(bits)),
        None => (s, None),
    };
    let ip: IpAddr = ip.parse().map_err(|_| "is not an IP address or range like 10.0.0.0/8".to_string())?;
    if let Some(bits) = bits {
        let max = if ip.is_ipv4() { 32 } else { 128 };
        match bits.parse::<u8>() {
            Ok(b) if b <= max => {}
            _ => return Err(format!("has a range size that must be 0 to {max}")),
        }
    }
    Ok(())
}

fn check_origin(s: &str) -> Result<(), String> {
    if s == "*" || ((s.starts_with("http://") || s.starts_with("https://")) && !s.contains(char::is_whitespace)) {
        Ok(())
    } else {
        Err("must start with http:// or https:// (or be * for any)".into())
    }
}

/// Checks a value for a setting. Returns the value to save (tidied up), or Null when the
/// value means "not set" (the setting goes back to its default).
pub fn validate(spec: &Spec, v: &Value) -> Result<Value, String> {
    match spec.kind {
        Kind::Bool => v.as_bool().map(Value::Bool).ok_or_else(|| "must be on or off (true or false)".into()),
        Kind::Int(min, max) => match v.as_i64() {
            Some(n) if (min..=max).contains(&n) => Ok(Value::from(n)),
            _ => Err(format!("must be a whole number from {min} to {max}")),
        },
        Kind::Text(max) => one_line(v, max).map(Value::String),
        Kind::OptText(max) => one_line(v, max).map(|s| if s.is_empty() { Value::Null } else { Value::String(s) }),
        Kind::Choice(choices) => {
            let s = v.as_str().map(|s| s.trim().to_lowercase()).ok_or("must be text")?;
            if choices.contains(&s.as_str()) {
                Ok(Value::String(s))
            } else {
                Err(format!("must be one of: {}", choices.join(", ")))
            }
        }
        Kind::Host => {
            let s = one_line(v, 64)?;
            match s.parse::<IpAddr>() {
                Ok(_) => Ok(Value::String(s)),
                Err(_) => Err("must be an IP address: 0.0.0.0 for every network, 127.0.0.1 for this computer only, or one of this machine's addresses".into()),
            }
        }
        Kind::Url => {
            let s = one_line(v, 2048)?;
            if s.is_empty() || ((s.starts_with("http://") || s.starts_with("https://")) && !s.contains(char::is_whitespace)) {
                Ok(Value::String(s))
            } else {
                Err("must be a web address starting with http:// or https://".into())
            }
        }
        Kind::Folder => folder(v).map(Value::String),
        Kind::OptFolder => {
            if one_line(v, 1024)?.is_empty() {
                Ok(Value::Null)
            } else {
                folder(v).map(Value::String)
            }
        }
        Kind::List => list(v, |_| Ok(())),
        Kind::Proxies => list(v, check_range),
        Kind::Origins => list(v, check_origin),
        Kind::Roles => list(v, check_role),
        Kind::Secret => {
            let s = v.as_str().ok_or("must be text")?;
            if s.len() > 4096 || s.chars().any(|c| c.is_control()) {
                return Err("must be a single line of at most 4096 characters".into());
            }
            Ok(Value::String(s.trim().to_string()))
        }
        Kind::Roots => validate_roots(v),
    }
}

fn validate_roots(v: &Value) -> Result<Value, String> {
    let items = v.as_array().ok_or("must be a list of folders")?;
    if items.len() > 100 {
        return Err("has too many folders (at most 100)".into());
    }
    let mut out: Vec<Value> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let n = i + 1;
        let obj = item.as_object().ok_or_else(|| format!("entry {n} is not a folder"))?;
        let path = folder(obj.get("path").unwrap_or(&Value::Null)).map_err(|e| format!("entry {n}: {e}"))?;
        let name = match obj.get("name") {
            Some(x) if !x.is_null() => one_line(x, 64).map_err(|e| format!("entry {n}: the name {e}"))?,
            _ => String::new(),
        };
        let name = if name.is_empty() {
            Path::new(&path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| path.clone())
        } else {
            name
        };
        let mut id = match obj.get("id") {
            Some(x) if !x.is_null() => one_line(x, 64).map_err(|e| format!("entry {n}: the id {e}"))?,
            _ => String::new(),
        };
        if id.is_empty() {
            id = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
        }
        if !id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("entry {n}: the id may only use letters, digits, - and _"));
        }
        let base = id.clone();
        let mut k = 2;
        while ids.contains(&id) {
            id = format!("{base}-{k}");
            k += 1;
        }
        ids.push(id.clone());
        let read_only = match obj.get("read_only") {
            None | Some(Value::Null) => false,
            Some(b) => b.as_bool().ok_or_else(|| format!("entry {n}: read only must be true or false"))?,
        };
        let roles = match obj.get("allowed_roles") {
            None | Some(Value::Null) => Value::Array(vec![]),
            Some(r) => list(r, check_role).map_err(|e| format!("entry {n}: allowed roles {e}"))?,
        };
        out.push(serde_json::json!({ "id": id, "name": name, "path": path, "read_only": read_only, "allowed_roles": roles }));
    }
    Ok(Value::Array(out))
}

/// The value of a dotted setting in a configuration, as JSON
pub fn value_in(config_json: &Value, key: &str) -> Value {
    let mut cur = config_json;
    for part in key.split('.') {
        match cur.get(part) {
            Some(v) => cur = v,
            None => return Value::Null,
        }
    }
    cur.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn saved(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn every_setting_exists_in_the_configuration() {
        // A misspelt key would be saved but silently do nothing
        let cfg = serde_json::to_value(AppConfig::default()).unwrap();
        for spec in SPECS {
            if matches!(spec.kind, Kind::Secret | Kind::OptText(_) | Kind::OptFolder) {
                continue; // not serialized, or absent while unset
            }
            assert!(!value_in(&cfg, spec.key).is_null(), "{} is not a setting Brum knows", spec.key);
        }
        for (key, _) in FILE_ONLY {
            assert!(spec(key).is_none(), "{key} is both file-only and changeable in the page");
        }
    }

    #[test]
    fn saved_settings_apply_and_the_file_wins() {
        let file: toml::Table = toml::from_str("[server]\nport = 4000\n[paranoid]\ntrash_enabled = false\n").unwrap();
        let s = saved(&[("server.port", json!(5000)), ("server.server_name", json!("Attic")), ("fleet.transfer_jobs", json!(5))]);
        let cfg = compose(Some(&file), &s).unwrap();
        assert_eq!(cfg.server.port, 4000, "the file wins");
        assert_eq!(cfg.server.server_name, "Attic", "a saved setting the file does not mention applies");
        assert_eq!(cfg.fleet.transfer_jobs, 5);
        assert!(!cfg.paranoid.trash_enabled, "the rest of the file still applies");
        assert!(cfg.paranoid.confirm_delete, "untouched settings keep their defaults");
        assert!(table_has(&file, "server.port") && !table_has(&file, "server.host"));
    }

    #[test]
    fn nothing_saved_means_the_file_alone() {
        let text = "[server]\nport = 4100\nserver_name = \"Shed\"\n";
        let file: toml::Table = toml::from_str(text).unwrap();
        let ours = compose(Some(&file), &BTreeMap::new()).unwrap();
        let theirs = super::super::ConfigManager::parse_config_str(text).unwrap();
        assert_eq!(serde_json::to_value(&ours).unwrap(), serde_json::to_value(&theirs).unwrap());
    }

    #[test]
    fn old_notedog_section_still_wins() {
        let file: toml::Table = toml::from_str("[notedog]\nnotes_folder = \"/old\"\n").unwrap();
        let s = saved(&[("notes.notes_folder", json!("/new"))]);
        assert_eq!(compose(Some(&file), &s).unwrap().notes.notes_folder, "/old");
        assert!(table_has(&file, "notes.notes_folder"));
    }

    #[test]
    fn settings_are_found_and_removed_in_a_file() {
        let mut t: toml::Table = toml::from_str("[server]\nport = 1\n[notedog]\nnotes_folder = \"/n\"\n[auth.oidc]\nenabled = true\n").unwrap();
        assert_eq!(table_get(&t, "server.port").and_then(|v| v.as_integer()), Some(1));
        assert_eq!(table_get(&t, "notes.notes_folder").and_then(|v| v.as_str()), Some("/n"));
        table_remove(&mut t, "auth.oidc.enabled");
        table_remove(&mut t, "notes.notes_folder");
        assert!(!t.contains_key("auth") && !t.contains_key("notedog"), "empty sections go too: {t:?}");
        assert!(t.contains_key("server"));
    }

    #[test]
    fn values_are_checked() {
        let port = spec("server.port").unwrap();
        assert!(validate(port, &json!(0)).is_err());
        assert!(validate(port, &json!("80")).is_err());
        assert_eq!(validate(port, &json!(8080)).unwrap(), json!(8080));
        let mode = spec("auth.mode").unwrap();
        assert!(validate(mode, &json!("none")).is_err(), "logins cannot be switched off from the page");
        assert_eq!(validate(mode, &json!(" PAM ")).unwrap(), json!("pam"));
        let host = spec("server.host").unwrap();
        assert!(validate(host, &json!("0.0.0.0")).is_ok());
        assert!(validate(host, &json!("bad host; rm")).is_err());
        let proxies = spec("server.trusted_proxies").unwrap();
        assert_eq!(validate(proxies, &json!(["10.0.0.0/8", "::1", ""])).unwrap(), json!(["10.0.0.0/8", "::1"]));
        assert!(validate(proxies, &json!(["10.0.0.0/33"])).is_err());
        let name = spec("server.server_name").unwrap();
        assert!(validate(name, &json!("two\nlines")).is_err());
        let trash = spec("paranoid.custom_trash_dir").unwrap();
        assert_eq!(validate(trash, &json!("")).unwrap(), Value::Null);
        assert!(validate(trash, &json!("/no/such/folder/for/brum")).is_err());
        let dir = tempfile::tempdir().unwrap();
        let roots = spec("storage.roots").unwrap();
        let r = validate(roots, &json!([{ "path": dir.path() }, { "path": dir.path(), "name": dir.path().file_name().unwrap().to_str() }])).unwrap();
        assert_ne!(r[0]["id"], r[1]["id"], "ids are made unique");
        assert_eq!(r[0]["read_only"], json!(false));
        assert!(validate(roots, &json!([{ "path": "/no/such/folder/for/brum" }])).is_err());
    }
}
