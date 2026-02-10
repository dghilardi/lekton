use chrono::Utc;

use crate::auth::models::AccessLevel;
use crate::db::models::{Document, IngestRequest, IngestResponse};
use crate::db::repository::DocumentRepository;
use crate::error::AppError;
use crate::storage::client::StorageClient;

/// Core ingestion logic — separated from the HTTP layer for testability.
///
/// Validates the request, uploads content to S3, and upserts metadata in MongoDB.
pub async fn process_ingest(
    repo: &dyn DocumentRepository,
    storage: &dyn StorageClient,
    request: IngestRequest,
    expected_token: &str,
) -> Result<IngestResponse, AppError> {
    // 1. Validate the service token
    if request.service_token != expected_token {
        return Err(AppError::Auth("Invalid service token".into()));
    }

    // 2. Validate the slug (must not be empty, must be URL-safe)
    if request.slug.is_empty() {
        return Err(AppError::BadRequest("Slug cannot be empty".into()));
    }

    // 3. Parse the access level
    let access_level = AccessLevel::from_str_ci(&request.access_level).ok_or_else(|| {
        AppError::BadRequest(format!(
            "Invalid access level '{}'. Expected: public, developer, architect, admin",
            request.access_level
        ))
    })?;

    // 4. Build the S3 key
    let s3_key = format!("docs/{}.md", request.slug.replace('/', "_"));

    // 5. Upload content to S3
    storage
        .put_object(&s3_key, request.content.into_bytes())
        .await?;

    // 6. Upsert document metadata in MongoDB
    let doc = Document {
        slug: request.slug.clone(),
        title: request.title,
        s3_key: s3_key.clone(),
        access_level,
        service_owner: request.service_owner,
        last_updated: Utc::now(),
        tags: request.tags,
        links_out: vec![],
        backlinks: vec![],
    };

    repo.create_or_update(doc).await?;

    Ok(IngestResponse {
        message: "Document ingested successfully".to_string(),
        slug: request.slug,
        s3_key,
    })
}

/// Axum handler for `POST /api/v1/ingest`.
///
/// Only available when the `ssr` feature is enabled.
#[cfg(feature = "ssr")]
pub async fn ingest_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<IngestRequest>,
) -> Result<axum::Json<IngestResponse>, AppError> {
    let response = process_ingest(
        state.document_repo.as_ref(),
        state.storage_client.as_ref(),
        request,
        &state.service_token,
    )
    .await?;

    Ok(axum::Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    // -- Mock implementations --

    struct MockStorage {
        objects: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                objects: Mutex::new(std::collections::HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl StorageClient for MockStorage {
        async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), AppError> {
            self.objects
                .lock()
                .unwrap()
                .insert(key.to_string(), content);
            Ok(())
        }

        async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, AppError> {
            Ok(self.objects.lock().unwrap().get(key).cloned())
        }
    }

    struct MockRepo {
        documents: Mutex<Vec<Document>>,
    }

    impl MockRepo {
        fn new() -> Self {
            Self {
                documents: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl DocumentRepository for MockRepo {
        async fn create_or_update(&self, doc: Document) -> Result<(), AppError> {
            let mut docs = self.documents.lock().unwrap();
            docs.retain(|d| d.slug != doc.slug);
            docs.push(doc);
            Ok(())
        }

        async fn find_by_slug(&self, slug: &str) -> Result<Option<Document>, AppError> {
            Ok(self
                .documents
                .lock()
                .unwrap()
                .iter()
                .find(|d| d.slug == slug)
                .cloned())
        }

        async fn list_accessible(
            &self,
            max_level: AccessLevel,
        ) -> Result<Vec<Document>, AppError> {
            Ok(self
                .documents
                .lock()
                .unwrap()
                .iter()
                .filter(|d| d.access_level <= max_level)
                .cloned()
                .collect())
        }
    }

    fn make_request(token: &str, slug: &str) -> IngestRequest {
        IngestRequest {
            service_token: token.to_string(),
            slug: slug.to_string(),
            title: "Test Doc".to_string(),
            content: "# Hello\nWorld".to_string(),
            access_level: "developer".to_string(),
            service_owner: "test-team".to_string(),
            tags: vec!["test".to_string()],
        }
    }

    #[tokio::test]
    async fn test_ingest_success() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let request = make_request("valid-token", "docs/hello");

        let result = process_ingest(&repo, &storage, request, "valid-token").await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response.slug, "docs/hello");
        assert!(response.s3_key.contains("docs_hello"));

        // Verify content was stored
        let stored = storage
            .objects
            .lock()
            .unwrap()
            .get(&response.s3_key)
            .cloned();
        assert!(stored.is_some());
        assert_eq!(
            String::from_utf8(stored.unwrap()).unwrap(),
            "# Hello\nWorld"
        );

        // Verify metadata was saved
        let doc = repo.find_by_slug("docs/hello").await.unwrap();
        assert!(doc.is_some());
        let doc = doc.unwrap();
        assert_eq!(doc.title, "Test Doc");
        assert_eq!(doc.access_level, AccessLevel::Developer);
    }

    #[tokio::test]
    async fn test_ingest_invalid_token() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let request = make_request("wrong-token", "docs/hello");

        let result = process_ingest(&repo, &storage, request, "valid-token").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("Invalid service token")),
            other => panic!("Expected Auth error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_empty_slug() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let request = make_request("valid-token", "");

        let result = process_ingest(&repo, &storage, request, "valid-token").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("Slug cannot be empty")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_invalid_access_level() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let mut request = make_request("valid-token", "docs/hello");
        request.access_level = "superadmin".to_string();

        let result = process_ingest(&repo, &storage, request, "valid-token").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("Invalid access level")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_upsert() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();

        // First ingest
        let request1 = make_request("valid-token", "docs/hello");
        process_ingest(&repo, &storage, request1, "valid-token")
            .await
            .unwrap();

        // Second ingest (update)
        let mut request2 = make_request("valid-token", "docs/hello");
        request2.title = "Updated Doc".to_string();
        process_ingest(&repo, &storage, request2, "valid-token")
            .await
            .unwrap();

        // Should have only one document
        let docs = repo.list_accessible(AccessLevel::Admin).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "Updated Doc");
    }
}
