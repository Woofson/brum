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
    extract_claims_or_local,
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
    crate::tools::diff::compare_folders(&left, &right, recursive, deep, payload.selected_items)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Folder diff failed: {}", e)))
}

pub async fn handle_convert_file(
    Json(payload): Json<crate::tools::converter::ConvertRequest>,
) -> Result<Json<crate::tools::converter::ConvertResponse>, (StatusCode, String)> {
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
    Json(payload): Json<DryRunRequest>,
) -> Result<Json<crate::tools::paranoid::DryRunPreview>, (StatusCode, String)> {
    ParanoidEngine::dry_run(&payload.action, &payload.sources, payload.destination.as_deref())
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Dry run failed: {}", e)))
}



// ---------------- NOTEDOG CHEWTOY HANDLERS ----------------

#[derive(Deserialize)]
pub struct NoteDogInfoQuery {
    folder: Option<String>,
}

pub async fn handle_notedog_info(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<NoteDogInfoQuery>,
) -> Result<Json<crate::tools::notedog::NoteDogInfo>, (StatusCode, String)> {
    let cfg_folder = state.config.notedog.notes_folder.as_str();
    let info = crate::tools::notedog::scan_notedog_hierarchy(query.folder.as_deref(), Some(cfg_folder));
    Ok(Json(info))
}

pub async fn handle_notedog_templates() -> Json<Vec<crate::tools::notedog::NoteTemplate>> {
    Json(crate::tools::notedog::get_builtin_templates())
}

#[derive(Deserialize)]
pub struct NoteDogVersionsQuery {
    path: String,
}

pub async fn handle_notedog_versions(
    axum::extract::Query(query): axum::extract::Query<NoteDogVersionsQuery>,
) -> Json<Vec<crate::tools::notedog::NoteVersionItem>> {
    let p = Path::new(&query.path);
    Json(crate::tools::notedog::list_note_versions(p))
}

#[derive(Deserialize)]
pub struct NoteDogSaveVersionRequest {
    path: String,
}

pub async fn handle_notedog_save_version(
    Json(payload): Json<NoteDogSaveVersionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    if p.exists() {
        if let Ok(bytes) = std::fs::read(p) {
            let _ = crate::tools::notedog::create_note_snapshot(p, &bytes);
        }
    }
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct NoteDogDecryptRequest {
    path: String,
    passphrase: Option<String>,
}

#[derive(Serialize)]
pub struct NoteDogDecryptResponse {
    content: String,
    path: String,
}

pub async fn handle_notedog_decrypt(
    Json(payload): Json<NoteDogDecryptRequest>,
) -> Result<Json<NoteDogDecryptResponse>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    if !p.exists() {
        return Err((StatusCode::NOT_FOUND, "Note file not found".to_string()));
    }
    let pass = payload.passphrase.as_deref().unwrap_or("notedog");
    match crate::tools::notedog::decrypt_note_file(p, pass) {
        Ok(content) => Ok(Json(NoteDogDecryptResponse {
            content,
            path: payload.path,
        })),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

#[derive(Deserialize)]
pub struct NoteDogEncryptRequest {
    path: String,
    content: String,
    passphrase: Option<String>,
    convert_to_plain: Option<bool>,
}

#[derive(Serialize)]
pub struct NoteDogEncryptResponse {
    success: bool,
    path: String,
    is_encrypted: bool,
}

pub async fn handle_notedog_encrypt(
    Json(payload): Json<NoteDogEncryptRequest>,
) -> Result<Json<NoteDogEncryptResponse>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    let pass = payload.passphrase.as_deref().unwrap_or("notedog");

    if payload.convert_to_plain.unwrap_or(false) {
        match crate::tools::notedog::decrypt_and_save_plain_note(p, pass) {
            Ok(new_path) => Ok(Json(NoteDogEncryptResponse {
                success: true,
                path: new_path.to_string_lossy().to_string(),
                is_encrypted: false,
            })),
            Err(err) => Err((StatusCode::BAD_REQUEST, err)),
        }
    } else {
        match crate::tools::notedog::encrypt_and_save_note(p, &payload.content, pass) {
            Ok(new_path) => Ok(Json(NoteDogEncryptResponse {
                success: true,
                path: new_path.to_string_lossy().to_string(),
                is_encrypted: true,
            })),
            Err(err) => Err((StatusCode::BAD_REQUEST, err)),
        }
    }
}

#[derive(Deserialize)]
pub struct NoteDogCreateNoteRequest {
    section_path: String,
    title: String,
    is_encrypted: Option<bool>,
    passphrase: Option<String>,
    initial_content: Option<String>,
}

pub async fn handle_notedog_create_note(
    Json(payload): Json<NoteDogCreateNoteRequest>,
) -> Result<Json<crate::tools::notedog::NoteDogFile>, (StatusCode, String)> {
    let sec_p = Path::new(&payload.section_path);
    match crate::tools::notedog::create_new_note(
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
pub struct NoteDogSectionActionRequest {
    path: String,
    passphrase: String,
    cached_pass: Option<String>,
}

pub async fn handle_notedog_section_encrypt(
    Json(payload): Json<NoteDogSectionActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    match crate::tools::notedog::encrypt_section_dir(p, &payload.passphrase, payload.cached_pass.as_deref()) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "encrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

pub async fn handle_notedog_section_decrypt(
    Json(payload): Json<NoteDogSectionActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    match crate::tools::notedog::decrypt_section_dir(p, &payload.passphrase) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "decrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

#[derive(Deserialize)]
pub struct NoteDogNotebookActionRequest {
    path: String,
    passphrase: String,
    cached_pass: Option<String>,
}

pub async fn handle_notedog_notebook_encrypt(
    Json(payload): Json<NoteDogNotebookActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    match crate::tools::notedog::encrypt_notebook_dir(p, &payload.passphrase, payload.cached_pass.as_deref()) {
        Ok(count) => Ok(Json(serde_json::json!({ "success": true, "encrypted_count": count }))),
        Err(err) => Err((StatusCode::BAD_REQUEST, err)),
    }
}

pub async fn handle_notedog_notebook_decrypt(
    Json(payload): Json<NoteDogNotebookActionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let p = Path::new(&payload.path);
    match crate::tools::notedog::decrypt_notebook_dir(p, &payload.passphrase) {
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
    let attachments_dir = crate::tools::notedog::get_notes_attachments_dir();
    let _ = std::fs::create_dir_all(&attachments_dir);

    while let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))? {
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

pub async fn handle_notes_migrate_export(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let is_admin = claims.role == "admin";
    let notes = state.auth.list_db_notes(&claims.sub, is_admin, None, None, None, None, false)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let root = crate::tools::notedog::get_notedog_root_dir(None, Some(&state.config.notedog.notes_folder));
    let mut exported_count = 0;

    for note in notes {
        let cat = if note.category.is_empty() { "General".to_string() } else { note.category };
        let sec = if note.section.is_empty() { "Default".to_string() } else { note.section };
        let target_dir = root.join(&cat).join(&sec);
        let _ = std::fs::create_dir_all(&target_dir);

        let sanitized_title: String = note.title.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
            .collect();
        let base_name = if sanitized_title.trim().is_empty() { "Untitled_Note" } else { sanitized_title.trim() };
        let file_path = target_dir.join(format!("{}.md", base_name));

        if std::fs::write(&file_path, note.content.as_bytes()).is_ok() {
            let _ = crate::tools::notedog::create_note_snapshot(&file_path, note.content.as_bytes());
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
    let info = crate::tools::notedog::scan_notedog_hierarchy(None, Some(&state.config.notedog.notes_folder));
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



// ---------------- TETRADOG CLASSIC ARCADE CHEWTOY & LEADERBOARD HANDLERS ----------------

#[derive(Deserialize)]
pub struct TetraDogScoresQuery {
    limit: Option<usize>,
    mode: Option<String>,
    user_only: Option<bool>,
}

pub async fn handle_tetradog_get_scores(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TetraDogScoresQuery>,
) -> Result<Json<crate::tools::tetradog::TetraLeaderboardResponse>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;
    let limit = query.limit.unwrap_or(50);
    let mode = query.mode.as_deref();
    let user_only = query.user_only.unwrap_or(false);

    let resp = crate::tools::tetradog::get_leaderboard(&conn, &claims.sub, limit, mode, user_only)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

pub async fn handle_tetradog_submit_score(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::tetradog::SubmitScoreRequest>,
) -> Result<Json<crate::tools::tetradog::SubmitScoreResponse>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;

    let resp = crate::tools::tetradog::submit_score(&conn, &claims.sub, payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

pub async fn handle_tetradog_clear_scores(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims_or_local(&state, &headers)?;
    let db_arc = state.auth.db();
    let conn = db_arc.lock().map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database lock poisoned".to_string()))?;

    let user_filter = if claims.role == "admin" { None } else { Some(claims.sub.as_str()) };
    let cleared = crate::tools::tetradog::clear_scores(&conn, user_filter)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "success": true, "cleared_count": cleared })))
}



// ---------------- PHASE 4 HANDLERS (SYNC, SEARCH, SCRIPT ACTIONS) ----------------

#[derive(Deserialize)]
pub struct SyncRequest {
    source: String,
    destination: String,
    options: crate::tools::sync::SyncOptions,
}

pub async fn handle_sync_analyze(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SyncRequest>,
) -> Result<Json<crate::tools::sync::SyncAnalysis>, (StatusCode, String)> {
    let source = validate_path_access(&state, &headers, &payload.source, false)?;
    let destination = validate_path_access(&state, &headers, &payload.destination, false)?;

    crate::tools::sync::DirectorySyncEngine::analyze(&source, &destination, &payload.options)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Sync analysis failed: {}", e)))
}

pub async fn handle_sync_execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SyncRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = validate_path_access(&state, &headers, &payload.source, false)?;
    let destination = validate_path_access(&state, &headers, &payload.destination, true)?;

    crate::tools::sync::DirectorySyncEngine::execute_sync(
        state.tasks.clone(),
        &source,
        &destination,
        payload.options,
    ).await
    .map(Json)
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Sync execution failed: {}", e)))
}

pub async fn handle_list_backup_profiles(
    State(state): State<AppState>,
) -> Json<Vec<crate::tools::sync::BackupProfile>> {
    Json(state.backup.list_profiles())
}

pub async fn handle_save_backup_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(profile): Json<crate::tools::sync::BackupProfile>,
) -> Result<Json<crate::tools::sync::BackupProfile>, (StatusCode, String)> {
    validate_path_access(&state, &headers, &profile.source_dir, false)?;
    validate_path_access(&state, &headers, &profile.dest_dir, true)?;

    state.backup.save_profile(profile)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save backup profile: {}", e)))
}

pub async fn handle_delete_backup_profile(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state.backup.delete_profile(&id)
        .map(|_| Json(serde_json::json!({ "success": true })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete backup profile: {}", e)))
}

pub async fn handle_run_backup_profile(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mgr = state.backup.clone();
    let tasks = state.tasks.clone();
    
    // Run in background Tokio task
    tokio::spawn(async move {
        if let Err(e) = mgr.execute_profile_job(&id, tasks).await {
            tracing::error!("Manual backup profile execution error for '{}': {}", id, e);
        }
    });

    Ok(Json(serde_json::json!({ "success": true, "message": "Backup job initiated in background" })))
}

pub async fn handle_toggle_backup_profile(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state.backup.toggle_profile(&id)
        .map(|new_enabled| Json(serde_json::json!({ "success": true, "enabled": new_enabled })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to toggle backup profile: {}", e)))
}

pub async fn handle_get_backup_history(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Vec<crate::tools::sync::BackupHistoryItem>> {
    let profile_id = query.get("profile_id").map(|s| s.as_str());
    let limit = query.get("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(50);
    Json(state.backup.list_history(profile_id, limit))
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
    Json(payload): Json<crate::tools::search::SearchRequest>,
) -> Result<Json<Vec<crate::tools::search::SearchResultItem>>, (StatusCode, String)> {
    crate::tools::search::SearchEngine::search(payload)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Search failed: {}", e)))
}

pub async fn handle_duplicates_scan(
    Json(payload): Json<crate::tools::duplicates::DuplicateScanRequest>,
) -> Json<crate::tools::duplicates::DuplicateScanResponse> {
    Json(crate::tools::duplicates::scan_duplicates(payload))
}

pub async fn handle_duplicates_clean(
    Json(payload): Json<crate::tools::duplicates::DuplicateCleanRequest>,
) -> Json<crate::tools::duplicates::DuplicateCleanResponse> {
    Json(crate::tools::duplicates::clean_duplicates(payload))
}

#[derive(Deserialize)]
pub struct ReadMetadataQuery {
    path: String,
}

pub async fn handle_metadata_read(
    Query(query): Query<ReadMetadataQuery>,
) -> Result<Json<crate::tools::metadata::FileMetadataResponse>, (StatusCode, String)> {
    crate::tools::metadata::read_file_metadata(&query.path)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

pub async fn handle_metadata_update(
    Json(payload): Json<crate::tools::metadata::UpdateMetadataRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::tools::metadata::update_file_metadata(payload)
        .map(|_| Json(serde_json::json!({ "success": true })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

pub async fn handle_metadata_batch(
    Json(payload): Json<crate::tools::metadata::BatchMetadataRequest>,
) -> Json<crate::tools::metadata::BatchMetadataResponse> {
    Json(crate::tools::metadata::batch_update_metadata(payload))
}

pub async fn handle_logviewer_tail(
    Query(payload): Query<crate::tools::logviewer::LogTailRequest>,
) -> Result<Json<crate::tools::logviewer::LogTailResponse>, (StatusCode, String)> {
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
    let custom_trash = query.custom_trash_dir.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
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
    let custom_trash = query.custom_trash_dir.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
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
    let custom_trash = payload.custom_trash_dir.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
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
    let custom_trash = payload.custom_trash_dir.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
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
    let custom_trash = payload.custom_trash_dir.as_deref().or(state.config.paranoid.custom_trash_dir.as_deref());
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
    Json(payload): Json<SyncthingScanRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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

pub async fn handle_git_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitPathQuery>,
) -> Result<Json<crate::tools::git::GitStatusResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::get_git_status(Path::new(&query.path)).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

pub async fn handle_git_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitDiffQuery>,
) -> Result<Json<crate::tools::git::GitDiffResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::get_git_diff(Path::new(&query.path), query.file.as_deref(), query.staged.unwrap_or(false)).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

pub async fn handle_git_stage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitFilesRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_stage(Path::new(&payload.path), &payload.files).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

pub async fn handle_git_unstage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitFilesRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_unstage(Path::new(&payload.path), &payload.files).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

pub async fn handle_git_commit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitCommitRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_commit(Path::new(&payload.path), &payload.message).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

pub async fn handle_git_push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitPushRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_push(Path::new(&payload.path), payload.remote.as_deref(), payload.branch.as_deref()).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

pub async fn handle_git_pull(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitPullRequest>,
) -> Result<Json<crate::tools::git::GitActionResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::git_pull(Path::new(&payload.path)).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}

pub async fn handle_git_log(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GitLogQuery>,
) -> Result<Json<Vec<crate::tools::git::GitCommitInfo>>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let res = crate::tools::git::get_git_log(Path::new(&query.path), query.count.unwrap_or(30)).await;
        return Ok(Json(res));
    }
    Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
}



// ---------------- FILE SPLITTER & COMBINER HANDLERS ----------------

pub async fn handle_split_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::splitter::SplitRequest>,
) -> Result<Json<crate::tools::splitter::SplitResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let source_path = Path::new(&payload.source_path);
        let dest_dir = payload.dest_dir.as_ref().map(|p| Path::new(p));
        let chunk_size = payload.chunk_size_mb.max(1) * 1024 * 1024;
        let gen_chk = payload.generate_checksum.unwrap_or(true);

        crate::tools::splitter::split_file_sync(source_path, dest_dir, chunk_size, gen_chk)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
}

pub async fn handle_combine_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<crate::tools::splitter::CombineRequest>,
) -> Result<Json<crate::tools::splitter::CombineResponse>, (StatusCode, String)> {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if let Some(token_str) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        let _ = state.auth.verify_token(token_str).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
        let parts: Vec<std::path::PathBuf> = payload.parts.iter().map(std::path::PathBuf::from).collect();
        let dest_path = Path::new(&payload.dest_path);

        crate::tools::splitter::combine_files_sync(&parts, dest_path, payload.expected_sha256.as_deref())
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Missing authorization token".to_string()))
    }
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
    let (bytes, mime) = state.plugins.get_asset(&id, &subpath)
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    let response = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate, max-age=0")
        .header(header::PRAGMA, "no-cache")
        .header(header::EXPIRES, "0")
        .header(header::CONTENT_SECURITY_POLICY, "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:;")
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(response)
}
