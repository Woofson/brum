use std::path::{Path, PathBuf};
use axum::{
    body::Body,
    extract::{Multipart, Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use crate::server::AppState;
use crate::server::middleware::{extract_client_ip, sanitize_uploaded_file_name};
use crate::server::fs_handlers::build_local_file_range_response;
use crate::vfs::archive::ArchiveHandler;

fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(|s| s.to_string())
}

fn safe_join_share_path(base_dir: &Path, rel_path: &str) -> Result<PathBuf, (StatusCode, String)> {
    let clean_rel = Path::new(rel_path);
    for comp in clean_rel.components() {
        match comp {
            std::path::Component::Normal(_) => {},
            std::path::Component::CurDir => {},
            _ => return Err((StatusCode::BAD_REQUEST, "Invalid file path in share request".to_string())),
        }
    }
    let target = base_dir.join(clean_rel);
    let base_canonical = base_dir.canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Shared directory not found on host".to_string()))?;
    let target_canonical = target.canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Requested file not found in share".to_string()))?;
    if !target_canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Access to path outside shared directory is forbidden".to_string()));
    }
    Ok(target_canonical)
}

#[derive(Serialize)]
pub struct ShareDirEntry {
    name: String,
    rel_path: String,
    is_dir: bool,
    size: u64,
    mime: String,
    mtime: u64,
}

fn list_share_dir_entries(base_path: &Path) -> Vec<ShareDirEntry> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = path.is_dir();
            let size = if is_dir { 0 } else { entry.metadata().map(|m| m.len()).unwrap_or(0) };
            let mtime = entry.metadata().ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let mime = if is_dir {
                "inode/directory".to_string()
            } else {
                mime_guess::from_path(&path).first_or_octet_stream().to_string()
            };
            results.push(ShareDirEntry {
                name: name.clone(),
                rel_path: name,
                is_dir,
                size,
                mime,
                mtime,
            });
        }
    }
    results.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });
    results
}

#[derive(Deserialize)]
pub struct CreateShareRequest {
    path: String,
    name: Option<String>,
    is_dir: bool,
    allow_upload: Option<bool>,
    allow_view: Option<bool>,
    allow_download: Option<bool>,
    password: Option<String>,
    expires_in_hours: Option<u64>,
    max_downloads: Option<u64>,
    allowed_emails: Option<String>,
    require_email: Option<bool>,
    watermark_enabled: Option<bool>,
    watermark_text: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateShareRequest {
    name: Option<String>,
    allow_upload: Option<bool>,
    allow_view: Option<bool>,
    allow_download: Option<bool>,
    password: Option<Option<String>>,
    expires_in_hours: Option<Option<u64>>,
    max_downloads: Option<u64>,
    allowed_emails: Option<String>,
    require_email: Option<bool>,
    watermark_enabled: Option<bool>,
    watermark_text: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
pub struct RevokeShareRequest {
    revoke: Option<bool>,
}

#[derive(Deserialize)]
pub struct VerifyPassRequest {
    password: String,
}

#[derive(Deserialize)]
pub struct VerifyEmailRequest {
    email: String,
}

#[derive(Deserialize)]
pub struct SharePublicQuery {
    file: Option<String>,
    password: Option<String>,
    email: Option<String>,
}

pub async fn handle_create_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateShareRequest>,
) -> Result<Json<crate::auth::ShareItem>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let name = payload.name.unwrap_or_else(|| {
        Path::new(&payload.path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    let expires_at = payload.expires_in_hours.map(|hrs| {
        (chrono::Utc::now() + chrono::Duration::hours(hrs as i64)).to_rfc3339()
    });

    let share = state.auth.create_share(
        &claims.sub,
        &payload.path,
        &name,
        payload.is_dir,
        payload.allow_upload.unwrap_or(false),
        payload.allow_view.unwrap_or(true),
        payload.allow_download.unwrap_or(true),
        payload.password.as_deref(),
        expires_at.as_deref(),
        payload.max_downloads.unwrap_or(0),
        payload.allowed_emails.as_deref(),
        payload.require_email.unwrap_or(false),
        payload.watermark_enabled.unwrap_or(false),
        payload.watermark_text.as_deref(),
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create share: {}", e)))?;

    Ok(Json(share))
}

pub async fn handle_update_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateShareRequest>,
) -> Result<Json<crate::auth::ShareItem>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let is_admin = claims.role == "admin";
    let name = payload.name.unwrap_or_else(|| "Share".to_string());

    let expires_at_str = payload.expires_in_hours.map(|opt| {
        opt.map(|hrs| (chrono::Utc::now() + chrono::Duration::hours(hrs as i64)).to_rfc3339())
    });

    let new_password_ref = payload.password.as_ref().map(|opt| opt.as_deref());
    let expires_at_ref = expires_at_str.as_ref().map(|opt| opt.as_deref());

    let share = state.auth.update_share(
        id,
        &claims.sub,
        is_admin,
        &name,
        payload.allow_upload.unwrap_or(false),
        payload.allow_view.unwrap_or(true),
        payload.allow_download.unwrap_or(true),
        new_password_ref,
        expires_at_ref,
        payload.max_downloads.unwrap_or(0),
        payload.allowed_emails.as_deref(),
        payload.require_email.unwrap_or(false),
        payload.watermark_enabled.unwrap_or(false),
        payload.watermark_text.as_deref(),
        payload.status.as_deref(),
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update share: {}", e)))?;

    Ok(Json(share))
}

pub async fn handle_revoke_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<RevokeShareRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let is_admin = claims.role == "admin";
    let revoke = payload.revoke.unwrap_or(true);
    state.auth.revoke_share(id, &claims.sub, is_admin, revoke)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to toggle revoke share: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true, "status": if revoke { "revoked" } else { "active" } })))
}

pub async fn handle_get_share_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<Vec<crate::auth::ShareAccessLog>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let is_admin = claims.role == "admin";
    let logs = state.auth.get_share_logs(id, &claims.sub, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to fetch access logs: {}", e)))?;

    Ok(Json(logs))
}

pub async fn handle_list_shares(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::auth::ShareItem>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let is_admin = claims.role == "admin";
    let list = state.auth.list_shares(&claims.sub, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list shares: {}", e)))?;

    Ok(Json(list))
}

pub async fn handle_delete_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header".to_string()))?;

    let token_str = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);
    let claims = state.auth.verify_token(token_str)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)))?;

    let is_admin = claims.role == "admin";
    state.auth.delete_share(id, &claims.sub, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete share: {}", e)))?;

    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn handle_public_get_share_meta(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let share = state.auth.get_share_by_token(&token)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or((StatusCode::NOT_FOUND, "Share not found".to_string()))?;

    if share.status == "revoked" {
        return Err((StatusCode::GONE, "This share link has been revoked by the owner".to_string()));
    }

    if let Some(ref exp) = share.expires_at {
        if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
            if chrono::Utc::now() > exp_time {
                return Err((StatusCode::GONE, "This share link has expired".to_string()));
            }
        }
    }

    if share.max_downloads > 0 && share.download_count >= share.max_downloads {
        return Err((StatusCode::GONE, "This share link has reached its maximum download limit".to_string()));
    }

    // If share has no password and no email requirement, log public visit immediately
    if !share.has_password && !share.require_email {
        let ip = extract_client_ip(&headers);
        let ua = extract_user_agent(&headers);
        let _ = state.auth.log_share_access(share.id, &token, None, &ip, ua.as_deref(), "visit", None);
    }

    let p = Path::new(&share.path);
    let size = if p.is_file() {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    let files = if share.is_dir && share.allow_view && p.exists() {
        list_share_dir_entries(p)
    } else {
        Vec::new()
    };

    Ok(Json(serde_json::json!({
        "id": share.id,
        "token": share.token,
        "name": share.name,
        "is_dir": share.is_dir,
        "allow_upload": share.allow_upload,
        "allow_view": share.allow_view,
        "allow_download": share.allow_download,
        "has_password": share.has_password,
        "require_email": share.require_email,
        "watermark_enabled": share.watermark_enabled,
        "watermark_text": share.watermark_text,
        "status": share.status,
        "size": size,
        "created_at": share.created_at,
        "expires_at": share.expires_at,
        "files": files,
    })))
}

pub async fn handle_public_verify_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Json(payload): Json<VerifyPassRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let ip = extract_client_ip(&headers);
    let rate_key = format!("share_verify:{}:{}", token, ip);
    if let Err(retry_after) = state.rate_limiter.check_and_record(&rate_key, 10, 60) {
        tracing::warn!(client_ip = %ip, token = %token, "Rate limit exceeded on share password verification");
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many verification attempts. Please retry after {} seconds.", retry_after),
        ));
    }

    let valid = state.auth.verify_share_password(&token, &payload.password)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Verification error: {}", e)))?;

    if valid {
        state.rate_limiter.reset(&rate_key);
        if let Ok(Some(share)) = state.auth.get_share_by_token(&token) {
            let ua = extract_user_agent(&headers);
            let _ = state.auth.log_share_access(share.id, &token, None, &ip, ua.as_deref(), "visit", None);
        }
        Ok(Json(serde_json::json!({ "valid": true })))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Incorrect password for this share".to_string()))
    }
}

pub async fn handle_public_verify_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Json(payload): Json<VerifyEmailRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let ip = extract_client_ip(&headers);
    let rate_key = format!("share_verify:{}:{}", token, ip);
    if let Err(retry_after) = state.rate_limiter.check_and_record(&rate_key, 10, 60) {
        tracing::warn!(client_ip = %ip, token = %token, "Rate limit exceeded on share email verification");
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many verification attempts. Please retry after {} seconds.", retry_after),
        ));
    }

    let valid = state.auth.verify_share_email(&token, &payload.email)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Email verification error: {}", e)))?;

    if valid {
        state.rate_limiter.reset(&rate_key);
        if let Ok(Some(share)) = state.auth.get_share_by_token(&token) {
            let ua = extract_user_agent(&headers);
            let _ = state.auth.log_share_access(share.id, &token, Some(&payload.email), &ip, ua.as_deref(), "visit", None);
        }
        Ok(Json(serde_json::json!({ "valid": true })))
    } else {
        Err((StatusCode::FORBIDDEN, "Email address is not authorized for this showcase share".to_string()))
    }
}

pub async fn handle_public_preview_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<SharePublicQuery>,
) -> Result<Response, (StatusCode, String)> {
    let share = state.auth.get_share_by_token(&token)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or((StatusCode::NOT_FOUND, "Share not found or expired".to_string()))?;

    if share.status == "revoked" {
        return Err((StatusCode::GONE, "This share link has been revoked".to_string()));
    }

    if let Some(ref exp) = share.expires_at {
        if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
            if chrono::Utc::now() > exp_time {
                return Err((StatusCode::GONE, "This share link has expired".to_string()));
            }
        }
    }

    if !share.allow_view {
        return Err((StatusCode::FORBIDDEN, "Online file preview is disabled for this share".to_string()));
    }

    if share.has_password {
        let pass = query.password.as_deref().unwrap_or("");
        let valid = state.auth.verify_share_password(&token, pass).unwrap_or(false);
        if !valid {
            return Err((StatusCode::UNAUTHORIZED, "Password required for preview".to_string()));
        }
    }

    if share.require_email {
        let email = query.email.as_deref().unwrap_or("");
        let valid = state.auth.verify_share_email(&token, email).unwrap_or(false);
        if !valid {
            return Err((StatusCode::FORBIDDEN, "Authorized email required for preview".to_string()));
        }
    }

    let target_path = if share.is_dir {
        if let Some(ref file_param) = query.file {
            safe_join_share_path(Path::new(&share.path), file_param)?
        } else {
            return Err((StatusCode::BAD_REQUEST, "File parameter is required when previewing a folder share".to_string()));
        }
    } else {
        PathBuf::from(&share.path)
    };

    if !target_path.exists() || !target_path.is_file() {
        return Err((StatusCode::NOT_FOUND, "Requested file not found on server".to_string()));
    }

    let file_name = target_path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let ip = extract_client_ip(&headers);
    let ua = extract_user_agent(&headers);
    let _ = state.auth.log_share_access(
        share.id,
        &token,
        query.email.as_deref(),
        &ip,
        ua.as_deref(),
        "preview",
        Some(&file_name),
    );

    let metadata = tokio::fs::metadata(&target_path).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file metadata: {}", e))
    })?;

    let mtime_sec = metadata.modified().ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let etag = format!("\"{:x}-{:x}\"", metadata.len(), mtime_sec);

    let mime = mime_guess::from_path(&target_path).first_or_octet_stream().to_string();
    let disposition = format!("inline; filename=\"{}\"", file_name);
    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

    build_local_file_range_response(&target_path, metadata.len(), mime, disposition, etag, range_header).await
}

pub async fn handle_public_download_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<SharePublicQuery>,
) -> Result<Response, (StatusCode, String)> {
    let share = state.auth.get_share_by_token(&token)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or((StatusCode::NOT_FOUND, "Share not found or expired".to_string()))?;

    if share.status == "revoked" {
        return Err((StatusCode::GONE, "This share link has been revoked".to_string()));
    }

    if let Some(ref exp) = share.expires_at {
        if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
            if chrono::Utc::now() > exp_time {
                return Err((StatusCode::GONE, "This share link has expired".to_string()));
            }
        }
    }

    if !share.allow_download {
        return Err((StatusCode::FORBIDDEN, "Download is disabled for this showcase link (view-only mode)".to_string()));
    }

    if share.max_downloads > 0 && share.download_count >= share.max_downloads {
        return Err((StatusCode::GONE, "This share link has reached its maximum download limit".to_string()));
    }

    if share.has_password {
        let pass = query.password.as_deref().unwrap_or("");
        let valid = state.auth.verify_share_password(&token, pass).unwrap_or(false);
        if !valid {
            return Err((StatusCode::UNAUTHORIZED, "Password required for download".to_string()));
        }
    }

    if share.require_email {
        let email = query.email.as_deref().unwrap_or("");
        let valid = state.auth.verify_share_email(&token, email).unwrap_or(false);
        if !valid {
            return Err((StatusCode::FORBIDDEN, "Authorized email required for download".to_string()));
        }
    }

    let path = Path::new(&share.path);
    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "Target file or folder not found on server".to_string()));
    }

    let _ = state.auth.increment_share_downloads(&token);

    let ip = extract_client_ip(&headers);
    let ua = extract_user_agent(&headers);

    // If downloading a specific file from a shared folder
    if share.is_dir {
        if let Some(ref file_param) = query.file {
            let target_file = safe_join_share_path(path, file_param)?;
            if !target_file.is_file() {
                return Err((StatusCode::NOT_FOUND, "Specified file not found in folder share".to_string()));
            }

            let file_name = target_file.file_name().unwrap_or_default().to_string_lossy().to_string();
            let _ = state.auth.log_share_access(
                share.id,
                &token,
                query.email.as_deref(),
                &ip,
                ua.as_deref(),
                "download",
                Some(&file_name),
            );

            let metadata = tokio::fs::metadata(&target_file).await.map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file metadata: {}", e))
            })?;

            let mtime_sec = metadata.modified().ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let etag = format!("\"{:x}-{:x}\"", metadata.len(), mtime_sec);
            let mime = mime_guess::from_path(&target_file).first_or_octet_stream().to_string();
            let disposition = format!("attachment; filename=\"{}\"", file_name);
            let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

            return build_local_file_range_response(&target_file, metadata.len(), mime, disposition, etag, range_header).await;
        }

        // Full directory download -> zip archive
        let _ = state.auth.log_share_access(
            share.id,
            &token,
            query.email.as_deref(),
            &ip,
            ua.as_deref(),
            "download",
            Some(&format!("{}.zip", share.name)),
        );

        let temp_zip = tempfile::Builder::new()
            .prefix("brum_share_")
            .suffix(".zip")
            .tempfile()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Temp file error: {}", e)))?;
        let temp_path = temp_zip.path().to_str().unwrap().to_string();

        ArchiveHandler::create_zip(&[share.path.clone()], &temp_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Zip creation failed: {}", e)))?;

        let file_bytes = tokio::fs::read(&temp_path).await.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read generated zip: {}", e))
        })?;

        let zip_name = format!("{}.zip", share.name);
        let response = Response::builder()
            .header(header::CONTENT_TYPE, "application/zip")
            .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", zip_name))
            .header(header::CONTENT_LENGTH, file_bytes.len().to_string())
            .body(Body::from(file_bytes))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

        return Ok(response);
    }

    // Single file download
    let file_name = share.name.clone();
    let _ = state.auth.log_share_access(
        share.id,
        &token,
        query.email.as_deref(),
        &ip,
        ua.as_deref(),
        "download",
        Some(&file_name),
    );

    let metadata = tokio::fs::metadata(path).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file metadata: {}", e))
    })?;

    let mtime_sec = metadata.modified().ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let etag = format!("\"{:x}-{:x}\"", metadata.len(), mtime_sec);

    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    let disposition = format!("attachment; filename=\"{}\"", share.name);
    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

    build_local_file_range_response(path, metadata.len(), mime, disposition, etag, range_header).await
}

pub async fn handle_public_upload_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<SharePublicQuery>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let share = state.auth.get_share_by_token(&token)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or((StatusCode::NOT_FOUND, "Share not found or expired".to_string()))?;

    if share.status == "revoked" {
        return Err((StatusCode::GONE, "This share link has been revoked".to_string()));
    }

    if !share.is_dir || !share.allow_upload {
        return Err((StatusCode::FORBIDDEN, "Guest uploads are not enabled for this share".to_string()));
    }

    if let Some(ref exp) = share.expires_at {
        if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
            if chrono::Utc::now() > exp_time {
                return Err((StatusCode::GONE, "This share link has expired".to_string()));
            }
        }
    }

    if share.has_password {
        let pass = query.password.as_deref().unwrap_or("");
        let valid = state.auth.verify_share_password(&token, pass).unwrap_or(false);
        if !valid {
            return Err((StatusCode::UNAUTHORIZED, "Password required for upload".to_string()));
        }
    }

    if share.require_email {
        let email = query.email.as_deref().unwrap_or("");
        let valid = state.auth.verify_share_email(&token, email).unwrap_or(false);
        if !valid {
            return Err((StatusCode::FORBIDDEN, "Authorized email required for upload".to_string()));
        }
    }

    let target_dir = Path::new(&share.path);
    if !target_dir.exists() || !target_dir.is_dir() {
        return Err((StatusCode::NOT_FOUND, "Target dropbox folder does not exist on server".to_string()));
    }

    let ip = extract_client_ip(&headers);
    let ua = extract_user_agent(&headers);
    let mut saved_count = 0;

    while let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))? {
        let raw_name = field.file_name().unwrap_or("uploaded_file");
        let safe_name = sanitize_uploaded_file_name(raw_name);
        if safe_name.is_empty() {
            continue;
        }

        let target_path = target_dir.join(&safe_name);
        let data = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        std::fs::write(&target_path, &data).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write file {}: {}", safe_name, e)))?;
        saved_count += 1;

        let _ = state.auth.log_share_access(
            share.id,
            &token,
            query.email.as_deref(),
            &ip,
            ua.as_deref(),
            "upload",
            Some(&safe_name),
        );
    }

    Ok(Json(serde_json::json!({ "success": true, "uploaded_files": saved_count })))
}

pub async fn handle_public_share_page(
    AxumPath(token): AxumPath<String>,
) -> Response {
    let html = format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
  <title>CommanderDog Showcase Portal</title>
  <link rel="icon" type="image/png" href="/assets/favicon.png">
  <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;600;700&family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
  <script src="https://cdn.jsdelivr.net/npm/three@0.160.0/build/three.min.js"></script>
  <style>
    :root {{
      --bg-dark: #121214;
      --bg-panel: #18181b;
      --bg-header: #202024;
      --bg-card: #27272a;
      --accent: #f59e0b;
      --accent-hover: #fbbf24;
      --accent-glow: rgba(245, 158, 11, 0.2);
      --text-main: #f4f4f5;
      --text-muted: #a1a1aa;
      --text-dim: #71717a;
      --border: #3f3f46;
      --radius: 8px;
      --font-mono: 'JetBrains Mono', monospace;
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; -webkit-tap-highlight-color: transparent; }}
    body {{
      font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
      background: var(--bg-dark);
      color: var(--text-main);
      min-height: 100vh;
      display: flex;
      flex-direction: column;
    }}
    .portal-nav {{
      background: var(--bg-header);
      border-bottom: 1px solid var(--border);
      padding: 10px 20px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      position: sticky;
      top: 0;
      z-index: 50;
    }}
    .nav-left {{
      display: flex;
      align-items: center;
      gap: 12px;
      min-width: 0;
    }}
    .nav-brand-logo {{
      width: 32px;
      height: 32px;
      border-radius: 6px;
      background: var(--accent-glow);
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 18px;
      flex-shrink: 0;
    }}
    .nav-titles {{
      display: flex;
      flex-direction: column;
      min-width: 0;
    }}
    .nav-title {{
      font-size: 14px;
      font-weight: 700;
      color: var(--text-main);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}
    .nav-subtitle {{
      font-size: 11px;
      color: var(--text-muted);
      display: flex;
      align-items: center;
      gap: 6px;
    }}
    .nav-right {{
      display: flex;
      align-items: center;
      gap: 8px;
      flex-shrink: 0;
    }}
    .badge {{
      display: inline-flex;
      align-items: center;
      gap: 4px;
      padding: 3px 8px;
      border-radius: 4px;
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.5px;
      text-transform: uppercase;
    }}
    .badge-showcase {{
      background: rgba(16, 185, 129, 0.15);
      color: #10b981;
      border: 1px solid rgba(16, 185, 129, 0.3);
    }}
    .badge-watermark {{
      background: rgba(245, 158, 11, 0.15);
      color: var(--accent);
      border: 1px solid rgba(245, 158, 11, 0.3);
    }}
    .badge-dropbox {{
      background: rgba(59, 130, 246, 0.15);
      color: #60a5fa;
      border: 1px solid rgba(59, 130, 246, 0.3);
    }}
    .btn {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 6px;
      padding: 7px 14px;
      font-size: 13px;
      font-weight: 600;
      border-radius: 6px;
      cursor: pointer;
      border: 1px solid transparent;
      transition: all 0.15s ease;
      text-decoration: none;
      user-select: none;
    }}
    .btn-accent {{
      background: var(--accent);
      color: #121214;
    }}
    .btn-accent:hover {{
      background: var(--accent-hover);
    }}
    .btn-outline {{
      background: transparent;
      border-color: var(--border);
      color: var(--text-main);
    }}
    .btn-outline:hover {{
      background: rgba(255, 255, 255, 0.05);
      border-color: var(--accent);
    }}
    .btn-icon {{
      padding: 6px;
      width: 32px;
      height: 32px;
    }}
    .portal-main {{
      flex: 1;
      padding: 24px;
      max-width: 1280px;
      width: 100%;
      margin: 0 auto;
      display: flex;
      flex-direction: column;
      gap: 20px;
    }}
    .toolbar-row {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      flex-wrap: wrap;
    }}
    .search-input {{
      background: var(--bg-panel);
      border: 1px solid var(--border);
      color: var(--text-main);
      padding: 8px 12px;
      border-radius: 6px;
      font-size: 13px;
      outline: none;
      min-width: 240px;
      flex: 1;
      max-width: 360px;
    }}
    .search-input:focus {{
      border-color: var(--accent);
    }}
    .grid-view {{
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: 16px;
    }}
    .list-view {{
      display: flex;
      flex-direction: column;
      gap: 8px;
    }}
    .file-card {{
      background: var(--bg-panel);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      padding: 14px;
      display: flex;
      flex-direction: column;
      gap: 10px;
      cursor: pointer;
      transition: transform 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease;
      position: relative;
    }}
    .file-card:hover {{
      border-color: var(--accent);
      transform: translateY(-2px);
      box-shadow: 0 6px 20px rgba(0,0,0,0.35);
    }}
    .file-preview-thumb {{
      height: 120px;
      background: var(--bg-header);
      border-radius: 4px;
      display: flex;
      align-items: center;
      justify-content: center;
      overflow: hidden;
      position: relative;
    }}
    .file-preview-thumb img {{
      max-width: 100%;
      max-height: 100%;
      object-fit: cover;
    }}
    .file-preview-thumb .type-icon {{
      font-size: 40px;
      color: var(--accent);
    }}
    .file-meta-row {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      font-size: 11px;
      color: var(--text-muted);
    }}
    .file-title {{
      font-weight: 600;
      font-size: 13px;
      word-break: break-all;
      line-height: 1.3;
    }}
    .list-row {{
      background: var(--bg-panel);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 10px 16px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      cursor: pointer;
      transition: border-color 0.15s ease;
    }}
    .list-row:hover {{
      border-color: var(--accent);
    }}
    .modal-backdrop {{
      position: fixed;
      top: 0; left: 0; right: 0; bottom: 0;
      background: rgba(0,0,0,0.85);
      backdrop-filter: blur(8px);
      z-index: 100;
      display: none;
      align-items: center;
      justify-content: center;
      padding: 16px;
    }}
    .modal-box {{
      background: var(--bg-panel);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      width: 100%;
      max-width: 480px;
      padding: 24px;
      box-shadow: 0 16px 40px rgba(0,0,0,0.6);
    }}
    .viewer-modal {{
      width: 100%;
      height: 100%;
      max-width: 1200px;
      max-height: 92vh;
      display: flex;
      flex-direction: column;
      padding: 0;
      overflow: hidden;
      position: relative;
    }}
    .viewer-header {{
      padding: 12px 16px;
      background: var(--bg-header);
      border-bottom: 1px solid var(--border);
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
    }}
    .viewer-stage {{
      flex: 1;
      display: flex;
      align-items: center;
      justify-content: center;
      background: #090a0f;
      position: relative;
      overflow: auto;
      user-select: none;
    }}
    .watermark-overlay {{
      position: absolute;
      top: 0; left: 0; right: 0; bottom: 0;
      pointer-events: none;
      z-index: 10;
      overflow: hidden;
    }}
    .dropzone {{
      border: 2px dashed var(--border);
      border-radius: var(--radius);
      padding: 24px;
      text-align: center;
      background: rgba(0,0,0,0.2);
      cursor: pointer;
      transition: all 0.2s ease;
    }}
    .dropzone.drag-over {{
      border-color: var(--accent);
      background: var(--accent-glow);
    }}
    @media (max-width: 600px) {{
      .portal-nav {{ padding: 8px 12px; }}
      .portal-main {{ padding: 12px; }}
      .grid-view {{ grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 10px; }}
      .file-preview-thumb {{ height: 90px; }}
      .search-input {{ min-width: 100%; max-width: 100%; }}
    }}
  </style>
</head>
<body>
  <!-- Navigation Header -->
  <header class="portal-nav">
    <div class="nav-left">
      <div class="nav-brand-logo">📦</div>
      <div class="nav-titles">
        <div class="nav-title" id="share-name-display">Brum Showcase Portal</div>
        <div class="nav-subtitle">
          <span id="share-meta-summary">Loading...</span>
          <span id="badge-container"></span>
        </div>
      </div>
    </div>
    <div class="nav-right" id="nav-actions">
      <button class="btn btn-accent" id="btn-portal-download" style="display: none;" onclick="downloadMainShare()">
        ⬇️ Download All
      </button>
    </div>
  </header>

  <!-- Main Showcase Container -->
  <main class="portal-main" id="portal-main">
    <div id="loading-state" style="text-align: center; padding: 60px 20px; color: var(--text-muted);">
      <div style="font-size: 28px; margin-bottom: 12px;">⏳</div>
      <div>Loading shared items...</div>
    </div>

    <div id="error-state" style="display: none; text-align: center; padding: 60px 20px; color: #ef4444;">
      <div style="font-size: 36px; margin-bottom: 12px;">❌</div>
      <div id="error-message" style="font-size: 15px; font-weight: 600;">Share link unavailable</div>
    </div>

    <!-- Active Showcase Content -->
    <div id="showcase-content" style="display: none; flex-direction: column; gap: 20px;">
      <div class="toolbar-row">
        <input type="text" id="file-search-input" class="search-input" placeholder="🔍 Filter files in showcase..." oninput="filterFiles(this.value)">
        <div style="display: flex; gap: 6px; align-items: center;">
          <button class="btn btn-outline btn-icon" id="btn-view-grid" onclick="setViewMode('grid')" title="Grid View">▦</button>
          <button class="btn btn-outline btn-icon" id="btn-view-list" onclick="setViewMode('list')" title="List View">☰</button>
        </div>
      </div>

      <!-- Single File Card (if sharing single file) -->
      <div id="single-file-section" style="display: none;">
        <div class="file-card" style="max-width: 460px; margin: 0 auto; text-align: center;" onclick="openSingleFilePreview()">
          <div class="file-preview-thumb" id="single-thumb">
            <span class="type-icon" id="single-type-icon">📄</span>
          </div>
          <div class="file-title" id="single-title">file_name</div>
          <div class="file-meta-row" style="justify-content: center; gap: 12px;">
            <span id="single-size">0 B</span>
            <span id="single-date">--</span>
          </div>
          <button class="btn btn-accent" id="single-preview-btn" style="margin-top: 8px;">👁️ Preview in Viewer</button>
        </div>
      </div>

      <!-- Directory Gallery View -->
      <div id="gallery-container" class="grid-view"></div>

      <!-- Guest Upload Dropzone -->
      <div id="guest-dropbox-container" style="display: none; margin-top: 20px;">
        <div class="dropzone" id="dropzone" onclick="document.getElementById('file-input').click()">
          <input type="file" id="file-input" multiple style="display:none;" onchange="handleGuestUpload(this.files)">
          <div style="font-size: 28px; margin-bottom: 8px;">📥</div>
          <div style="font-weight: 700; font-size: 14px;">Guest Upload Dropbox</div>
          <div style="font-size: 12px; color: var(--text-muted); margin-top: 4px;">Drag & drop files here or click to browse</div>
          <div id="upload-status-text" style="font-size: 12px; color: var(--accent); margin-top: 8px;"></div>
        </div>
      </div>
    </div>
  </main>

  <!-- Password Gate Modal -->
  <div class="modal-backdrop" id="password-gate-modal">
    <div class="modal-box">
      <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 14px;">
        <span style="font-size: 24px;">🔒</span>
        <div style="font-weight: 700; font-size: 16px;">Password Protected Share</div>
      </div>
      <p style="font-size: 13px; color: var(--text-muted); margin-bottom: 14px;">Enter the password provided by the author to unlock this showcase:</p>
      <input type="password" id="gate-pass-input" class="search-input" style="width: 100%; margin-bottom: 14px;" placeholder="Enter password..." onkeydown="if(event.key==='Enter') submitPasswordGate()">
      <div id="gate-pass-error" style="color: #ef4444; font-size: 12px; margin-bottom: 10px; display: none;"></div>
      <button class="btn btn-accent" style="width: 100%;" onclick="submitPasswordGate()">Unlock Showcase</button>
    </div>
  </div>

  <!-- Email Whitelist Gate Modal -->
  <div class="modal-backdrop" id="email-gate-modal">
    <div class="modal-box">
      <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 14px;">
        <span style="font-size: 24px;">🛡️</span>
        <div style="font-weight: 700; font-size: 16px;">Authorized Guest Access</div>
      </div>
      <p style="font-size: 13px; color: var(--text-muted); margin-bottom: 14px;">This showcase requires an authorized guest email. Enter your email to proceed:</p>
      <input type="email" id="gate-email-input" class="search-input" style="width: 100%; margin-bottom: 14px;" placeholder="name@company.com" onkeydown="if(event.key==='Enter') submitEmailGate()">
      <div id="gate-email-error" style="color: #ef4444; font-size: 12px; margin-bottom: 10px; display: none;"></div>
      <button class="btn btn-accent" style="width: 100%;" onclick="submitEmailGate()">Verify & Enter</button>
    </div>
  </div>

  <!-- Internal Showcase Viewer Modal (Lightbox / Document Reader / Media Player) -->
  <div class="modal-backdrop" id="viewer-modal">
    <div class="modal-box viewer-modal">
      <div class="viewer-header">
        <div style="display: flex; align-items: center; gap: 8px; min-width: 0;">
          <span id="viewer-icon" style="font-size: 18px;">📄</span>
          <div id="viewer-filename" style="font-weight: 700; font-size: 13px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">filename</div>
        </div>
        <div style="display: flex; align-items: center; gap: 8px;">
          <button class="btn btn-accent btn-sm" id="btn-viewer-download" style="display: none;" onclick="downloadCurrentPreviewFile()">⬇️ Download</button>
          <button class="btn btn-outline btn-icon" onclick="closeViewerModal()" title="Close Viewer">✕</button>
        </div>
      </div>
      <div class="viewer-stage" id="viewer-stage" oncontextmenu="return !shareMeta?.watermark_enabled;">
        <!-- Dynamic Watermark Canvas Overlay -->
        <canvas id="watermark-canvas" class="watermark-overlay" style="display: none;"></canvas>
        <div id="viewer-mount" style="width: 100%; height: 100%; display: flex; align-items: center; justify-content: center; overflow: auto;"></div>
      </div>
    </div>
  </div>

  <script>
    const token = "{token}";
    let shareMeta = null;
    let verifiedPassword = sessionStorage.getItem(`brum_share_pass_${{token}}`) || '';
    let verifiedEmail = sessionStorage.getItem(`brum_share_email_${{token}}`) || '';
    let currentViewMode = 'grid';
    let currentPreviewFile = null;

    function formatBytes(bytes) {{
      if (!bytes || bytes === 0) return '0 B';
      const k = 1024;
      const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i];
    }}

    function getFileType(name, mime) {{
      const ext = name.split('.').pop().toLowerCase();
      if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp'].includes(ext) || mime.startsWith('image/')) return 'image';
      if (['mp4', 'webm', 'mov', 'mkv', 'avi'].includes(ext) || mime.startsWith('video/')) return 'video';
      if (['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a'].includes(ext) || mime.startsWith('audio/')) return 'audio';
      if (ext === 'pdf' || mime === 'application/pdf') return 'pdf';
      if (['stl', 'obj', 'gltf', 'glb', '3mf', 'step', 'stp', 'iges', 'igs', 'dxf', 'ply', 'off'].includes(ext)) return '3d';
      if (['txt', 'md', 'json', 'rs', 'js', 'ts', 'html', 'css', 'toml', 'yaml', 'yml', 'py', 'c', 'cpp', 'h', 'sh', 'sql', 'log'].includes(ext) || mime.startsWith('text/')) return 'text';
      return 'generic';
    }}

    async function loadShare() {{
      try {{
        const res = await fetch(`/api/public/shares/${{token}}`);
        if (!res.ok) {{
          const err = await res.text();
          showError(err || 'Share link unavailable');
          return;
        }}
        shareMeta = await res.json();
        
        if (shareMeta.require_email && !verifiedEmail) {{
          document.getElementById('email-gate-modal').style.display = 'flex';
          return;
        }}

        if (shareMeta.has_password && !verifiedPassword) {{
          document.getElementById('password-gate-modal').style.display = 'flex';
          return;
        }}

        initShowcaseView();
      }} catch (e) {{
        showError('Network error connecting to share portal');
      }}
    }}

    function showError(msg) {{
      document.getElementById('loading-state').style.display = 'none';
      document.getElementById('showcase-content').style.display = 'none';
      document.getElementById('error-state').style.display = 'block';
      document.getElementById('error-message').textContent = msg;
    }}

    async function submitPasswordGate() {{
      const pass = document.getElementById('gate-pass-input').value;
      if (!pass) return;
      const errEl = document.getElementById('gate-pass-error');
      errEl.style.display = 'none';

      try {{
        const res = await fetch(`/api/public/shares/${{token}}/verify`, {{
          method: 'POST',
          headers: {{ 'Content-Type': 'application/json' }},
          body: JSON.stringify({{ password: pass }})
        }});
        if (res.ok) {{
          verifiedPassword = pass;
          sessionStorage.setItem(`brum_share_pass_${{token}}`, pass);
          document.getElementById('password-gate-modal').style.display = 'none';
          initShowcaseView();
        }} else {{
          errEl.textContent = 'Incorrect password. Please try again.';
          errEl.style.display = 'block';
        }}
      }} catch (e) {{
        errEl.textContent = 'Verification error';
        errEl.style.display = 'block';
      }}
    }}

    async function submitEmailGate() {{
      const email = document.getElementById('gate-email-input').value.trim();
      if (!email) return;
      const errEl = document.getElementById('gate-email-error');
      errEl.style.display = 'none';

      try {{
        const res = await fetch(`/api/public/shares/${{token}}/verify-email`, {{
          method: 'POST',
          headers: {{ 'Content-Type': 'application/json' }},
          body: JSON.stringify({{ email: email }})
        }});
        if (res.ok) {{
          verifiedEmail = email;
          sessionStorage.setItem(`brum_share_email_${{token}}`, email);
          document.getElementById('email-gate-modal').style.display = 'none';
          if (shareMeta.has_password && !verifiedPassword) {{
            document.getElementById('password-gate-modal').style.display = 'flex';
          }} else {{
            initShowcaseView();
          }}
        }} else {{
          errEl.textContent = 'Email address not authorized for this showcase.';
          errEl.style.display = 'block';
        }}
      }} catch (e) {{
        errEl.textContent = 'Verification network error';
        errEl.style.display = 'block';
      }}
    }}

    function initShowcaseView() {{
      document.getElementById('loading-state').style.display = 'none';
      document.getElementById('showcase-content').style.display = 'flex';
      document.getElementById('share-name-display').textContent = shareMeta.name;

      const badges = [];
      if (!shareMeta.allow_download) badges.push('<span class="badge badge-showcase">Showcase Mode (View Only)</span>');
      if (shareMeta.watermark_enabled) badges.push('<span class="badge badge-watermark">Watermarked</span>');
      if (shareMeta.allow_upload) badges.push('<span class="badge badge-dropbox">Guest Dropbox</span>');
      document.getElementById('badge-container').innerHTML = badges.join(' ');

      if (shareMeta.allow_download) {{
        const btnDl = document.getElementById('btn-portal-download');
        btnDl.style.display = 'inline-flex';
        btnDl.textContent = shareMeta.is_dir ? '⬇️ Download Folder (.zip)' : '⬇️ Download File';
      }}

      if (shareMeta.is_dir) {{
        document.getElementById('share-meta-summary').textContent = `${{shareMeta.files ? shareMeta.files.length : 0}} items`;
        renderGallery(shareMeta.files || []);
        if (shareMeta.allow_upload) {{
          document.getElementById('guest-dropbox-container').style.display = 'block';
          setupDropzone();
        }}
      }} else {{
        document.getElementById('share-meta-summary').textContent = formatBytes(shareMeta.size);
        document.getElementById('single-file-section').style.display = 'block';
        document.getElementById('single-title').textContent = shareMeta.name;
        document.getElementById('single-size').textContent = formatBytes(shareMeta.size);
        document.getElementById('single-date').textContent = shareMeta.created_at ? new Date(shareMeta.created_at).toLocaleDateString() : '';
      }}
    }}

    function renderGallery(files) {{
      const container = document.getElementById('gallery-container');
      container.className = currentViewMode === 'grid' ? 'grid-view' : 'list-view';

      if (!files || files.length === 0) {{
        container.innerHTML = '<div style="grid-column: 1/-1; text-align: center; color: var(--text-muted); padding: 40px;">No files available in this showcase.</div>';
        return;
      }}

      container.innerHTML = files.map(f => {{
        const type = getFileType(f.name, f.mime);
        let icon = '📄';
        if (type === 'image') icon = '🖼️';
        else if (type === 'video') icon = '🎬';
        else if (type === 'audio') icon = '🎵';
        else if (type === 'pdf') icon = '📑';
        else if (type === '3d') icon = '🧊';
        else if (f.is_dir) icon = '📁';

        const previewUrl = `/api/public/shares/${{token}}/preview?file=${{encodeURIComponent(f.rel_path)}}${{verifiedPassword ? '&password=' + encodeURIComponent(verifiedPassword) : ''}}${{verifiedEmail ? '&email=' + encodeURIComponent(verifiedEmail) : ''}}`;

        if (currentViewMode === 'grid') {{
          const thumbHtml = type === 'image'
            ? `<img src="${{previewUrl}}" alt="${{f.name}}" loading="lazy" onerror="this.outerHTML='<span class=\"type-icon\">🖼️</span>'">`
            : `<span class="type-icon">${{icon}}</span>`;

          return `
            <div class="file-card" onclick="openFilePreview('${{encodeURIComponent(f.rel_path)}}', '${{escapeHtml(f.name)}}', '${{f.mime}}', ${{f.size}})">
              <div class="file-preview-thumb">${{thumbHtml}}</div>
              <div class="file-title" title="${{escapeHtml(f.name)}}">${{escapeHtml(f.name)}}</div>
              <div class="file-meta-row">
                <span>${{formatBytes(f.size)}}</span>
                <span>${{f.mtime ? new Date(f.mtime * 1000).toLocaleDateString() : ''}}</span>
              </div>
            </div>
          `;
        }} else {{
          return `
            <div class="list-row" onclick="openFilePreview('${{encodeURIComponent(f.rel_path)}}', '${{escapeHtml(f.name)}}', '${{f.mime}}', ${{f.size}})">
              <div style="display: flex; align-items: center; gap: 12px; min-width: 0;">
                <span style="font-size: 20px;">${{icon}}</span>
                <div style="font-weight: 600; font-size: 13px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">${{escapeHtml(f.name)}}</div>
              </div>
              <div style="display: flex; align-items: center; gap: 16px; font-size: 12px; color: var(--text-muted);">
                <span>${{formatBytes(f.size)}}</span>
                <span>${{f.mtime ? new Date(f.mtime * 1000).toLocaleDateString() : ''}}</span>
              </div>
            </div>
          `;
        }}
      }}).join('');
    }}

    function filterFiles(query) {{
      if (!shareMeta || !shareMeta.files) return;
      const q = query.toLowerCase().trim();
      const filtered = shareMeta.files.filter(f => f.name.toLowerCase().includes(q));
      renderGallery(filtered);
    }}

    function setViewMode(mode) {{
      currentViewMode = mode;
      renderGallery(shareMeta.files || []);
    }}

    function openSingleFilePreview() {{
      openFilePreview('', shareMeta.name, 'application/octet-stream', shareMeta.size);
    }}

    async function openFilePreview(relPath, filename, mime, size) {{
      currentPreviewFile = {{ relPath, filename, mime, size }};
      const type = getFileType(filename, mime);
      const mount = document.getElementById('viewer-mount');
      mount.innerHTML = '<div style="color:var(--text-muted); padding:20px;">Loading preview...</div>';

      document.getElementById('viewer-filename').textContent = filename;
      document.getElementById('viewer-icon').textContent = type === 'image' ? '🖼️' : (type === 'video' ? '🎬' : (type === 'audio' ? '🎵' : (type === '3d' ? '🧊' : '📄')));
      
      const btnDl = document.getElementById('btn-viewer-download');
      if (shareMeta.allow_download) {{
        btnDl.style.display = 'inline-flex';
      }} else {{
        btnDl.style.display = 'none';
      }}

      document.getElementById('viewer-modal').style.display = 'flex';

      const fileParam = relPath ? `&file=${{relPath}}` : '';
      const passParam = verifiedPassword ? `&password=${{encodeURIComponent(verifiedPassword)}}` : '';
      const emailParam = verifiedEmail ? `&email=${{encodeURIComponent(verifiedEmail)}}` : '';
      const previewUrl = `/api/public/shares/${{token}}/preview?${{fileParam ? fileParam.slice(1) : ''}}${{passParam}}${{emailParam}}`;

      // Render Dynamic Watermark
      if (shareMeta.watermark_enabled) {{
        renderWatermarkOverlay();
      }} else {{
        document.getElementById('watermark-canvas').style.display = 'none';
      }}

      if (type === 'image') {{
        mount.innerHTML = `<img src="${{previewUrl}}" style="max-width:100%; max-height:100%; object-fit:contain; border-radius:4px;" draggable="false" oncontextmenu="return false;">`;
      }} else if (type === 'video') {{
        mount.innerHTML = `<video src="${{previewUrl}}" controls autoplay playsinline style="max-width:100%; max-height:100%;"></video>`;
      }} else if (type === 'audio') {{
        mount.innerHTML = `
          <div style="padding: 40px; text-align: center; background: var(--bg-card); border-radius: 8px; border: 1px solid var(--border);">
            <div style="font-size: 48px; margin-bottom: 16px;">🎵</div>
            <div style="font-weight: 700; margin-bottom: 12px;">${{escapeHtml(filename)}}</div>
            <audio src="${{previewUrl}}" controls autoplay style="width: 100%; max-width: 380px;"></audio>
          </div>
        `;
      }} else if (type === 'pdf') {{
        mount.innerHTML = `<iframe src="${{previewUrl}}" style="width:100%; height:100%; border:none;"></iframe>`;
      }} else if (type === '3d') {{
        mount.innerHTML = `
          <div style="width:100%; height:100%; position:relative; display:flex; flex-direction:column; background:#121214; border-radius:6px; overflow:hidden;">
            <canvas id="showcase-3d-canvas" style="width:100%; height:100%; display:block;"></canvas>
            <div style="position:absolute; bottom:8px; left:8px; font-size:10px; color:var(--text-muted); background:rgba(0,0,0,0.6); padding:4px 8px; border-radius:4px; pointer-events:none;">
              3D CAD Preview • Drag to rotate • Wheel to zoom
            </div>
          </div>
        `;
        initShowcase3dViewer(previewUrl, filename);
      }} else if (type === 'text') {{
        try {{
          const txtRes = await fetch(previewUrl);
          const txt = await txtRes.text();
          mount.innerHTML = `
            <pre style="width:100%; height:100%; margin:0; padding:16px; background:#0d0e12; color:#f4f4f5; font-family:var(--font-mono); font-size:12px; line-height:1.5; overflow:auto; white-space:pre-wrap;">${{escapeHtml(txt)}}</pre>
          `;
        }} catch (e) {{
          mount.innerHTML = '<div style="color:#ef4444;">Failed to load text preview</div>';
        }}
      }} else {{
        mount.innerHTML = `
          <div style="padding: 40px; text-align: center; color: var(--text-muted);">
            <div style="font-size: 48px; margin-bottom: 12px;">📄</div>
            <div style="font-weight: 600; font-size: 14px; margin-bottom: 8px;">${{escapeHtml(filename)}}</div>
            <div style="font-size: 12px; margin-bottom: 16px;">Inline preview not supported for this file format.</div>
            ${{shareMeta.allow_download ? `<button class="btn btn-accent" onclick="downloadCurrentPreviewFile()">⬇️ Download File (${{formatBytes(size)}})</button>` : ''}}
          </div>
        `;
      }}
    }}

    async function initShowcase3dViewer(url, filename) {{
      const canvas = document.getElementById('showcase-3d-canvas');
      if (!canvas || !window.THREE) return;
      const parent = canvas.parentElement;
      const width = parent.clientWidth || 600;
      const height = parent.clientHeight || 450;

      const scene = new THREE.Scene();
      scene.background = new THREE.Color(0x121214);
      const camera = new THREE.PerspectiveCamera(45, width / height, 0.1, 10000);
      camera.position.set(100, 100, 100);

      const renderer = new THREE.WebGLRenderer({{ canvas: canvas, antialias: true, preserveDrawingBuffer: true }});
      renderer.setSize(width, height);
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));

      scene.add(new THREE.HemisphereLight(0xffffff, 0x444444, 1.2));
      const dl = new THREE.DirectionalLight(0xffffff, 1.5);
      dl.position.set(100, 150, 100);
      scene.add(dl);

      let theta = Math.PI / 4, phi = Math.PI / 3, radius = 100;
      let target = new THREE.Vector3(0, 0, 0);

      function updateCam() {{
        camera.position.set(
          target.x + radius * Math.sin(phi) * Math.sin(theta),
          target.y + radius * Math.cos(phi),
          target.z + radius * Math.sin(phi) * Math.cos(theta)
        );
        camera.lookAt(target);
      }}

      let isDragging = false, startX = 0, startY = 0, startTheta = 0, startPhi = 0;
      canvas.addEventListener('mousedown', e => {{
        isDragging = true;
        startX = e.clientX; startY = e.clientY;
        startTheta = theta; startPhi = phi;
      }});
      window.addEventListener('mousemove', e => {{
        if (!isDragging) return;
        theta = startTheta - (e.clientX - startX) * 0.006;
        phi = Math.max(0.01, Math.min(Math.PI - 0.01, startPhi - (e.clientY - startY) * 0.006));
        updateCam();
      }});
      window.addEventListener('mouseup', () => {{ isDragging = false; }});
      canvas.addEventListener('wheel', e => {{
        e.preventDefault();
        radius = Math.max(1, Math.min(10000, radius * (e.deltaY > 0 ? 1.1 : 0.9)));
        updateCam();
      }}, {{ passive: false }});

      function animate() {{
        if (document.getElementById('showcase-3d-canvas') === canvas) {{
          renderer.render(scene, camera);
          requestAnimationFrame(animate);
        }}
      }}
      animate();

      try {{
        const ext = filename.split('.').pop().toLowerCase();
        const res = await fetch(url);
        if (!res.ok) return;

        let geom = null;
        if (ext === 'stl') {{
          const buffer = await res.arrayBuffer();
          const dv = new DataView(buffer);
          let isBin = buffer.byteLength >= 84 && (84 + dv.getUint32(80, true) * 50 === buffer.byteLength);
          if (isBin) {{
            const count = dv.getUint32(80, true);
            const pos = new Float32Array(count * 9);
            let off = 84, pIdx = 0;
            for (let i = 0; i < count; i++) {{
              off += 12;
              for (let v = 0; v < 3; v++) {{
                pos[pIdx++] = dv.getFloat32(off, true);
                pos[pIdx++] = dv.getFloat32(off + 4, true);
                pos[pIdx++] = dv.getFloat32(off + 8, true);
                off += 12;
              }}
              off += 2;
            }}
            geom = new THREE.BufferGeometry();
            geom.setAttribute('position', new THREE.BufferAttribute(pos, 3));
            geom.computeVertexNormals();
          }}
        }} else if (ext === 'obj') {{
          const txt = await res.text();
          const verts = [], pos = [];
          for (const line of txt.split('\n')) {{
            const p = line.trim().split(/\s+/);
            if (p[0] === 'v') verts.push([parseFloat(p[1]), parseFloat(p[2]), parseFloat(p[3])]);
            else if (p[0] === 'f' && p.length >= 4) {{
              const i0 = parseInt(p[1]) - 1, i1 = parseInt(p[2]) - 1, i2 = parseInt(p[3]) - 1;
              if (verts[i0] && verts[i1] && verts[i2]) {{
                pos.push(...verts[i0], ...verts[i1], ...verts[i2]);
              }}
            }}
          }}
          if (pos.length > 0) {{
            geom = new THREE.BufferGeometry();
            geom.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pos), 3));
            geom.computeVertexNormals();
          }}
        }}

        if (geom) {{
          geom.computeBoundingBox();
          const box = geom.boundingBox;
          const center = new THREE.Vector3();
          box.getCenter(center);
          geom.translate(-center.x, -box.min.y, -center.z);
          geom.computeBoundingBox();
          geom.computeBoundingSphere();

          const size = new THREE.Vector3();
          geom.boundingBox.getSize(size);
          const maxDim = Math.max(size.x, size.y, size.z) || 50;
          radius = maxDim * 2.2;
          target.set(0, size.y / 2, 0);
          updateCam();

          const mat = new THREE.MeshStandardMaterial({{ color: 0x475569, roughness: 0.35, metalness: 0.3, side: THREE.DoubleSide }});
          const mesh = new THREE.Mesh(geom, mat);
          scene.add(mesh);

          const grid = new THREE.GridHelper(Math.ceil(maxDim * 2 / 10) * 10, 20, 0xf59e0b, 0x27272a);
          scene.add(grid);
        }}
      }} catch (e) {{
        console.error('Showcase 3D viewer error:', e);
      }}
    }}

    function renderWatermarkOverlay() {{
      const canvas = document.getElementById('watermark-canvas');
      const stage = document.getElementById('viewer-stage');
      canvas.style.display = 'block';
      canvas.width = stage.clientWidth || 800;
      canvas.height = stage.clientHeight || 600;

      const ctx = canvas.getContext('2d');
      ctx.clearRect(0, 0, canvas.width, canvas.height);

      let text = shareMeta.watermark_text || 'CONFIDENTIAL • {{email}} • {{date}}';
      const guestId = verifiedEmail || 'GUEST';
      const today = new Date().toISOString().slice(0, 10);
      text = text.replace(/\{{email\}}/gi, guestId).replace(/\{{date\}}/gi, today);

      ctx.font = 'bold 15px sans-serif';
      ctx.fillStyle = 'rgba(245, 158, 11, 0.18)';
      ctx.textAlign = 'center';

      const stepX = 240;
      const stepY = 160;
      ctx.rotate(-25 * Math.PI / 180);

      for (let x = -canvas.width; x < canvas.width * 2; x += stepX) {{
        for (let y = -canvas.height; y < canvas.height * 2; y += stepY) {{
          ctx.fillText(text, x, y);
        }}
      }}
      ctx.setTransform(1, 0, 0, 1, 0, 0);
    }}

    function closeViewerModal() {{
      document.getElementById('viewer-modal').style.display = 'none';
      document.getElementById('viewer-mount').innerHTML = '';
      currentPreviewFile = null;
    }}

    function downloadMainShare() {{
      const passParam = verifiedPassword ? `?password=${{encodeURIComponent(verifiedPassword)}}` : '';
      const emailParam = verifiedEmail ? `${{passParam ? '&' : '?'}}email=${{encodeURIComponent(verifiedEmail)}}` : '';
      window.location.href = `/api/public/shares/${{token}}/download${{passParam}}${{emailParam}}`;
    }}

    function downloadCurrentPreviewFile() {{
      if (!currentPreviewFile) return;
      const fileParam = currentPreviewFile.relPath ? `?file=${{encodeURIComponent(currentPreviewFile.relPath)}}` : '';
      const passParam = verifiedPassword ? `${{fileParam ? '&' : '?'}}password=${{encodeURIComponent(verifiedPassword)}}` : '';
      const emailParam = verifiedEmail ? `${{fileParam || passParam ? '&' : '?'}}email=${{encodeURIComponent(verifiedEmail)}}` : '';
      window.location.href = `/api/public/shares/${{token}}/download${{fileParam}}${{passParam}}${{emailParam}}`;
    }}

    function setupDropzone() {{
      const dz = document.getElementById('dropzone');
      dz.ondragover = (e) => {{ e.preventDefault(); dz.classList.add('drag-over'); }};
      dz.ondragleave = () => dz.classList.remove('drag-over');
      dz.ondrop = (e) => {{
        e.preventDefault();
        dz.classList.remove('drag-over');
        if (e.dataTransfer.files) handleGuestUpload(e.dataTransfer.files);
      }};
    }}

    async function handleGuestUpload(files) {{
      if (!files || files.length === 0) return;
      const status = document.getElementById('upload-status-text');
      status.textContent = `Uploading ${{files.length}} file(s)...`;

      const fd = new FormData();
      for (let f of files) fd.append('files', f);

      const passParam = verifiedPassword ? `?password=${{encodeURIComponent(verifiedPassword)}}` : '';
      const emailParam = verifiedEmail ? `${{passParam ? '&' : '?'}}email=${{encodeURIComponent(verifiedEmail)}}` : '';

      try {{
        const res = await fetch(`/api/public/shares/${{token}}/upload${{passParam}}${{emailParam}}`, {{
          method: 'POST',
          body: fd
        }});
        if (res.ok) {{
          status.textContent = `✅ Successfully uploaded ${{files.length}} file(s)!`;
          setTimeout(() => {{ loadShare(); }}, 1000);
        }} else {{
          status.textContent = `❌ Upload failed: ${{await res.text()}}`;
        }}
      }} catch (e) {{
        status.textContent = '❌ Upload failed due to network error';
      }}
    }}

    function escapeHtml(str) {{
      return String(str).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
    }}

    window.addEventListener('resize', () => {{
      if (shareMeta && shareMeta.watermark_enabled && document.getElementById('viewer-modal').style.display === 'flex') {{
        renderWatermarkOverlay();
      }}
    }});

    window.addEventListener('keydown', (e) => {{
      if (e.key === 'Escape') {{
        closeViewerModal();
      }}
    }});

    loadShare();
  </script>
</body>
</html>"#, token = token);

    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "HTML load error").into_response())
}
