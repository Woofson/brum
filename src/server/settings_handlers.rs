//! Server settings for admins (#195): read them, and save the ones that are not already set in
//! config.toml or by an environment variable. Saved settings take effect when Brum restarts.

use crate::config::saved::{self, Kind};
use crate::config::{AppConfig, ConfigManager};
use crate::server::middleware::{extract_claims_or_local, require_admin};
use crate::server::AppState;
use axum::{extract::State, http::{HeaderMap, StatusCode}, response::Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

type ApiError = (StatusCode, String);

/// What a secret looks like in the page when it has a value; sending it back keeps the value
pub const SECRET_MASK: &str = "********";

/// Why settings cannot be changed in the page right now, if they cannot
fn locked_reason(state: &AppState, file: Option<&toml::Table>) -> Option<String> {
    let off_in_file = file
        .and_then(|f| f.get("server")?.get("allow_web_config_edit")?.as_bool())
        .map(|b| !b)
        .unwrap_or(false);
    if !state.config.server.allow_web_config_edit || off_in_file {
        Some("Changing settings in the web page is switched off on this machine (allow_web_config_edit = false in config.toml). Change them in the file instead.".into())
    } else {
        None
    }
}

fn file_label(state: &AppState) -> String {
    state.config.server.config_file.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "config.toml".into())
}

/// The configuration Brum will start with next time (without environment variables)
fn next_start(file: Option<&toml::Table>, saved_now: &std::collections::BTreeMap<String, Value>) -> Result<AppConfig, String> {
    saved::compose(file, saved_now)
}

pub async fn handle_get_server_settings(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    require_admin(&extract_claims_or_local(&state, &headers)?)?;
    let file = ConfigManager::read_config_file_table(state.config.server.config_file.as_deref());
    let saved_now = state.auth.saved_config_settings();
    let next = next_start(file.as_ref(), &saved_now).unwrap_or_else(|_| (*state.config).clone());
    let next_json = serde_json::to_value(&next).unwrap_or(Value::Null);
    let running_json = serde_json::to_value(&*state.config).unwrap_or(Value::Null);

    let mut settings = Vec::new();
    for spec in saved::SPECS {
        let env = saved::env_override(spec.key);
        let in_file = file.as_ref().map(|f| saved::table_has(f, spec.key)).unwrap_or(false);
        let source = if env.is_some() {
            "env"
        } else if in_file {
            "file"
        } else if saved_now.contains_key(spec.key) {
            "brum"
        } else {
            "default"
        };
        let (value, restart_needed) = if matches!(spec.kind, Kind::Secret) {
            // Only whether it has a value is shown; the value itself never leaves the server
            let shown_cfg = if env.is_some() { &*state.config } else { &next };
            let set = !secret_of(shown_cfg, spec.key).is_empty();
            let pending = env.is_none() && secret_of(&next, spec.key) != secret_of(&state.config, spec.key);
            (Value::String(if set { SECRET_MASK.into() } else { String::new() }), pending)
        } else if env.is_some() {
            (saved::value_in(&running_json, spec.key), false)
        } else {
            let v = saved::value_in(&next_json, spec.key);
            let pending = v != saved::value_in(&running_json, spec.key);
            (v, pending)
        };
        let mut entry = json!({
            "key": spec.key,
            "kind": saved::describe(spec.kind),
            "value": value,
            "source": source,
            "restart_needed": restart_needed,
        });
        if let Some(var) = env {
            entry["env_var"] = json!(var);
        }
        settings.push(entry);
    }
    let file_only: Vec<Value> = saved::FILE_ONLY.iter().map(|(k, why)| json!({ "key": k, "why": why })).collect();
    Ok(Json(json!({
        "editable": locked_reason(&state, file.as_ref()).is_none(),
        "locked_reason": locked_reason(&state, file.as_ref()),
        "config_file": state.config.server.config_file.as_ref().map(|p| p.display().to_string()),
        "settings": settings,
        "file_only": file_only,
        "running": { "host": state.config.server.host, "port": state.config.server.port },
        "movable": file.as_ref().map(|f| movable(f).len()).unwrap_or(0),
        "trial_seconds_left": trial_seconds_left(&state),
    })))
}

fn secret_of<'a>(cfg: &'a AppConfig, key: &str) -> &'a str {
    match key {
        "auth.oidc.client_secret" => &cfg.auth.oidc.client_secret,
        "syncthing.api_key" => &cfg.syncthing.api_key,
        _ => "",
    }
}

#[derive(Deserialize)]
pub struct SaveSettingsRequest {
    /// Setting name → new value, or null to go back to the default
    pub changes: Map<String, Value>,
}

pub async fn handle_save_server_settings(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<SaveSettingsRequest>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    let file = ConfigManager::read_config_file_table(state.config.server.config_file.as_deref());
    if let Some(why) = locked_reason(&state, file.as_ref()) {
        return Err((StatusCode::FORBIDDEN, why));
    }
    if req.changes.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Nothing to save".into()));
    }
    if req.changes.len() > saved::SPECS.len() {
        return Err((StatusCode::BAD_REQUEST, "Too many changes at once".into()));
    }

    let mut saved_now = state.auth.saved_config_settings();
    let mut writes: Vec<(String, Option<String>)> = Vec::new();
    for (key, value) in &req.changes {
        let Some(spec) = saved::spec(key) else {
            return Err(match saved::file_only_reason(key) {
                Some(why) => (StatusCode::BAD_REQUEST, format!("{key} can only be set in config.toml: {why}.")),
                None => (StatusCode::BAD_REQUEST, format!("Brum has no setting called {key} that can be changed here.")),
            });
        };
        if value.is_null() {
            // Back to the default; always allowed, it only removes what was saved in Brum
            saved_now.remove(key);
            writes.push((key.clone(), None));
            continue;
        }
        if let Some(var) = saved::env_override(key) {
            return Err((StatusCode::CONFLICT, format!("{key} is set by the environment variable {var}, which always wins. Change it there.")));
        }
        if file.as_ref().map(|f| saved::table_has(f, key)).unwrap_or(false) {
            return Err((StatusCode::CONFLICT, format!("{key} is set in {}, which always wins. Change it there, or remove it from the file to manage it here.", file_label(&state))));
        }
        if matches!(spec.kind, Kind::Secret) && value.as_str() == Some(SECRET_MASK) {
            continue; // the page sent back the mask: keep the secret as it is
        }
        let checked = saved::validate(spec, value).map_err(|e| (StatusCode::BAD_REQUEST, format!("{key} {e}.")))?;
        if checked.is_null() {
            saved_now.remove(key);
            writes.push((key.clone(), None));
        } else {
            writes.push((key.clone(), Some(saved::stored_text(spec, &checked))));
            saved_now.insert(key.clone(), checked);
        }
    }
    if writes.is_empty() {
        return Ok(Json(json!({ "success": true, "saved": 0, "restart_needed": false, "message": "Nothing changed." })));
    }

    // Brum must still be able to start with everything together
    let next = next_start(file.as_ref(), &saved_now).map_err(|e| (StatusCode::BAD_REQUEST, format!("These settings do not fit together: {e}")))?;

    // A new address or port must be one Brum can listen on, and is tried out after the restart
    let running = (state.config.server.host.as_str(), state.config.server.port);
    let network_change = (next.server.host != running.0 || next.server.port != running.1)
        && writes.iter().any(|(k, _)| NETWORK_KEYS.contains(&k.as_str()));
    if network_change {
        can_listen(&next.server.host, next.server.port, running.1).map_err(|e| (StatusCode::BAD_REQUEST, format!("The new address cannot work: {e}.")))?;
    }
    if network_change {
        let keys: Vec<&str> = writes.iter().map(|(k, _)| k.as_str()).filter(|k| NETWORK_KEYS.contains(k)).collect();
        note_network_change(&state, &keys, &claims.sub);
    }
    state.auth.save_config_settings(&writes).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not save the settings: {e}")))?;

    let keys: Vec<&str> = writes.iter().map(|(k, _)| k.as_str()).collect();
    crate::tools::activity::record(&claims.sub, "server settings changed", &keys.join(", "), "", true);
    tracing::info!("{} changed server settings: {}", claims.sub, keys.join(", "));
    Ok(Json(json!({
        "success": true,
        "saved": writes.len(),
        "restart_needed": true,
        "network_change": network_change.then(|| json!({ "host": next.server.host, "port": next.server.port, "seconds": TRIAL_SECONDS })),
        "message": "Saved. The changes take effect when Brum restarts.",
    })))
}

// ---------------------------------------------------------------------------------------------
// Trying out a new address or port (#195): after the restart that applies it, an admin has two
// minutes to say "keep it" from the new address. If nobody does (for example because Brum can no
// longer be reached), the previous address and port come back and Brum restarts with them.

/// Database key of the pending trial (not a `config:` key, so it is never used as a setting)
const TRIAL_KEY: &str = "config_trial";
/// How long an admin has to keep the new address after the restart
pub const TRIAL_SECONDS: i64 = 120;
/// A trial saved longer ago than this when Brum starts was not tried at once (Brum was restarted
/// some other way, much later): the change is simply kept
const TRIAL_FRESH_SECONDS: i64 = 15 * 60;
/// The settings that can lock everyone out
const NETWORK_KEYS: &[&str] = &["server.host", "server.port"];

#[derive(Serialize, Deserialize, Default)]
struct Trial {
    saved_at: i64,
    /// When the restart that applies it happened (None: not yet)
    started_at: Option<i64>,
    /// What was stored for each network setting before (None: nothing, the default or the file)
    previous: std::collections::BTreeMap<String, Option<String>>,
    by: String,
}

fn read_trial(state: &AppState) -> Option<Trial> {
    serde_json::from_str(&state.auth.get_server_setting(TRIAL_KEY)?).ok()
}

fn write_trial(state: &AppState, t: &Trial) {
    if let Ok(text) = serde_json::to_string(t) {
        let _ = state.auth.set_server_setting(TRIAL_KEY, &text);
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Seconds left to keep the new address, while a trial is running
fn trial_seconds_left(state: &AppState) -> Option<i64> {
    let t = read_trial(state)?;
    Some((t.started_at? + TRIAL_SECONDS - now()).max(0))
}

/// Remembers the address and port as they were, before a change to them is saved
fn note_network_change(state: &AppState, keys: &[&str], who: &str) {
    let mut t = read_trial(state).filter(|t| t.started_at.is_none()).unwrap_or_default();
    for key in keys {
        // The first value noted is the one that last worked
        t.previous.entry(key.to_string()).or_insert_with(|| state.auth.get_server_setting(&format!("{}{}", saved::KEY_PREFIX, key)));
    }
    t.saved_at = now();
    t.by = who.to_string();
    write_trial(state, &t);
}

/// Puts the previous address and port back
fn undo_trial(state: &AppState, t: &Trial) {
    let writes: Vec<(String, Option<String>)> = t.previous.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Err(e) = state.auth.save_config_settings(&writes) {
        tracing::error!("Could not put the previous address and port back: {e}");
        return;
    }
    let _ = state.auth.delete_server_setting(TRIAL_KEY);
    crate::tools::activity::record(&t.by, "server settings undone", &NETWORK_KEYS.join(", "), "the new address was not kept within two minutes", true);
}

/// Runs once when Brum starts: watches a new address or port that is being tried out
pub fn start_trial_watch(state: AppState) {
    let Some(mut t) = read_trial(&state) else { return };
    if t.started_at.is_some() {
        // Brum stopped while the previous trial was running: it was never kept
        tracing::warn!("The new address or port was not kept before Brum stopped; the previous ones come back now");
        undo_trial(&state, &t);
        crate::server::system_handlers::restart_brum_soon();
        return;
    }
    if now() - t.saved_at > TRIAL_FRESH_SECONDS {
        let _ = state.auth.delete_server_setting(TRIAL_KEY);
        return;
    }
    t.started_at = Some(now());
    write_trial(&state, &t);
    tracing::warn!("Trying out the new address or port: an admin has {TRIAL_SECONDS} seconds to keep it in Server settings, or the previous ones come back");
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    handle.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(TRIAL_SECONDS as u64)).await;
        if let Some(t) = read_trial(&state) {
            tracing::warn!("Nobody kept the new address or port within {TRIAL_SECONDS} seconds; putting the previous ones back and restarting");
            undo_trial(&state, &t);
            crate::server::system_handlers::restart_brum_soon();
        }
    });
}

/// "Keep it": the new address and port stay
pub async fn handle_keep_network_settings(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    if read_trial(&state).is_none() {
        return Ok(Json(json!({ "success": true, "message": "Nothing to keep." })));
    }
    state.auth.delete_server_setting(TRIAL_KEY).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::tools::activity::record(&claims.sub, "server settings kept", &NETWORK_KEYS.join(", "), "", true);
    Ok(Json(json!({ "success": true, "message": "The new address and port are kept." })))
}

/// For the page at start-up: whether an admin should be asked to keep a new address
pub async fn handle_network_trial(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    require_admin(&extract_claims_or_local(&state, &headers)?)?;
    Ok(Json(json!({ "seconds_left": trial_seconds_left(&state) })))
}

/// Whether Brum could listen on this address and port (checked before saving a change to them)
fn can_listen(host: &str, port: u16, running_port: u16) -> Result<(), String> {
    let ip: std::net::IpAddr = host.parse().map_err(|_| format!("{host} is not an IP address"))?;
    if !ip.is_unspecified() && std::net::TcpListener::bind((ip, 0)).is_err() {
        return Err(format!("this machine has no network address {host}"));
    }
    // The port Brum itself uses now is free for it once it restarts
    if port != running_port {
        if let Err(e) = std::net::TcpListener::bind((ip, port)) {
            return Err(format!("port {port} cannot be used ({e}); another program may be using it"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Move my settings into Brum (#195): the settings written in config.toml that the page can
// manage move into Brum's database, and the file keeps only the rest. A copy of the old file
// is kept next to it. Brum works exactly the same afterwards; that is checked before anything
// is changed.

/// The settings in the config file that could move into Brum
fn movable(file: &toml::Table) -> Vec<&'static saved::Spec> {
    saved::SPECS.iter().filter(|s| saved::table_has(file, s.key) && saved::env_override(s.key).is_none()).collect()
}

/// Every setting in a parsed config file, as dotted names (lists count as one setting)
fn leaf_keys(table: &toml::Table) -> Vec<String> {
    fn go(t: &toml::Table, prefix: &str, out: &mut Vec<String>) {
        for (k, v) in t {
            if k.contains('.') {
                continue; // quoted names with dots: left alone
            }
            let name = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            match v {
                toml::Value::Table(inner) => go(inner, &name, out),
                _ => out.push(name),
            }
        }
    }
    let mut out = Vec::new();
    go(table, "", &mut out);
    out
}

/// The text at the top of a config file after its settings moved into Brum
const MOVED_HEADER: &str = "# Brum settings file\n#\n# Most settings are now in Brum itself: Admin → Server Settings.\n# This file keeps only what Brum needs before it starts, and what differs from\n# Brum's defaults that Server Settings does not offer. Anything here wins over\n# Server Settings. The file as it was before is kept next to this one\n# (.before-move).\n\n";

/// A free name for the copy of the old file
fn backup_path(path: &std::path::Path) -> std::path::PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "config.toml".into());
    let day = chrono::Local::now().format("%Y-%m-%d");
    let mut candidate = path.with_file_name(format!("{name}.before-move-{day}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{name}.before-move-{day}-{n}"));
        n += 1;
    }
    candidate
}

/// Replaces a file in one step, keeping its permissions
fn replace_file(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_file_name(format!(".{}.new", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    std::fs::write(&tmp, text)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub async fn handle_move_from_file(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    let Some(path) = state.config.server.config_file.clone() else {
        return Err((StatusCode::BAD_REQUEST, "Brum started without a config file, so there is nothing to move.".into()));
    };
    let text = std::fs::read_to_string(&path).map_err(|e| (StatusCode::BAD_REQUEST, format!("Could not read {}: {e}", path.display())))?;
    let file = ConfigManager::parse_table_str(&text).ok_or((StatusCode::BAD_REQUEST, format!("{} could not be read as a settings file.", path.display())))?;
    if let Some(why) = locked_reason(&state, Some(&file)) {
        return Err((StatusCode::FORBIDDEN, why));
    }

    // What moves, and what stays in the file (with the reason)
    let mut moving: Vec<(&'static saved::Spec, Value)> = Vec::new();
    let mut staying: Vec<Value> = Vec::new();
    for spec in movable(&file) {
        let Some(raw) = saved::table_get(&file, spec.key) else { continue };
        let value = serde_json::to_value(raw).unwrap_or(Value::Null);
        match saved::validate(spec, &value) {
            // The value is moved exactly as written, so Brum gets exactly what it got from the file
            Ok(_) => moving.push((spec, value)),
            Err(e) => staying.push(json!({ "key": spec.key, "why": format!("{} {e}", spec.key) })),
        }
    }
    if moving.is_empty() {
        return Ok(Json(json!({ "success": true, "moved": [], "stayed": staying, "message": "There is nothing in the file to move." })));
    }

    let mut slim = file.clone();
    for (spec, _) in &moving {
        saved::table_remove(&mut slim, spec.key);
    }
    let saved_before = state.auth.saved_config_settings();
    let mut saved_after = saved_before.clone();
    for (spec, value) in &moving {
        saved_after.insert(spec.key.to_string(), value.clone());
    }
    // Brum must work exactly the same afterwards
    let before = saved::compose(Some(&file), &saved_before).map_err(|e| (StatusCode::BAD_REQUEST, format!("The current settings cannot be read: {e}")))?;
    // Whatever is the same as Brum's own default does not need to stay in the file either
    let target = serde_json::to_value(&before).ok();
    for leaf in leaf_keys(&slim) {
        let mut trial = slim.clone();
        saved::table_remove(&mut trial, &leaf);
        if saved::compose(Some(&trial), &saved_after).ok().and_then(|c| serde_json::to_value(&c).ok()) == target {
            slim = trial;
        }
    }
    let after = saved::compose(Some(&slim), &saved_after).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Moving would break the settings ({e}); nothing was changed.")))?;
    if serde_json::to_value(&before).ok() != serde_json::to_value(&after).ok() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Moving would change how Brum works, so nothing was changed. Please report this.".into()));
    }

    // Brum must be allowed to change the file before anything happens
    if std::fs::OpenOptions::new().append(true).open(&path).is_err() {
        return Err((StatusCode::FORBIDDEN, format!("Brum is not allowed to change {}. Nothing was changed. Brum may run as a different account than the file's owner, or the file may be mounted read-only (Docker).", path.display())));
    }
    let backup = backup_path(&path);
    std::fs::copy(&path, &backup).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not keep a copy of the old file ({e}); nothing was changed.")))?;

    let previous: Vec<(String, Option<String>)> = moving.iter().map(|(s, _)| (s.key.to_string(), state.auth.get_server_setting(&format!("{}{}", saved::KEY_PREFIX, s.key)))).collect();
    let writes: Vec<(String, Option<String>)> = moving.iter().map(|(s, v)| (s.key.to_string(), Some(saved::stored_text(s, v)))).collect();
    state.auth.save_config_settings(&writes).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not save the settings ({e}); nothing was changed.")))?;

    let body = toml::to_string_pretty(&slim).unwrap_or_default();
    if let Err(e) = replace_file(&path, &format!("{MOVED_HEADER}{body}")) {
        let _ = state.auth.save_config_settings(&previous);
        let _ = std::fs::remove_file(&backup);
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Could not write {} ({e}); nothing was changed.", path.display())));
    }

    let keys: Vec<&str> = moving.iter().map(|(s, _)| s.key).collect();
    crate::tools::activity::record(&claims.sub, "server settings moved into Brum", &path.display().to_string(), &format!("{} settings; old file kept as {}", keys.len(), backup.display()), true);
    tracing::info!("{} moved {} settings from {} into Brum (old file kept as {})", claims.sub, keys.len(), path.display(), backup.display());
    Ok(Json(json!({
        "success": true,
        "moved": keys,
        "stayed": staying,
        "backup": backup.display().to_string(),
        "config_file": path.display().to_string(),
        "message": format!("{} settings moved into Brum. Brum works exactly as before; nothing needs a restart.", keys.len()),
    })))
}
