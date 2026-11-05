//! Proton Drive setup: finding or installing the program, and each user's sign-in

use axum::{extract::State, http::{HeaderMap, StatusCode}, Json};
use serde_json::{json, Value};

use crate::server::middleware::{extract_claims_or_local, require_admin};
use crate::server::AppState;
use crate::vfs::proton;

type ApiResult = Result<Json<Value>, (StatusCode, String)>;

fn user_of(state: &AppState, headers: &HeaderMap) -> Result<crate::auth::Claims, (StatusCode, String)> {
    extract_claims_or_local(state, headers)
}

fn status_json(claims: &crate::auth::Claims) -> Value {
    let mut v = serde_json::to_value(proton::status(&claims.sub)).unwrap_or_else(|_| json!({}));
    v["is_admin"] = json!(require_admin(claims).is_ok());
    v
}

/// What is installed, and whether this user is signed in
pub async fn handle_status(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    Ok(Json(status_json(&claims)))
}

/// The newest release Proton offers for this machine (admins, for Install and Update)
pub async fn handle_latest(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    require_admin(&claims)?;
    let rel = proton::latest_release().await.map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(json!(rel)))
}

/// Downloads the newest release into Brum's data folder (admins)
pub async fn handle_install(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    require_admin(&claims)?;
    proton::start_install().map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    tracing::info!("Proton Drive program install started by {}", claims.sub);
    Ok(Json(status_json(&claims)))
}

/// Starts signing in; the status then carries the link to open
pub async fn handle_login(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    proton::start_login(&claims.sub).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    // The link is printed within a few seconds; wait briefly so the first answer usually has it
    for _ in 0..40 {
        if proton::login_view(&claims.sub).map(|v| v.state != "starting").unwrap_or(true) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Ok(Json(status_json(&claims)))
}

pub async fn handle_login_cancel(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    proton::cancel_login(&claims.sub);
    proton::clear_login(&claims.sub);
    Ok(Json(status_json(&claims)))
}

/// After signing in: asks Proton which account it is
pub async fn handle_refresh_account(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    proton::refresh_account(&claims.sub).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    proton::clear_login(&claims.sub);
    Ok(Json(status_json(&claims)))
}

pub async fn handle_logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    proton::sign_out(&claims.sub).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    tracing::info!("{} signed out of Proton Drive", claims.sub);
    Ok(Json(status_json(&claims)))
}

/// The current sign-in link as a QR code, for signing in on a phone
pub async fn handle_login_qr(State(state): State<AppState>, headers: HeaderMap) -> Result<axum::response::Response, (StatusCode, String)> {
    use axum::response::IntoResponse;
    let claims = user_of(&state, &headers)?;
    let url = proton::login_view(&claims.sub).and_then(|v| v.url).ok_or((StatusCode::NOT_FOUND, "No sign-in in progress".to_string()))?;
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok(([(axum::http::header::CONTENT_TYPE, "image/svg+xml"), (axum::http::header::CACHE_CONTROL, "no-store")], svg).into_response())
}

#[derive(serde::Deserialize)]
pub struct SharePath {
    path: String,
}

#[derive(serde::Deserialize)]
pub struct ShareRequest {
    path: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    expires: String,
}

fn proton_path(state: &AppState, headers: &HeaderMap, path: &str, write: bool) -> Result<String, (StatusCode, String)> {
    let p = crate::server::middleware::validate_path_access(state, headers, path, write)?;
    if !p.starts_with("proton://") {
        return Err((StatusCode::BAD_REQUEST, "Not a Proton Drive item".to_string()));
    }
    Ok(p)
}

/// The public link of a Proton Drive item, if any
pub async fn handle_share_status(State(state): State<AppState>, headers: HeaderMap, axum::extract::Query(q): axum::extract::Query<SharePath>) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    let path = proton_path(&state, &headers, &q.path, false)?;
    let link = proton::share_status(&claims.sub, &path).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({ "link": link })))
}

/// Creates or changes a public link (it makes the item reachable by anyone with the link)
pub async fn handle_share_set(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<ShareRequest>) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    crate::server::middleware::require_write(&claims)?;
    let path = proton_path(&state, &headers, &req.path, true)?;
    let link = proton::share_set(&claims.sub, &path, &req.role, req.password.trim(), req.expires.trim()).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    tracing::info!("{} set a Proton Drive public link on {}", claims.sub, path);
    Ok(Json(json!({ "link": link })))
}

pub async fn handle_share_remove(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<SharePath>) -> ApiResult {
    let claims = user_of(&state, &headers)?;
    crate::server::middleware::require_write(&claims)?;
    let path = proton_path(&state, &headers, &req.path, true)?;
    proton::share_remove(&claims.sub, &path).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    tracing::info!("{} removed the Proton Drive public link on {}", claims.sub, path);
    Ok(Json(json!({ "link": null })))
}
