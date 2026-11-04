//! Lets another Brum in the Fleet keep backups on this machine.
//!
//! The sending Brum's backup engine uses these addresses as its storage: list folders, read a
//! file, write a file (streamed to a temporary name, keeping its time), move and remove. Each
//! request runs as the user its token belongs to, with that user's folder permissions.

use crate::server::middleware::{extract_claims_or_local, require_write, validate_path_access};
use crate::server::AppState;
use crate::tools::sync::store::{LocalStore, Store};
use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Json, Response},
};
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use tokio::io::AsyncWriteExt;

type ApiError = (StatusCode, String);

fn bad(e: impl ToString) -> ApiError {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn param<'a>(q: &'a HashMap<String, String>, key: &str) -> Result<&'a str, ApiError> {
    q.get(key).map(|s| s.as_str()).filter(|s| !s.is_empty()).ok_or_else(|| bad(format!("Missing {key}")))
}

/// A checked, absolute path on this machine
fn checked(state: &AppState, headers: &HeaderMap, raw: &str, write: bool) -> Result<PathBuf, ApiError> {
    if raw.contains("://") {
        return Err(bad("Only folders on this machine can be used as a backup target"));
    }
    if write {
        require_write(&extract_claims_or_local(state, headers)?)?;
    }
    Ok(PathBuf::from(validate_path_access(state, headers, raw, write)?))
}

fn mtime_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `path` and `recursive=1|0`: every file below a folder (recursive), or one folder level
pub async fn handle_backup_target_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let path = checked(&state, &headers, param(&q, "path")?, false)?;
    let recursive = q.get("recursive").is_some_and(|v| v == "1" || v == "true");
    tokio::task::spawn_blocking(move || {
        if !path.is_dir() {
            return Ok(Json(serde_json::json!({ "exists": path.exists(), "entries": [], "warnings": [] })));
        }
        let store = LocalStore::new(&path);
        if recursive {
            let mut warnings = Vec::new();
            let listing = store.list("", &|_, _| false, &AtomicBool::new(false), &mut warnings).map_err(bad)?;
            let entries: Vec<_> = listing.iter().map(|(rel, m)| serde_json::json!({ "path": rel, "size": m.size, "mtime": m.mtime, "is_dir": false })).collect();
            Ok(Json(serde_json::json!({ "exists": true, "entries": entries, "warnings": warnings })))
        } else {
            let items = store.list_dir("").map_err(bad)?;
            let entries: Vec<_> = items.iter().map(|i| serde_json::json!({ "path": i.name, "size": i.size, "mtime": i.mtime, "is_dir": i.is_dir })).collect();
            Ok(Json(serde_json::json!({ "exists": true, "entries": entries, "warnings": [] })))
        }
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}

/// `path`: whether it is a file or a folder, with size and time
pub async fn handle_backup_target_stat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let path = checked(&state, &headers, param(&q, "path")?, false)?;
    Ok(Json(match std::fs::symlink_metadata(&path) {
        Ok(m) if m.is_file() => serde_json::json!({ "kind": "file", "size": m.len(), "mtime": mtime_of(&m) }),
        Ok(m) if m.is_dir() => serde_json::json!({ "kind": "dir", "size": 0, "mtime": mtime_of(&m) }),
        _ => serde_json::json!({ "kind": null }),
    }))
}

/// `path`: the file's contents, streamed
pub async fn handle_backup_target_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let path = checked(&state, &headers, param(&q, "path")?, false)?;
    let file = tokio::fs::File::open(&path).await.map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, len)
        .body(Body::from_stream(tokio_util::io::ReaderStream::new(file)))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// `path` (a temporary `.brum-part` name) and `mtime`: stores the request body there, keeping the time.
/// The sender swaps it into place with a rename once the copy is checked.
pub async fn handle_backup_target_write(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    body: Body,
) -> Result<Json<serde_json::Value>, ApiError> {
    let raw = param(&q, "path")?;
    if !raw.ends_with(crate::tools::sync::store::PART_SUFFIX) {
        return Err(bad("Backups are written to a temporary name first"));
    }
    let path = checked(&state, &headers, raw, true)?;
    let mtime: i64 = q.get("mtime").and_then(|m| m.parse().ok()).ok_or_else(|| bad("Missing mtime"))?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(bad)?;
    }
    let result: Result<u64, String> = async {
        let mut file = tokio::fs::File::create(&path).await.map_err(|e| e.to_string())?;
        let mut stream = body.into_data_stream();
        let mut written = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Transfer interrupted: {e}"))?;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            written += chunk.len() as u64;
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        filetime::set_file_mtime(&path, filetime::FileTime::from_unix_time(mtime, 0)).map_err(|e| e.to_string())?;
        Ok(written)
    }
    .await;
    match result {
        Ok(bytes) => Ok(Json(serde_json::json!({ "bytes": bytes }))),
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            Err(bad(e))
        }
    }
}

#[derive(Deserialize)]
pub struct TargetOp {
    /// "rename", "remove_file", "remove_dir_all", "remove_empty_parents", "mkdir"
    op: String,
    path: String,
    #[serde(default)]
    to: Option<String>,
    /// For remove_empty_parents: the backup's top folder, which is never removed
    #[serde(default)]
    stop_at: Option<String>,
}

/// Moving, removing and making folders
pub async fn handle_backup_target_op(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TargetOp>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let path = checked(&state, &headers, &req.path, true)?;
    let to = match req.to.as_deref() {
        Some(t) => Some(checked(&state, &headers, t, true)?),
        None => None,
    };
    let stop_at = match req.stop_at.as_deref() {
        Some(s) => Some(checked(&state, &headers, s, true)?),
        None => None,
    };
    let op = req.op.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        match op.as_str() {
            "rename" => {
                let to = to.ok_or("Missing destination")?;
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::rename(&path, &to).map_err(|e| e.to_string())
            }
            "remove_file" => std::fs::remove_file(&path).map_err(|e| e.to_string()),
            "remove_dir_all" => std::fs::remove_dir_all(&path).map_err(|e| e.to_string()),
            "mkdir" => std::fs::create_dir_all(&path).map_err(|e| e.to_string()),
            "remove_empty_parents" => {
                let stop = stop_at.ok_or("Missing top folder")?;
                let mut cur: Option<&Path> = path.parent();
                while let Some(dir) = cur {
                    if dir == stop || !dir.starts_with(&stop) || std::fs::remove_dir(dir).is_err() {
                        break;
                    }
                    cur = dir.parent();
                }
                Ok(())
            }
            other => Err(format!("Unknown operation: {other}")),
        }
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(bad)?;
    Ok(Json(serde_json::json!({ "success": true })))
}
