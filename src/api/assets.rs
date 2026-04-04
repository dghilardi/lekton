use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::asset_repository::AssetRepository;
use crate::db::models::Asset;
use crate::error::AppError;
use crate::storage::client::StorageClient;

/// Compute the SHA-256 content hash for an asset in `sha256:<base64url>` format.
pub fn compute_content_hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use base64::engine::{general_purpose::URL_SAFE_NO_PAD, Engine as _};
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
pub async fn process_upload_asset(
    asset_repo: &dyn AssetRepository,
    storage: &dyn StorageClient,
    key: &str,
    content_type: &str,
    data: Vec<u8>,
    uploaded_by: &str,
    expected_token: &str,
    service_token: &str,
    max_size: u64,
) -> Result<AssetUploadResponse, AppError> {
    // Validate token
    if service_token != expected_token {
        return Err(AppError::Auth("Invalid service token".into()));
    }

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
pub async fn process_serve_asset(
    asset_repo: &dyn AssetRepository,
    storage: &dyn StorageClient,
    key: &str,
) -> Result<(String, Vec<u8>), AppError> {
    let asset = asset_repo
        .find_by_key(key)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Asset '{}' not found", key)))?;

    let data = storage
        .get_object(&asset.s3_key)
        .await?
        .ok_or_else(|| AppError::Storage(format!("Asset content missing in storage for '{}'", key)))?;

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
pub async fn process_delete_asset(
    asset_repo: &dyn AssetRepository,
    storage: &dyn StorageClient,
    key: &str,
    expected_token: &str,
    service_token: &str,
) -> Result<(), AppError> {
    if service_token != expected_token {
        return Err(AppError::Auth("Invalid service token".into()));
    }

    let asset = asset_repo
        .find_by_key(key)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Asset '{}' not found", key)))?;

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
pub async fn process_check_hashes(
    asset_repo: &dyn AssetRepository,
    entries: &[CheckHashEntry],
    expected_token: &str,
    service_token: &str,
) -> Result<CheckHashesResponse, AppError> {
    if service_token != expected_token {
        return Err(AppError::Auth("Invalid service token".into()));
    }

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

/// Core editor upload logic — no token validation, generates key from filename.
pub async fn process_editor_upload(
    asset_repo: &dyn AssetRepository,
    storage: &dyn StorageClient,
    file_name: &str,
    content_type: &str,
    data: Vec<u8>,
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
        uploaded_by: "web-editor".to_string(),
        referenced_by: vec![],
        content_hash,
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
        &state.service_token,
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
    let mut content_type = None;

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
                content_type = Some(
                    field
                        .content_type()
                        .unwrap_or("application/octet-stream")
                        .to_string(),
                );
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
    let data =
        file_data.ok_or_else(|| AppError::BadRequest("Missing file field".into()))?;
    let content_type = content_type.unwrap_or_else(|| "application/octet-stream".to_string());

    let response = process_upload_asset(
        state.asset_repo.as_ref(),
        state.storage_client.as_ref(),
        &key,
        &content_type,
        data,
        &service_token, // use token as uploader identity for now
        &state.service_token,
        &service_token,
        state.max_attachment_size_bytes,
    )
    .await?;

    Ok(axum::Json(response))
}

/// Axum handler for `GET /api/v1/assets/{*key}`.
#[cfg(feature = "ssr")]
pub async fn serve_asset_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;

    let (content_type, data) = process_serve_asset(
        state.asset_repo.as_ref(),
        state.storage_client.as_ref(),
        &key,
    )
    .await?;

    Ok((
        [(axum::http::header::CONTENT_TYPE, content_type)],
        data,
    )
        .into_response())
}

/// Axum handler for `GET /api/v1/assets`.
#[cfg(feature = "ssr")]
pub async fn list_assets_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Query(query): axum::extract::Query<ListAssetsQuery>,
) -> Result<axum::Json<Vec<AssetListItem>>, AppError> {
    let items = process_list_assets(
        state.asset_repo.as_ref(),
        query.prefix.as_deref(),
    )
    .await?;

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
        &state.service_token,
        &request.service_token,
    )
    .await?;

    Ok(axum::Json(
        serde_json::json!({"message": format!("Asset '{}' deleted", key)}),
    ))
}

/// Axum handler for `POST /api/v1/editor/upload-asset`.
///
/// Editor-based upload — no service token required. Accepts multipart with `file` field.
#[cfg(feature = "ssr")]
pub async fn editor_upload_asset_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    mut multipart: axum::extract::Multipart,
) -> Result<axum::Json<EditorUploadResponse>, AppError> {
    let mut file_data = None;
    let mut content_type = None;
    let mut file_name = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            file_name = Some(
                field
                    .file_name()
                    .unwrap_or("upload.bin")
                    .to_string(),
            );
            content_type = Some(
                field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string(),
            );
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
    let content_type = content_type.unwrap_or_else(|| "application/octet-stream".to_string());
    let file_name = file_name.unwrap_or_else(|| "upload.bin".to_string());

    let response = process_editor_upload(
        state.asset_repo.as_ref(),
        state.storage_client.as_ref(),
        &file_name,
        &content_type,
        data,
    )
    .await?;

    Ok(axum::Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    use crate::test_utils::MockStorage;

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
                return Err(AppError::NotFound(format!(
                    "Asset '{}' not found",
                    key
                )));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_upload_asset_success() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();
        let data = b"hello world".to_vec();

        let result = process_upload_asset(
            &repo, &storage, "project/file.txt", "text/plain", data,
            "ci-bot", "valid-token", "valid-token",
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
            &repo, &storage, "file.txt", "text/plain", vec![1, 2, 3],
            "ci-bot", "valid-token", "wrong-token",
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
            &repo, &storage, "", "text/plain", vec![1],
            "ci-bot", "valid-token", "valid-token",
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
            &repo, &storage, "project/../secret/file.txt", "text/plain", vec![1],
            "ci-bot", "valid-token", "valid-token",
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
            &repo, &storage, "/absolute/path.txt", "text/plain", vec![1],
            "ci-bot", "valid-token", "valid-token",
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
            &repo, &storage, "logo.png", "image/png", vec![1, 2, 3],
            "ci-bot", "valid-token", "valid-token",
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
            &repo, &storage, "logo.png", "image/png", vec![4, 5, 6, 7],
            "ci-bot", "valid-token", "valid-token",
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
            &repo, &storage, "docs/manual.pdf", "application/pdf", content.clone(),
            "ci-bot", "valid-token", "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        // Serve
        let (ct, data) = process_serve_asset(&repo, &storage, "docs/manual.pdf")
            .await
            .unwrap();

        assert_eq!(ct, "application/pdf");
        assert_eq!(data, content);
    }

    #[tokio::test]
    async fn test_serve_asset_not_found() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_serve_asset(&repo, &storage, "nonexistent.txt").await;

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
                &repo, &storage, name, "text/plain", vec![1],
                "ci-bot", "valid-token", "valid-token",
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

        for name in &["project-a/config.yaml", "project-a/logo.png", "project-b/readme.md"] {
            process_upload_asset(
                &repo, &storage, name, "text/plain", vec![1],
                "ci-bot", "valid-token", "valid-token",
                DEFAULT_MAX_ATTACHMENT_SIZE,
            )
            .await
            .unwrap();
        }

        let list = process_list_assets(&repo, Some("project-a/")).await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_asset_success() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        process_upload_asset(
            &repo, &storage, "temp/file.txt", "text/plain", vec![1, 2, 3],
            "ci-bot", "valid-token", "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let result = process_delete_asset(
            &repo, &storage, "temp/file.txt", "valid-token", "valid-token",
        )
        .await;

        assert!(result.is_ok());

        // Verify removed from repo
        assert!(repo.find_by_key("temp/file.txt").await.unwrap().is_none());

        // Verify removed from storage
        assert!(storage.objects.lock().unwrap().get("assets/temp/file.txt").is_none());
    }

    #[tokio::test]
    async fn test_delete_asset_not_found() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        let result = process_delete_asset(
            &repo, &storage, "nonexistent.txt", "valid-token", "valid-token",
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::NotFound(msg) => assert!(msg.contains("nonexistent.txt")),
            other => panic!("Expected NotFound error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_delete_asset_invalid_token() {
        let repo = MockAssetRepo::new();
        let storage = MockStorage::new();

        process_upload_asset(
            &repo, &storage, "file.txt", "text/plain", vec![1],
            "ci-bot", "valid-token", "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let result = process_delete_asset(
            &repo, &storage, "file.txt", "valid-token", "wrong-token",
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
        assert_eq!(asset.uploaded_by, "web-editor");
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
            &repo, &storage, "existing.txt", "text/plain", b"hello".to_vec(),
            "ci-bot", "valid-token", "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let existing_hash = compute_content_hash(b"hello");

        let entries = vec![
            // Same hash — should NOT be in to_upload
            CheckHashEntry { key: "existing.txt".to_string(), content_hash: existing_hash.clone() },
            // Different hash — should be in to_upload
            CheckHashEntry { key: "existing.txt".to_string(), content_hash: "sha256:different".to_string() },
            // Missing key — should be in to_upload
            CheckHashEntry { key: "missing.txt".to_string(), content_hash: "sha256:whatever".to_string() },
        ];

        // Use a unique key for the "different hash" case
        let entries = vec![
            CheckHashEntry { key: "existing.txt".to_string(), content_hash: existing_hash },
            CheckHashEntry { key: "missing.txt".to_string(), content_hash: "sha256:whatever".to_string() },
        ];

        let result = process_check_hashes(&repo, &entries, "valid-token", "valid-token")
            .await
            .unwrap();

        assert_eq!(result.to_upload, vec!["missing.txt".to_string()]);
    }

    #[tokio::test]
    async fn test_check_hashes_invalid_token() {
        let repo = MockAssetRepo::new();

        let result = process_check_hashes(&repo, &[], "valid-token", "wrong-token").await;

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
            &repo, &storage, "file.txt", "text/plain", b"version1".to_vec(),
            "ci-bot", "valid-token", "valid-token",
            DEFAULT_MAX_ATTACHMENT_SIZE,
        )
        .await
        .unwrap();

        let new_hash = compute_content_hash(b"version2");

        let entries = vec![
            CheckHashEntry { key: "file.txt".to_string(), content_hash: new_hash },
        ];

        let result = process_check_hashes(&repo, &entries, "valid-token", "valid-token")
            .await
            .unwrap();

        assert_eq!(result.to_upload, vec!["file.txt".to_string()]);
    }
}
