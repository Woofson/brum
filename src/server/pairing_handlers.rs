//! Joining the Fleet with a pairing code (#193); the rules are in `crate::tools::pairing`.

use crate::server::middleware::{extract_claims_or_local, require_admin};
use crate::server::AppState;
use crate::tools::pairing::{self, JoinAnswer, JoinRequest, PairedMachine};
use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

type ApiError = (StatusCode, String);

fn bad(e: impl ToString) -> ApiError {
    (StatusCode::BAD_REQUEST, json!({ "error": e.to_string() }).to_string())
}

/// This machine's name as other machines show it
fn own_name(state: &AppState) -> String {
    if !state.config.ui.hostname_badge.trim().is_empty() {
        state.config.ui.hostname_badge.clone()
    } else if !state.config.server.server_name.trim().is_empty() {
        state.config.server.server_name.clone()
    } else {
        crate::server::system_handlers::get_system_hostname()
    }
}

/// An admin asks for a code to add a machine
pub async fn handle_new_pairing_code(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    let code = pairing::new_code(&claims.sub);
    crate::tools::activity::record(&claims.sub, "pairing code made", "Fleet", "valid 10 minutes, once", true);
    Ok(Json(json!({ "code": pairing::display(&code), "expires_in": pairing::CODE_LIFETIME.as_secs(), "name": own_name(&state) })))
}

/// Whether the code was used yet (the page shows when the machine has joined)
pub async fn handle_pairing_code_status(State(state): State<AppState>, headers: HeaderMap, AxumPath(code): AxumPath<String>) -> Result<Json<pairing::CodeStatus>, ApiError> {
    require_admin(&extract_claims_or_local(&state, &headers)?)?;
    Ok(Json(pairing::status(&code)))
}

/// A new machine joins with a code (no login: the code is the permission)
pub async fn handle_pairing_join(State(state): State<AppState>, Json(req): Json<JoinRequest>) -> Result<Json<JoinAnswer>, ApiError> {
    let name = req.name.trim().chars().take(80).collect::<String>();
    if name.is_empty() {
        return Err(bad("The new machine sent no name"));
    }
    // Its addresses are used for Fleet requests later, so they get the Fleet proxy's checks
    let mut urls = Vec::new();
    for u in req.urls.iter().take(8) {
        let u = u.trim().trim_end_matches('/').to_string();
        match crate::server::middleware::extract_remote_scheme_host(&u) {
            Some((_, host)) if !crate::server::middleware::is_prohibited_remote_host(&host, state.config.storage.allow_remote_loopback) => urls.push(u),
            _ => {}
        }
    }
    if urls.is_empty() {
        return Err(bad("The new machine sent no address this Brum can use"));
    }
    let by = pairing::redeem(&req.code, &name).map_err(|e| (StatusCode::FORBIDDEN, json!({ "error": e }).to_string()))?;
    let existing = pairing::list().into_iter().find(|m| m.urls.iter().any(|u| urls.contains(u)));
    let taken: Vec<String> = crate::server::fleet::all_nodes(&state).into_iter().map(|n| n.id).collect();
    let id = existing.map(|m| m.id).unwrap_or_else(|| pairing::make_id(&name, &taken));
    pairing::add(PairedMachine { id, name: name.clone(), urls: urls.clone(), os: req.os.clone(), role: "joined".into(), paired_at: chrono::Utc::now().timestamp(), paired_by: by.clone(), ..Default::default() })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e }).to_string()))?;
    crate::tools::activity::record(&by, "machine joined the Fleet", &name, &urls.join(", "), true);
    tracing::info!("Fleet: {} joined with a pairing code made by {} ({})", name, by, urls.join(", "));
    Ok(Json(JoinAnswer { secret: state.auth.signing_secret().to_string(), name: own_name(&state), os: std::env::consts::OS.to_string(), version: env!("CARGO_PKG_VERSION").to_string() }))
}

#[derive(Deserialize)]
pub struct JoinFromHereRequest {
    /// The main Brum's address, as typed
    pub main_url: String,
    pub code: String,
}

/// On the new machine (Settings): join the Fleet of another Brum. Brum restarts afterwards.
pub async fn handle_join_fleet(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<JoinFromHereRequest>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    if !state.config.server.jwt_secret_source.is_empty() {
        return Err(bad(format!(
            "This machine already uses a shared secret ({}). Remove it to join with a code, or give it the Fleet's secret instead.",
            state.config.server.jwt_secret_source
        )));
    }
    let main_url = req.main_url.trim().to_string();
    if main_url.is_empty() || normalize_len(&req.code) != 8 {
        return Err(bad("Type the main Brum's address and the 8-character code it shows"));
    }
    let urls = pairing::own_addresses(state.config.server.port, &main_url);
    if urls.is_empty() {
        return Err(bad("This machine has no address other machines could use"));
    }
    let listens_locally = crate::instance::is_loopback_host(&state.config.server.host);
    let (name, db_path, code) = (own_name(&state), state.auth.db_path().to_string(), req.code.clone());
    let joined = tokio::task::spawn_blocking(move || pairing::join(&main_url, &code, &name, urls, &db_path))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    match joined {
        Ok(main_name) => {
            crate::tools::activity::record(&claims.sub, "joined a Fleet", &main_name, &req.main_url, true);
            Ok(Json(json!({
                "main": main_name,
                "restart": true,
                "warning": listens_locally.then_some("This Brum only listens on this machine (host = 127.0.0.1), so the other machines cannot reach it yet. Set host = \"0.0.0.0\" in config.toml."),
            })))
        }
        Err(e) => {
            crate::tools::activity::record(&claims.sub, "joined a Fleet", &req.main_url, &e, false);
            Err(bad(e))
        }
    }
}

fn normalize_len(code: &str) -> usize {
    pairing::normalize(code).len()
}

pub async fn handle_list_paired(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Vec<PairedMachine>>, ApiError> {
    extract_claims_or_local(&state, &headers)?;
    Ok(Json(pairing::list()))
}

/// Removes a paired machine from this Brum's list. It keeps the Fleet secret, so it can still
/// sign people in here until the secret is changed.
pub async fn handle_remove_paired(State(state): State<AppState>, headers: HeaderMap, AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    let name = pairing::list().into_iter().find(|m| m.id == id).map(|m| m.name).unwrap_or_else(|| id.clone());
    let removed = pairing::remove(&id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    crate::tools::activity::record(&claims.sub, "paired machine removed", &name, "", removed);
    Ok(Json(json!({ "success": removed })))
}

// ---------- Machines added by hand (#195) ----------

/// A machine added by hand, as the Manage Fleet form sends it
#[derive(Deserialize)]
pub struct MachineForm {
    pub name: String,
    pub urls: Vec<String>,
    /// The API token: empty for the shared Fleet secret; "********" keeps the saved one
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub details: pairing::MachineDetails,
}

const TOKEN_MASK: &str = "********";

fn one_line(s: &str, max: usize, what: &str) -> Result<String, ApiError> {
    let s = s.trim();
    if s.chars().count() > max || s.chars().any(|c| c.is_control()) {
        return Err((StatusCode::BAD_REQUEST, format!("The {what} must be one line of at most {max} characters.")));
    }
    Ok(s.to_string())
}

/// Checks the form and tidies it up
fn checked(form: &MachineForm) -> Result<(String, Vec<String>, pairing::MachineDetails), ApiError> {
    let name = one_line(&form.name, 64, "name")?;
    let mut urls: Vec<String> = Vec::new();
    for u in &form.urls {
        let u = one_line(u, 512, "address")?.trim_end_matches('/').to_string();
        if u.is_empty() {
            continue;
        }
        if !(u.starts_with("http://") || u.starts_with("https://")) || u.contains(char::is_whitespace) {
            return Err((StatusCode::BAD_REQUEST, format!("{u} is not a web address; it starts with http:// or https://, like http://nas:3140.")));
        }
        if !urls.iter().any(|x| x.eq_ignore_ascii_case(&u)) {
            urls.push(u);
        }
    }
    if urls.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "The machine needs an address, like http://nas:3140.".into()));
    }
    if urls.len() > 10 {
        return Err((StatusCode::BAD_REQUEST, "At most 10 addresses per machine.".into()));
    }
    let d = &form.details;
    let start_path = match &d.start_path {
        Some(p) => Some(one_line(p, 1024, "start folder")?).filter(|p| !p.is_empty()),
        None => None,
    };
    let mut start_paths = std::collections::BTreeMap::new();
    for (system, path) in &d.start_paths {
        let system = system.trim().to_lowercase();
        if system.is_empty() || system.len() > 16 || !system.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err((StatusCode::BAD_REQUEST, "A start folder is for a system such as linux, windows or macos.".into()));
        }
        let path = one_line(path, 1024, "start folder")?;
        if !path.is_empty() {
            start_paths.insert(system, path);
        }
    }
    let color_accent = match &d.color_accent {
        Some(c) => Some(one_line(c, 32, "colour")?).filter(|c| !c.is_empty()),
        None => None,
    };
    let mut tags = Vec::new();
    for t in &d.tags {
        let t = one_line(t, 32, "group")?;
        if !t.is_empty() && !tags.contains(&t) {
            tags.push(t);
        }
    }
    if tags.len() > 20 {
        return Err((StatusCode::BAD_REQUEST, "At most 20 groups per machine.".into()));
    }
    Ok((name, urls, pairing::MachineDetails { start_path, start_paths, read_only: d.read_only, color_accent, tags }))
}

fn checked_token(t: &Option<String>, keep: Option<String>) -> Result<Option<String>, ApiError> {
    match t.as_deref().map(str::trim) {
        Some(TOKEN_MASK) => Ok(keep),
        None | Some("") => Ok(None),
        Some(t) if t.len() > 4096 || t.chars().any(|c| c.is_control() || c.is_whitespace()) => Err((StatusCode::BAD_REQUEST, "The token must be one word without spaces.".into())),
        Some(t) => Ok(Some(t.to_string())),
    }
}

/// Another machine already listed under one of these addresses (not `except`)
fn address_taken(state: &AppState, urls: &[String], except: Option<&str>) -> Option<String> {
    crate::server::fleet::all_nodes(state)
        .into_iter()
        .filter(|n| Some(n.id.as_str()) != except)
        .find(|n| n.addresses().iter().any(|a| urls.iter().any(|u| u.eq_ignore_ascii_case(a))))
        .map(|n| n.name)
}

pub async fn handle_add_machine(State(state): State<AppState>, headers: HeaderMap, Json(form): Json<MachineForm>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    let (name, urls, details) = checked(&form)?;
    if let Some(other) = address_taken(&state, &urls, None) {
        return Err((StatusCode::CONFLICT, format!("{other} already uses that address.")));
    }
    let taken: Vec<String> = crate::server::fleet::all_nodes(&state).into_iter().map(|n| n.id).collect();
    let m = PairedMachine {
        id: pairing::make_id(&name, &taken),
        name: name.clone(),
        urls,
        role: "manual".into(),
        paired_at: chrono::Utc::now().timestamp(),
        paired_by: claims.sub.clone(),
        details,
        token: checked_token(&form.token, None)?,
        ..Default::default()
    };
    let id = m.id.clone();
    pairing::add(m).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    crate::tools::activity::record(&claims.sub, "Fleet machine added", &name, "", true);
    Ok(Json(json!({ "success": true, "id": id })))
}

pub async fn handle_edit_machine(State(state): State<AppState>, headers: HeaderMap, AxumPath(id): AxumPath<String>, Json(form): Json<MachineForm>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    let Some(old) = pairing::list().into_iter().find(|m| m.id == id) else {
        return Err((StatusCode::NOT_FOUND, "That machine is not in this Brum's list.".into()));
    };
    if old.role != "manual" {
        return Err((StatusCode::BAD_REQUEST, "Machines paired with a code are changed by pairing them again.".into()));
    }
    if state.config.fleet.nodes.iter().any(|n| n.id == id) && !pairing::moved_from_file(&id) {
        return Err((StatusCode::CONFLICT, "This machine is also written in config.toml, which wins. Change it there, or remove it from the file.".into()));
    }
    let (name, urls, details) = checked(&form)?;
    if let Some(other) = address_taken(&state, &urls, Some(&id)) {
        return Err((StatusCode::CONFLICT, format!("{other} already uses that address.")));
    }
    let m = PairedMachine { name: name.clone(), urls, details, token: checked_token(&form.token, old.token.clone())?, ..old };
    pairing::add(m).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    crate::server::fleet::forget_live_address(&id);
    crate::tools::activity::record(&claims.sub, "Fleet machine changed", &name, "", true);
    Ok(Json(json!({ "success": true, "id": id })))
}
