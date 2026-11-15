//! Starting copies and moves between Fleet machines (#190); the work itself is in
//! `crate::tools::transfer`.
//!
//! The page asks the Brum it is connected to. When both machines are other Fleet machines, that
//! Brum first asks the receiving machine to fetch the files itself (direct). When the two cannot
//! reach each other, or the receiving machine is older, it copies between them itself (relay).
//! Either way the data never passes through the browser, and the job shows in this Brum's task
//! list, where it can be paused, stopped and retried.

use crate::server::fleet::{choose_live_address, ResolvedFleetTarget};
use crate::server::middleware::{extract_claims_or_local, require_write, validate_path_access};
use crate::server::AppState;
use crate::tools::sync::fleet_store::FleetStore;
use crate::tools::transfer::{ItemSpec, Job, Place};
use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type ApiError = (StatusCode, String);

/// Version of the copy-job protocol; machines without `/api/transfer/info` get the old
/// browser copy for now
pub const TRANSFER_PROTOCOL: u64 = 1;

pub async fn handle_transfer_info(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    extract_claims_or_local(&state, &headers)?;
    Ok(Json(json!({ "protocol": TRANSFER_PROTOCOL, "version": env!("CARGO_PKG_VERSION") })))
}

#[derive(Deserialize, Clone, Debug)]
pub struct SideRequest {
    /// "local" (this Brum), a Fleet node id, or "remote" (an address given with `url`, used when
    /// one Brum hands a direct copy to another)
    pub node: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    pub folder: String,
    /// Only for the source: the chosen files and folders, by name, inside `folder`
    #[serde(default)]
    pub items: Vec<ItemSpec>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TransferRequest {
    /// "copy" or "move"
    pub action: String,
    /// Check every file with a checksum (Security & Integrity Checks)
    #[serde(default = "yes")]
    pub verify: bool,
    pub source: SideRequest,
    pub dest: SideRequest,
    /// Let the receiving machine fetch the files itself when both are other machines
    #[serde(default = "yes")]
    pub direct: bool,
}

fn yes() -> bool {
    true
}

/// One side, ready to use
struct Side {
    label: String,
    /// The node's address and the login to use there, for other machines
    remote: Option<(String, Option<String>)>,
    folder: String,
}

fn fallback(reason: String) -> ApiError {
    (StatusCode::CONFLICT, json!({ "fallback": true, "reason": reason }).to_string())
}

fn bad(e: impl ToString) -> ApiError {
    (StatusCode::BAD_REQUEST, e.to_string())
}

/// Items are names inside the source folder, or paths below it written with `/` (from Compare);
/// never anything that leaves the folder
fn check_items(items: &[ItemSpec]) -> Result<(), ApiError> {
    if items.is_empty() {
        return Err(bad("Nothing chosen to copy"));
    }
    for i in items {
        let n = i.name.as_str();
        let bad_part = |p: &str| p.is_empty() || p == "." || p == ".." || (cfg!(windows) && p.contains(':'));
        if n.contains('\\') || n.contains('\0') || n.split('/').any(bad_part) {
            return Err(bad(format!("Invalid name: {n}")));
        }
    }
    Ok(())
}

fn join(folder: &str, name: &str) -> String {
    if folder.ends_with('/') || folder.ends_with('\\') {
        format!("{folder}{name}")
    } else {
        format!("{folder}/{name}")
    }
}

/// Finds the machine for one side and the login to use there: the node's token from the
/// settings file, else the one the page sent, else a short login for this user (shared secret)
async fn resolve_side(state: &AppState, headers: &HeaderMap, claims: &crate::auth::Claims, side: &SideRequest, write: bool) -> Result<Side, ApiError> {
    if side.folder.contains("://") {
        return Err(fallback("Places like SFTP or Samba inside a pane are copied the old way".into()));
    }
    match side.node.as_str() {
        "local" | "" => {
            let folder = validate_path_access(state, headers, &side.folder, write)?;
            for i in &side.items {
                validate_path_access(state, headers, &join(&side.folder, &i.name), write)?;
            }
            let label = if !state.config.ui.hostname_badge.trim().is_empty() {
                state.config.ui.hostname_badge.clone()
            } else if !state.config.server.server_name.trim().is_empty() {
                state.config.server.server_name.clone()
            } else {
                crate::server::system_handlers::get_system_hostname()
            };
            Ok(Side { label, remote: None, folder })
        }
        "remote" => {
            let url = side.url.clone().unwrap_or_default().trim().trim_end_matches('/').to_string();
            match crate::server::middleware::extract_remote_scheme_host(&url) {
                Some((_, host)) if !crate::server::middleware::is_prohibited_remote_host(&host, state.config.storage.allow_remote_loopback) => {}
                _ => return Err((StatusCode::FORBIDDEN, "That machine's address is not allowed".into())),
            }
            Ok(Side { label: url.clone(), remote: Some((url, side.token.clone().filter(|t| !t.trim().is_empty()))), folder: side.folder.clone() })
        }
        id => {
            let mut target = match state.config.fleet.nodes.iter().find(|n| n.id == id) {
                Some(n) => {
                    if n.read_only && write {
                        return Err((StatusCode::FORBIDDEN, format!("{} is set to read-only", n.name)));
                    }
                    let addresses = n.addresses();
                    ResolvedFleetTarget {
                        id: n.id.clone(),
                        name: n.name.clone(),
                        endpoint_url: crate::server::fleet::fleet_node_address(n),
                        addresses,
                        auth_token: n.token.clone().filter(|t| !t.trim().is_empty()).or_else(|| side.token.clone()),
                    }
                }
                None => {
                    let url = side.url.clone().unwrap_or_default().trim().trim_end_matches('/').to_string();
                    match crate::server::middleware::extract_remote_scheme_host(&url) {
                        Some((_, host)) if !crate::server::middleware::is_prohibited_remote_host(&host, state.config.storage.allow_remote_loopback) => {}
                        _ => return Err((StatusCode::FORBIDDEN, format!("The address of Fleet machine '{id}' is not allowed"))),
                    }
                    ResolvedFleetTarget { id: id.to_string(), name: id.to_string(), endpoint_url: url.clone(), addresses: vec![url], auth_token: side.token.clone() }
                }
            };
            choose_live_address(&mut target, false).await;
            let token = target.auth_token.clone().filter(|t| !t.trim().is_empty()).or_else(|| state.auth.mint_token_for_claims(claims).ok());
            Ok(Side { label: target.name.clone(), remote: Some((target.endpoint_url.clone(), token)), folder: side.folder.clone() })
        }
    }
}

fn place(side: &Side) -> Result<Place, ApiError> {
    Ok(match &side.remote {
        None => Place::Local(side.folder.clone().into()),
        Some((url, token)) => Place::Remote(FleetStore::with_root(url, token.as_deref(), &side.folder, &side.label).map_err(bad)?),
    })
}

/// Asks a machine whether it understands copy jobs
async fn supports(side: &Side) -> Result<bool, String> {
    let Some((url, token)) = side.remote.clone() else { return Ok(true) };
    let label = side.label.clone();
    tokio::task::spawn_blocking(move || FleetStore::with_root(&url, token.as_deref(), "/", &label)?.supports_transfers())
        .await
        .map_err(|e| e.to_string())?
}

async fn handle_transfer_start_inner(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<TransferRequest>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_write(&claims)?;
    check_items(&req.source.items)?;
    let is_move = match req.action.as_str() {
        "copy" => false,
        "move" => true,
        other => return Err(bad(format!("Unknown action: {other}"))),
    };
    let src = resolve_side(&state, &headers, &claims, &req.source, is_move).await?;
    let dst = resolve_side(&state, &headers, &claims, &req.dest, true).await?;

    // Every machine involved must understand copy jobs
    for side in [&src, &dst] {
        match supports(side).await {
            Ok(true) => {}
            Ok(false) => return Err(fallback(format!("{} runs an older Brum; update it for direct copies", side.label))),
            Err(e) if req.source.node == "remote" && side.remote == src.remote => {
                // Handed a direct copy, but this machine cannot reach the source: the asking Brum relays instead
                return Err((StatusCode::BAD_GATEWAY, json!({ "code": "source_unreachable", "error": e }).to_string()));
            }
            Err(e) => return Err((StatusCode::BAD_GATEWAY, format!("{}: {e}", side.label))),
        }
    }

    let verb = if is_move { "Move" } else { "Copy" };
    let n = req.source.items.len();
    let name = format!("{verb} {n} item(s): {} → {}", src.label, dst.label);
    let source_text = format!("{}:{}", src.label, join(&req.source.folder, &req.source.items[0].name)) + &if n > 1 { format!(" (+{} more)", n - 1) } else { String::new() };
    let dest_text = format!("{}:{}", dst.label, req.dest.folder);

    // Both are other machines: let the receiving one fetch the files itself
    if req.direct && src.remote.is_some() && dst.remote.is_some() {
        match start_direct(&req, &src, &dst).await {
            Ok(remote_id) => {
                let id = state.tasks.sync_create_task(&name, &req.action, &source_text, &dest_text, 0);
                state.tasks.set_owner(&id, &claims.sub);
                state.tasks.sync_edit(&id, |t| t.retryable = true);
                state.tasks.sync_log(&id, &format!("{} fetches the files straight from {}", dst.label, src.label));
                let (url, token) = dst.remote.clone().unwrap();
                mirrors().lock().unwrap().insert(id.clone(), Mirror { url, token, remote_id });
                spawn_mirror(state.clone(), id.clone(), dst.label.clone());
                return Ok(Json(json!({ "task_id": id, "mode": "direct" })));
            }
            Err(why) => {
                tracing::info!("Direct copy {} → {} not possible ({why}); relaying", src.label, dst.label);
                let id = start_relay(&state, &claims, &req, &src, &dst, is_move, &name, &source_text, &dest_text)?;
                state.tasks.sync_log(&id, &format!("{} cannot reach {} directly; this Brum passes the files between them", dst.label, src.label));
                return Ok(Json(json!({ "task_id": id, "mode": "relay" })));
            }
        }
    }
    let id = start_relay(&state, &claims, &req, &src, &dst, is_move, &name, &source_text, &dest_text)?;
    Ok(Json(json!({ "task_id": id, "mode": if src.remote.is_some() && dst.remote.is_some() { "relay" } else { "direct" } })))
}

#[allow(clippy::too_many_arguments)]
fn start_relay(state: &AppState, claims: &crate::auth::Claims, req: &TransferRequest, src: &Side, dst: &Side, is_move: bool, name: &str, source_text: &str, dest_text: &str) -> Result<String, ApiError> {
    let (src_place, dst_place) = (place(src)?, place(dst)?);
    let id = state.tasks.sync_create_task(name, &req.action, source_text, dest_text, 0);
    state.tasks.set_owner(&id, &claims.sub);
    state.tasks.sync_edit(&id, |t| t.retryable = true);
    let job = Job::new(id.clone(), is_move, req.verify, src_place, dst_place, req.source.items.clone(), state.config.fleet.transfer_jobs);
    let tasks = state.tasks.clone();
    std::thread::spawn(move || job.run(&tasks, false));
    Ok(id)
}

/// Starts a copy between machines, and records it in the activity log (#193)
pub async fn handle_transfer_start(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<TransferRequest>) -> Result<Json<Value>, ApiError> {
    let who = crate::server::activity_handlers::actor(&state, &headers);
    let side = |s: &SideRequest| format!("{}:{}", if s.node.is_empty() { "local" } else if s.node == "remote" { s.url.as_deref().unwrap_or("remote") } else { &s.node }, s.folder);
    let target = req.source.items.first().map(|i| format!("{}/{}", side(&req.source), i.name)).unwrap_or_else(|| side(&req.source));
    let n = req.source.items.len();
    let mut detail = format!("{}to {}", if n > 1 { format!("{n} items ") } else { String::new() }, side(&req.dest));
    let action = if req.action == "move" { "move between machines" } else { "copy between machines" };
    let r = handle_transfer_start_inner(State(state), headers, Json(req)).await;
    if let Ok(Json(v)) = &r {
        if let Some(mode) = v.get("mode").and_then(|m| m.as_str()) {
            detail.push_str(if mode == "direct" { " (direct)" } else { " (passed on by this Brum)" });
        }
    }
    crate::server::activity_handlers::log_result(who, action, &target, detail, &r);
    r
}

/// Hands the copy to the receiving machine; returns its task id there
async fn start_direct(req: &TransferRequest, src: &Side, dst: &Side) -> Result<String, String> {
    let (src_url, src_token) = src.remote.clone().unwrap();
    let (dst_url, dst_token) = dst.remote.clone().unwrap();
    let body = json!({
        "action": req.action,
        "verify": req.verify,
        "direct": false,
        "source": { "node": "remote", "url": src_url, "token": src_token, "folder": req.source.folder, "items": req.source.items.iter().map(|i| json!({ "name": i.name, "conflict": i.conflict })).collect::<Vec<_>>() },
        "dest": { "node": "local", "folder": req.dest.folder },
    });
    let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let mut r = client.post(format!("{dst_url}/api/transfer/start")).json(&body);
    if let Some(t) = &dst_token {
        r = r.bearer_auth(t);
    }
    let resp = r.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{status}: {text}"));
    }
    serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("task_id").and_then(|t| t.as_str()).map(String::from)).ok_or_else(|| "no task id".into())
}

/// A copy running on another machine, shown in this Brum's task list
#[derive(Clone)]
struct Mirror {
    url: String,
    token: Option<String>,
    remote_id: String,
}

static MIRRORS: OnceLock<Mutex<HashMap<String, Mirror>>> = OnceLock::new();

fn mirrors() -> &'static Mutex<HashMap<String, Mirror>> {
    MIRRORS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remote_req(client: &reqwest::Client, m: &Mirror, method: reqwest::Method, what: &str) -> reqwest::RequestBuilder {
    let r = client.request(method, format!("{}/api/{what}", m.url));
    match &m.token {
        Some(t) => r.bearer_auth(t),
        None => r,
    }
}

/// Keeps the local task in step with the copy on the receiving machine, and passes Pause,
/// Resume and Stop on to it
fn spawn_mirror(state: AppState, id: String, machine: String) {
    tokio::spawn(async move {
        let Some(m) = mirrors().lock().unwrap().get(&id).cloned() else { return };
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build().unwrap();
        let mut misses = 0u32;
        let mut seen_logs = 0usize;
        loop {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let Some(local) = state.tasks.sync_get(&id) else { return };
            let resp = remote_req(&client, &m, reqwest::Method::GET, &format!("tasks/{}", m.remote_id)).send().await;
            let remote: Value = match resp {
                Ok(r) if r.status().is_success() => match r.json().await {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                _ => {
                    misses += 1;
                    if misses >= 40 {
                        state.tasks.sync_fail_task(&id, &format!("Lost contact with {machine}; the copy may still be running there"));
                        return;
                    }
                    continue;
                }
            };
            misses = 0;
            let rstatus = remote.get("status").and_then(|s| s.as_str()).unwrap_or("running").to_string();
            let terminal = matches!(rstatus.as_str(), "completed" | "failed" | "cancelled");
            // What the user asked here goes to the other machine
            let ask = match (local.status.as_str(), rstatus.as_str()) {
                ("cancelled", r) if !matches!(r, "completed" | "failed" | "cancelled") => Some("cancel"),
                ("paused", "running") => Some("pause"),
                ("running", "paused") => Some("resume"),
                _ => None,
            };
            if let Some(a) = ask {
                let _ = remote_req(&client, &m, reqwest::Method::POST, &format!("tasks/{}/{a}", m.remote_id)).send().await;
            }
            let logs: Vec<String> = remote.get("log_entries").and_then(|l| l.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let new_logs: Vec<String> = logs.iter().skip(seen_logs.min(logs.len())).cloned().collect();
            seen_logs = logs.len();
            let u = |k: &str| remote.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
            state.tasks.sync_edit(&id, |t| {
                t.total_files = u("total_files");
                t.total_bytes = u("total_bytes");
                t.files_processed = u("files_processed");
                t.bytes_processed = u("bytes_processed");
                t.speed_bytes_per_sec = u("speed_bytes_per_sec");
                t.current_file_bytes = u("current_file_bytes");
                t.current_file_total_bytes = u("current_file_total_bytes");
                t.verified_files = u("verified_files");
                t.paranoid = remote.get("paranoid").and_then(|v| v.as_bool()).unwrap_or(false);
                t.current_file = remote.get("current_file").and_then(|v| v.as_str()).map(String::from);
                t.last_hash = remote.get("last_hash").and_then(|v| v.as_str()).map(String::from);
                for l in &new_logs {
                    // Their lines already carry a time; skip the task's own start line
                    if !l.contains("Task started:") && !l.contains("Task completed successfully") {
                        if t.log_entries.len() > 200 {
                            t.log_entries.remove(0);
                        }
                        t.log_entries.push(format!("{l} ({machine})"));
                    }
                }
            });
            if terminal && local.status != "cancelled" {
                match rstatus.as_str() {
                    "completed" => state.tasks.sync_complete_task(&id),
                    "failed" => state.tasks.sync_fail_task(&id, remote.get("error_message").and_then(|e| e.as_str()).unwrap_or("The copy failed")),
                    _ => state.tasks.sync_edit(&id, |t| t.status = "cancelled".into()),
                }
                return;
            }
            if terminal {
                return;
            }
        }
    });
}

/// Retry: copies again the files that failed; unfinished copies continue where they stopped
pub async fn handle_transfer_retry(State(state): State<AppState>, headers: HeaderMap, AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let task = crate::server::fs_handlers::owned_task(&state, &headers, &id).await?;
    if task.status == "running" || task.status == "paused" {
        return Err(bad("The copy is still running"));
    }
    let mirror = mirrors().lock().unwrap().get(&id).cloned();
    if let Some(m) = mirror {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let resp = remote_req(&client, &m, reqwest::Method::POST, &format!("transfer/{}/retry", m.remote_id)).send().await.map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
        if !resp.status().is_success() {
            return Err((StatusCode::BAD_GATEWAY, resp.text().await.unwrap_or_default()));
        }
        state.tasks.sync_restart(&id);
        let machine = task.destination.split(':').next().unwrap_or("the other machine").to_string();
        spawn_mirror(state.clone(), id.clone(), machine);
        return Ok(Json(json!({ "success": true })));
    }
    let job = crate::tools::transfer::job(&id).ok_or_else(|| bad("This copy can no longer be retried (Brum was restarted). Start it again."))?;
    if !job.has_failed() {
        return Err(bad("Nothing left to retry"));
    }
    state.tasks.sync_restart(&id);
    let tasks = state.tasks.clone();
    std::thread::spawn(move || job.run(&tasks, true));
    Ok(Json(json!({ "success": true })))
}

// ---------- Comparing folders on two machines (#191) ----------

#[derive(Deserialize)]
pub struct CompareListRequest {
    pub folder: String,
    #[serde(default)]
    pub recursive: bool,
    /// Also work out each file's SHA-256 (exact, slower)
    #[serde(default)]
    pub deep: bool,
}

/// Sets the flag when the request is dropped, so the listing stops when nobody waits for it
struct StopOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

async fn list_here(folder: String, recursive: bool, deep: bool) -> Result<crate::tools::diff::SideMap, String> {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _guard = StopOnDrop(stop.clone());
    tokio::task::spawn_blocking(move || crate::tools::diff::list_side(&folder, recursive, deep, None, &stop))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

fn side_json(map: &crate::tools::diff::SideMap) -> Value {
    Value::Array(map.iter().map(|(k, (d, s, m, h))| json!([k, d, s, m, h])).collect())
}

fn side_from_json(v: &Value) -> crate::tools::diff::SideMap {
    let mut map = crate::tools::diff::SideMap::new();
    for e in v.get("entries").and_then(|e| e.as_array()).into_iter().flatten() {
        let Some(a) = e.as_array() else { continue };
        let Some(rel) = a.first().and_then(|x| x.as_str()) else { continue };
        // A listing from another machine stays inside the compared folder
        let Ok(rel) = crate::tools::sync::store::safe_rel(rel) else { continue };
        if rel.is_empty() {
            continue;
        }
        map.insert(rel, (
            a.get(1).and_then(|x| x.as_bool()).unwrap_or(false),
            a.get(2).and_then(|x| x.as_u64()).unwrap_or(0),
            a.get(3).and_then(|x| x.as_u64()),
            a.get(4).and_then(|x| x.as_str()).map(String::from),
        ));
    }
    map
}

/// Lists a folder on this machine for a comparison made by another Brum
pub async fn handle_compare_list(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<CompareListRequest>) -> Result<Json<Value>, ApiError> {
    let folder = validate_path_access(&state, &headers, &req.folder, false)?;
    let map = list_here(folder, req.recursive, req.deep).await.map_err(bad)?;
    Ok(Json(json!({ "entries": side_json(&map) })))
}

#[derive(Deserialize)]
pub struct CompareAcrossRequest {
    pub left: SideRequest,
    pub right: SideRequest,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub deep: bool,
}

async fn list_side_of(side: &Side, recursive: bool, deep: bool) -> Result<crate::tools::diff::SideMap, String> {
    let Some((url, token)) = &side.remote else {
        return list_here(side.folder.clone(), recursive, deep).await;
    };
    let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    let mut r = client.post(format!("{url}/api/compare/list")).json(&json!({ "folder": side.folder, "recursive": recursive, "deep": deep }));
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    let resp = r.send().await.map_err(|e| format!("Cannot reach {}: {e}", side.label))?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Err(format!("{} runs an older Brum; update it to compare folders across machines", side.label)),
        401 => return Err(format!("{} refused the login. Check the node's token.", side.label)),
        _ => {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from)).unwrap_or(text);
            return Err(format!("{}: {msg}", side.label));
        }
    }
    let v: Value = resp.json().await.map_err(|e| format!("{}: {e}", side.label))?;
    Ok(side_from_json(&v))
}

/// Compares a folder on one machine with a folder on another; each machine lists (and with
/// `deep`, checksums) its own side, so no file travels to be compared
pub async fn handle_compare_across(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<CompareAcrossRequest>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let left = resolve_side(&state, &headers, &claims, &req.left, false).await?;
    let right = resolve_side(&state, &headers, &claims, &req.right, false).await?;
    let (l, r) = tokio::join!(list_side_of(&left, req.recursive, req.deep), list_side_of(&right, req.recursive, req.deep));
    let (l, r) = (l.map_err(bad)?, r.map_err(bad)?);
    let diff = crate::tools::diff::build_folder_diff(&req.left.folder, &req.right.folder, l, r, req.recursive);
    let mut v = serde_json::to_value(diff).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    v["left_machine"] = json!(left.label);
    v["right_machine"] = json!(right.label);
    Ok(Json(v))
}

// ---------- Two files on two machines, side by side (#192) ----------

/// Larger files are not compared as text
const TEXT_COMPARE_LIMIT: u64 = 5 * 1024 * 1024;

#[derive(Deserialize)]
pub struct FileSide {
    pub node: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    /// The file's full path on that machine
    pub path: String,
}

#[derive(Deserialize)]
pub struct CompareFilesAcrossRequest {
    pub left: FileSide,
    pub right: FileSide,
}

/// Splits a path into its folder and name, for `/` and `\` paths alike
fn split_path(path: &str) -> Option<(String, String)> {
    let cut = path.trim_end_matches(['/', '\\']).rfind(['/', '\\'])?;
    let (dir, name) = path.split_at(cut);
    let name = name[1..].trim_end_matches(['/', '\\']).to_string();
    let dir = if dir.is_empty() { "/".to_string() } else if dir.len() == 2 && dir.ends_with(':') { format!("{dir}\\") } else { dir.to_string() };
    (!name.is_empty()).then_some((dir, name))
}

async fn read_text(state: &AppState, headers: &HeaderMap, claims: &crate::auth::Claims, f: &FileSide) -> Result<(String, String), ApiError> {
    let (dir, name) = split_path(&f.path).ok_or_else(|| bad(format!("Not a file path: {}", f.path)))?;
    let side = resolve_side(state, headers, claims, &SideRequest { node: f.node.clone(), url: f.url.clone(), token: f.token.clone(), folder: dir, items: vec![ItemSpec { name: name.clone(), conflict: None }] }, false).await?;
    let label = side.label.clone();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let place = place(&side).map_err(|e| e.1)?;
        let mut reader = place.read_limited(&name, TEXT_COMPARE_LIMIT + 1)?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut buf).map_err(|e| e.to_string())?;
        if buf.len() as u64 > TEXT_COMPARE_LIMIT {
            return Err(format!("{name} is larger than 5 MB; too big to compare as text"));
        }
        Ok(buf)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| bad(format!("{label}: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| bad(format!("{} on {label} is not a text file", f.path)))?;
    Ok((format!("{label}: {}", f.path), text))
}

/// Compares two text files on different machines (or the same one) line by line
pub async fn handle_compare_files_across(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<CompareFilesAcrossRequest>) -> Result<Json<crate::tools::diff::FileDiffResult>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let (l, r) = tokio::join!(read_text(&state, &headers, &claims, &req.left), read_text(&state, &headers, &claims, &req.right));
    let ((ln, lt), (rn, rt)) = (l?, r?);
    Ok(Json(crate::tools::diff::compare_files_text(&ln, &lt, &rn, &rt)))
}

#[cfg(test)]
mod tests {
    use super::split_path;

    #[test]
    fn splits_paths_from_both_systems() {
        assert_eq!(split_path("/etc/fstab"), Some(("/etc".into(), "fstab".into())));
        assert_eq!(split_path("/fstab"), Some(("/".into(), "fstab".into())));
        assert_eq!(split_path(r"C:\Windows\win.ini"), Some((r"C:\Windows".into(), "win.ini".into())));
        assert_eq!(split_path(r"C:\boot.ini"), Some((r"C:\".into(), "boot.ini".into())));
        assert_eq!(split_path("fstab"), None);
    }
}
