use chrono::Utc;

use crate::auth::models::AccessLevel;
use crate::db::models::{Document, IngestRequest, IngestResponse};
use crate::db::repository::DocumentRepository;
use crate::error::AppError;
use crate::rendering::links::extract_internal_links;
use crate::search::client::SearchService;
use crate::storage::client::StorageClient;

/// Core ingestion logic — separated from the HTTP layer for testability.
///
/// Validates the request, uploads content to S3, upserts metadata in MongoDB,
/// and optionally indexes the document in Meilisearch.
pub async fn process_ingest(
    repo: &dyn DocumentRepository,
    storage: &dyn StorageClient,
    search: Option<&dyn SearchService>,
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

    // 4. Extract internal links from content
    let links_out = extract_internal_links(&request.content);

    // 5. Get old document to diff backlinks
    let old_doc = repo.find_by_slug(&request.slug).await?;
    let old_links = old_doc
        .as_ref()
        .map(|d| d.links_out.clone())
        .unwrap_or_default();

    // 6. Build the S3 key
    let s3_key = format!("docs/{}.md", request.slug.replace('/', "_"));

    // Keep raw content for search indexing before uploading
    let raw_content = request.content.clone();

    // 7. Upload content to S3
    storage
        .put_object(&s3_key, request.content.into_bytes())
        .await?;

    // 8. Upsert document metadata in MongoDB
    let doc = Document {
        slug: request.slug.clone(),
        title: request.title,
        s3_key: s3_key.clone(),
        access_level,
        service_owner: request.service_owner,
        last_updated: Utc::now(),
        tags: request.tags.clone(),
        links_out: links_out.clone(),
        backlinks: old_doc.as_ref().map(|d| d.backlinks.clone()).unwrap_or_default(),
        // Use request values if provided, otherwise preserve from old doc or use defaults
        parent_slug: if request.parent_slug.is_some() {
            request.parent_slug.clone()
        } else {
            old_doc.as_ref().and_then(|d| d.parent_slug.clone())
        },
        order: if request.order > 0 {
            request.order
        } else {
            old_doc.as_ref().map(|d| d.order).unwrap_or(0)
        },
        is_hidden: if request.is_hidden {
            true
        } else {
            old_doc.as_ref().map(|d| d.is_hidden).unwrap_or(false)
        },
    };

    // 9. Build search document before ownership transfer
    let search_doc = search
        .as_ref()
        .map(|_| crate::search::client::build_search_document(&doc, &raw_content));

    repo.create_or_update(doc).await?;

    // 10. Update backlinks on referenced documents
    repo.update_backlinks(&request.slug, &old_links, &links_out)
        .await?;

    // 11. Index in Meilisearch (if available)
    if let (Some(search_svc), Some(search_doc)) = (search, search_doc) {
        if let Err(e) = search_svc.index_document(&search_doc).await {
            tracing::warn!("Failed to index document in search: {e}");
        }
    }

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
    let search = state
        .search_service
        .as_deref();

    let response = process_ingest(
        state.document_repo.as_ref(),
        state.storage_client.as_ref(),
        search,
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

        async fn update_backlinks(
            &self,
            source_slug: &str,
            old_links: &[String],
            new_links: &[String],
        ) -> Result<(), AppError> {
            let mut docs = self.documents.lock().unwrap();

            // Remove source from backlinks of old targets no longer linked
            let removed: Vec<String> = old_links
                .iter()
                .filter(|l| !new_links.contains(l))
                .cloned()
                .collect();

            for doc in docs.iter_mut() {
                if removed.contains(&doc.slug) {
                    doc.backlinks.retain(|b| b != source_slug);
                }
            }

            // Add source to backlinks of new targets
            let added: Vec<String> = new_links
                .iter()
                .filter(|l| !old_links.contains(l))
                .cloned()
                .collect();

            for doc in docs.iter_mut() {
                if added.contains(&doc.slug) && !doc.backlinks.contains(&source_slug.to_string()) {
                    doc.backlinks.push(source_slug.to_string());
                }
            }

            Ok(())
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
            parent_slug: None,
            order: 0,
            is_hidden: false,
        }
    }

    #[tokio::test]
    async fn test_ingest_success() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let request = make_request("valid-token", "docs/hello");

        let result = process_ingest(&repo, &storage, None, request, "valid-token").await;
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

        let result = process_ingest(&repo, &storage, None, request, "valid-token").await;
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

        let result = process_ingest(&repo, &storage, None, request, "valid-token").await;
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

        let result = process_ingest(&repo, &storage, None, request, "valid-token").await;
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
        process_ingest(&repo, &storage, None, request1, "valid-token")
            .await
            .unwrap();

        // Second ingest (update)
        let mut request2 = make_request("valid-token", "docs/hello");
        request2.title = "Updated Doc".to_string();
        process_ingest(&repo, &storage, None, request2, "valid-token")
            .await
            .unwrap();

        // Should have only one document
        let docs = repo.list_accessible(AccessLevel::Admin).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "Updated Doc");
    }
}
