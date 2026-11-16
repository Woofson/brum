//! Server settings for admins (#195): read them, and save the ones that are not already set in
//! config.toml or by an environment variable. Saved settings take effect when Brum restarts.

use crate::config::saved::{self, Kind};
use crate::config::{AppConfig, ConfigManager};
use crate::server::middleware::{extract_claims_or_local, require_admin};
use crate::server::AppState;
use axum::{extract::State, http::{HeaderMap, StatusCode}, response::Json};
use serde::Deserialize;
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
    next_start(file.as_ref(), &saved_now).map_err(|e| (StatusCode::BAD_REQUEST, format!("These settings do not fit together: {e}")))?;
    state.auth.save_config_settings(&writes).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not save the settings: {e}")))?;

    let keys: Vec<&str> = writes.iter().map(|(k, _)| k.as_str()).collect();
    crate::tools::activity::record(&claims.sub, "server settings changed", &keys.join(", "), "", true);
    tracing::info!("{} changed server settings: {}", claims.sub, keys.join(", "));
    Ok(Json(json!({
        "success": true,
        "saved": writes.len(),
        "restart_needed": true,
        "message": "Saved. The changes take effect when Brum restarts.",
    })))
}
