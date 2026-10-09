use std::path::Path;
use std::fs;
use std::sync::Arc;
use std::collections::HashMap;
use axum::{
    body::Body,
    extract::{Multipart, Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Json, Response},
};
use serde::Deserialize;
use crate::server::{AppState, ManualsAsset};
use crate::server::middleware::{
    extract_claims_or_local, sanitize_uploaded_relative_path, validate_path_access,
};
use crate::vfs::local::LocalFs;
use crate::vfs::archive::ArchiveHandler;
use crate::vfs::checksum::calculate_checksum;
use crate::vfs::sftp::SftpClient;
use crate::vfs::webdav::WebDavClient;
use crate::vfs::DirectoryListing;
use crate::tools::tasks::TaskInfo;

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct ListQuery {
    path: Option<String>,
    show_hidden: Option<bool>,
    flat: Option<bool>,
    max_depth: Option<usize>,
    max_entries: Option<usize>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    pass: Option<String>,
    windows_native_ops: Option<bool>,
    windows_native_file_ops: Option<bool>,
    custom_trash_dir: Option<String>,
}

#[derive(Deserialize)]
pub struct GetTagsQuery {
    path: Option<String>,
}

pub async fn handle_get_file_tags(
    State(state): State<AppState>,
    Query(query): Query<GetTagsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Some(p) = query.path {
        let tag_info = state.tags.get_tags_for_path(&p);
        Ok(Json(serde_json::json!({ "success": true, "tag": tag_info })))
    } else {
        let all = state.tags.get_all_tags().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(Json(serde_json::json!({ "success": true, "tags": all })))
    }
}

pub async fn handle_get_all_tags(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let all = state.tags.get_all_tags().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "success": true, "tags": all })))
}

pub async fn handle_set_tags(
    State(state): State<AppState>,
    Json(payload): Json<crate::tools::tags::SetTagsRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let count = state.tags.set_tags(payload).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "success": true, "count": count })))
}

pub async fn handle_list_dir(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<DirectoryListing>, (StatusCode, String)> {
    let raw_path = query.path.unwrap_or_else(|| state.config.server.root_path.clone());
    let target_path = validate_path_access(&state, &headers, &raw_path, false)?;
    let show_hidden = query.show_hidden.unwrap_or(state.config.ui.show_hidden_files);

    if target_path == "trash://" || target_path == "recycle://" || target_path == "shell:recyclebinfolder" {
        let claims = extract_claims_or_local(&state, &headers)?;
        let custom_trash = query.custom_trash_dir.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
        let use_native = query.windows_native_ops
            .or(query.windows_native_file_ops)
            .unwrap_or(state.config.paranoid.windows_native_file_ops);
        let listing = crate::tools::trash::TrashManager::list_trash_directory_entries(
            custom_trash,
            Some(&claims.home_dir),
            use_native,
        );
        return Ok(Json(listing));
    } else if target_path.starts_with("vault://") {
        let rest = target_path.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        state.vaults.list_vault_contents(vault_file, subpath)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))
    } else if target_path.starts_with("archive://") {
        let rest = target_path.strip_prefix("archive://").unwrap();
        let parts: Vec<&str> = rest.split('#').collect();
        let archive_file = parts[0].to_string();
        let subpath = if parts.len() > 1 { parts[1].to_string() } else { String::new() };
        tokio::task::spawn_blocking(move || {
            ArchiveHandler::list_archive_contents(&archive_file, &subpath)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Archive listing task join error: {}", e)))?
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to list archive: {}", e)))
    } else if target_path.starts_with("sftp://") || target_path.starts_with("ssh://") {
        let params = SftpClient::parse_uri(&target_path, query.user.as_deref(), query.pass.as_deref())
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?
            .with_config(&state.config.get_sftp_config());
        SftpClient::list_dir(&params)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("SFTP list failed: {}", e)))
    } else if target_path.starts_with("webdav://") || target_path.starts_with("http://") || target_path.starts_with("https://") {
        let url = if target_path.starts_with("webdav://") {
            format!("http://{}", target_path.strip_prefix("webdav://").unwrap())
        } else {
            target_path
        };
        WebDavClient::list_dir(&url, query.user.as_deref(), query.pass.as_deref()).await
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to list WebDAV: {}", e)))
    } else if target_path.starts_with("proton://") {
        let entries = crate::vfs::proton::ProtonDriveClient::list_directory(&target_path).await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Proton Drive list failed: {}", e)))?;
        let total_files = entries.iter().filter(|e| !e.is_dir).count();
        let total_dirs = entries.iter().filter(|e| e.is_dir).count();
        let total_size = entries.iter().map(|e| e.size).sum();
        let parent_path = if target_path == "proton://" || target_path == "proton:///" {
            None
        } else {
            let clean = target_path.trim_start_matches("proton://").trim_start_matches('/');
            let p = std::path::Path::new(clean);
            p.parent().and_then(|par| {
                let s = par.to_string_lossy().to_string();
                if s.is_empty() { Some("proton:///".to_string()) } else { Some(format!("proton:///{}", s)) }
            })
        };

        Ok(Json(DirectoryListing {
            current_path: target_path,
            parent_path,
            entries,
            total_files,
            total_dirs,
            total_size,
            protocol: "proton".to_string(),
            is_truncated: None,
            max_limit: None,
        }))
    } else if target_path.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&target_path, query.user.as_deref(), query.pass.as_deref())
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        crate::vfs::smb::SmbClient::list_dir(&params)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("SMB list failed: {}", e)))
    } else if target_path.starts_with("nfs://") {
        let params = crate::vfs::nfs::NfsClient::parse_uri(&target_path)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid NFS URI: {}", e)))?;
        crate::vfs::nfs::NfsClient::list_dir(&params, show_hidden)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("NFS list failed: {}", e)))
    } else if query.flat.unwrap_or(false) {
        let max_d = query.max_depth;
        let max_e = query.max_entries;
        let p = target_path.clone();
        tokio::task::spawn_blocking(move || {
            LocalFs::list_branch_view(&p, show_hidden, max_d, max_e)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Branch view task failed: {}", e)))?
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to list branch view: {}", e)))
    } else {
        let is_trash_files = target_path.ends_with(".local/share/Trash/files")
            || target_path.ends_with(r".local\share\Trash\files")
            || target_path.ends_with("/brum_trash/files");
        if is_trash_files {
            let trash_p = LocalFs::resolve_local_path(&target_path);
            if !trash_p.exists() {
                let _ = std::fs::create_dir_all(&trash_p);
            }
        }
        LocalFs::list_dir(&target_path, show_hidden)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to list local directory: {}", e)))
    }
}

#[derive(Deserialize)]
pub struct ReadFileQuery {
    path: String,
    max_bytes: Option<usize>,
}

pub async fn handle_read_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReadFileQuery>,
) -> Result<Json<crate::vfs::FileContentResponse>, (StatusCode, String)> {
    let target_path = validate_path_access(&state, &headers, &query.path, false)?;
    let max_b = query.max_bytes.unwrap_or(10_000_000);

    if target_path.starts_with("vault://") {
        let rest = target_path.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        state.vaults.read_vault_file(vault_file, subpath)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))
    } else if target_path.starts_with("archive://") {
        let rest = target_path.strip_prefix("archive://").unwrap();
        let parts: Vec<&str> = rest.split('#').collect();
        let archive_file = parts[0].to_string();
        let subpath = if parts.len() > 1 { parts[1].to_string() } else { String::new() };
        tokio::task::spawn_blocking(move || {
            ArchiveHandler::read_archive_entry(&archive_file, &subpath, max_b)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Archive read task join error: {}", e)))?
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read archive item: {}", e)))
    } else if target_path.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        crate::vfs::smb::SmbClient::read_file(&params)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read SMB file: {}", e)))
    } else if target_path.starts_with("sftp://") || target_path.starts_with("ssh://") {
        let params = SftpClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?
            .with_config(&state.config.get_sftp_config());
        let bytes = SftpClient::download_file_with_params(&params, max_b)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read SFTP file: {}", e)))?;
        let mime = mime_guess::from_path(&params.remote_path).first_or_octet_stream().to_string();
        let is_text = mime.starts_with("text/") || mime.contains("json") || mime.contains("javascript") || mime.contains("xml") || mime.contains("yaml") || mime.contains("toml");
        use base64::Engine;
        let content = if is_text {
            String::from_utf8(bytes.clone()).unwrap_or_else(|_| base64::engine::general_purpose::STANDARD.encode(&bytes))
        } else {
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        };
        let file_name = params.remote_path.rsplit('/').next().unwrap_or(&params.remote_path).to_string();
        Ok(Json(crate::vfs::FileContentResponse {
            path: target_path,
            name: file_name,
            content,
            size: bytes.len() as u64,
            mime_type: mime,
            is_binary: !is_text,
        }))
    } else if target_path.starts_with("manual://") {
        let clean_name = target_path.strip_prefix("manual://").unwrap().trim_start_matches('/').replace("..", "");
        let disk_path = Path::new("manuals").join(&clean_name);
        let content_str = if disk_path.is_file() {
            fs::read_to_string(&disk_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        } else if let Some(file) = ManualsAsset::get(&clean_name) {
            String::from_utf8_lossy(&file.data).to_string()
        } else {
            return Err((StatusCode::NOT_FOUND, format!("Manual '{}' not found", clean_name)));
        };

        Ok(Json(crate::vfs::FileContentResponse {
            path: target_path,
            name: clean_name,
            content: content_str.clone(),
            is_binary: false,
            size: content_str.len() as u64,
            mime_type: "text/markdown".to_string(),
        }))
    } else {
        let tp = target_path.clone();
        tokio::task::spawn_blocking(move || {
            LocalFs::read_file(&tp, max_b)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Read task join error: {}", e)))?
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read file: {}", e)))
    }
}

#[derive(Deserialize)]
pub struct WriteFileRequest {
    path: String,
    content: String,
    atomic: Option<bool>,
    is_base64: Option<bool>,
}

pub async fn handle_write_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<WriteFileRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let target_path = validate_path_access(&state, &headers, &payload.path, true)?;

    if target_path.starts_with("manual://") {
        return Err((StatusCode::FORBIDDEN, "Built-in repository user and QA testing manuals are read-only".to_string()));
    }

    let raw_bytes: Vec<u8> = if payload.is_base64.unwrap_or(false) {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&payload.content)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid base64 payload: {}", e)))?
    } else {
        payload.content.into_bytes()
    };

    if target_path.starts_with("vault://") {
        let rest = target_path.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        state.vaults.write_vault_file(vault_file, subpath, &raw_bytes)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
    } else if target_path.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        crate::vfs::smb::SmbClient::write_file(&params, &raw_bytes)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save SMB file: {}", e)))
    } else if target_path.starts_with("sftp://") || target_path.starts_with("ssh://") {
        let params = SftpClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?
            .with_config(&state.config.get_sftp_config());
        SftpClient::write_file(&params, &raw_bytes)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save SFTP file: {}", e)))
    } else {
        let tp = target_path.clone();
        let atomic = payload.atomic.unwrap_or(state.config.paranoid.atomic_writes);
        tokio::task::spawn_blocking(move || {
            LocalFs::write_file(&tp, &raw_bytes, atomic)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Write task join error: {}", e)))?
        .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save file: {}", e)))
    }
}

#[derive(Deserialize)]
pub struct MkdirRequest {
    path: String,
}

pub async fn handle_mkdir(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<MkdirRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let target_path = validate_path_access(&state, &headers, &payload.path, true)?;

    if target_path.starts_with("vault://") {
        let rest = target_path.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        state.vaults.mkdir_vault(vault_file, subpath)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
    } else if target_path.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        crate::vfs::smb::SmbClient::mkdir(&params)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create SMB folder: {}", e)))
    } else if target_path.starts_with("sftp://") || target_path.starts_with("ssh://") {
        let params = crate::vfs::sftp::SftpClient::parse_uri(&target_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?
            .with_config(&state.config.get_sftp_config());
        crate::vfs::sftp::SftpClient::mkdir(&params)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create SFTP folder: {}", e)))
    } else if target_path.starts_with("nfs://") {
        let params = crate::vfs::nfs::NfsClient::parse_uri(&target_path)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid NFS URI: {}", e)))?;
        let mount = crate::vfs::nfs::NfsClient::ensure_mounted(&params)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("NFS mount error: {}", e)))?;
        let local_target = mount.join(params.subpath.trim_start_matches('/'));
        LocalFs::create_dir(&local_target.to_string_lossy())
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create NFS folder: {}", e)))
    } else {
        LocalFs::create_dir(&target_path)
            .map(|_| Json(serde_json::json!({ "success": true, "path": target_path })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to create folder: {}", e)))
    }
}

#[derive(Deserialize)]
pub struct RenameRequest {
    from: String,
    to: String,
    windows_native_file_ops: Option<bool>,
    detect_locking_processes: Option<bool>,
}

pub async fn handle_rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RenameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let from_path = validate_path_access(&state, &headers, &payload.from, true)?;
    let to_path = validate_path_access(&state, &headers, &payload.to, true)?;
    let use_native_ops = payload.windows_native_file_ops.unwrap_or(state.config.paranoid.windows_native_file_ops);
    let detect_locks = payload.detect_locking_processes.unwrap_or(state.config.paranoid.detect_locking_processes);

    if from_path.starts_with("smb://") {
        let params_from = crate::vfs::smb::SmbClient::parse_uri(&from_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        let target_subpath = if to_path.starts_with("smb://") {
            let params_to = crate::vfs::smb::SmbClient::parse_uri(&to_path, None, None)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
            params_to.subpath
        } else {
            to_path
        };
        crate::vfs::smb::SmbClient::rename(&params_from, &target_subpath)
            .map(|_| Json(serde_json::json!({ "success": true })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to rename SMB item: {}", e)))
    } else if from_path.starts_with("sftp://") || from_path.starts_with("ssh://") {
        let params_from = crate::vfs::sftp::SftpClient::parse_uri(&from_path, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?
            .with_config(&state.config.get_sftp_config());
        let target_remote = if to_path.starts_with("sftp://") || to_path.starts_with("ssh://") {
            let params_to = crate::vfs::sftp::SftpClient::parse_uri(&to_path, None, None)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?
                .with_config(&state.config.get_sftp_config());
            params_to.remote_path
        } else {
            to_path
        };
        crate::vfs::sftp::SftpClient::rename(&params_from, &target_remote)
            .map(|_| Json(serde_json::json!({ "success": true })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to rename SFTP item: {}", e)))
    } else if from_path.starts_with("nfs://") {
        let params_from = crate::vfs::nfs::NfsClient::parse_uri(&from_path)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid NFS URI: {}", e)))?;
        let mount = crate::vfs::nfs::NfsClient::ensure_mounted(&params_from)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("NFS mount error: {}", e)))?;
        let local_from = mount.join(params_from.subpath.trim_start_matches('/'));
        let local_to = if to_path.starts_with("nfs://") {
            let params_to = crate::vfs::nfs::NfsClient::parse_uri(&to_path)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid NFS URI: {}", e)))?;
            mount.join(params_to.subpath.trim_start_matches('/'))
        } else {
            std::path::PathBuf::from(&to_path)
        };
        LocalFs::rename_entry_with_opts(&local_from.to_string_lossy(), &local_to.to_string_lossy(), use_native_ops, detect_locks)
            .map(|_| Json(serde_json::json!({ "success": true })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to rename NFS item: {}", e)))
    } else {
        LocalFs::rename_entry_with_opts(&from_path, &to_path, use_native_ops, detect_locks)
            .map(|_| Json(serde_json::json!({ "success": true })))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to rename: {}", e)))
    }
}

#[derive(Deserialize)]
pub struct BatchRenameItem {
    from: String,
    to: String,
}

#[derive(Deserialize)]
pub struct BatchRenameRequest {
    renames: Vec<BatchRenameItem>,
}

pub async fn handle_batch_rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchRenameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut renamed = 0;
    let mut errors = Vec::new();
    let use_native_ops = state.config.paranoid.windows_native_file_ops;
    let detect_locks = state.config.paranoid.detect_locking_processes;

    for item in payload.renames {
        if item.from != item.to {
            let from_res = validate_path_access(&state, &headers, &item.from, true);
            let to_res = validate_path_access(&state, &headers, &item.to, true);
            match (from_res, to_res) {
                (Ok(from_path), Ok(to_path)) => {
                    match LocalFs::rename_entry_with_opts(&from_path, &to_path, use_native_ops, detect_locks) {
                        Ok(_) => renamed += 1,
                        Err(e) => errors.push(format!("{}: {}", item.from, e)),
                    }
                }
                (Err((_, e)), _) | (_, Err((_, e))) => {
                    errors.push(format!("{}: {}", item.from, e));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(Json(serde_json::json!({ "success": true, "renamed": renamed })))
    } else {
        Err((StatusCode::MULTI_STATUS, format!("Batch rename errors: {}", errors.join("; "))))
    }
}

#[derive(Deserialize)]
pub struct DeleteRequest {
    paths: Vec<String>,
    use_trash: Option<bool>,
    custom_trash_dir: Option<String>,
    windows_native_file_ops: Option<bool>,
    detect_locking_processes: Option<bool>,
}

pub async fn handle_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DeleteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let use_trash = payload.use_trash.unwrap_or(state.config.paranoid.trash_enabled);
    let custom_trash = payload.custom_trash_dir.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
    let use_native_ops = payload.windows_native_file_ops.unwrap_or(state.config.paranoid.windows_native_file_ops);
    let detect_locks = payload.detect_locking_processes.unwrap_or(state.config.paranoid.detect_locking_processes);
    let mut deleted = Vec::new();
    let mut errors = Vec::new();

    for path in &payload.paths {
        let valid_path = match validate_path_access(&state, &headers, path, true) {
            Ok(p) => p,
            Err((_, e)) => {
                errors.push(format!("{}: {}", path, e));
                continue;
            }
        };

        if valid_path.starts_with("vault://") {
            let rest = valid_path.strip_prefix("vault://").unwrap();
            let (vault_file, subpath) = match rest.split_once('#') {
                Some((v, s)) => (v, s),
                None => (rest, ""),
            };
            match state.vaults.delete_vault_file(vault_file, subpath) {
                Ok(_) => deleted.push(path.clone()),
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        } else if valid_path.starts_with("smb://") {
            match crate::vfs::smb::SmbClient::parse_uri(&valid_path, None, None) {
                Ok(params) => {
                    match crate::vfs::smb::SmbClient::delete(&params, false) {
                        Ok(_) => deleted.push(path.clone()),
                        Err(e) => errors.push(format!("{}: {}", path, e)),
                    }
                }
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        } else if valid_path.starts_with("sftp://") || valid_path.starts_with("ssh://") {
            match crate::vfs::sftp::SftpClient::parse_uri(&valid_path, None, None) {
                Ok(params) => {
                    let params = params.with_config(&state.config.get_sftp_config());
                    match crate::vfs::sftp::SftpClient::delete(&params, false) {
                        Ok(_) => deleted.push(path.clone()),
                        Err(e) => errors.push(format!("{}: {}", path, e)),
                    }
                }
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        } else if valid_path.starts_with("nfs://") {
            match crate::vfs::nfs::NfsClient::parse_uri(&valid_path) {
                Ok(params) => {
                    match crate::vfs::nfs::NfsClient::ensure_mounted(&params) {
                        Ok(mount) => {
                            let local_target = mount.join(params.subpath.trim_start_matches('/'));
                            match LocalFs::delete_entry_with_opts(&local_target.to_string_lossy(), false, None, use_native_ops, detect_locks) {
                                Ok(_) => deleted.push(path.clone()),
                                Err(e) => errors.push(format!("{}: {}", path, e)),
                            }
                        }
                        Err(e) => errors.push(format!("{}: {}", path, e)),
                    }
                }
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        } else {
            match LocalFs::delete_entry_with_opts(&valid_path, use_trash, custom_trash, use_native_ops, detect_locks) {
                Ok(_) => deleted.push(path.clone()),
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        }
    }

    if errors.is_empty() {
        Ok(Json(serde_json::json!({ "success": true, "deleted": deleted })))
    } else {
        Err((StatusCode::MULTI_STATUS, format!("Encountered errors: {}", errors.join("; "))))
    }
}

#[derive(Deserialize)]
pub struct ChmodRequest {
    paths: Vec<String>,
    mode: u32,
    recursive: Option<bool>,
}

pub async fn handle_chmod(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChmodRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rec = payload.recursive.unwrap_or(false);
    for p in &payload.paths {
        let valid_path = validate_path_access(&state, &headers, p, true)?;
        LocalFs::chmod_entry(&valid_path, payload.mode, rec)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Chmod failed for {}: {}", p, e)))?;
    }
    Ok(Json(serde_json::json!({ "success": true, "mode": format!("{:04o}", payload.mode) })))
}

#[derive(Deserialize)]
pub struct ChownRequest {
    paths: Vec<String>,
    owner: Option<String>,
    group: Option<String>,
    recursive: Option<bool>,
}

pub async fn handle_chown(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChownRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rec = payload.recursive.unwrap_or(false);

    let uid_val: Option<u32> = if let Some(ref o) = payload.owner {
        if let Ok(u) = o.parse::<u32>() {
            Some(u)
        } else if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
            passwd.lines().find_map(|l| {
                let parts: Vec<&str> = l.split(':').collect();
                if parts.len() >= 3 && parts[0] == o {
                    parts[2].parse::<u32>().ok()
                } else {
                    None
                }
            })
        } else {
            None
        }
    } else {
        None
    };

    let gid_val: Option<u32> = if let Some(ref g) = payload.group {
        if let Ok(gid) = g.parse::<u32>() {
            Some(gid)
        } else if let Ok(grp) = fs::read_to_string("/etc/group") {
            grp.lines().find_map(|l| {
                let parts: Vec<&str> = l.split(':').collect();
                if parts.len() >= 3 && parts[0] == g {
                    parts[2].parse::<u32>().ok()
                } else {
                    None
                }
            })
        } else {
            None
        }
    } else {
        None
    };

    for p in &payload.paths {
        let valid_path = validate_path_access(&state, &headers, p, true)?;
        LocalFs::chown_entry(&valid_path, uid_val, gid_val, rec)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Chown failed for {}: {}", p, e)))?;
    }

    Ok(Json(serde_json::json!({ "success": true, "uid": uid_val, "gid": gid_val })))
}

#[derive(Deserialize)]
pub struct TransferRequest {
    sources: Vec<String>,
    destination: String,
    paranoid: Option<bool>,
    conflict_resolution: Option<String>,
}

pub async fn handle_copy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TransferRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut validated_sources = Vec::new();
    for s in &payload.sources {
        validated_sources.push(validate_path_access(&state, &headers, s, false)?);
    }
    let validated_dest = validate_path_access(&state, &headers, &payload.destination, true)?;

    let paranoid = payload.paranoid.unwrap_or(state.config.paranoid.verify_after_transfer);
    let conflict_resolution = payload.conflict_resolution.clone();
    let task_id = state.tasks.create_task(
        &format!("Copy {} items", validated_sources.len()),
        "copy",
        &validated_sources.join(", "),
        &validated_dest,
        0,
    ).await;

    let tasks_mgr = state.tasks.clone();
    let tid = task_id.clone();
    let sources = validated_sources;
    let destination = validated_dest;

    tokio::spawn(async move {
        crate::vfs::transfer::VfsTransfer::execute_batch_transfer(
            tasks_mgr,
            tid,
            sources,
            destination,
            false,
            paranoid,
            conflict_resolution,
        ).await;
    });

    Ok(Json(serde_json::json!({ "success": true, "task_id": task_id, "copied_count": payload.sources.len() })))
}

#[derive(Deserialize)]
pub struct DeltaCopyRequest {
    sources: Vec<String>,
    destination: String,
    options: Option<crate::tools::deltacopy::DeltaCopyOptions>,
}

pub async fn handle_deltacopy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DeltaCopyRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut validated_sources = Vec::new();
    for s in &payload.sources {
        validated_sources.push(validate_path_access(&state, &headers, s, false)?);
    }
    let validated_dest = validate_path_access(&state, &headers, &payload.destination, true)?;

    let opts = payload.options.unwrap_or_default();
    let tasks_mgr = state.tasks.clone();
    let cancel_token = Arc::new(std::sync::atomic::AtomicBool::new(false));

    tokio::spawn(async move {
        let _ = crate::tools::deltacopy::DeltaCopyEngine::run_deltacopy(
            tasks_mgr,
            validated_sources,
            validated_dest,
            opts,
            cancel_token,
        ).await;
    });

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "DeltaCopy transfer started in background queue",
    })))
}

pub async fn handle_move(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TransferRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut validated_sources = Vec::new();
    for s in &payload.sources {
        validated_sources.push(validate_path_access(&state, &headers, s, true)?);
    }
    let validated_dest = validate_path_access(&state, &headers, &payload.destination, true)?;

    let paranoid = payload.paranoid.unwrap_or(state.config.paranoid.verify_after_transfer);
    let conflict_resolution = payload.conflict_resolution.clone();
    let task_id = state.tasks.create_task(
        &format!("Move {} items", validated_sources.len()),
        "move",
        &validated_sources.join(", "),
        &validated_dest,
        0,
    ).await;

    let tasks_mgr = state.tasks.clone();
    let tid = task_id.clone();
    let sources = validated_sources;
    let destination = validated_dest;

    tokio::spawn(async move {
        crate::vfs::transfer::VfsTransfer::execute_batch_transfer(
            tasks_mgr,
            tid,
            sources,
            destination,
            true,
            paranoid,
            conflict_resolution,
        ).await;
    });

    Ok(Json(serde_json::json!({ "success": true, "task_id": task_id, "moved_count": payload.sources.len() })))
}

#[derive(Deserialize)]
pub struct TestRemoteRequest {
    protocol: String,
    host: String,
    port: Option<u16>,
    user: Option<String>,
    pass: Option<String>,
    bucket: Option<String>,
    region: Option<String>,
}

pub async fn handle_test_remote(
    Json(payload): Json<TestRemoteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match payload.protocol.to_lowercase().as_str() {
        "sftp" => {
            let port = payload.port.unwrap_or(22);
            let user = payload.user.unwrap_or_else(|| "root".to_string());
            let params = crate::vfs::sftp::SftpParams {
                host: payload.host,
                port,
                user,
                password: payload.pass.filter(|p| !p.trim().is_empty()),
                key_path: None,
                remote_path: "/".to_string(),
                host_key_checking: None,
                known_hosts_file: None,
            };
            match SftpClient::list_dir(&params) {
                Ok(listing) => Ok(Json(serde_json::json!({
                    "success": true,
                    "message": format!("Connected successfully to SFTP server (found {} items)", listing.entries.len()),
                }))),
                Err(e) => Err((StatusCode::BAD_REQUEST, format!("SFTP connection failed: {}", e))),
            }
        }
        "webdav" => {
            let url = payload.host;
            match WebDavClient::list_dir(&url, payload.user.as_deref(), payload.pass.as_deref()).await {
                Ok(listing) => Ok(Json(serde_json::json!({
                    "success": true,
                    "message": format!("Connected successfully to WebDAV storage (found {} items)", listing.entries.len()),
                }))),
                Err(e) => Err((StatusCode::BAD_REQUEST, format!("WebDAV connection failed: {}", e))),
            }
        }
        "s3" => {
            let s3_conf = crate::vfs::s3::S3Config {
                endpoint: payload.host,
                bucket: payload.bucket.unwrap_or_default(),
                region: payload.region.unwrap_or_else(|| "us-east-1".to_string()),
                access_key_id: payload.user.unwrap_or_default(),
                secret_access_key: payload.pass.unwrap_or_default(),
                path_style: Some(true),
            };
            let client = crate::vfs::s3::S3Client::new(s3_conf);
            match client.test_connection().await {
                Ok(_) => Ok(Json(serde_json::json!({
                    "success": true,
                    "message": "Connected successfully to S3 / Cloud Object Storage bucket!",
                }))),
                Err(e) => Err((StatusCode::BAD_REQUEST, format!("S3 connection failed: {}", e))),
            }
        }
        "proton" => {
            let status = crate::vfs::proton::ProtonDriveClient::check_status().await;
            if status.installed {
                if status.authenticated {
                    Ok(Json(serde_json::json!({
                        "success": true,
                        "message": format!("Proton Drive integration ready! Backend: {} (Version: {})", status.cli_type, status.version.unwrap_or_default()),
                    })))
                } else {
                    Ok(Json(serde_json::json!({
                        "success": false,
                        "message": format!("Proton Drive CLI detected ({}) but not yet logged in. Run '{} login' in terminal.", status.cli_type, status.cli_type),
                    })))
                }
            } else {
                Err((StatusCode::BAD_REQUEST, "Proton Drive CLI or rclone not detected on host. Install 'proton-drive' or configure an rclone proton remote.".to_string()))
            }
        }
        "smb" => {
            let port = payload.port.unwrap_or(445);
            let share = payload.bucket.unwrap_or_else(|| "share".to_string());
            let params = crate::vfs::smb::SmbParams {
                host: payload.host,
                port,
                share,
                subpath: "".to_string(),
                username: payload.user,
                password: payload.pass,
                domain: payload.region,
            };
            crate::vfs::smb::SmbClient::test_connection(&params)
                .map(|msg| Json(serde_json::json!({ "success": true, "message": msg })))
                .map_err(|e| (StatusCode::BAD_REQUEST, e))
        }
        "nfs" => {
            let port = payload.port.unwrap_or(2049);
            let export_path = payload.bucket.unwrap_or_else(|| "/".to_string());
            let params = crate::vfs::nfs::NfsParams {
                host: payload.host,
                port,
                export_path,
                subpath: "".to_string(),
                version: payload.region,
            };
            crate::vfs::nfs::NfsClient::test_connection(&params)
                .map(|msg| Json(serde_json::json!({ "success": true, "message": msg })))
                .map_err(|e| (StatusCode::BAD_REQUEST, e))
        }
        _ => Err((StatusCode::BAD_REQUEST, "Unsupported protocol".to_string())),
    }
}

pub async fn handle_proton_status() -> Json<crate::vfs::proton::ProtonStatus> {
    Json(crate::vfs::proton::ProtonDriveClient::check_status().await)
}

pub async fn handle_list_tasks(
    State(state): State<AppState>,
) -> Json<Vec<TaskInfo>> {
    Json(state.tasks.list_tasks().await)
}

pub async fn handle_get_task(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<TaskInfo>, (StatusCode, String)> {
    match state.tasks.get_task(&id).await {
        Some(task) => Ok(Json(task)),
        None => Err((StatusCode::NOT_FOUND, "Task not found".to_string())),
    }
}

pub async fn handle_cancel_task(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let cancelled = state.tasks.cancel_task(&id).await;
    Json(serde_json::json!({ "success": cancelled }))
}

pub async fn handle_pause_task(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let paused = state.tasks.pause_task(&id).await;
    Json(serde_json::json!({ "success": paused }))
}

pub async fn handle_resume_task(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let resumed = state.tasks.resume_task(&id).await;
    Json(serde_json::json!({ "success": resumed }))
}

pub async fn handle_clear_completed_tasks(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    state.tasks.clear_completed().await;
    Json(serde_json::json!({ "success": true }))
}

pub async fn handle_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let raw_dest = query.get("destination").cloned().unwrap_or_else(|| "~".to_string());
    let dest_dir = validate_path_access(&state, &headers, &raw_dest, true)?;

    let mut uploaded_files = Vec::new();
    let mut uploaded_hashes: HashMap<String, String> = HashMap::new();
    let is_silent = query.get("silent").map(|v| v == "true" || v == "1").unwrap_or(false)
        || query.get("no_task").map(|v| v == "true" || v == "1").unwrap_or(false);

    let task_id_opt = if !is_silent {
        Some(state.tasks.create_task("Upload Files", "upload", "Browser", &dest_dir, 0).await)
    } else {
        None
    };

    let conflict_mode = query.get("conflict").or_else(|| query.get("conflict_resolution")).map(|s| s.as_str()).unwrap_or("overwrite");

    while let Ok(Some(field)) = multipart.next_field().await {
        let file_name = sanitize_uploaded_relative_path(field.file_name().unwrap_or("upload.bin"));

        if let Ok(data) = field.bytes().await {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let sha256_hex = hex::encode(hasher.finalize());
            uploaded_hashes.insert(file_name.clone(), sha256_hex.clone());

            let file_size = data.len() as u64;
            if let Some(ref tid) = task_id_opt {
                state.tasks.update_task_details(
                    tid,
                    Some(&file_name),
                    file_size,
                    file_size,
                    uploaded_files.len() as u64 + 1,
                    uploaded_files.len() as u64 + 1,
                    file_size,
                    0,
                    Some(uploaded_files.len() as u64 + 1),
                    Some(&format!("SHA-256 Match: {}", sha256_hex)),
                    Some(&format!("Uploaded {} | SHA-256 Match: {}", file_name, sha256_hex)),
                ).await;
            }

            let should_extract = query.get("extract").map(|v| v == "true" || v == "1").unwrap_or(false)
                || query.get("unarchive").map(|v| v == "true" || v == "1").unwrap_or(false)
                || query.get("is_dir").map(|v| v == "true" || v == "1").unwrap_or(false);

            let is_zip_magic = data.len() >= 4 && &data[0..4] == b"PK\x03\x04";
            let is_gzip_magic = data.len() >= 2 && &data[0..2] == b"\x1f\x8b";

            if should_extract && (is_zip_magic || is_gzip_magic) && !dest_dir.starts_with("smb://") && !dest_dir.starts_with("sftp://") && !dest_dir.starts_with("ssh://") {
                let suffix = if is_zip_magic { ".zip" } else { ".tar.gz" };
                let temp_archive = tempfile::Builder::new()
                    .prefix("brum_upload_extract_")
                    .suffix(suffix)
                    .tempfile()
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Temp archive error: {}", e)))?;
                let temp_path = temp_archive.path().to_str().unwrap().to_string();
                std::fs::write(&temp_path, &data)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write temp archive: {}", e)))?;

                if let Err(e) = ArchiveHandler::extract_archive(&temp_path, &dest_dir) {
                    let err_msg = format!("Failed to extract uploaded directory archive: {}", e);
                    if let Some(ref tid) = task_id_opt {
                        state.tasks.fail_task(tid, &err_msg).await;
                    }
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, err_msg));
                }
                uploaded_files.push(file_name);
                continue;
            }

            if dest_dir.starts_with("smb://") {
                let params = match crate::vfs::smb::SmbClient::parse_uri(&dest_dir, None, None) {
                    Ok(mut p) => {
                        p.subpath = if p.subpath.is_empty() { file_name.clone() } else { format!("{}/{}", p.subpath, file_name) };
                        p
                    }
                    Err(e) => {
                        let err_msg = format!("Invalid SMB destination: {}", e);
                        if let Some(ref tid) = task_id_opt {
                            state.tasks.fail_task(tid, &err_msg).await;
                        }
                        return Err((StatusCode::BAD_REQUEST, err_msg));
                    }
                };
                if let Err(e) = crate::vfs::smb::SmbClient::write_file(&params, &data) {
                    let err_msg = format!("Failed to write SMB upload: {}", e);
                    if let Some(ref tid) = task_id_opt {
                        state.tasks.fail_task(tid, &err_msg).await;
                    }
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, err_msg));
                }
            } else if dest_dir.starts_with("sftp://") || dest_dir.starts_with("ssh://") {
                let params = match SftpClient::parse_uri(&dest_dir, None, None) {
                    Ok(mut p) => {
                        p.remote_path = if p.remote_path.is_empty() || p.remote_path == "/" {
                            format!("/{}", file_name)
                        } else {
                            format!("{}/{}", p.remote_path.trim_end_matches('/'), file_name)
                        };
                        p.with_config(&state.config.get_sftp_config())
                    }
                    Err(e) => {
                        let err_msg = format!("Invalid SFTP destination: {}", e);
                        if let Some(ref tid) = task_id_opt {
                            state.tasks.fail_task(tid, &err_msg).await;
                        }
                        return Err((StatusCode::BAD_REQUEST, err_msg));
                    }
                };
                if let Err(e) = SftpClient::write_file(&params, &data) {
                    let err_msg = format!("Failed to write SFTP upload: {}", e);
                    if let Some(ref tid) = task_id_opt {
                        state.tasks.fail_task(tid, &err_msg).await;
                    }
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, err_msg));
                }
            } else {
                let raw_target = Path::new(&dest_dir).join(&file_name);
                let target_path = match conflict_mode {
                    "skip" if raw_target.exists() => {
                        continue;
                    }
                    "rename" if raw_target.exists() => {
                        crate::vfs::transfer::generate_unique_destination_path(&raw_target)
                    }
                    _ => raw_target,
                };
                if let Err(e) = LocalFs::write_file(&target_path.to_string_lossy(), &data, true) {
                    let err_msg = format!("Failed to write upload: {}", e);
                    if let Some(ref tid) = task_id_opt {
                        state.tasks.fail_task(tid, &err_msg).await;
                    }
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, err_msg));
                }
            }
            uploaded_files.push(file_name);
        }
    }

    if let Some(ref tid) = task_id_opt {
        state.tasks.complete_task(tid).await;
    }
    Ok(Json(serde_json::json!({
        "success": true,
        "uploaded": uploaded_files,
        "hashes": uploaded_hashes,
        "task_id": task_id_opt
    })))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpRange {
    pub start: u64,
    pub end: u64, // inclusive
}

impl HttpRange {
    pub fn parse(range_header: &str, total_len: u64) -> Option<Result<HttpRange, ()>> {
        if total_len == 0 {
            return Some(Err(()));
        }

        let range_str = range_header.trim();
        if !range_str.starts_with("bytes=") {
            return None;
        }

        let spec = &range_str["bytes=".len()..].trim();
        let spec = spec.split(',').next()?.trim();

        if let Some((start_str, end_str)) = spec.split_once('-') {
            let start_str = start_str.trim();
            let end_str = end_str.trim();

            if start_str.is_empty() {
                // Suffix range: bytes=-500 (last 500 bytes)
                if let Ok(suffix_len) = end_str.parse::<u64>() {
                    if suffix_len == 0 {
                        return Some(Err(()));
                    }
                    let actual_len = suffix_len.min(total_len);
                    let start = total_len - actual_len;
                    let end = total_len - 1;
                    return Some(Ok(HttpRange { start, end }));
                }
            } else if end_str.is_empty() {
                // Prefix range: bytes=500-
                if let Ok(start) = start_str.parse::<u64>() {
                    if start >= total_len {
                        return Some(Err(())); // 416 Range Not Satisfiable
                    }
                    let end = total_len - 1;
                    return Some(Ok(HttpRange { start, end }));
                }
            } else {
                // Explicit range: bytes=500-999
                if let (Ok(start), Ok(end)) = (start_str.parse::<u64>(), end_str.parse::<u64>()) {
                    if start > end || start >= total_len {
                        return Some(Err(())); // 416
                    }
                    let end = end.min(total_len - 1);
                    return Some(Ok(HttpRange { start, end }));
                }
            }
        }

        None
    }
}

fn build_bytes_range_response(
    file_bytes: Vec<u8>,
    mime: String,
    disposition: String,
    etag: Option<String>,
    range_header: Option<&str>,
) -> Result<Response, (StatusCode, String)> {
    let total_len = file_bytes.len() as u64;

    if let Some(range_raw) = range_header {
        if let Some(range_res) = HttpRange::parse(range_raw, total_len) {
            match range_res {
                Ok(range) => {
                    let start = range.start as usize;
                    let end = (range.end as usize).min(file_bytes.len().saturating_sub(1));
                    let slice = if start <= end && start < file_bytes.len() {
                        file_bytes[start..=end].to_vec()
                    } else {
                        Vec::new()
                    };
                    let slice_len = slice.len();

                    let mut builder = Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, mime)
                        .header(header::CONTENT_DISPOSITION, disposition)
                        .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, total_len))
                        .header(header::CONTENT_LENGTH, slice_len.to_string())
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CONTENT_ENCODING, "identity");

                    if let Some(et) = etag {
                        builder = builder
                            .header(header::ETAG, et)
                            .header(header::CACHE_CONTROL, "no-cache, must-revalidate");
                    }

                    return builder
                        .body(Body::from(slice))
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)));
                }
                Err(()) => {
                    return Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(header::CONTENT_RANGE, format!("bytes */{}", total_len))
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CONTENT_ENCODING, "identity")
                        .body(Body::empty())
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)));
                }
            }
        }
    }

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, total_len.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_ENCODING, "identity");

    if let Some(et) = etag {
        builder = builder
            .header(header::ETAG, et)
            .header(header::CACHE_CONTROL, "no-cache, must-revalidate");
    }

    builder
        .body(Body::from(file_bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))
}

pub async fn build_local_file_range_response(
    path: &Path,
    total_len: u64,
    mime: String,
    disposition: String,
    etag: String,
    range_header: Option<&str>,
) -> Result<Response, (StatusCode, String)> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    use tokio_util::io::ReaderStream;

    if let Some(range_raw) = range_header {
        if let Some(range_res) = HttpRange::parse(range_raw, total_len) {
            match range_res {
                Ok(range) => {
                    let slice_len = range.end - range.start + 1;
                    let mut file = tokio::fs::File::open(path).await.map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open file: {}", e))
                    })?;

                    file.seek(std::io::SeekFrom::Start(range.start)).await.map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to seek file: {}", e))
                    })?;

                    let stream = ReaderStream::new(file.take(slice_len));
                    let body = Body::from_stream(stream);

                    let response = Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, mime)
                        .header(header::CONTENT_DISPOSITION, disposition)
                        .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", range.start, range.end, total_len))
                        .header(header::CONTENT_LENGTH, slice_len.to_string())
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::ETAG, etag)
                        .header(header::CACHE_CONTROL, "no-cache, must-revalidate")
                        .header(header::CONTENT_ENCODING, "identity")
                        .body(body)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

                    return Ok(response);
                }
                Err(()) => {
                    let response = Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(header::CONTENT_RANGE, format!("bytes */{}", total_len))
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CONTENT_ENCODING, "identity")
                        .body(Body::empty())
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

                    return Ok(response);
                }
            }
        }
    }

    let file = tokio::fs::File::open(path).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open file: {}", e))
    })?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, total_len.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "no-cache, must-revalidate")
        .header(header::CONTENT_ENCODING, "identity")
        .body(body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

    Ok(response)
}

pub async fn handle_download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, (StatusCode, String)> {
    let raw_path = query.get("path").ok_or((StatusCode::BAD_REQUEST, "Missing path param".to_string()))?;

    let mut modified_headers = headers.clone();
    if !modified_headers.contains_key(header::AUTHORIZATION) {
        if let Some(tok) = query.get("token").or_else(|| query.get("auth")) {
            if let Ok(hv) = header::HeaderValue::from_str(&format!("Bearer {}", tok)) {
                modified_headers.insert(header::AUTHORIZATION, hv);
            }
        }
    }

    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

    let path_str = validate_path_access(&state, &modified_headers, raw_path, false)?;

    if path_str.starts_with("vault://") {
        let rest = path_str.strip_prefix("vault://").unwrap();
        let (vault_file, subpath) = match rest.split_once('#') {
            Some((v, s)) => (v, s),
            None => (rest, ""),
        };
        let file_res = state.vaults.read_vault_file(vault_file, subpath)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

        use base64::Engine;
        let file_bytes = if file_res.is_binary && file_res.content.starts_with("data:application/octet-stream;base64,") {
            let b64 = file_res.content.strip_prefix("data:application/octet-stream;base64,").unwrap();
            base64::engine::general_purpose::STANDARD.decode(b64).unwrap_or_default()
        } else {
            file_res.content.into_bytes()
        };

        let file_name = file_res.name;
        let mime = file_res.mime_type;
        let is_media_type = mime.starts_with("image/")
            || mime.starts_with("video/")
            || mime.starts_with("audio/")
            || mime == "application/pdf"
            || mime.starts_with("text/");
        let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

        let disposition = if is_inline {
            format!("inline; filename=\"{}\"", file_name)
        } else {
            format!("attachment; filename=\"{}\"", file_name)
        };

        return build_bytes_range_response(file_bytes, mime, disposition, None, range_header);
    } else if path_str.starts_with("archive://") {
        let rest = path_str.strip_prefix("archive://").unwrap();
        let (archive_file, subpath) = match rest.split_once('#') {
            Some((a, s)) => (a, s),
            None => (rest, ""),
        };
        let file_res = ArchiveHandler::read_archive_entry(archive_file, subpath, 0)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read archive entry: {}", e)))?;

        use base64::Engine;
        let file_bytes = if file_res.is_binary {
            base64::engine::general_purpose::STANDARD.decode(&file_res.content).unwrap_or_default()
        } else {
            file_res.content.into_bytes()
        };

        let file_name = file_res.name;
        let mime = file_res.mime_type;
        let is_media_type = mime.starts_with("image/")
            || mime.starts_with("video/")
            || mime.starts_with("audio/")
            || mime == "application/pdf"
            || mime.starts_with("text/");
        let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

        let disposition = if is_inline {
            format!("inline; filename=\"{}\"", file_name)
        } else {
            format!("attachment; filename=\"{}\"", file_name)
        };

        return build_bytes_range_response(file_bytes, mime, disposition, None, range_header);
    } else if path_str.starts_with("smb://") {
        let params = crate::vfs::smb::SmbClient::parse_uri(&path_str, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SMB URI: {}", e)))?;
        let file_bytes = crate::vfs::smb::SmbClient::read_bytes(&params)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to download SMB file: {}", e)))?;
        let file_name = params.subpath.rsplit('/').next().unwrap_or(&params.subpath).to_string();
        let mime = mime_guess::from_path(&file_name).first_or_octet_stream().to_string();
        let is_media_type = mime.starts_with("image/")
            || mime.starts_with("video/")
            || mime.starts_with("audio/")
            || mime == "application/pdf"
            || mime.starts_with("text/");
        let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

        let disposition = if is_inline {
            format!("inline; filename=\"{}\"", file_name)
        } else {
            format!("attachment; filename=\"{}\"", file_name)
        };

        return build_bytes_range_response(file_bytes, mime, disposition, None, range_header);
    } else if path_str.starts_with("sftp://") || path_str.starts_with("ssh://") {
        let params = SftpClient::parse_uri(&path_str, None, None)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid SFTP URI: {}", e)))?
            .with_config(&state.config.get_sftp_config());
        let file_bytes = SftpClient::download_file_with_params(&params, 0)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to download SFTP file: {}", e)))?;
        let file_name = params.remote_path.rsplit('/').next().unwrap_or(&params.remote_path).to_string();
        let mime = mime_guess::from_path(&file_name).first_or_octet_stream().to_string();
        let is_media_type = mime.starts_with("image/")
            || mime.starts_with("video/")
            || mime.starts_with("audio/")
            || mime == "application/pdf"
            || mime.starts_with("text/");
        let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

        let disposition = if is_inline {
            format!("inline; filename=\"{}\"", file_name)
        } else {
            format!("attachment; filename=\"{}\"", file_name)
        };

        return build_bytes_range_response(file_bytes, mime, disposition, None, range_header);
    }

    let path = Path::new(&path_str);

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "File or folder not found".to_string()));
    }

    if path.is_dir() {
        let fmt = query.get("format").map(|s| s.to_lowercase()).unwrap_or_else(|| "zip".to_string());
        let dir_name = path.file_name().unwrap_or_default().to_string_lossy();
        let safe_name = if dir_name.is_empty() { "folder".to_string() } else { dir_name.to_string() };

        if fmt == "tar.gz" || fmt == "targz" || fmt == "tgz" {
            let temp_tar = tempfile::Builder::new()
                .prefix("brum_folder_")
                .suffix(".tar.gz")
                .tempfile()
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Temp file error: {}", e)))?;
            let temp_path = temp_tar.path().to_str().unwrap().to_string();

            ArchiveHandler::create_targz(&[path_str.clone()], &temp_path)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Tar.gz creation failed: {}", e)))?;

            let file_bytes = tokio::fs::read(&temp_path).await.map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read generated tar.gz: {}", e))
            })?;

            let tar_name = format!("{}.tar.gz", safe_name);

            let response = Response::builder()
                .header(header::CONTENT_TYPE, "application/gzip")
                .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", tar_name))
                .header("X-Is-Directory", "true")
                .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
                .header(header::CONTENT_LENGTH, file_bytes.len().to_string())
                .body(Body::from(file_bytes))
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

            return Ok(response);
        } else {
            let temp_zip = tempfile::Builder::new()
                .prefix("brum_folder_")
                .suffix(".zip")
                .tempfile()
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Temp file error: {}", e)))?;
            let temp_path = temp_zip.path().to_str().unwrap().to_string();

            ArchiveHandler::create_zip(&[path_str.clone()], &temp_path)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Zip creation failed: {}", e)))?;

            let file_bytes = tokio::fs::read(&temp_path).await.map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read generated zip: {}", e))
            })?;

            let zip_name = format!("{}.zip", safe_name);

            let response = Response::builder()
                .header(header::CONTENT_TYPE, "application/zip")
                .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", zip_name))
                .header("X-Is-Directory", "true")
                .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
                .header(header::CONTENT_LENGTH, file_bytes.len().to_string())
                .body(Body::from(file_bytes))
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

            return Ok(response);
        }
    }

    let metadata = tokio::fs::metadata(path).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file metadata: {}", e))
    })?;

    let mtime_sec = metadata.modified().ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let etag = format!("\"{:x}-{:x}\"", metadata.len(), mtime_sec);

    if let Some(inm) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if inm == etag || inm == "*" {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, etag)
                .header(header::CACHE_CONTROL, "no-cache, must-revalidate")
                .body(Body::empty())
                .unwrap());
        }
    }

    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    let is_media_type = mime.starts_with("image/")
        || mime.starts_with("video/")
        || mime.starts_with("audio/")
        || mime == "application/pdf"
        || mime.starts_with("text/");
    let is_inline = query.get("inline").map(|v| v == "true" || v == "1").unwrap_or(is_media_type);

    let disposition = if is_inline {
        format!("inline; filename=\"{}\"", file_name)
    } else {
        format!("attachment; filename=\"{}\"", file_name)
    };

    build_local_file_range_response(path, metadata.len(), mime, disposition, etag, range_header).await
}

#[derive(Deserialize)]
pub struct BatchDownloadRequest {
    paths: Vec<String>,
}

pub async fn handle_download_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchDownloadRequest>,
) -> Result<Response, (StatusCode, String)> {
    if payload.paths.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No paths specified for download".to_string()));
    }

    let mut validated_paths = Vec::new();
    for p in &payload.paths {
        validated_paths.push(validate_path_access(&state, &headers, p, false)?);
    }

    let temp_zip = tempfile::Builder::new()
        .prefix("brum_batch_")
        .suffix(".zip")
        .tempfile()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Temp file error: {}", e)))?;
    let temp_path = temp_zip.path().to_str().unwrap().to_string();

    let v_paths = validated_paths.clone();
    let t_path = temp_path.clone();
    tokio::task::spawn_blocking(move || {
        ArchiveHandler::create_zip(&v_paths, &t_path)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Batch task join error: {}", e)))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Batch zip creation failed: {}", e)))?;

    let file_bytes = std::fs::read(&temp_path).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read generated zip: {}", e))
    })?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let zip_name = if validated_paths.len() == 1 {
        let p = Path::new(&validated_paths[0]);
        let base = p.file_name().unwrap_or_default().to_string_lossy();
        format!("{}.zip", base)
    } else {
        format!("brum_download_{}.zip", timestamp)
    };

    let response = Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", zip_name))
        .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
        .body(Body::from(file_bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))?;

    Ok(response)
}

#[derive(Deserialize)]
pub struct ArchiveCreateRequest {
    sources: Vec<String>,
    target_path: String,
    format: String,
}

pub async fn handle_archive_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ArchiveCreateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut validated_sources = Vec::new();
    for s in &payload.sources {
        validated_sources.push(validate_path_access(&state, &headers, s, false)?);
    }
    let validated_target = validate_path_access(&state, &headers, &payload.target_path, true)?;

    let sources = validated_sources.clone();
    let target = validated_target.clone();
    let fmt = payload.format.to_lowercase();
    tokio::task::spawn_blocking(move || match fmt.as_str() {
        "zip" => ArchiveHandler::create_zip(&sources, &target).map_err(|e| e.to_string()),
        "targz" | "tar.gz" => ArchiveHandler::create_targz(&sources, &target).map_err(|e| e.to_string()),
        _ => Err("Unsupported archive format".to_string()),
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Archive task join error: {}", e)))?
    .map(|_| Json(serde_json::json!({ "success": true, "archive": validated_target })))
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Archive creation failed: {}", e)))
}

#[derive(Deserialize)]
pub struct ArchiveExtractRequest {
    archive_path: String,
    target_dir: String,
}

pub async fn handle_archive_extract(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ArchiveExtractRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_archive = validate_path_access(&state, &headers, &payload.archive_path, false)?;
    let valid_target = validate_path_access(&state, &headers, &payload.target_dir, true)?;

    let arch = valid_archive.clone();
    let tgt = valid_target.clone();
    tokio::task::spawn_blocking(move || {
        ArchiveHandler::extract_archive(&arch, &tgt)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Extract task join error: {}", e)))?
    .map(|_| Json(serde_json::json!({ "success": true })))
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Extract failed: {}", e)))
}

#[derive(Deserialize)]
pub struct ChecksumRequest {
    path: String,
    algorithm: Option<String>,
}

pub async fn handle_calculate_checksum(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChecksumRequest>,
) -> Result<Json<crate::vfs::checksum::ChecksumResult>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, false)?;
    let algo = payload.algorithm.unwrap_or_else(|| "sha256".to_string());
    let vp = valid_path.clone();
    let alg = algo.clone();
    tokio::task::spawn_blocking(move || {
        calculate_checksum(&vp, &alg)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Checksum task join error: {}", e)))?
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, format!("Checksum failed: {}", e)))
}

