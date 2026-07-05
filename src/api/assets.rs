use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::db::asset_repository::AssetRepository;
use crate::db::models::Asset;
use crate::db::repository::DocumentRepository;
use crate::error::AppError;
use crate::storage::client::StorageClient;

/// Derive a safe content type from a filename extension.
///
/// Ignores client-supplied Content-Type to prevent XSS via uploaded HTML/JS.
/// Unknown or dangerous extensions fall back to `application/octet-stream`.
pub fn safe_content_type_from_filename(filename: &str) -> &'static str {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e)
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Decide the `Content-Disposition` for a served asset from its content type.
///
/// Image and PDF types that browsers render safely are served `inline`. SVG is
/// served `attachment` because it can carry executable script (a stored-XSS
/// vector on direct navigation while still embeddable via `<img>`); everything
/// else (including `application/octet-stream`) is also `attachment`, so untrusted
/// bytes are never rendered as a top-level document in our origin.
pub fn content_disposition_for(content_type: &str) -> &'static str {
    match content_type {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "application/pdf" => "inline",
        _ => "attachment",
    }
}

/// Compute the SHA-256 content hash for an asset in `sha256:<base64url>` format.
pub fn compute_content_hash(data: &[u8]) -> String {
    use base64::engine::{general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(data);
    format!("sha256:{}", URL_SAFE_NO_PAD.encode(hash))
}

/// Response from a successful asset upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetUploadResponse {
    pub message: String,
    pub key: String,
    pub s3_key: String,
    pub content_type: String,
    pub size_bytes: u64,
}

/// An asset entry in list responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetListItem {
    pub key: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub uploaded_at: DateTime<Utc>,
}

/// Response from an editor-based asset upload (no service token required).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorUploadResponse {
    pub key: String,
    pub url: String,
    pub content_type: String,
    pub size_bytes: u64,
}

/// Query parameters for listing assets.
#[derive(Debug, Deserialize)]
pub struct ListAssetsQuery {
    pub prefix: Option<String>,
}

/// Default maximum attachment size in bytes (25 MB).
pub const DEFAULT_MAX_ATTACHMENT_SIZE: u64 = 25 * 1024 * 1024;

/// Core upload logic — separated from HTTP layer for testability.
#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)]
pub async fn process_upload_asset(
    asset_repo: &dyn AssetRepository,
    storage: &dyn StorageClient,
    key: &str,
    content_type: &str,
    data: Vec<u8>,
    uploaded_by: &str,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    service_token: &str,
    max_size: u64,
) -> Result<AssetUploadResponse, AppError> {
    // Validate token (legacy or DB-backed)
    crate::api::token_validation::validate_service_token(
        service_token_repo,
        legacy_token,
        service_token,
    )
    .await?;

    // Validate key
    if key.is_empty() {
        return Err(AppError::BadRequest("Asset key cannot be empty".into()));
    }
    if key.starts_with('/') {
        return Err(AppError::BadRequest(
            "Asset key must not start with '/'".into(),
        ));
    }
    if key.contains("..") {
        return Err(AppError::BadRequest(
            "Asset key must not contain '..'".into(),
        ));
    }

    let size_bytes = data.len() as u64;
    if size_bytes > max_size {
        return Err(AppError::BadRequest(format!(
            "File size ({:.1} MB) exceeds maximum allowed size ({:.1} MB)",
            size_bytes as f64 / (1024.0 * 1024.0),
            max_size as f64 / (1024.0 * 1024.0),
        )));
    }
    let s3_key = format!("assets/{}", key);
    let content_hash = Some(compute_content_hash(&data));

    // Upload to S3
    storage.put_object(&s3_key, data).await?;

    // Preserve referenced_by from existing asset if updating
    let referenced_by = if let Some(existing) = asset_repo.find_by_key(key).await? {
        existing.referenced_by
    } else {
        vec![]
    };

    let asset = Asset {
        key: key.to_string(),
        content_type: content_type.to_string(),
        size_bytes,
        s3_key: s3_key.clone(),
        uploaded_at: Utc::now(),
        uploaded_by: uploaded_by.to_string(),
        referenced_by,
        content_hash,
        extraction_status: crate::db::models::ExtractionStatus::Pending,
        extraction_error: None,
        extracted_content_hash: None,
        extracted_at: None,
        indexed_chunks: None,
    };

    asset_repo.create_or_update(asset).await?;

    Ok(AssetUploadResponse {
        message: "Asset uploaded successfully".to_string(),
        key: key.to_string(),
        s3_key,
        content_type: content_type.to_string(),
        size_bytes,
    })
}

/// Core serve logic — returns (content_type, data).
///
/// Access is derived from the documents that reference the asset:
/// - No referencing documents → only the uploader or an admin may serve it.
/// - At least one referencing document → allowed if any referenced doc passes
///   `doc_is_accessible(access_level, is_draft, allowed_levels, include_draft)`.
pub async fn process_serve_asset(
    asset_repo: &dyn AssetRepository,
    document_repo: &dyn DocumentRepository,
    storage: &dyn StorageClient,
    key: &str,
    allowed_levels: Option<&[String]>,
    include_draft: bool,
    user_email: Option<&str>,
) -> Result<(String, Vec<u8>), AppError> {
    let asset = asset_repo
        .find_by_key(key)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Asset '{}' not found", key)))?;

    if asset.referenced_by.is_empty() {
        let is_admin = allowed_levels.is_none();
        let is_uploader = user_email == Some(asset.uploaded_by.as_str());
        if !is_admin && !is_uploader {
            return Err(AppError::Forbidden("Asset access denied".into()));
        }
    } else {
        let documents: HashMap<_, _> = document_repo
            .find_by_slugs(&asset.referenced_by)
            .await?
            .into_iter()
            .map(|doc| (doc.slug.clone(), doc))
            .collect();
        let accessible = asset.referenced_by.iter().any(|slug| {
            documents.get(slug).is_some_and(|doc| {
                crate::app::doc_is_accessible(
                    &doc.access_level,
                    doc.is_draft,
                    allowed_levels,
                    include_draft,
                )
            })
        });
        if !accessible {
            return Err(AppError::Forbidden("Asset access denied".into()));
        }
    }

    let data = storage.get_object(&asset.s3_key).await?.ok_or_else(|| {
        AppError::Storage(format!("Asset content missing in storage for '{}'", key))
    })?;

    Ok((asset.content_type, data))
}

/// Core list logic.
pub async fn process_list_assets(
    asset_repo: &dyn AssetRepository,
    prefix: Option<&str>,
) -> Result<Vec<AssetListItem>, AppError> {
    let assets = match prefix {
        Some(p) if !p.is_empty() => asset_repo.list_by_prefix(p).await?,
        _ => asset_repo.list_all().await?,
    };

    Ok(assets
        .into_iter()
        .map(|a| AssetListItem {
            key: a.key,
            content_type: a.content_type,
            size_bytes: a.size_bytes,
            uploaded_at: a.uploaded_at,
        })
        .collect())
}

/// Core delete logic.
#[cfg(feature = "ssr")]
pub async fn process_delete_asset(
    asset_repo: &dyn AssetRepository,
    storage: &dyn StorageClient,
    key: &str,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    service_token: &str,
) -> Result<(), AppError> {
    crate::api::token_validation::validate_service_token(
        service_token_repo,
        legacy_token,
        service_token,
    )
    .await?;

    let asset = asset_repo.find_by_key(key).await?;

    let Some(asset) = asset else {
        return Ok(());
    };

    storage.delete_object(&asset.s3_key).await?;
    asset_repo.delete(key).await?;

    Ok(())
}

/// Request for checking which assets need uploading based on content hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckHashesRequest {
    pub service_token: String,
    pub entries: Vec<CheckHashEntry>,
}

/// A single entry in a check-hashes request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckHashEntry {
    pub key: String,
    pub content_hash: String,
}

/// Response indicating which asset keys need uploading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckHashesResponse {
    pub to_upload: Vec<String>,
}

/// Core check-hashes logic: returns which keys are missing or have a different hash.
#[cfg(feature = "ssr")]
pub async fn process_check_hashes(
    asset_repo: &dyn AssetRepository,
    entries: &[CheckHashEntry],
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    service_token: &str,
) -> Result<CheckHashesResponse, AppError> {
    crate::api::token_validation::validate_service_token(
        service_token_repo,
        legacy_token,
        service_token,
    )
    .await?;

    let mut to_upload = Vec::new();
    for entry in entries {
        match asset_repo.find_by_key(&entry.key).await? {
            Some(asset) => {
                if asset.content_hash.as_deref() != Some(&entry.content_hash) {
                    to_upload.push(entry.key.clone());
                }
            }
            None => {
                to_upload.push(entry.key.clone());
            }
        }
    }

    Ok(CheckHashesResponse { to_upload })
}

/// Core editor upload logic — generates key from filename, records uploader identity.
pub async fn process_editor_upload(
    asset_repo: &dyn AssetRepository,
    storage: &dyn StorageClient,
    file_name: &str,
    content_type: &str,
    data: Vec<u8>,
    uploaded_by: &str,
) -> Result<EditorUploadResponse, AppError> {
    let sanitized_name: String = file_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let timestamp = Utc::now().timestamp_millis();
    let key = format!("editor/{}_{}", timestamp, sanitized_name);
    let s3_key = format!("assets/{}", key);
    let size_bytes = data.len() as u64;
    let content_hash = Some(compute_content_hash(&data));

    storage.put_object(&s3_key, data).await?;

    let asset = Asset {
        key: key.clone(),
        content_type: content_type.to_string(),
        size_bytes,
        s3_key,
        uploaded_at: Utc::now(),
        uploaded_by: uploaded_by.to_string(),
        referenced_by: vec![],
        content_hash,
        extraction_status: crate::db::models::ExtractionStatus::Pending,
        extraction_error: None,
        extracted_content_hash: None,
        extracted_at: None,
        indexed_chunks: None,
    };

    asset_repo.create_or_update(asset).await?;

    Ok(EditorUploadResponse {
        url: format!("/api/v1/assets/{}", key),
        key,
        content_type: content_type.to_string(),
        size_bytes,
    })
}

// --- HTTP Handlers ---

/// Axum handler for `POST /api/v1/assets/check-hashes`.
///
/// Accepts a JSON body with service_token and a list of (key, content_hash) entries.
/// Returns which keys need uploading (missing or hash mismatch).
#[cfg(feature = "ssr")]
pub async fn check_hashes_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<CheckHashesRequest>,
) -> Result<axum::Json<CheckHashesResponse>, AppError> {
    let response = process_check_hashes(
        state.asset_repo.as_ref(),
        &request.entries,
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        &request.service_token,
    )
    .await?;

    Ok(axum::Json(response))
}

/// Axum handler for `PUT /api/v1/assets/{*key}`.
///
/// Accepts a multipart form with `service_token` and `file` fields.
#[cfg(feature = "ssr")]
pub async fn upload_asset_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Path(key): axum::extract::Path<String>,
    mut multipart: axum::extract::Multipart,
) -> Result<axum::Json<AssetUploadResponse>, AppError> {
    let mut service_token = None;
    let mut file_data = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "service_token" => {
                service_token = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("Failed to read token: {e}")))?,
                );
            }
            "file" => {
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("Failed to read file: {e}")))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    let service_token =
        service_token.ok_or_else(|| AppError::BadRequest("Missing service_token field".into()))?;
    let data = file_data.ok_or_else(|| AppError::BadRequest("Missing file field".into()))?;
    let content_type = safe_content_type_from_filename(&key).to_string();

    let response = process_upload_asset(
        state.asset_repo.as_ref(),
        state.storage_client.as_ref(),
        &key,
        &content_type,
        data,
        &service_token, // use token as uploader identity for now
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        &service_token,
        state.max_attachment_size_bytes,
    )
    .await?;

    if let Some(queue) = &state.attachment_queue {
        queue.enqueue(&key);
    }

    Ok(axum::Json(response))
}

/// Axum handler for `GET /api/v1/assets/{*key}`.
#[cfg(feature = "ssr")]
pub async fn serve_asset_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::OptionalAuthUser(user): crate::auth::extractor::OptionalAuthUser,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;

    let user_email = user.as_ref().map(|u| u.email.as_str());
    let (allowed_levels, include_draft) =
        crate::app::resolve_user_visibility(&state, user.as_ref()).await?;

    let (content_type, data) = process_serve_asset(
        state.asset_repo.as_ref(),
        state.document_repo.as_ref(),
        state.storage_client.as_ref(),
        &key,
        allowed_levels.as_deref(),
        include_draft,
        user_email,
    )
    .await?;

    let disposition = content_disposition_for(&content_type);
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, content_type),
            (
                axum::http::header::CONTENT_DISPOSITION,
                disposition.to_string(),
            ),
            (
                axum::http::header::X_CONTENT_TYPE_OPTIONS,
                "nosniff".to_string(),
            ),
        ],
        data,
    )
        .into_response())
}

/// Axum handler for `GET /api/v1/assets`.
#[cfg(feature = "ssr")]
pub async fn list_assets_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::RequiredAuthUser(user): crate::auth::extractor::RequiredAuthUser,
    axum::extract::Query(query): axum::extract::Query<ListAssetsQuery>,
) -> Result<axum::Json<Vec<AssetListItem>>, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin privileges required".into()));
    }

    let items = process_list_assets(state.asset_repo.as_ref(), query.prefix.as_deref()).await?;

    Ok(axum::Json(items))
}

/// Request body for asset deletion.
#[derive(Debug, Deserialize)]
pub struct DeleteAssetRequest {
    pub service_token: String,
}

/// Axum handler for `DELETE /api/v1/assets/{*key}`.
#[cfg(feature = "ssr")]
pub async fn delete_asset_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Path(key): axum::extract::Path<String>,
    axum::Json(request): axum::Json<DeleteAssetRequest>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    process_delete_asset(
        state.asset_repo.as_ref(),
        state.storage_client.as_ref(),
        &key,
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        &request.service_token,
    )
    .await?;

    // Remove any RAG chunks indexed from this attachment.
    if let Some(rag) = &state.rag_service {
        if let Err(e) = rag.delete_attachment(&key).await {
            tracing::warn!(key, "Failed to delete attachment chunks from RAG: {e}");
        }
    }
    if let Some(search) = &state.attachment_search_service {
        if let Err(e) = search.delete_attachment(&key).await {
            tracing::warn!(key, "Failed to delete attachment chunks from search: {e}");
        }
    }

    Ok(axum::Json(
        serde_json::json!({"message": format!("Asset '{}' deleted", key)}),
    ))
}

/// Axum handler for `POST /api/v1/editor/upload-asset`.
///
/// Editor-based upload — requires an authenticated session. Accepts multipart with `file` field.
#[cfg(feature = "ssr")]
pub async fn editor_upload_asset_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::RequiredAuthUser(user): crate::auth::extractor::RequiredAuthUser,
    mut multipart: axum::extract::Multipart,
) -> Result<axum::Json<EditorUploadResponse>, AppError> {
    let mut file_data = None;
    let mut file_name = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            file_name = Some(field.file_name().unwrap_or("upload.bin").to_string());
            file_data = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("Failed to read file: {e}")))?
                    .to_vec(),
            );
        }
    }

    let data = file_data.ok_or_else(|| AppError::BadRequest("Missing file field".into()))?;
    let file_name = file_name.unwrap_or_else(|| "upload.bin".to_string());
    let content_type = safe_content_type_from_filename(&file_name).to_string();

    let response = process_editor_upload(
        state.asset_repo.as_ref(),
        state.storage_client.as_ref(),
        &file_name,
        &content_type,
        data,
        &user.email,
    )
    .await?;

    if let Some(queue) = &state.attachment_queue {
        queue.enqueue(&response.key);
    }

    Ok(axum::Json(response))
}

/// Axum handler for `POST /api/v1/document-upload/asset`.
///
/// Admin-only upload backing the guided document-upload form. Accepts multipart
/// with a `file` field and PDF content. Mounted only when the `document_upload`
/// feature is enabled, so it stays available even in a read-only portal where
/// the editor upload surface is gated off.
#[cfg(feature = "ssr")]
pub async fn admin_upload_asset_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::RequiredAuthUser(user): crate::auth::extractor::RequiredAuthUser,
    mut multipart: axum::extract::Multipart,
) -> Result<axum::Json<EditorUploadResponse>, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin privileges required".into()));
    }

    let mut file_data = None;
    let mut file_name = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Multipart error: {e}")))?
    {
        if field.name() == Some("file") {
            file_name = Some(field.file_name().unwrap_or("upload.pdf").to_string());
            file_data = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("Failed to read file: {e}")))?
                    .to_vec(),
            );
        }
    }

    let data = file_data.ok_or_else(|| AppError::BadRequest("Missing file field".into()))?;
    let file_name = file_name.unwrap_or_else(|| "upload.pdf".to_string());
    let content_type = safe_content_type_from_filename(&file_name).to_string();
    if !content_type.starts_with("application/pdf") {
        return Err(AppError::BadRequest("Only PDF files are supported".into()));
    }

    let response = process_editor_upload(
        state.asset_repo.as_ref(),
        state.storage_client.as_ref(),
        &file_name,
        &content_type,
        data,
        &user.email,
    )
    .await?;

    // Note: indexing is intentionally NOT enqueued here. For the document-upload
    // flow the asset is indexed when the document is saved
    // (`save_document_with_attachment`), so extraction/embedding does not compete
    // with AI summary generation for LLM quota, and chunks are indexed with the
    // document's access levels already known (no `access_levels=[]` pass).

    Ok(axum::Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    use crate::db::models::Document;
    use crate::db::repository::DocumentRepository;
    use crate::db::service_token_models::ServiceToken;
    use crate::db::service_token_repository::ServiceTokenRepository;
    use crate::test_utils::MockStorage;

    struct MockServiceTokenRepo;

    #[async_trait]
    impl ServiceTokenRepository for MockServiceTokenRepo {
        async fn create(&self, _: ServiceToken) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn find_by_hash(&self, _: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(None)
        }
        async fn find_by_name(&self, _: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(None)
        }
        async fn find_by_id(&self, _: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(None)
        }
        async fn list_all(&self) -> Result<Vec<ServiceToken>, AppError> {
            Ok(vec![])
        }
        async fn deactivate(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn touch_last_used(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn check_scope_overlap(
            &self,
            _: &[String],
            _: Option<&str>,
        ) -> Result<bool, AppError> {
            Ok(false)
        }
        async fn set_active(&self, _: &str, _: bool) -> Result<(), AppError> {
            Ok(())
        }
        async fn list_by_user_id(&self, _: &str) -> Result<Vec<ServiceToken>, AppError> {
            Ok(vec![])
        }
        async fn list_pats_paginated(
            &self,
            _: u64,
            _: u64,
        ) -> Result<(Vec<ServiceToken>, u64), AppError> {
            Ok((vec![], 0))
        }
        async fn delete_pat(&self, _: &str, _: &str) -> Result<(), AppError> {
            Ok(())
        }
    }

    struct MockDocumentRepo {
        docs: Vec<Document>,
        single_lookups: Mutex<usize>,
        batch_lookups: Mutex<usize>,
    }

    impl MockDocumentRepo {
        fn empty() -> Self {
            Self {
                docs: vec![],
                single_lookups: Mutex::new(0),
                batch_lookups: Mutex::new(0),
            }
        }

        fn with(docs: Vec<Document>) -> Self {
            Self {
                docs,
                single_lookups: Mutex::new(0),
                batch_lookups: Mutex::new(0),
            }
        }

        fn single_lookup_count(&self) -> usize {
            *self.single_lookups.lock().unwrap()
        }

        fn batch_lookup_count(&self) -> usize {
            *self.batch_lookups.lock().unwrap()
        }
    }

    #[async_trait]
    impl DocumentRepository for MockDocumentRepo {
        async fn create_or_update(&self, _: Document) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn find_by_slug(&self, slug: &str) -> Result<Option<Document>, AppError> {
            *self.single_lookups.lock().unwrap() += 1;
            Ok(self.docs.iter().find(|d| d.slug == slug).cloned())
        }
        async fn find_by_slugs(&self, slugs: &[String]) -> Result<Vec<Document>, AppError> {
            *self.batch_lookups.lock().unwrap() += 1;
            Ok(self
                .docs
                .iter()
                .filter(|doc| slugs.iter().any(|slug| slug == &doc.slug))
                .cloned()
                .collect())
        }
        async fn list_all(&self) -> Result<Vec<Document>, AppError> {
            Ok(self.docs.clone())
        }
        async fn list_by_access_levels(
            &self,
            _: Option<&[String]>,
            _: bool,
        ) -> Result<Vec<Document>, AppError> {
            unimplemented!()
        }
        async fn update_backlinks(
            &self,
            _: &str,
            _: &[String],
            _: &[String],
        ) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn find_by_slug_prefix(&self, _: &str) -> Result<Vec<Document>, AppError> {
            unimplemented!()
        }
        async fn set_archived(&self, _: &str, _: bool) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn rename_slug(&self, _: &str, _: &str) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn find_by_source_path(&self, _: &str) -> Result<Option<Document>, AppError> {
            unimplemented!()
        }
        async fn find_all_by_source_id(&self, _: &str) -> Result<Vec<Document>, AppError> {
            unimplemented!()
        }
    }

    struct MockAssetRepo {
        assets: Mutex<Vec<Asset>>,
    }

    impl MockAssetRepo {
        fn new() -> Self {
            Self {
                assets: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl AssetRepository for MockAssetRepo {
        async fn create_or_update(&self, asset: Asset) -> Result<(), AppError> {
            let mut assets = self.assets.lock().unwrap();
            assets.retain(|a| a.key != asset.key);
            assets.push(asset);
            Ok(())
        }

        async fn find_by_key(&self, key: &str) -> Result<Option<Asset>, AppError> {
            Ok(self
                .assets
                .lock()
                .unwrap()
                .iter()
                .find(|a| a.key == key)
                .cloned())
        }

        async fn list_all(&self) -> Result<Vec<Asset>, AppError> {
            Ok(self.assets.lock().unwrap().clone())
        }

        async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Asset>, AppError> {
            Ok(self
                .assets
                .lock()
                .unwrap()
                .iter()
                .filter(|a| a.key.starts_with(prefix))
                .cloned()
                .collect())
        }

        async fn delete(&self, key: &str) -> Result<(), AppError> {
            let mut assets = self.assets.lock().unwrap();
            let len_before = assets.len();
            assets.retain(|a| a.key != key);
            if assets.len() == len_before {
                return Err(AppError::NotFound(format!("Asset '{}' not found", key)));
            }
            Ok(())
        }

        async fn update_extraction(
            &self,
            key: &str,
            update: crate::db::asset_repository::ExtractionUpdate,
        ) -> Result<(), AppError> {
            let mut assets = self.assets.lock().unwrap();
            if let Some(a) = assets.iter_mut().find(|a| a.key == key) {
                a.extraction_status = update.status;
                a.extraction_error = update.error;
                a.extracted_content_hash = update.extracted_content_hash;
                a.extracted_at = update.extracted_at;
                a.indexed_chunks = update.indexed_chunks;
            }
            Ok(())
        }

        async fn set_references(
            &self,
            source_slug: &str,
            keys: &[String],
        ) -> Result<Vec<String>, AppError> {
            let mut assets = self.assets.lock().unwrap();
            let mut affected = Vec::new();
            for a in assets.iter_mut() {
                let referenced = keys.contains(&a.key);
                let has = a.referenced_by.iter().any(|s| s == source_slug);
                if referenced && !has {
                    a.referenced_by.push(source_slug.to_string());
                    affected.push(a.key.clone());
                } else if !referenced && has {
                    a.referenced_by.retain(|s| s != source_slug);
                    affected.push(a.key.clone());
                }
            }
            Ok(affected)
        }
    }

    #[tokio::test]
    async fn test_upload_asset_success() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();
        let data = b"hello world".to_vec();

        let result = process_upload_asset(
            &repo,
            &storage,
            "project/file.txt",
            "text/plain",
            data,
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.key, "project/file.txt");
        assert_eq!(response.s3_key, "assets/project/file.txt");
        assert_eq!(response.content_type, "text/plain");
        assert_eq!(response.size_bytes, 11);

        // Verify in repo
        let asset = repo.find_by_key("project/file.txt").await.unwrap().unwrap();
        assert_eq!(asset.content_type, "text/plain");
        assert_eq!(asset.size_bytes, 11);

        // Verify in storage
        let stored = storage.objects.lock().unwrap();
        assert!(stored.contains_key("assets/project/file.txt"));
    }

    #[tokio::test]
    async fn test_upload_asset_invalid_token() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_upload_asset(
            &repo,
            &storage,
            "file.txt",
            "text/plain",
            vec![1, 2, 3],
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "wrong-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("Invalid service token")),
            other => panic!("Expected Auth error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_upload_asset_empty_key() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_upload_asset(
            &repo,
            &storage,
            "",
            "text/plain",
            vec![1],
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("cannot be empty")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_upload_asset_key_with_dotdot_rejected() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_upload_asset(
            &repo,
            &storage,
            "project/../secret/file.txt",
            "text/plain",
            vec![1],
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("..")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_upload_asset_key_starting_with_slash_rejected() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_upload_asset(
            &repo,
            &storage,
            "/absolute/path.txt",
            "text/plain",
            vec![1],
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("must not start with '/'")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_upload_asset_replaces_existing_preserves_referenced_by() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        // Upload initial version
        process_upload_asset(
            &repo,
            &storage,
            "logo.png",
            "image/png",
            vec![1, 2, 3],
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        // Simulate referenced_by being set (as Phase 5b would do)
        {
            let mut assets = repo.assets.lock().unwrap();
            assets[0].referenced_by = vec!["deployment-guide".to_string()];
        }

        // Upload replacement
        process_upload_asset(
            &repo,
            &storage,
            "logo.png",
            "image/png",
            vec![4, 5, 6, 7],
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let asset = repo.find_by_key("logo.png").await.unwrap().unwrap();
        assert_eq!(asset.size_bytes, 4);
        assert_eq!(asset.referenced_by, vec!["deployment-guide".to_string()]);
    }

    #[tokio::test]
    async fn test_serve_asset_success() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();
        let content = b"PDF content here".to_vec();

        // Upload first
        process_upload_asset(
            &repo,
            &storage,
            "docs/manual.pdf",
            "application/pdf",
            content.clone(),
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        // Serve (admin visibility — unrestricted)
        let doc_repo = MockDocumentRepo::empty();
        let (ct, data) = process_serve_asset(
            &repo,
            &doc_repo,
            &storage,
            "docs/manual.pdf",
            None,
            true,
            None,
        )
        .await
        .unwrap();

        assert_eq!(ct, "application/pdf");
        assert_eq!(data, content);
    }

    #[tokio::test]
    async fn test_serve_asset_batches_referenced_document_lookup() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();
        let doc_repo = MockDocumentRepo::with(vec![
            Document {
                slug: "docs/a".to_string(),
                title: "Doc A".to_string(),
                summary: None,
                s3_key: "docs_a.md".to_string(),
                access_level: "public".to_string(),
                is_draft: false,
                service_owner: "team-a".to_string(),
                last_updated: Utc::now(),
                tags: vec![],
                links_out: vec![],
                backlinks: vec![],
                parent_slug: None,
                order: 0,
                is_hidden: false,
                content_hash: None,
                metadata_hash: None,
                is_archived: false,
                source_path: None,
                source_id: None,
                needs_reindex: false,
                skip_rag: false,
            },
            Document {
                slug: "docs/b".to_string(),
                title: "Doc B".to_string(),
                summary: None,
                s3_key: "docs_b.md".to_string(),
                access_level: "internal".to_string(),
                is_draft: false,
                service_owner: "team-b".to_string(),
                last_updated: Utc::now(),
                tags: vec![],
                links_out: vec![],
                backlinks: vec![],
                parent_slug: None,
                order: 0,
                is_hidden: false,
                content_hash: None,
                metadata_hash: None,
                is_archived: false,
                source_path: None,
                source_id: None,
                needs_reindex: false,
                skip_rag: false,
            },
        ]);

        repo.create_or_update(Asset {
            key: "docs/shared.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            size_bytes: 3,
            s3_key: "assets/docs/shared.pdf".to_string(),
            uploaded_at: Utc::now(),
            uploaded_by: "ci-bot".to_string(),
            referenced_by: vec!["docs/a".to_string(), "docs/b".to_string()],
            content_hash: None,
            extraction_status: crate::db::models::ExtractionStatus::Pending,
            extraction_error: None,
            extracted_content_hash: None,
            extracted_at: None,
            indexed_chunks: None,
        })
        .await
        .unwrap();
        storage
            .put_object("assets/docs/shared.pdf", b"pdf".to_vec())
            .await
            .unwrap();

        let result = process_serve_asset(
            &repo,
            &doc_repo,
            &storage,
            "docs/shared.pdf",
            Some(&["internal".to_string()]),
            false,
            Some("reader@example.com"),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(doc_repo.single_lookup_count(), 0);
        assert_eq!(doc_repo.batch_lookup_count(), 1);
    }

    #[tokio::test]
    async fn test_serve_asset_not_found() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let doc_repo = MockDocumentRepo::empty();
        let result = process_serve_asset(
            &repo,
            &doc_repo,
            &storage,
            "nonexistent.txt",
            None,
            true,
            None,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::NotFound(msg) => assert!(msg.contains("nonexistent.txt")),
            other => panic!("Expected NotFound error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_assets_all() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        for name in &["a/file1.txt", "b/file2.txt", "c/file3.txt"] {
            process_upload_asset(
                &repo,
                &storage,
                name,
                "text/plain",
                vec![1],
                "ci-bot",
                &MockServiceTokenRepo,
                Some("valid-token"),
                "valid-token",
                DEFAULT_MAX_ATTACHMENT_SIZE,
            )
            .await
            .unwrap();
        }

        let list = process_list_assets(&repo, None).await.unwrap();
        assert_eq!(list.len(), 3);
    }

    #[tokio::test]
    async fn test_list_assets_with_prefix() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        for name in &[
            "project-a/config.yaml",
            "project-a/logo.png",
            "project-b/readme.md",
        ] {
            process_upload_asset(
                &repo,
                &storage,
                name,
                "text/plain",
                vec![1],
                "ci-bot",
                &MockServiceTokenRepo,
                Some("valid-token"),
                "valid-token",
                DEFAULT_MAX_ATTACHMENT_SIZE,
            )
            .await
            .unwrap();
        }

        let list = process_list_assets(&repo, Some("project-a/"))
            .await
            .unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_asset_success() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        process_upload_asset(
            &repo,
            &storage,
            "temp/file.txt",
            "text/plain",
            vec![1, 2, 3],
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let result = process_delete_asset(
            &repo,
            &storage,
            "temp/file.txt",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
        )
        .await;

        assert!(result.is_ok());

        // Verify removed from repo
        assert!(repo.find_by_key("temp/file.txt").await.unwrap().is_none());

        // Verify removed from storage
        assert!(storage
            .objects
            .lock()
            .unwrap()
            .get("assets/temp/file.txt")
            .is_none());
    }

    #[tokio::test]
    async fn test_delete_asset_missing_asset_is_idempotent() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_delete_asset(
            &repo,
            &storage,
            "nonexistent.txt",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_asset_invalid_token() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        process_upload_asset(
            &repo,
            &storage,
            "file.txt",
            "text/plain",
            vec![1],
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let result = process_delete_asset(
            &repo,
            &storage,
            "file.txt",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "wrong-token",
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("Invalid service token")),
            other => panic!("Expected Auth error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_editor_upload_success() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_editor_upload(
            &repo,
            &storage,
            "test image.png",
            "image/png",
            vec![0x89, 0x50, 0x4E, 0x47],
            "editor@example.com",
        )
        .await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.key.starts_with("editor/"));
        assert!(response.key.contains("test_image.png"));
        assert_eq!(response.content_type, "image/png");
        assert_eq!(response.size_bytes, 4);
        assert!(response.url.starts_with("/api/v1/assets/editor/"));

        // Verify asset was stored in repo
        let asset = repo.find_by_key(&response.key).await.unwrap().unwrap();
        assert_eq!(asset.uploaded_by, "editor@example.com");
        assert_eq!(asset.content_type, "image/png");

        // Verify in storage
        let stored = storage.objects.lock().unwrap();
        assert!(stored.contains_key(&format!("assets/{}", response.key)));
    }

    #[tokio::test]
    async fn test_editor_upload_sanitizes_filename() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_editor_upload(
            &repo,
            &storage,
            "my file (1).png",
            "image/png",
            vec![1, 2, 3],
            "editor@example.com",
        )
        .await
        .unwrap();

        // Spaces and parens should be sanitized to underscores
        assert!(result.key.contains("my_file__1_.png"));
    }

    #[tokio::test]
    async fn test_check_hashes_identifies_missing_and_changed() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        // Upload an asset so it exists with a known hash
        process_upload_asset(
            &repo,
            &storage,
            "existing.txt",
            "text/plain",
            b"hello".to_vec(),
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let existing_hash = compute_content_hash(b"hello");

        let _entries = [
            // Same hash — should NOT be in to_upload
            CheckHashEntry {
                key: "existing.txt".to_string(),
                content_hash: existing_hash.clone(),
            },
            // Different hash — should be in to_upload
            CheckHashEntry {
                key: "existing.txt".to_string(),
                content_hash: "sha256:different".to_string(),
            },
            // Missing key — should be in to_upload
            CheckHashEntry {
                key: "missing.txt".to_string(),
                content_hash: "sha256:whatever".to_string(),
            },
        ];

        // Use a unique key for the "different hash" case
        let entries = vec![
            CheckHashEntry {
                key: "existing.txt".to_string(),
                content_hash: existing_hash,
            },
            CheckHashEntry {
                key: "missing.txt".to_string(),
                content_hash: "sha256:whatever".to_string(),
            },
        ];

        let result = process_check_hashes(
            &repo,
            &entries,
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
        )
        .await
        .unwrap();

        assert_eq!(result.to_upload, vec!["missing.txt".to_string()]);
    }

    #[tokio::test]
    async fn test_check_hashes_invalid_token() {
        let repo = MockAssetRepo::new();

        let result = process_check_hashes(
            &repo,
            &[],
            &MockServiceTokenRepo,
            Some("valid-token"),
            "wrong-token",
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("Invalid service token")),
            other => panic!("Expected Auth error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_check_hashes_changed_content() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        process_upload_asset(
            &repo,
            &storage,
            "file.txt",
            "text/plain",
            b"version1".to_vec(),
            "ci-bot",
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let new_hash = compute_content_hash(b"version2");

        let entries = vec![CheckHashEntry {
            key: "file.txt".to_string(),
            content_hash: new_hash,
        }];

        let result = process_check_hashes(
            &repo,
            &entries,
            &MockServiceTokenRepo,
            Some("valid-token"),
            "valid-token",
        )
        .await
        .unwrap();

        assert_eq!(result.to_upload, vec!["file.txt".to_string()]);
    }

    fn test_doc(slug: &str, access_level: &str, is_draft: bool) -> Document {
        Document {
            slug: slug.to_string(),
            title: "Test".to_string(),
            summary: None,
            s3_key: format!("docs/{}.md", slug),
            access_level: access_level.to_string(),
            is_draft,
            service_owner: "platform".to_string(),
            last_updated: Utc::now(),
            tags: vec![],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            content_hash: None,
            metadata_hash: None,
            is_archived: false,
            source_path: None,
            source_id: None,
            needs_reindex: false,
            skip_rag: false,
        }
    }

    fn asset_with_refs(key: &str, uploaded_by: &str, referenced_by: Vec<String>) -> Asset {
        Asset {
            key: key.to_string(),
            content_type: "image/png".to_string(),
            size_bytes: 4,
            s3_key: format!("assets/{}", key),
            uploaded_at: Utc::now(),
            uploaded_by: uploaded_by.to_string(),
            referenced_by,
            content_hash: None,
            extraction_status: crate::db::models::ExtractionStatus::Pending,
            extraction_error: None,
            extracted_content_hash: None,
            extracted_at: None,
            indexed_chunks: None,
        }
    }

    #[tokio::test]
    async fn test_serve_asset_admin_always_allowed_unreferenced() {
        let asset_repo = MockAssetRepo::new();
        let doc_repo = MockDocumentRepo::empty();
        let storage = MockStorage::new();

        let asset = asset_with_refs("img.png", "ci-bot", vec![]);
        asset_repo.assets.lock().unwrap().push(asset.clone());
        storage
            .objects
            .lock()
            .unwrap()
            .insert(asset.s3_key.clone(), b"data".to_vec());

        let result = process_serve_asset(
            &asset_repo,
            &doc_repo,
            &storage,
            "img.png",
            None, // admin
            true,
            None,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_serve_asset_uploader_allowed_unreferenced() {
        let asset_repo = MockAssetRepo::new();
        let doc_repo = MockDocumentRepo::empty();
        let storage = MockStorage::new();

        let asset = asset_with_refs("img.png", "user@example.com", vec![]);
        asset_repo.assets.lock().unwrap().push(asset.clone());
        storage
            .objects
            .lock()
            .unwrap()
            .insert(asset.s3_key.clone(), b"data".to_vec());

        let allowed = vec!["public".to_string(), "internal".to_string()];
        let result = process_serve_asset(
            &asset_repo,
            &doc_repo,
            &storage,
            "img.png",
            Some(&allowed),
            false,
            Some("user@example.com"),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_serve_asset_unauthenticated_denied_unreferenced() {
        let asset_repo = MockAssetRepo::new();
        let doc_repo = MockDocumentRepo::empty();
        let storage = MockStorage::new();

        let asset = asset_with_refs("img.png", "ci-bot", vec![]);
        asset_repo.assets.lock().unwrap().push(asset.clone());
        storage
            .objects
            .lock()
            .unwrap()
            .insert(asset.s3_key.clone(), b"data".to_vec());

        let allowed = vec!["public".to_string()];
        let result = process_serve_asset(
            &asset_repo,
            &doc_repo,
            &storage,
            "img.png",
            Some(&allowed),
            false,
            None, // no user
        )
        .await;

        assert!(matches!(result.unwrap_err(), AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn test_serve_asset_allowed_via_accessible_referenced_doc() {
        let asset_repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let asset = asset_with_refs("img.png", "ci-bot", vec!["guide".to_string()]);
        asset_repo.assets.lock().unwrap().push(asset.clone());
        storage
            .objects
            .lock()
            .unwrap()
            .insert(asset.s3_key.clone(), b"data".to_vec());

        let doc_repo = MockDocumentRepo::with(vec![test_doc("guide", "public", false)]);
        let allowed = vec!["public".to_string()];
        let result = process_serve_asset(
            &asset_repo,
            &doc_repo,
            &storage,
            "img.png",
            Some(&allowed),
            false,
            None,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_serve_asset_denied_when_all_referenced_docs_inaccessible() {
        let asset_repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let asset = asset_with_refs("img.png", "ci-bot", vec!["private-doc".to_string()]);
        asset_repo.assets.lock().unwrap().push(asset.clone());
        storage
            .objects
            .lock()
            .unwrap()
            .insert(asset.s3_key.clone(), b"data".to_vec());

        // Doc exists but is internal; caller only has public access
        let doc_repo = MockDocumentRepo::with(vec![test_doc("private-doc", "internal", false)]);
        let allowed = vec!["public".to_string()];
        let result = process_serve_asset(
            &asset_repo,
            &doc_repo,
            &storage,
            "img.png",
            Some(&allowed),
            false,
            None,
        )
        .await;

        assert!(matches!(result.unwrap_err(), AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn test_serve_asset_denied_when_referenced_doc_is_draft_and_user_cannot_read_draft() {
        let asset_repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let asset = asset_with_refs("img.png", "ci-bot", vec!["draft-doc".to_string()]);
        asset_repo.assets.lock().unwrap().push(asset.clone());
        storage
            .objects
            .lock()
            .unwrap()
            .insert(asset.s3_key.clone(), b"data".to_vec());

        let doc_repo = MockDocumentRepo::with(vec![test_doc("draft-doc", "public", true)]);
        let allowed = vec!["public".to_string()];
        let result = process_serve_asset(
            &asset_repo,
            &doc_repo,
            &storage,
            "img.png",
            Some(&allowed),
            false, // cannot read draft
            None,
        )
        .await;

        assert!(matches!(result.unwrap_err(), AppError::Forbidden(_)));
    }

    #[test]
    fn safe_content_type_derives_from_extension_not_client_header() {
        assert_eq!(safe_content_type_from_filename("photo.png"), "image/png");
        assert_eq!(safe_content_type_from_filename("photo.JPG"), "image/jpeg");
        assert_eq!(
            safe_content_type_from_filename("doc.pdf"),
            "application/pdf"
        );
        assert_eq!(
            safe_content_type_from_filename("readme.md"),
            "text/plain; charset=utf-8"
        );
        // Dangerous types fall back to octet-stream
        assert_eq!(
            safe_content_type_from_filename("xss.html"),
            "application/octet-stream"
        );
        assert_eq!(
            safe_content_type_from_filename("evil.js"),
            "application/octet-stream"
        );
        assert_eq!(
            safe_content_type_from_filename("no_extension"),
            "application/octet-stream"
        );
    }

    #[test]
    fn content_disposition_is_attachment_for_svg_and_unknown_types() {
        // Browser-safe render types are inline.
        assert_eq!(content_disposition_for("image/png"), "inline");
        assert_eq!(content_disposition_for("image/jpeg"), "inline");
        assert_eq!(content_disposition_for("application/pdf"), "inline");
        // SVG can carry script → never rendered as a top-level document.
        assert_eq!(content_disposition_for("image/svg+xml"), "attachment");
        // Everything else is downloaded, not rendered.
        assert_eq!(
            content_disposition_for("text/plain; charset=utf-8"),
            "attachment"
        );
        assert_eq!(
            content_disposition_for("application/octet-stream"),
            "attachment"
        );
    }
}
