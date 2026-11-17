//! Settings → Cloud Storage (admins): installing rclone and managing its remotes (#189)

use axum::{extract::State, http::{HeaderMap, StatusCode}, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::server::middleware::{extract_claims_or_local, require_admin};
use crate::server::AppState;
use crate::vfs::rclone;

type ApiResult = Result<Json<Value>, (StatusCode, String)>;

fn admin(state: &AppState, headers: &HeaderMap) -> Result<crate::auth::Claims, (StatusCode, String)> {
    let claims = extract_claims_or_local(state, headers)?;
    require_admin(&claims)?;
    Ok(claims)
}

async fn status_json() -> Value {
    let mut v = serde_json::to_value(rclone::status()).unwrap_or_else(|_| json!({}));
    match rclone::remotes().await {
        Ok(list) => v["remotes"] = json!(list),
        Err(e) => {
            v["remotes"] = json!([]);
            v["remotes_error"] = json!(e);
        }
    }
    v
}

/// For everyone: whether cloud storage can be offered in the location menu
pub async fn handle_available(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = extract_claims_or_local(&state, &headers)?;
    Ok(Json(json!({ "available": rclone::is_available(), "may_use": crate::server::middleware::rclone_any_allowed(&claims) })))
}

/// What is installed, where the remotes are kept, and the remotes
pub async fn handle_status(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    admin(&state, &headers)?;
    Ok(Json(status_json().await))
}

/// The newest rclone for this machine (for Install and Update)
pub async fn handle_latest(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    admin(&state, &headers)?;
    let rel = rclone::latest_release().await.map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(json!(rel)))
}

/// Downloads the newest rclone into Brum's data folder
pub async fn handle_install(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = admin(&state, &headers)?;
    rclone::start_install().map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    tracing::info!("rclone install started by {}", claims.sub);
    Ok(Json(status_json().await))
}

#[derive(Deserialize)]
pub struct RemoteName {
    name: String,
}

/// Proves the remote's login works and tells its storage use where the service does
pub async fn handle_about(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<RemoteName>) -> ApiResult {
    admin(&state, &headers)?;
    let about = rclone::about(p.name.trim()).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({ "ok": true, "about": about })))
}

pub async fn handle_remove(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<RemoteName>) -> ApiResult {
    let claims = admin(&state, &headers)?;
    rclone::remove_remote(p.name.trim()).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    tracing::info!("rclone remote \"{}\" removed by {}", p.name.trim(), claims.sub);
    Ok(Json(status_json().await))
}

#[derive(Deserialize)]
pub struct AddJottacloud {
    name: String,
    token: String,
}

/// Adds a Jottacloud remote from a personal login token (never logged)
pub async fn handle_add_jottacloud(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<AddJottacloud>) -> ApiResult {
    let claims = admin(&state, &headers)?;
    rclone::add_jottacloud(p.name.trim(), &p.token).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    tracing::info!("Jottacloud remote \"{}\" added by {}", p.name.trim(), claims.sub);
    Ok(Json(status_json().await))
}
