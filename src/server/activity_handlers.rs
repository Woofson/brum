//! The activity log (#193) for admins: list, search and download. The recording helpers here
//! are shared by the handlers that change files, shares, accounts and settings.

use crate::server::middleware::{extract_claims_or_local, require_admin};
use crate::server::AppState;
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};

/// The person doing it; nothing is recorded for requests that are not signed in
pub fn actor(state: &AppState, headers: &HeaderMap) -> Option<String> {
    extract_claims_or_local(state, headers).ok().map(|c| c.sub)
}

/// Records how one operation went: `detail` says more; a failure adds its reason
pub fn log_result<T>(who: Option<String>, action: &str, target: &str, detail: String, r: &Result<T, (StatusCode, String)>) {
    let Some(who) = who else { return };
    let (ok, detail) = match r {
        Ok(_) => (true, detail),
        Err((_, e)) if detail.is_empty() => (false, e.clone()),
        Err((_, e)) => (false, format!("{detail}; {e}")),
    };
    crate::tools::activity::record(&who, action, target, &detail, ok);
}

pub async fn handle_list_activity(State(state): State<AppState>, headers: HeaderMap, Query(f): Query<crate::tools::activity::Filter>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_admin(&extract_claims_or_local(&state, &headers)?)?;
    let (entries, total) = tokio::task::spawn_blocking(move || crate::tools::activity::query(&f))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(serde_json::json!({ "entries": entries, "total": total, "retention_days": state.config.activity.retention_days, "enabled": state.config.activity.enabled })))
}

/// The matching entries as a CSV file, up to 100 000 of them
pub async fn handle_download_activity(State(state): State<AppState>, headers: HeaderMap, Query(mut f): Query<crate::tools::activity::Filter>) -> Result<Response, (StatusCode, String)> {
    require_admin(&extract_claims_or_local(&state, &headers)?)?;
    f.limit = Some(100_000);
    f.offset = None;
    let (entries, _) = tokio::task::spawn_blocking(move || crate::tools::activity::query(&f))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    let name = format!("brum-activity-{}.csv", chrono::Local::now().format("%Y-%m-%d"));
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
        ],
        crate::tools::activity::to_csv(&entries),
    )
        .into_response())
}
