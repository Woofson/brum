#[derive(Serialize)]
pub struct ListPluginsResponse {
    pub plugins: Vec<crate::plugins::PluginInfo>,
    pub can_install: bool,
    pub allow_user_installs: bool,
}
use std::path::Path;
use std::collections::HashMap;
use axum::{
    body::Body,
    extract::{Multipart, Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Json, Response},
};
use serde::{Deserialize, Serialize};
use crate::server::AppState;
use crate::server::middleware::{
    extract_claims_or_local, require_write, validate_custom_trash_dir,
    validate_path_access, sanitize_uploaded_file_name,
};
use crate::tools::diff::compare_files_text;
use crate::tools::paranoid::ParanoidEngine;

#[derive(Deserialize)]
pub struct DiffFilesRequest {
    file_left: String,
    file_right: String,
}

pub async fn handle_diff_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DiffFilesRequest>,
) -> Result<Json<crate::tools::diff::FileDiffResult>, (StatusCode, String)> {
    let left = validate_path_access(&state, &headers, &payload.file_left, false)?;
    let right = validate_path_access(&state, &headers, &payload.file_right, false)?;

    let text_l = std::fs::read_to_string(&left)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Cannot read left file {}: {}", left, e)))?;
    let text_r = std::fs::read_to_string(&right)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Cannot read right file {}: {}", right, e)))?;

    let diff = compare_files_text(&left, &text_l, &right, &text_r);
    Ok(Json(diff))
}

#[derive(Deserialize)]
pub struct DiffFoldersRequest {
    dir_left: String,
    dir_right: String,
    recursive: Option<bool>,
    deep_hash: Option<bool>,
    selected_items: Option<Vec<String>>,
}

pub async fn handle_diff_folders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DiffFoldersRequest>,
) -> Result<Json<crate::tools::diff::FolderDiffResult>, (StatusCode, String)> {
    let left = validate_path_access(&state, &headers, &payload.dir_left, false)?;
    let right = validate_path_access(&state, &headers, &payload.dir_right, false)?;

    let recursive = payload.recursive.unwrap_or(false);
    let deep = payload.deep_hash.unwrap_or(false);
    // Runs off the request threads, and stops when the browser stops waiting (Stop, closing the window)
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _stop_when_dropped = CancelOnDrop(cancel.clone());
    let selected = payload.selected_items;
    tokio::task::spawn_blocking(move || crate::tools::diff::compare_folders_cancellable(&left, &right, recursive, deep, selected, &cancel))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Folder diff failed: {}", e)))
}

/// Sets the flag when the request is dropped, so background work stops with it
struct CancelOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Deserialize)]
pub struct DiffEstimateRequest {
    pub dir_left: String,
    pub dir_right: String,
    #[serde(default)]
    pub recursive: bool,
}

/// A quick count of both folders, so the browser can ask before a very large comparison
pub async fn handle_diff_estimate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DiffEstimateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let left = validate_path_access(&state, &headers, &payload.dir_left, false)?;
    let right = validate_path_access(&state, &headers, &payload.dir_right, false)?;
    let recursive = payload.recursive;
    let (l, r) = tokio::task::spawn_blocking(move || {
        let limit = 100_000;
        let time = std::time::Duration::from_secs(3);
        (crate::tools::diff::estimate_folder(&left, recursive, limit, time), crate::tools::diff::estimate_folder(&right, recursive, limit, time))
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "left": l, "right": r })))
}

#[derive(Deserialize)]
pub struct DiffChecksumRequest {
    pub dir_left: String,
    pub dir_right: String,
    pub relative_paths: Vec<String>,
}

pub async fn handle_diff_checksum(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DiffChecksumRequest>,
) -> Result<Json<crate::tools::diff::BatchChecksumResult>, (StatusCode, String)> {
    let left = validate_path_access(&state, &headers, &payload.dir_left, false)?;
    let right = validate_path_access(&state, &headers, &payload.dir_right, false)?;

    let paths = payload.relative_paths;
    tokio::task::spawn_blocking(move || crate::tools::diff::calculate_checksums_for_entries(&left, &right, &paths))
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Validates a list of paths, stopping at the first one the caller may not access
fn validate_paths(
    state: &AppState,
    headers: &HeaderMap,
    paths: &[String],
    is_write: bool,
) -> Result<Vec<String>, (StatusCode, String)> {
    paths.iter().map(|p| validate_path_access(state, headers, p, is_write)).collect()
}

/// Checks write access to the folder a tool writes into when no output path is given
fn validate_sibling_output(
    state: &AppState,
    headers: &HeaderMap,
    source_path: &str,
) -> Result<(), (StatusCode, String)> {
    let parent = Path::new(source_path).parent().unwrap_or(Path::new("."));
    validate_path_access(state, headers, &parent.to_string_lossy(), true).map(|_| ())
}

pub async fn handle_convert_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<crate::tools::converter::ConvertRequest>,
) -> Result<Json<crate::tools::converter::ConvertResponse>, (StatusCode, String)> {
    payload.source_path = validate_path_access(&state, &headers, &payload.source_path, false)?;
    payload.output_path = match payload.output_path.take().filter(|o| !o.trim().is_empty()) {
        Some(output) => Some(validate_path_access(&state, &headers, &output, true)?),
        None => {
            validate_sibling_output(&state, &headers, &payload.source_path)?;
            None
        }
    };
    crate::tools::converter::ConvertEngine::convert_file(&payload)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Conversion failed: {}", e)))
}

#[derive(Deserialize)]
pub struct DryRunRequest {
    action: String,
    sources: Vec<String>,
    destination: Option<String>,
}

pub async fn handle_paranoid_dry_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DryRunRequest>,
) -> Result<Json<crate::tools::paranoid::DryRunPreview>, (StatusCode, String)> {
    let sources = validate_paths(&state, &headers, &payload.sources, false)?;
    let destination = payload.destination.as_deref()
        .map(|d| validate_path_access(&state, &headers, d, false))
        .transpose()?;
    ParanoidEngine::dry_run(&payload.action, &sources, destination.as_deref())
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Dry run failed: {}", e)))
}



// ---------------- NOTES CHEWTOY HANDLERS ----------------

#[derive(Deserialize)]
pub struct NotesInfoQuery {
    folder: Option<String>,
}

pub async fn handle_notes_info(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<NotesInfoQuery>,
) -> Result<Json<crate::tools::notes::NotesInfo>, (StatusCode, String)> {
    let cfg_folder = state.config.notes.notes_folder.as_str();
    let folder = query.folder.as_deref()
        .filter(|f| !f.trim().is_empty())
        .map(|f| validate_path_access(&state, &headers, f, false))
        .transpose()?;
    let info = crate::tools::notes::scan_notes_hierarchy(folder.as_deref(), Some(cfg_folder));
    Ok(Json(info))
}

pub async fn handle_notes_templates() -> Json<Vec<crate::tools::notes::NoteTemplate>> {
    Json(crate::tools::notes::get_builtin_templates())
}

#[derive(Deserialize)]
pub struct NotesVersionsQuery {
    path: String,
}

pub async fn handle_notes_versions(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<NotesVersionsQuery>,
) -> Result<Json<Vec<crate::tools::notes::NoteVersionItem>>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &query.path, false)?;
    Ok(Json(crate::tools::notes::list_note_versions(Path::new(&valid_path))))
}

#[derive(Deserialize)]
pub struct NotesSaveVersionRequest {
    path: String,
}

pub async fn handle_notes_save_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NotesSaveVersionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, true)?;
    let p = Path::new(&valid_path);
    if p.exists() {
        if let Ok(bytes) = std::fs::read(p) {
            let _ = crate::tools::notes::create_note_snapshot(p, &bytes);
        }
    }
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct NotesDecryptRequest {
    path: String,
    passphrase: Option<String>,
}

#[derive(Serialize)]
pub struct NotesDecryptResponse {
    content: String,
    path: String,
}

pub async fn handle_notes_decrypt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NotesDecryptRequest>,
) -> Result<Json<NotesDecryptResponse>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, false)?;
    let p = Path::new(&valid_path);
    if !p.exists() {
        return Err((StatusCode::NOT_FOUND, "Note file not found".to_string()));
    }
    let pass = payload.passphrase.as_deref().unwrap_or(crate::tools::notes::DEFAULT_NOTE_PASSPHRASE);
    match crate::tools::notes::decrypt_note_file(p, pass) {
        Ok(content) => Ok(Json(NotesDecryptResponse {
            content,
            path: valid_path.clone(),
        })),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

#[derive(Deserialize)]
pub struct NotesEncryptRequest {
    path: String,
    content: String,
    passphrase: Option<String>,
    convert_to_plain: Option<bool>,
}

#[derive(Serialize)]
pub struct NotesEncryptResponse {
    success: bool,
    path: String,
    is_encrypted: bool,
}

pub async fn handle_notes_encrypt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NotesEncryptRequest>,
) -> Result<Json<NotesEncryptResponse>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, true)?;
    let p = Path::new(&valid_path);
    let pass = payload.passphrase.as_deref().unwrap_or(crate::tools::notes::DEFAULT_NOTE_PASSPHRASE);

    if payload.convert_to_plain.unwrap_or(false) {
        match crate::tools::notes::decrypt_and_save_plain_note(p, pass) {
            Ok(new_path) => Ok(Json(NotesEncryptResponse {
                success: true,
                path: new_path.to_string_lossy().to_string(),
                is_encrypted: false,
            })),
            Err(err) => Err((StatusCode::BAD_REQUEST, err)),
        }
    } else {
        match crate::tools::notes::encrypt_and_save_note(p, &payload.content, pass) {
            Ok(new_path) => Ok(Json(NotesEncryptResponse {
                success: true,
                path: new_path.to_string_lossy().to_string(),
                is_encrypted: true,
            })),
            Err(err) => Err((StatusCode::BAD_REQUEST, err)),
        }
    }
}

#[derive(Deserialize)]
pub struct NotesCreateNoteRequest {
    section_path: String,
    title: String,
    is_encrypted: Option<bool>,
    passphrase: Option<String>,
    initial_content: Option<String>,
}

pub async fn handle_notes_create_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NotesCreateNoteRequest>,
) -> Result<Json<crate::tools::notes::NotesFile>, (StatusCode, String)> {
    let valid_section = validate_path_access(&state, &headers, &payload.section_path, true)?;
    let sec_p = Path::new(&valid_section);
    match crate::tools::notes::create_new_note(
        sec_p,
        &payload.title,
        payload.is_encrypted.unwrap_or(false),
        payload.passphrase.as_deref(),
        payload.initial_content.as_deref(),
    ) {
        Ok(note) => Ok(Json(note)),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

#[derive(Deserialize)]
pub struct NotesSectionActionRequest {
    path: String,
    passphrase: String,
    cached_pass: Option<String>,
}

pub async fn handle_notes_section_encrypt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NotesSectionActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, true)?;
    let p = Path::new(&valid_path);
    match crate::tools::notes::encrypt_section_dir(p, &payload.passphrase, payload.cached_pass.as_deref()) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "encrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

pub async fn handle_notes_section_decrypt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NotesSectionActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, true)?;
    let p = Path::new(&valid_path);
    match crate::tools::notes::decrypt_section_dir(p, &payload.passphrase) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "decrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

#[derive(Deserialize)]
pub struct NotesNotebookActionRequest {
    path: String,
    passphrase: String,
    cached_pass: Option<String>,
}

pub async fn handle_notes_notebook_encrypt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NotesNotebookActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, true)?;
    let p = Path::new(&valid_path);
    match crate::tools::notes::encrypt_notebook_dir(p, &payload.passphrase, payload.cached_pass.as_deref()) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "encrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

pub async fn handle_notes_notebook_decrypt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<NotesNotebookActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &payload.path, true)?;
    let p = Path::new(&valid_path);
    match crate::tools::notes::decrypt_notebook_dir(p, &payload.passphrase) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "decrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}



// ---------------- PERSISTENT DATABASE NOTES & ATTACHMENTS HANDLERS ----------------

#[derive(Deserialize)]
pub struct ListDbNotesQuery {
    search: Option<String>,
    tag: Option<String>,
    category: Option<String>,
    section: Option<String>,
    archived: Option<bool>,
}

pub async fn handle_list_db_notes(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<ListDbNotesQuery>,
) -> Result<Json<Vec<crate::auth::DbNote>>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let is_admin = claims.role == "admin";
    let notes = state.auth.list_db_notes(
        &claims.sub,
        is_admin,
        query.search.as_deref(),
        query.tag.as_deref(),
        query.category.as_deref(),
        query.section.as_deref(),
        query.archived.unwrap_or(false),
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list notes: {}", e)))?;
    Ok(Json(notes))
}

#[derive(Deserialize)]
pub struct CreateDbNoteRequest {
    title: String,
    content: Option<String>,
    category: Option<String>,
    section: Option<String>,
    tags: Option<String>,
    is_pinned: Option<bool>,
    is_encrypted: Option<bool>,
    color: Option<String>,
}

pub async fn handle_create_db_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateDbNoteRequest>,
) -> Result<Json<crate::auth::DbNote>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let title = payload.title.trim();
    if title.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Title cannot be empty".to_string()));
    }
    let content = payload.content.as_deref().unwrap_or("");
    let category = payload.category.as_deref().unwrap_or("General");
    let section = payload.section.as_deref().unwrap_or("Default");
    let tags = payload.tags.as_deref().unwrap_or("[]");
    let is_pinned = payload.is_pinned.unwrap_or(false);
    let is_encrypted = payload.is_encrypted.unwrap_or(false);
    let color = payload.color.as_deref();

    let note = state.auth.create_db_note(
        &claims.sub,
        title,
        content,
        category,
        section,
        tags,
        is_pinned,
        is_encrypted,
        color,
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create note: {}", e)))?;

    Ok(Json(note))
}

pub async fn handle_get_db_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<crate::auth::DbNote>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let is_admin = claims.role == "admin";
    let note = state.auth.get_db_note(id, &claims.sub, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Note not found".to_string()))?;
    Ok(Json(note))
}

#[derive(Deserialize)]
pub struct UpdateDbNoteRequest {
    title: Option<String>,
    content: Option<String>,
    category: Option<String>,
    section: Option<String>,
    tags: Option<String>,
    is_pinned: Option<bool>,
    is_archived: Option<bool>,
    color: Option<Option<String>>,
}

pub async fn handle_update_db_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateDbNoteRequest>,
) -> Result<Json<crate::auth::DbNote>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let is_admin = claims.role == "admin";
    let color_opt = payload.color.as_ref().map(|opt| opt.as_deref());

    let note = state.auth.update_db_note(
        id,
        &claims.sub,
        is_admin,
        payload.title.as_deref(),
        payload.content.as_deref(),
        payload.category.as_deref(),
        payload.section.as_deref(),
        payload.tags.as_deref(),
        payload.is_pinned,
        payload.is_archived,
        color_opt,
    ).map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to update note: {}", e)))?;

    Ok(Json(note))
}

pub async fn handle_delete_db_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let is_admin = claims.role == "admin";
    let attachments_to_delete = state.auth.delete_db_note(id, &claims.sub, is_admin)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to delete note: {}", e)))?;

    for path_str in attachments_to_delete {
        let _ = std::fs::remove_file(Path::new(&path_str));
    }

    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn handle_list_db_note_attachments(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<Vec<crate::auth::DbNoteAttachment>>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let is_admin = claims.role == "admin";
    let note = state.auth.get_db_note(id, &claims.sub, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Note not found".to_string()))?;
    Ok(Json(note.attachments))
}

pub async fn handle_upload_db_note_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    multipart: Multipart,
) -> Result<Json<crate::auth::DbNoteAttachment>, (StatusCode, String)> {
    save_uploaded_note_attachment(&state, &headers, Some(id), multipart).await
}

#[derive(Deserialize)]
pub struct UploadAttachmentQuery {
    note_id: Option<i64>,
}

pub async fn handle_upload_db_note_attachment_standalone(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<UploadAttachmentQuery>,
    multipart: Multipart,
) -> Result<Json<crate::auth::DbNoteAttachment>, (StatusCode, String)> {
    save_uploaded_note_attachment(&state, &headers, query.note_id, multipart).await
}

async fn save_uploaded_note_attachment(
    state: &AppState,
    headers: &HeaderMap,
    note_id: Option<i64>,
    mut multipart: Multipart,
) -> Result<Json<crate::auth::DbNoteAttachment>, (StatusCode, String)> {
    use sha2::{Digest, Sha256};
    let claims = extract_claims_or_local(state, headers)?;
    let attachments_dir = crate::tools::notes::get_notes_attachments_dir();
    let _ = std::fs::create_dir_all(&attachments_dir);

    if let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))? {
        let original_name = sanitize_uploaded_file_name(field.file_name().unwrap_or("attachment"));
        let content_type = field.content_type().unwrap_or("application/octet-stream").to_string();
        let data = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let size_bytes = data.len() as i64;
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let sha256 = format!("{:x}", hasher.finalize());

        let clean_filename: String = original_name.chars().map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).collect();
        let unique_name = format!("{}_{}", uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string(), clean_filename);
        let storage_path = attachments_dir.join(&unique_name);

        std::fs::write(&storage_path, &data)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write attachment file: {}", e)))?;

        let att = state.auth.create_db_note_attachment(
            note_id,
            &claims.sub,
            &original_name,
            &content_type,
            size_bytes,
            &storage_path.to_string_lossy(),
            Some(&sha256),
        ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save attachment to DB: {}", e)))?;

        return Ok(Json(att));
    }

    Err((StatusCode::BAD_REQUEST, "No file provided in multipart payload".to_string()))
}

async fn serve_attachment_binary(
    state: &AppState,
    headers: &HeaderMap,
    attachment_id: i64,
) -> Result<Response, (StatusCode, String)> {
    let claims = extract_claims_or_local(state, headers)?;
    let is_admin = claims.role == "admin";
    let att = state.auth.get_db_note_attachment(attachment_id, &claims.sub, is_admin)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {}", e)))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Attachment not found".to_string()))?;

    let p = Path::new(&att.storage_path);
    if !p.exists() || !p.is_file() {
        return Err((StatusCode::NOT_FOUND, "Attachment file missing on server".to_string()));
    }

    let bytes = std::fs::read(p)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file: {}", e)))?;

    let content_type = if !att.mime_type.is_empty() && att.mime_type != "application/octet-stream" {
        att.mime_type
    } else {
        mime_guess::from_path(&att.filename)
            .first_or_octet_stream()
            .to_string()
    };

    let filename_header = format!("inline; filename=\"{}\"", att.filename);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, filename_header)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(response)
}

pub async fn handle_get_db_note_attachment_binary(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(attachment_id): AxumPath<i64>,
) -> Result<Response, (StatusCode, String)> {
    serve_attachment_binary(&state, &headers, attachment_id).await
}

pub async fn handle_get_db_note_attachment_binary_with_name(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((attachment_id, _filename)): AxumPath<(i64, String)>,
) -> Result<Response, (StatusCode, String)> {
    serve_attachment_binary(&state, &headers, attachment_id).await
}

pub async fn handle_delete_db_note_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(attachment_id): AxumPath<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let is_admin = claims.role == "admin";
    let storage_path = state.auth.delete_db_note_attachment(attachment_id, &claims.sub, is_admin)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to delete attachment: {}", e)))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Attachment not found or access denied".to_string()))?;

    let _ = std::fs::remove_file(Path::new(&storage_path));
    Ok(Json(serde_json::json!({ "success": true })))
}

/// A single folder name from user text: separators and other unsafe characters become `_`,
/// and names made only of dots are replaced by the fallback
fn safe_folder_name(text: &str, fallback: &str) -> String {
    let cleaned: String = text
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '&' | '(' | ')' | ',') { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        fallback.to_string()
    } else {
        cleaned
    }
}

pub async fn handle_notes_migrate_export(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let is_admin = claims.role == "admin";
    let notes = state.auth.list_db_notes(&claims.sub, is_admin, None, None, None, None, false)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let root = crate::tools::notes::get_notes_root_dir(None, Some(&state.config.notes.notes_folder));
    let mut exported_count = 0;

    for note in notes {
        // Category and section become one folder name each, always inside the notes folder
        let cat = safe_folder_name(&note.category, "General");
        let sec = safe_folder_name(&note.section, "Default");
        let target_dir = root.join(&cat).join(&sec);
        let _ = std::fs::create_dir_all(&target_dir);

        let sanitized_title: String = note.title.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
            .collect();
        let base_name = if sanitized_title.trim().is_empty() { "Untitled_Note" } else { sanitized_title.trim() };
        let file_path = target_dir.join(format!("{}.md", base_name));

        if std::fs::write(&file_path, note.content.as_bytes()).is_ok() {
            let _ = crate::tools::notes::create_note_snapshot(&file_path, note.content.as_bytes());
            exported_count += 1;
        }
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "exported_count": exported_count,
        "target_directory": root.to_string_lossy()
    })))
}

pub async fn handle_notes_migrate_import(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let info = crate::tools::notes::scan_notes_hierarchy(None, Some(&state.config.notes.notes_folder));
    let mut imported_count = 0;

    for nb in info.notebooks {
        for sec in nb.sections {
            for note in sec.notes {
                if !note.is_encrypted {
                    if let Ok(content) = std::fs::read_to_string(&note.path) {
                        let title = note.name;
                        if state.auth.create_db_note(
                            &claims.sub,
                            &title,
                            &content,
                            &nb.name,
                            &sec.name,
                            "[]",
                            false,
                            false,
                            None,
                        ).is_ok() {
                            imported_count += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "imported_count": imported_count
    })))
}



// ---------------- TETRION CLASSIC ARCADE CHEWTOY & LEADERBOARD HANDLERS ----------------

#[derive(Deserialize)]
pub struct TetrionScoresQuery {
    limit: Option<usize>,
    mode: Option<String>,
    user_only: Option<bool>,
}

pub async fn handle_tetrion_get_scores(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TetrionScoresQuery>,
) -> Result<Json<crate::tools::tetrion::TetraLeaderboardResponse>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;
    let limit = query.limit.unwrap_or(50);
    let mode = query.mode.as_deref();
    let user_only = query.user_only.unwrap_or(false);

    let resp = crate::tools::tetrion::get_leaderboard(&conn, &claims.sub, limit, mode, user_only)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

pub async fn handle_tetrion_submit_score(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::tetrion::SubmitScoreRequest>,
) -> Result<Json<crate::tools::tetrion::SubmitScoreResponse>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;

    let resp = crate::tools::tetrion::submit_score(&conn, &claims.sub, payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

pub async fn handle_tetrion_clear_scores(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;

    let user_filter = if claims.role == "admin" { None } else { Some(claims.sub.as_str()) };
    let cleared = crate::tools::tetrion::clear_scores(&conn, user_filter)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "success": true, "cleared_count": cleared })))
}



// ---------------- PHASE 4 HANDLERS (SYNC, SEARCH, SCRIPT ACTIONS) ----------------

// ---------------- Backup & Sync ----------------

use crate::tools::sync::{BackupManager, BackupProfile, ProfileView, RestoreRequest};

type ApiError = (StatusCode, String);

/// Admins see every backup; other users only their own
fn can_use_profile(claims: &crate::auth::Claims, profile: &BackupProfile) -> bool {
    claims.role.eq_ignore_ascii_case("admin") || profile.owner.as_deref() == Some(claims.sub.as_str())
}

/// The backup, if this user may see and control it. Other users' backups look like missing ones.
fn owned_profile(state: &AppState, headers: &HeaderMap, id: &str) -> Result<BackupProfile, ApiError> {
    let claims = extract_claims_or_local(state, headers)?;
    state.backup.get_profile(id)
        .filter(|p| can_use_profile(&claims, p))
        .ok_or((StatusCode::NOT_FOUND, "Backup not found".to_string()))
}

/// Checks both folders against the user's access and stores them as full paths
fn resolve_profile_paths(state: &AppState, headers: &HeaderMap, p: &mut BackupProfile) -> Result<(), ApiError> {
    let two_way = p.mode() == crate::tools::sync::engine::Mode::TwoWay;
    let mut checked = Vec::new();
    for src in p.all_sources() {
        checked.push(validate_path_access(state, headers, src.trim(), two_way)?);
    }
    p.sources = checked;
    p.source_dir = p.sources.first().cloned().unwrap_or_default();
    p.dest_dir = validate_path_access(state, headers, p.dest_dir.trim(), true)?;
    if p.source_dir.to_lowercase().starts_with("fleet://") {
        return Err(bad_request("A Fleet machine can only be where backups are kept, not what is backed up".into()));
    }
    if p.sources.iter().any(|s| s.to_lowercase().starts_with("proton://")) {
        return Err(bad_request("Proton Drive can be where backups are kept, not what is backed up (yet)".into()));
    }
    if p.dest_dir.to_lowercase().starts_with("proton://") {
        use crate::tools::sync::engine::Mode;
        if matches!(p.mode(), Mode::TwoWay | Mode::Git) {
            return Err(bad_request("Backups to Proton Drive can copy, or copy and keep old versions. Keeping two folders the same and git need a folder on a drive.".into()));
        }
        crate::vfs::proton::check_writable(&crate::vfs::proton::cli_path_of(&p.dest_dir))
            .map_err(|_| bad_request("Choose a folder inside My files, for example proton:///my-files/Backups".into()))?;
    }
    p.dest_endpoint = None;
    if p.dest_dir.to_lowercase().starts_with("fleet://") {
        resolve_fleet_destination(state, headers, p)?;
    }
    if let Some(remote) = p.git_remote.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(String::from) {
        p.git_remote = Some(check_git_remote(state, headers, &remote)?);
    } else {
        p.git_remote = None;
    }
    Ok(())
}

/// Only ordinary git remotes: SSH (`user@host:path`, `ssh://`), `https://`, `http://`, `git://`,
/// or a folder on this machine the user may write to. Never git's command-running transports.
fn check_git_remote(state: &AppState, headers: &HeaderMap, remote: &str) -> Result<String, ApiError> {
    let lower = remote.to_lowercase();
    if remote.starts_with('-') || remote.contains(char::is_whitespace) || lower.contains("::") {
        return Err(bad_request("This remote address is not allowed".into()));
    }
    if ["ssh://", "https://", "http://", "git://"].iter().any(|p| lower.starts_with(p)) {
        return Ok(remote.to_string());
    }
    // scp-style SSH: user@host:path
    if let Some((host_part, _path)) = remote.split_once(':') {
        if !host_part.contains('/') && host_part.contains('@') {
            return Ok(remote.to_string());
        }
    }
    if remote.starts_with('/') || remote.starts_with('~') {
        return validate_path_access(state, headers, remote, true);
    }
    Err(bad_request("Use a remote like git@github.com:you/repo.git, ssh://…, https://… or a folder on this machine".into()))
}

/// Finds the Fleet machine of a fleet:// destination: first in Brum's settings file, then in the
/// user's own saved Fleet machines. Its address must pass the same network checks as the Fleet proxy.
fn resolve_fleet_destination(state: &AppState, headers: &HeaderMap, p: &mut BackupProfile) -> Result<(), ApiError> {
    let (node_id, _) = crate::tools::sync::fleet_store::parse(&p.dest_dir).map_err(bad_request)?;
    let claims = extract_claims_or_local(state, headers)?;
    // The user's own saved Fleet machines (address and token), as the browser keeps them
    let saved: Option<(String, Option<String>)> = (|| {
        let prefs = state.auth.get_user_preferences(&claims.sub).ok().flatten()?;
        let v: serde_json::Value = serde_json::from_str(&prefs).ok()?;
        let node = v.get("fleet_nodes")?.as_array()?.iter().find(|n| n.get("id").and_then(|i| i.as_str()) == Some(node_id.as_str()))?;
        Some((node.get("endpoint_url")?.as_str()?.to_string(), node.get("auth_token").and_then(|t| t.as_str()).map(String::from).filter(|t| !t.trim().is_empty())))
    })();
    let found = match state.config.fleet.nodes.iter().find(|n| n.id == node_id) {
        // A machine from the settings file without a token can still have one saved by the user
        Some(n) => Some((n.url.clone(), n.token.clone().filter(|t| !t.trim().is_empty()).or_else(|| saved.as_ref().and_then(|s| s.1.clone())))),
        None => saved,
    };
    let (url, token) = found.ok_or_else(|| bad_request(format!("Fleet machine '{node_id}' is not known. Add it in Fleet first.")))?;
    let url = url.trim().trim_end_matches('/').to_string();
    match crate::server::middleware::extract_remote_scheme_host(&url) {
        Some((_, host)) if !crate::server::middleware::is_prohibited_remote_host(&host, state.config.storage.allow_remote_loopback) => {}
        _ => return Err((StatusCode::FORBIDDEN, format!("The address of Fleet machine '{node_id}' is not allowed"))),
    }
    p.dest_endpoint = Some(url);
    if p.dest_password.as_deref().unwrap_or("").is_empty() {
        p.dest_password = token.filter(|t| !t.trim().is_empty());
    }
    Ok(())
}

fn bad_request(e: String) -> ApiError {
    (StatusCode::BAD_REQUEST, e)
}

pub async fn handle_list_backup_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProfileView>>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    Ok(Json(state.backup.list_profiles().into_iter()
        .filter(|p| can_use_profile(&claims, p))
        .map(|p| state.backup.view(p))
        .collect()))
}

pub async fn handle_save_backup_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut profile): Json<BackupProfile>,
) -> Result<Json<ProfileView>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_write(&claims)?;
    resolve_profile_paths(&state, &headers, &mut profile)?;
    // Editing keeps the original owner; a new backup belongs to whoever made it
    profile.owner = match (!profile.id.is_empty()).then(|| state.backup.get_profile(&profile.id)).flatten() {
        Some(existing) if can_use_profile(&claims, &existing) => existing.owner,
        Some(_) => return Err((StatusCode::NOT_FOUND, "Backup not found".to_string())),
        None => {
            profile.id = String::new();
            Some(claims.sub.clone())
        }
    };
    let saved = state.backup.save_profile(profile).map_err(bad_request)?;
    Ok(Json(state.backup.view(saved)))
}

#[derive(Deserialize)]
pub struct OpenCollectionRequest {
    /// A collection repository already on this machine
    #[serde(default)]
    pub folder: Option<String>,
    /// Or the remote to download it from
    #[serde(default)]
    pub url: Option<String>,
    /// Where to download it; default `~/Collections/<repository name>`
    #[serde(default)]
    pub into: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// For a collection with encrypted files
    #[serde(default)]
    pub passphrase: Option<String>,
}

/// Opens an existing git collection (from another machine) as a collection in Brum. Its files
/// are placed on this machine with the home folder remapped; nothing is restored yet.
pub async fn handle_open_collection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OpenCollectionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    require_write(&claims)?;
    if !crate::tools::sync::git_repo::is_available() {
        return Err(bad_request("Collections need git, which is not installed on this machine".into()));
    }
    let clean = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    // A folder downloaded here, removed again if the collection cannot be opened
    let mut downloaded: Option<std::path::PathBuf> = None;
    let repo = match (clean(&req.folder), clean(&req.url)) {
        (Some(folder), _) => validate_path_access(&state, &headers, &folder, true)?,
        (None, Some(url)) => {
            let url = check_git_remote(&state, &headers, &url)?;
            let repo_name = url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("").trim_end_matches(".git").to_string();
            let repo_name = if repo_name.is_empty() || repo_name.starts_with('.') { "collection".to_string() } else { repo_name };
            let into = clean(&req.into).unwrap_or_else(|| format!("~/Collections/{repo_name}"));
            let into = validate_path_access(&state, &headers, &into, true)?;
            let into_path = std::path::PathBuf::from(&into);
            if into_path.exists() && std::fs::read_dir(&into_path).map(|mut d| d.next().is_some()).unwrap_or(true) {
                return Err(bad_request(format!("{into} already exists and is not empty. Choose another folder, or open it as a folder on this machine.")));
            }
            let (u, dir) = (url.clone(), into_path.clone());
            tokio::task::spawn_blocking(move || {
                if let Some(parent) = dir.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                crate::tools::sync::git_repo::clone(&u, &dir)
            })
            .await
            .map_err(|e| bad_request(e.to_string()))?
            .map_err(|e| bad_request(format!("Could not download the collection: {e}")))?;
            downloaded = Some(into_path);
            into
        }
        (None, None) => return Err(bad_request("Choose a collection folder or a remote address".into())),
    };
    let undo = |e: String| {
        if let Some(dir) = &downloaded {
            let _ = std::fs::remove_dir_all(dir);
        }
        bad_request(e)
    };
    let (mut profile, items, mut left_out) = state.backup.collection_from_repo(&repo, req.name.as_deref(), req.passphrase.as_deref()).map_err(undo)?;
    // Only the places this Brum user may write to
    let mut kept = Vec::new();
    let mut kept_items = Vec::new();
    for item in items {
        match validate_path_access(&state, &headers, &item.path, true) {
            Ok(path) => {
                kept.push(path);
                kept_items.push(item);
            }
            Err(_) => left_out.push(format!("{}: your account may not write there", item.path)),
        }
    }
    if kept.is_empty() {
        return Err(undo("Your account may not write to any of the places in this collection".into()));
    }
    profile.source_dir = kept[0].clone();
    profile.sources = kept;
    profile.git_remote = profile.git_remote.and_then(|r| check_git_remote(&state, &headers, &r).ok());
    profile.owner = Some(claims.sub.clone());
    let saved = state.backup.save_profile(profile).map_err(undo)?;
    Ok(Json(serde_json::json!({ "profile": state.backup.view(saved), "items": kept_items, "left_out": left_out })))
}

pub async fn handle_delete_backup_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_write(&extract_claims_or_local(&state, &headers)?)?;
    owned_profile(&state, &headers, &id)?;
    state.backup.delete_profile(&id).map_err(bad_request)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn handle_run_backup_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_write(&extract_claims_or_local(&state, &headers)?)?;
    owned_profile(&state, &headers, &id)?;
    let run_id = state.backup.start_backup(&id, "manual", state.tasks.clone()).map_err(|e| (StatusCode::CONFLICT, e))?;
    Ok(Json(serde_json::json!({ "success": true, "run_id": run_id })))
}

/// Pushes a git collection to its remote
pub async fn handle_push_backup_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_write(&extract_claims_or_local(&state, &headers)?)?;
    owned_profile(&state, &headers, &id)?;
    let run_id = state.backup.start_push(&id).map_err(|e| (StatusCode::CONFLICT, e))?;
    Ok(Json(serde_json::json!({ "success": true, "run_id": run_id })))
}

pub async fn handle_stop_backup_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    owned_profile(&state, &headers, &id)?;
    Ok(Json(serde_json::json!({ "success": state.backup.stop(&id) })))
}

pub async fn handle_toggle_backup_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_write(&extract_claims_or_local(&state, &headers)?)?;
    owned_profile(&state, &headers, &id)?;
    let enabled = state.backup.toggle_profile(&id).map_err(bad_request)?;
    Ok(Json(serde_json::json!({ "success": true, "enabled": enabled })))
}

/// What a backup (saved or not yet saved) would do now
pub async fn handle_backup_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut profile): Json<BackupProfile>,
) -> Result<Json<crate::tools::sync::PreviewResult>, ApiError> {
    resolve_profile_paths(&state, &headers, &mut profile)?;
    if !profile.id.is_empty() {
        owned_profile(&state, &headers, &profile.id)?;
    }
    let mgr = state.backup.clone();
    tokio::task::spawn_blocking(move || mgr.preview(&profile))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .map_err(bad_request)
}

pub async fn handle_list_backup_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<crate::tools::sync::RunRecord>>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let limit = query.get("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(50).clamp(1, 500);
    if let Some(id) = query.get("profile_id").filter(|s| !s.is_empty()) {
        owned_profile(&state, &headers, id)?;
        return Ok(Json(state.backup.list_runs(Some(id), limit)));
    }
    let own: std::collections::HashSet<String> = state.backup.list_profiles().into_iter()
        .filter(|p| can_use_profile(&claims, p))
        .map(|p| p.id)
        .collect();
    let mut runs: Vec<_> = state.backup.list_runs(None, 500).into_iter().filter(|r| own.contains(&r.profile_id)).collect();
    runs.truncate(limit);
    Ok(Json(runs))
}

fn owned_run(state: &AppState, headers: &HeaderMap, run_id: &str) -> Result<crate::tools::sync::RunRecord, ApiError> {
    let run = state.backup.get_run(run_id).ok_or((StatusCode::NOT_FOUND, "Run not found".to_string()))?;
    owned_profile(state, headers, &run.profile_id).map_err(|_| (StatusCode::NOT_FOUND, "Run not found".to_string()))?;
    Ok(run)
}

pub async fn handle_backup_run_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run = owned_run(&state, &headers, &run_id)?;
    let offset = query.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let limit = query.get("limit").and_then(|v| v.parse().ok()).unwrap_or(200);
    let problems = query.get("problems").is_some_and(|v| v == "1" || v == "true");
    let (files, total) = state.backup.run_files(&run_id, offset, limit, problems);
    Ok(Json(serde_json::json!({ "run": run, "files": files, "total": total })))
}

pub async fn handle_backup_run_log(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let run = owned_run(&state, &headers, &run_id)?;
    let text = state.backup.run_log_text(&run_id).unwrap_or_default();
    let day = chrono::DateTime::from_timestamp(run.started_at, 0).map(|d| d.format("%Y-%m-%d_%H%M").to_string()).unwrap_or_default();
    let safe_name: String = run.profile_name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    Response::builder()
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}-{}-{}.log\"", safe_name, run.kind, day))
        .body(Body::from(text))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_backup_restore_points(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let p = owned_profile(&state, &headers, &id)?;
    let mgr = state.backup.clone();
    let p2 = p.clone();
    let points = tokio::task::spawn_blocking(move || mgr.restore_points(&p2))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(bad_request)?;
    Ok(Json(serde_json::json!({ "points": points, "default_folder": BackupManager::default_restore_folder(&p), "original_folder": p.source_dir })))
}

pub async fn handle_backup_browse(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<crate::tools::sync::store::DirItem>>, ApiError> {
    let p = owned_profile(&state, &headers, &id)?;
    let point = query.get("point").cloned().unwrap_or_else(|| "latest".into());
    let path = query.get("path").cloned().unwrap_or_default();
    let mgr = state.backup.clone();
    tokio::task::spawn_blocking(move || mgr.browse(&p, &point, &path))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .map_err(bad_request)
}

/// The rclone remotes Brum's user has set up (names and kinds, no settings), for choosing one in the backup form
pub async fn handle_backup_rclone_remotes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claims = extract_claims_or_local(&state, &headers)?;
    if !claims.role.eq_ignore_ascii_case("admin") {
        return Ok(Json(serde_json::json!({ "available": false, "remotes": [], "kinds": {} })));
    }
    let res = tokio::task::spawn_blocking(crate::tools::sync::rclone_store::list_remotes)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(match res {
        Ok(remotes) => {
            let kinds: serde_json::Map<String, serde_json::Value> = remotes.iter().map(|(n, k)| (n.clone(), serde_json::Value::String(k.clone()))).collect();
            serde_json::json!({ "available": true, "remotes": remotes.into_iter().map(|(n, _)| n).collect::<Vec<_>>(), "kinds": kinds })
        }
        Err(_) => serde_json::json!({ "available": false, "remotes": [], "kinds": {} }),
    }))
}

/// Restores into a separate folder by default; into the original folder only with confirmation
pub async fn handle_backup_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<RestoreRequest>,
) -> Result<Json<crate::tools::sync::RestorePreview>, ApiError> {
    require_write(&extract_claims_or_local(&state, &headers)?)?;
    let p = owned_profile(&state, &headers, &id)?;
    let wanted = match req.target.as_str() {
        "original" => p.source_dir.clone(),
        "folder" => req.folder.clone().filter(|f| !f.trim().is_empty()).ok_or_else(|| bad_request("Choose a folder to restore into".into()))?,
        _ => BackupManager::default_restore_folder(&p),
    };
    let target = validate_path_access(&state, &headers, wanted.trim(), true)?;
    if req.target == "original" {
        // Several sources are restored into each of their own folders
        for src in p.all_sources() {
            validate_path_access(&state, &headers, src.trim(), true)?;
        }
    }
    let mgr = state.backup.clone();
    let tasks = state.tasks.clone();
    // Listing a large backup can take a while; keep it off the request threads
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let _guard = handle.enter();
        mgr.restore(&p, &req, &target, tasks)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map(Json)
    .map_err(bad_request)
}

pub async fn handle_disk_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<crate::tools::disk_usage::DiskUsageReport>, (StatusCode, String)> {
    let raw_path = query.get("path").ok_or((StatusCode::BAD_REQUEST, "Missing path param".to_string()))?;
    let path = validate_path_access(&state, &headers, raw_path, false)?;

    tokio::task::spawn_blocking(move || {
        crate::tools::disk_usage::DiskUsageEngine::analyze(&path)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Task join error: {}", e)))?
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, format!("Disk usage analysis failed: {}", e)))
}

pub async fn handle_get_disks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::tools::disk_usage::DiskMountInfo>>, (StatusCode, String)> {
    let _claims = extract_claims_or_local(&state, &headers)?;
    let roots = state.config.storage.roots.clone();
    let allow_entire_system = state.config.storage.allow_entire_system;
    let disks = tokio::task::spawn_blocking(move || {
        crate::tools::disk_usage::get_system_disks(&roots, allow_entire_system)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Disk enumeration failed: {}", e)))?;

    Ok(Json(disks))
}

pub async fn handle_list_usb(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::tools::usb::UsbDevice>>, (StatusCode, String)> {
    let _claims = extract_claims_or_local(&state, &headers)?;
    let devices = tokio::task::spawn_blocking(crate::tools::usb::list_usb_devices)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("USB scan failed: {}", e)))?;
    Ok(Json(devices))
}

pub async fn handle_mount_usb(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::usb::MountUsbRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    if claims.role.eq_ignore_ascii_case("readonly") {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot mount devices".to_string()));
    }

    let dev_path = payload.device_path.clone();
    let mnt_opt = payload.mount_point.clone();
    let mount_point = tokio::task::spawn_blocking(move || {
        crate::tools::usb::mount_usb_partition(&dev_path, mnt_opt.as_deref())
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Mount task failed: {}", e)))?
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "device_path": payload.device_path,
        "mount_point": mount_point,
        "message": format!("Mounted {} successfully at {}", payload.device_path, mount_point)
    })))
}

pub async fn handle_unmount_usb(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::usb::UnmountUsbRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    if claims.role.eq_ignore_ascii_case("readonly") {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot unmount devices".to_string()));
    }

    let target = payload.device_path.clone().or_else(|| payload.mount_point.clone())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "device_path or mount_point required".to_string()))?;

    let t = target.clone();
    tokio::task::spawn_blocking(move || {
        crate::tools::usb::unmount_usb_partition(&t)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Unmount task failed: {}", e)))?
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "target": target,
        "message": format!("Unmounted {} successfully", target)
    })))
}

pub async fn handle_eject_usb(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::usb::EjectUsbRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    if claims.role.eq_ignore_ascii_case("readonly") {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot eject devices".to_string()));
    }

    let dev_path = payload.device_path.clone();
    tokio::task::spawn_blocking(move || {
        crate::tools::usb::eject_usb_device(&dev_path)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Eject task failed: {}", e)))?
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "device_path": payload.device_path,
        "message": format!("Safe to remove device {}", payload.device_path)
    })))
}

pub async fn handle_search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<crate::tools::search::SearchRequest>,
) -> Result<Json<Vec<crate::tools::search::SearchResultItem>>, (StatusCode, String)> {
    payload.path = validate_path_access(&state, &headers, &payload.path, false)?;
    crate::tools::search::SearchEngine::search(payload)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Search failed: {}", e)))
}

pub async fn handle_duplicates_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<crate::tools::duplicates::DuplicateScanRequest>,
) -> Result<Json<crate::tools::duplicates::DuplicateScanResponse>, (StatusCode, String)> {
    payload.path = payload.path
        .map(|p| validate_path_access(&state, &headers, &p, false))
        .transpose()?;
    payload.paths = payload.paths
        .map(|paths| validate_paths(&state, &headers, &paths, false))
        .transpose()?;
    // With no folder given the scanner walks the whole disk
    if payload.path.is_none() && payload.paths.as_ref().map_or(true, |p| p.is_empty()) {
        validate_path_access(&state, &headers, "/", false)?;
    }
    Ok(Json(crate::tools::duplicates::scan_duplicates(payload)))
}

pub async fn handle_duplicates_clean(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<crate::tools::duplicates::DuplicateCleanRequest>,
) -> Result<Json<crate::tools::duplicates::DuplicateCleanResponse>, (StatusCode, String)> {
    payload.files = payload.files
        .map(|files| validate_paths(&state, &headers, &files, true))
        .transpose()?;
    payload.custom_trash_dir = validate_custom_trash_dir(&state, &headers, payload.custom_trash_dir.as_deref(), true)?;
    Ok(Json(crate::tools::duplicates::clean_duplicates(payload)))
}

#[derive(Deserialize)]
pub struct ReadMetadataQuery {
    path: String,
}

pub async fn handle_metadata_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReadMetadataQuery>,
) -> Result<Json<crate::tools::metadata::FileMetadataResponse>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &query.path, false)?;
    crate::tools::metadata::read_file_metadata(&valid_path)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

pub async fn handle_metadata_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<crate::tools::metadata::UpdateMetadataRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    payload.path = validate_path_access(&state, &headers, &payload.path, true)?;
    crate::tools::metadata::update_file_metadata(payload)
        .map(|_| Json(serde_json::json!({ "success": true })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

pub async fn handle_metadata_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<crate::tools::metadata::BatchMetadataRequest>,
) -> Result<Json<crate::tools::metadata::BatchMetadataResponse>, (StatusCode, String)> {
    payload.files = payload.files
        .map(|files| validate_paths(&state, &headers, &files, true))
        .transpose()?;
    if let Some(updates) = payload.updates.as_mut() {
        for update in updates.iter_mut() {
            update.path = validate_path_access(&state, &headers, &update.path, true)?;
        }
    }
    Ok(Json(crate::tools::metadata::batch_update_metadata(payload)))
}

pub async fn handle_logviewer_tail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(mut payload): Query<crate::tools::logviewer::LogTailRequest>,
) -> Result<Json<crate::tools::logviewer::LogTailResponse>, (StatusCode, String)> {
    payload.path = validate_path_access(&state, &headers, &payload.path, false)?;
    crate::tools::logviewer::tail_log_file(payload)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
pub struct TrashQuery {
    custom_trash_dir: Option<String>,
    windows_native_ops: Option<bool>,
    windows_native_file_ops: Option<bool>,
}

pub async fn handle_trash_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TrashQuery>,
) -> Result<Json<crate::tools::trash::TrashSummary>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let requested_trash = validate_custom_trash_dir(&state, &headers, query.custom_trash_dir.as_deref(), false)?;
    let custom_trash = requested_trash.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
    let use_native = query.windows_native_ops
        .or(query.windows_native_file_ops)
        .unwrap_or(state.config.paranoid.windows_native_file_ops);
    match crate::tools::trash::TrashManager::get_trash_summary(custom_trash, Some(&claims.home_dir), use_native) {
        Ok(summary) => Ok(Json(summary)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get trash summary: {}", e))),
    }
}

pub async fn handle_trash_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TrashQuery>,
) -> Result<Json<Vec<crate::tools::trash::TrashItem>>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let requested_trash = validate_custom_trash_dir(&state, &headers, query.custom_trash_dir.as_deref(), false)?;
    let custom_trash = requested_trash.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
    let use_native = query.windows_native_ops
        .or(query.windows_native_file_ops)
        .unwrap_or(state.config.paranoid.windows_native_file_ops);
    match crate::tools::trash::TrashManager::list_trash_items(custom_trash, Some(&claims.home_dir), use_native) {
        Ok(items) => Ok(Json(items)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list trash items: {}", e))),
    }
}

pub async fn handle_trash_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::trash::TrashRestoreRequest>,
) -> Result<Json<crate::tools::trash::TrashActionResult>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    if claims.role.eq_ignore_ascii_case("readonly") {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot restore files".to_string()));
    }
    let requested_trash = validate_custom_trash_dir(&state, &headers, payload.custom_trash_dir.as_deref(), true)?;
    let custom_trash = requested_trash.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
    let use_native_ops = payload.windows_native_ops.unwrap_or(state.config.paranoid.windows_native_file_ops);
    let res = crate::tools::trash::TrashManager::restore_items(payload.items, custom_trash, Some(&claims.home_dir), use_native_ops);
    Ok(Json(res))
}

pub async fn handle_trash_empty(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::trash::TrashEmptyRequest>,
) -> Result<Json<crate::tools::trash::TrashActionResult>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    if claims.role.eq_ignore_ascii_case("readonly") {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot empty trash".to_string()));
    }
    let requested_trash = validate_custom_trash_dir(&state, &headers, payload.custom_trash_dir.as_deref(), true)?;
    let custom_trash = requested_trash.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
    let use_native_ops = payload.windows_native_ops.unwrap_or(state.config.paranoid.windows_native_file_ops);
    let res = crate::tools::trash::TrashManager::empty_trash(custom_trash, Some(&claims.home_dir), use_native_ops);
    Ok(Json(res))
}

pub async fn handle_trash_open_native(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _claims = extract_claims_or_local(&state, &headers)?;
    #[cfg(windows)]
    {
        let status = std::process::Command::new("explorer.exe")
            .arg("shell:RecycleBinFolder")
            .spawn();
        match status {
            Ok(_) => Ok(Json(serde_json::json!({ "success": true, "message": "Opened Windows Recycle Bin" }))),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open Windows Recycle Bin: {}", e))),
        }
    }
    #[cfg(not(windows))]
    {
        let status = open::that("trash:///");
        match status {
            Ok(_) => Ok(Json(serde_json::json!({ "success": true, "message": "Opened Trash" }))),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open Trash: {}", e))),
        }
    }
}

pub async fn handle_trash_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::trash::TrashDeleteRequest>,
) -> Result<Json<crate::tools::trash::TrashActionResult>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    if claims.role.eq_ignore_ascii_case("readonly") {
        return Err((StatusCode::FORBIDDEN, "Read-only users cannot delete items".to_string()));
    }
    let requested_trash = validate_custom_trash_dir(&state, &headers, payload.custom_trash_dir.as_deref(), true)?;
    let custom_trash = requested_trash.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
    let use_native_ops = payload.windows_native_ops.unwrap_or(state.config.paranoid.windows_native_file_ops);
    let res = crate::tools::trash::TrashManager::delete_items(payload.items, custom_trash, Some(&claims.home_dir), use_native_ops);
    Ok(Json(res))
}

pub async fn handle_syncthing_status(
    State(state): State<AppState>,
) -> Json<crate::tools::syncthing::SyncthingStatusResponse> {
    let client = crate::tools::syncthing::SyncthingClient::new(state.config.syncthing.clone());
    Json(client.get_status().await)
}

#[derive(Deserialize)]
pub struct SyncthingScanRequest {
    folder_id: Option<String>,
    subpath: Option<String>,
}

pub async fn handle_syncthing_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SyncthingScanRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_write(&extract_claims_or_local(&state, &headers)?)?;
    let client = crate::tools::syncthing::SyncthingClient::new(state.config.syncthing.clone());
    match client.trigger_scan(payload.folder_id.as_deref(), payload.subpath.as_deref()).await {
        Ok(msg) => Ok(Json(serde_json::json!({ "success": true, "message": msg }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e)),
    }
}



// ---------------- GIT & VERSION CONTROL HANDLERS ----------------

#[derive(Deserialize)]
pub struct GitPathQuery {
    path: String,
}

#[derive(Deserialize)]
pub struct GitDiffQuery {
    path: String,
    file: Option<String>,
    #[serde(default)]
    staged: Option<bool>,
}

#[derive(Deserialize)]
pub struct GitLogQuery {
    path: String,
    #[serde(default)]
    count: Option<usize>,
}

#[derive(Deserialize)]
pub struct GitFilesRequest {
    path: String,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Deserialize)]
pub struct GitCommitRequest {
    path: String,
    message: String,
}

#[derive(Deserialize)]
pub struct GitPushRequest {
    path: String,
    remote: Option<String>,
    branch: Option<String>,
}

#[derive(Deserialize)]
pub struct GitPullRequest {
    path: String,
}

#[derive(PartialEq)]
enum GitAccess {
    Read,
    Write,
    /// Push and pull reach other servers with someone's keys
    Network,
}

/// The system account Git should run as for this user, and whether that account is the user's own.
/// A root server switches to the user's account; otherwise Git runs as the server, which is the
/// user's own account only when both are the same.
fn git_identity(claims: &crate::auth::Claims) -> (Option<crate::tools::git::RunAs>, bool) {
    #[cfg(unix)]
    {
        let process_uid = unsafe { libc::getuid() };
        match crate::server::terminal::lookup_posix_user(&claims.sub) {
            Some(pu) if process_uid == 0 && pu.uid != 0 => (
                Some(crate::tools::git::RunAs { name: pu.name, uid: pu.uid, gid: pu.gid, home: pu.dir }),
                true,
            ),
            Some(pu) => (None, pu.uid == process_uid),
            None => (None, false),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = claims;
        (None, false)
    }
}

/// Gatekeeper for the Git tool: the folder and its repository must be accessible,
/// and the repository's own settings must not ask Git to launch anything
async fn authorize_git(
    state: &AppState,
    headers: &HeaderMap,
    raw_path: &str,
    access: GitAccess,
) -> Result<crate::tools::git::GitTarget, (StatusCode, String)> {
    let is_write = access != GitAccess::Read;
    let claims = extract_claims_or_local(state, headers)?;
    let is_admin = claims.role.eq_ignore_ascii_case("admin");
    let (run_as, own_account) = git_identity(&claims);
    if access == GitAccess::Network && !is_admin && !own_account {
        return Err((
            StatusCode::FORBIDDEN,
            "Push and pull need a system account of your own on this server. Ask an admin, or use the terminal.".to_string(),
        ));
    }

    let valid_path = validate_path_access(state, headers, raw_path, is_write)?;
    let target = crate::tools::git::GitTarget { dir: std::path::PathBuf::from(valid_path), run_as };
    if let Some(root) = crate::tools::git::repo_root(&target).await {
        validate_path_access(state, headers, &root, is_write)?;
    }
    crate::tools::git::ensure_safe_repo_config(&target, is_admin).await
        .map_err(|e| (StatusCode::FORBIDDEN, e))?;
    Ok(target)
}

pub async fn handle_git_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitPathQuery>,
) -> Result<Json<crate::tools::git::GitStatusResponse>, (StatusCode, String)> {
    let repo = authorize_git(&state, &headers, &query.path, GitAccess::Read).await?;
    let res = crate::tools::git::get_git_status(&repo).await;
    Ok(Json(res))
}

pub async fn handle_git_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitDiffQuery>,
) -> Result<Json<crate::tools::git::GitDiffResponse>, (StatusCode, String)> {
    let repo = authorize_git(&state, &headers, &query.path, GitAccess::Read).await?;
    let res = crate::tools::git::get_git_diff(&repo, query.file.as_deref(), query.staged.unwrap_or(false)).await;
    Ok(Json(res))
}

pub async fn handle_git_stage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitFilesRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let repo = authorize_git(&state, &headers, &payload.path, GitAccess::Write).await?;
    let res = crate::tools::git::git_stage(&repo, &payload.files).await;
    Ok(Json(res))
}

pub async fn handle_git_unstage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitFilesRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let repo = authorize_git(&state, &headers, &payload.path, GitAccess::Write).await?;
    let res = crate::tools::git::git_unstage(&repo, &payload.files).await;
    Ok(Json(res))
}

pub async fn handle_git_commit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitCommitRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let repo = authorize_git(&state, &headers, &payload.path, GitAccess::Write).await?;
    let res = crate::tools::git::git_commit(&repo, &payload.message).await;
    Ok(Json(res))
}

pub async fn handle_git_push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitPushRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let repo = authorize_git(&state, &headers, &payload.path, GitAccess::Network).await?;
    let res = crate::tools::git::git_push(&repo, payload.remote.as_deref(), payload.branch.as_deref()).await;
    Ok(Json(res))
}

pub async fn handle_git_pull(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitPullRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let repo = authorize_git(&state, &headers, &payload.path, GitAccess::Network).await?;
    let res = crate::tools::git::git_pull(&repo).await;
    Ok(Json(res))
}

pub async fn handle_git_log(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitLogQuery>,
) -> Result<Json<Vec<crate::tools::git::GitCommitInfo>>, (StatusCode, String)> {
    let repo = authorize_git(&state, &headers, &query.path, GitAccess::Read).await?;
    let res = crate::tools::git::get_git_log(&repo, query.count.unwrap_or(30)).await;
    Ok(Json(res))
}



// ---------------- FILE SPLITTER & COMBINER HANDLERS ----------------

pub async fn handle_split_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::splitter::SplitRequest>,
) -> Result<Json<crate::tools::splitter::SplitResponse>, (StatusCode, String)> {
    let valid_source = validate_path_access(&state, &headers, &payload.source_path, false)?;
    let valid_dest = match payload.dest_dir.as_deref() {
        Some(dir) => Some(validate_path_access(&state, &headers, dir, true)?),
        None => {
            validate_sibling_output(&state, &headers, &valid_source)?;
            None
        }
    };
    let chunk_size = payload.chunk_size_mb.max(1) * 1024 * 1024;
    let gen_chk = payload.generate_checksum.unwrap_or(true);

    crate::tools::splitter::split_file_sync(Path::new(&valid_source), valid_dest.as_deref().map(Path::new), chunk_size, gen_chk)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_combine_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::splitter::CombineRequest>,
) -> Result<Json<crate::tools::splitter::CombineResponse>, (StatusCode, String)> {
    let parts: Vec<std::path::PathBuf> = validate_paths(&state, &headers, &payload.parts, false)?
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect();
    let valid_dest = validate_path_access(&state, &headers, &payload.dest_path, true)?;

    crate::tools::splitter::combine_files_sync(&parts, Path::new(&valid_dest), payload.expected_sha256.as_deref())
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}



// ---------------- PDF TOOL HANDLERS ----------------

#[derive(Deserialize)]
pub struct PdfInfoQuery {
    path: String,
}

pub async fn handle_pdf_info(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PdfInfoQuery>,
) -> Result<Json<crate::tools::pdf::PdfInfo>, (StatusCode, String)> {
    let valid_path = validate_path_access(&state, &headers, &query.path, false)?;
    let local_path = crate::vfs::local::LocalFs::resolve_local_path(&valid_path);
    crate::tools::pdf::PdfEngine::get_info(&local_path)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

pub async fn handle_pdf_merge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::pdf::MergeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut local_sources = Vec::new();
    for src in &payload.sources {
        let valid_src = validate_path_access(&state, &headers, src, false)?;
        local_sources.push(crate::vfs::local::LocalFs::resolve_local_path(&valid_src));
    }
    let valid_dst = validate_path_access(&state, &headers, &payload.destination, true)?;
    let local_dst = crate::vfs::local::LocalFs::resolve_local_path(&valid_dst);

    crate::tools::pdf::PdfEngine::merge(&local_sources, &local_dst, payload.add_bookmarks.unwrap_or(true))
        .map(|_| Json(serde_json::json!({ "success": true, "destination": valid_dst })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

pub async fn handle_pdf_split(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::pdf::SplitRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_src = validate_path_access(&state, &headers, &payload.source, false)?;
    let local_src = crate::vfs::local::LocalFs::resolve_local_path(&valid_src);

    let valid_dst_dir = validate_path_access(&state, &headers, &payload.destination_dir, true)?;
    let local_dst_dir = crate::vfs::local::LocalFs::resolve_local_path(&valid_dst_dir);

    crate::tools::pdf::PdfEngine::split(
        &local_src,
        &local_dst_dir,
        &payload.split_mode,
        payload.page_ranges.as_deref(),
        payload.chunk_size,
        payload.output_prefix.as_deref(),
    )
    .map(|files| {
        let files_str: Vec<String> = files.iter().map(|p| p.to_string_lossy().to_string()).collect();
        Json(serde_json::json!({ "success": true, "files": files_str }))
    })
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

pub async fn handle_pdf_reorder(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::pdf::PageReorderRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let valid_src = validate_path_access(&state, &headers, &payload.source, false)?;
    let local_src = crate::vfs::local::LocalFs::resolve_local_path(&valid_src);

    let valid_dst = validate_path_access(&state, &headers, &payload.destination, true)?;
    let local_dst = crate::vfs::local::LocalFs::resolve_local_path(&valid_dst);

    crate::tools::pdf::PdfEngine::reorder_and_rotate(&local_src, &payload.pages, &local_dst)
        .map(|_| Json(serde_json::json!({ "success": true, "destination": valid_dst })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}



pub async fn handle_list_plugins(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ListPluginsResponse>, (StatusCode, String)> {
    let mut is_admin = false;
    let mut can_install = false;
    let mut allowed_plugins_json = "[\"*\"]".to_string();
    let mut blocked_plugins_json = "[]".to_string();

    if state.config.server.standalone || !state.config.server.enable_auth {
        is_admin = true;
        can_install = true;
    } else {
        let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
        if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
            if let Ok(claims) = state.auth.verify_token(token_str) {
                if claims.role == "admin" {
                    is_admin = true;
                    can_install = true;
                } else if let Ok(Some(user)) = state.auth.get_user_by_username(&claims.sub) {
                    can_install = user.can_install_plugins || state.config.plugins.allow_user_installs;
                    allowed_plugins_json = user.allowed_plugins;
                    blocked_plugins_json = user.blocked_plugins;
                }
            }
        }
    }

    let all_plugins = state.plugins.scan_plugins()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to scan plugins: {}", e)))?;

    let filtered = all_plugins.into_iter()
        .filter(|p| state.plugins.is_allowed_for_user(&p.id, is_admin, &allowed_plugins_json, &blocked_plugins_json))
        .collect();

    Ok(Json(ListPluginsResponse {
        plugins: filtered,
        can_install,
        allow_user_installs: state.config.plugins.allow_user_installs,
    }))
}

pub async fn handle_install_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<crate::plugins::PluginInfo>, (StatusCode, String)> {
    let mut is_admin = false;
    let mut can_install = false;
    let mut username = "local".to_string();

    if state.config.server.standalone || !state.config.server.enable_auth {
        is_admin = true;
        can_install = true;
        username = "admin".to_string();
    } else {
        let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
        if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
            if let Ok(claims) = state.auth.verify_token(token_str) {
                username = claims.sub.clone();
                if claims.role == "admin" {
                    is_admin = true;
                    can_install = true;
                } else if let Ok(Some(user)) = state.auth.get_user_by_username(&claims.sub) {
                    can_install = user.can_install_plugins || state.config.plugins.allow_user_installs;
                }
            }
        }
    }

    if !can_install && !is_admin {
        return Err((StatusCode::FORBIDDEN, "Permission denied: you do not have permission to install plugins".to_string()));
    }

    let raw_bytes = body.to_vec();
    if raw_bytes.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty payload received for plugin installation".to_string()));
    }

    // Handle direct ZIP payload or extract ZIP stream from multipart
    let zip_bytes = if raw_bytes.starts_with(&[0x50, 0x4B]) {
        raw_bytes
    } else if let Some(pos) = raw_bytes.windows(4).position(|w| w == [0x50, 0x4B, 0x03, 0x04]) {
        if let Some(eocd_pos) = raw_bytes.windows(4).rposition(|w| w == [0x50, 0x4B, 0x05, 0x06]) {
            if eocd_pos + 22 <= raw_bytes.len() {
                let comment_len = u16::from_le_bytes([raw_bytes[eocd_pos + 20], raw_bytes[eocd_pos + 21]]) as usize;
                let end_pos = (eocd_pos + 22 + comment_len).min(raw_bytes.len());
                raw_bytes[pos..end_pos].to_vec()
            } else {
                raw_bytes[pos..].to_vec()
            }
        } else {
            raw_bytes[pos..].to_vec()
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Invalid package format: file is not a valid .grr (ZIP) archive".to_string()));
    };

    let installed = state.plugins.install_grr(&zip_bytes, &username, is_admin)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Plugin installation failed: {}", e)))?;

    Ok(Json(installed))
}

#[derive(Deserialize)]
pub struct TogglePluginRequest {
    enabled: bool,
}

pub async fn handle_toggle_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<TogglePluginRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.config.server.standalone && state.config.server.enable_auth {
        let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
        if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
            let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
            if claims.role != "admin" {
                return Err((StatusCode::FORBIDDEN, "Only administrators can enable or disable plugins globally".to_string()));
            }
        } else {
            return Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()));
        }
    }

    let result = state.plugins.toggle_plugin(&id, payload.enabled)
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    Ok(Json(serde_json::json!({ "success": true, "enabled": result })))
}

pub async fn handle_delete_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut is_admin = false;
    let username;

    if state.config.server.standalone || !state.config.server.enable_auth {
        is_admin = true;
        username = "admin".to_string();
    } else {
        let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
        if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
            let claims = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
            username = claims.sub.clone();
            if claims.role == "admin" {
                is_admin = true;
            }
        } else {
            return Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()));
        }
    }

    state.plugins.uninstall_plugin(&id, &username, is_admin)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    Ok(Json(serde_json::json!({ "success": true, "uninstalled": id })))
}

pub async fn handle_plugin_asset(
    State(state): State<AppState>,
    AxumPath((id, subpath)): AxumPath<(String, String)>,
) -> Result<Response, (StatusCode, String)> {
    plugin_file_response(&state, &id, &subpath)
}

fn plugin_file_response(state: &AppState, id: &str, subpath: &str) -> Result<Response, (StatusCode, String)> {
    let (bytes, mime) = state.plugins.get_asset(id, subpath)
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    let response = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate, max-age=0")
        .header(header::PRAGMA, "no-cache")
        .header(header::EXPIRES, "0")
        .header(header::CONTENT_SECURITY_POLICY, "sandbox allow-scripts allow-forms allow-modals allow-popups; default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:;")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(response)
}

/// How long a signed Chewtoy address stays valid
const CHEWTOY_FILES_TTL_SECS: i64 = 12 * 3600;

/// Whether the signed-in user may open a Chewtoy: it exists, is enabled and allowed for them
fn may_open_plugin(state: &AppState, username: &str, role: &str, id: &str) -> bool {
    let Some(plugin) = state.plugins.get_plugin(id) else { return false };
    if !plugin.is_enabled {
        return false;
    }
    let is_admin = role == "admin" || state.config.server.standalone || !state.config.server.enable_auth;
    let (allowed, blocked) = match state.auth.get_user_by_username(username) {
        Ok(Some(user)) => (user.allowed_plugins, user.blocked_plugins),
        _ => ("[\"*\"]".to_string(), "[]".to_string()),
    };
    state.plugins.is_allowed_for_user(id, is_admin, &allowed, &blocked)
}

/// Gives the page a signed address for a Chewtoy's files. The Chewtoy runs sandboxed, where
/// the browser sends no login cookie; files loaded below this address need none.
pub async fn handle_plugin_launch(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    if !may_open_plugin(&state, &claims.sub, &claims.role, &id) {
        return Err((StatusCode::NOT_FOUND, format!("Plugin '{}' not found", id)));
    }
    let exp = chrono::Utc::now().timestamp() + CHEWTOY_FILES_TTL_SECS;
    let sig = state.auth.sign_chewtoy_files(&id, &claims.sub, exp);
    Ok(Json(serde_json::json!({
        "base": format!("/api/plugins/{}/files/{}/", id, sig),
        "expires_at": exp,
    })))
}

/// A Chewtoy file requested below a signed address (no login cookie needed)
pub async fn handle_plugin_signed_file(
    State(state): State<AppState>,
    AxumPath((id, sig, subpath)): AxumPath<(String, String, String)>,
) -> Result<Response, (StatusCode, String)> {
    const EXPIRED: &str = "This Chewtoy address has expired or is not valid. Reopen the Chewtoy.";
    let username = state.auth.verify_chewtoy_files(&id, &sig).ok_or((StatusCode::FORBIDDEN, EXPIRED.to_string()))?;
    // Still allowed this Chewtoy (system-account users have no Brum user record)
    let role = state.auth.get_user_by_username(&username).ok().flatten().map(|u| u.role).unwrap_or_default();
    if !may_open_plugin(&state, &username, &role, &id) {
        return Err((StatusCode::FORBIDDEN, EXPIRED.to_string()));
    }
    plugin_file_response(&state, &id, &subpath)
}

// ---------------- Bundled Chewtoys ----------------

/// Whether the caller may install Chewtoys (admins, or users allowed to install)
fn caller_can_install_plugins(state: &AppState, headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let claims = extract_claims_or_local(state, headers)?;
    if claims.role == "admin" || state.config.server.standalone || !state.config.server.enable_auth {
        return Ok(claims.sub);
    }
    let allowed = state.auth.get_user_by_username(&claims.sub).ok().flatten()
        .map(|u| u.can_install_plugins || state.config.plugins.allow_user_installs)
        .unwrap_or(false);
    if allowed {
        Ok(claims.sub)
    } else {
        Err((StatusCode::FORBIDDEN, "Permission denied: you do not have permission to install plugins".to_string()))
    }
}

/// The Chewtoys that ship with Brum, with installed and bundled versions
pub async fn handle_list_bundled_plugins(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::plugins::BundledPluginInfo>>, (StatusCode, String)> {
    extract_claims_or_local(&state, &headers)?;
    Ok(Json(state.plugins.list_bundled()))
}

/// Installs, reinstalls or updates a bundled Chewtoy
pub async fn handle_install_bundled_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<crate::plugins::PluginInfo>, (StatusCode, String)> {
    let user = caller_can_install_plugins(&state, &headers)?;
    state.plugins.install_bundled(&id, &user).map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

/// The bundled `.grr` package, to keep or install elsewhere
pub async fn handle_download_bundled_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, (StatusCode, String)> {
    extract_claims_or_local(&state, &headers)?;
    let bytes = crate::plugins::PluginManager::bundled_package(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("'{}' is not a bundled Chewtoy", id)))?;
    let version = crate::plugins::read_grr_manifest(bytes).map(|m| m.plugin.version).unwrap_or_default();
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}-{}.grr\"", id, version))
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}
