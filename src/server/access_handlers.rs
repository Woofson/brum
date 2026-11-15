//! Folder rules (#193) for admins: list, add and remove. The rules themselves are checked in
//! `middleware::validate_path_access`.

use crate::server::middleware::{extract_claims_or_local, require_admin};
use crate::server::AppState;
use crate::tools::access::{self, Rule};
use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde_json::{json, Value};

type ApiError = (StatusCode, String);

fn shown(r: &Rule, from_config: bool) -> Value {
    json!({ "id": r.id, "path": r.path, "access": r.access, "users": r.users, "roles": r.roles, "note": r.note, "who": r.who(), "what": r.access.describe(), "from_config": from_config })
}

pub async fn handle_list_access_rules(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    require_admin(&extract_claims_or_local(&state, &headers)?)?;
    let mut rules: Vec<Value> = state.config.access.rules.iter().map(|r| shown(r, true)).collect();
    rules.extend(access::saved().iter().map(|r| shown(r, false)));
    Ok(Json(json!({ "rules": rules })))
}

pub async fn handle_add_access_rule(State(state): State<AppState>, headers: HeaderMap, Json(rule): Json<Rule>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    let path = rule.path.trim();
    let absolute = path.starts_with('/') || (path.len() >= 3 && path.as_bytes()[1] == b':' && (path.as_bytes()[2] == b'\\' || path.as_bytes()[2] == b'/'));
    if path.contains("://") || !absolute {
        return Err((StatusCode::BAD_REQUEST, "A folder rule needs a full folder path on this machine, like /srv/media or D:\\Media".into()));
    }
    let saved = access::add(rule, &claims.sub).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    crate::tools::activity::record(&claims.sub, "folder rule added", &saved.path, &format!("{}: {}", saved.who(), saved.access.describe()), true);
    Ok(Json(shown(&saved, false)))
}

pub async fn handle_remove_access_rule(State(state): State<AppState>, headers: HeaderMap, AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_admin(&claims)?;
    match access::remove(&id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))? {
        Some(r) => {
            crate::tools::activity::record(&claims.sub, "folder rule removed", &r.path, &format!("{}: {}", r.who(), r.access.describe()), true);
            Ok(Json(json!({ "success": true })))
        }
        None => Err((StatusCode::NOT_FOUND, "No such rule (rules from config.toml are changed there)".into())),
    }
}
