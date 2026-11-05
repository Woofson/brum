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
