use serde::{Deserialize, Serialize};

use crate::error::AppError;
#[cfg(feature = "ssr")]
use crate::search::client::SearchService;

/// A single document entry in a sync request.
#[derive(Debug, Clone, Deserialize)]
pub struct SyncDocumentEntry {
    pub slug: String,
    pub content_hash: String,
    /// Hash of front-matter metadata (title, access_level, …).
    /// When present, the sync endpoint uses it alongside `content_hash` to
    /// detect metadata-only changes.  Absent in requests from older CLI
    /// versions — treated as "metadata unchanged" for backwards compatibility.
    #[serde(default)]
    pub metadata_hash: Option<String>,
}

/// Request payload for `POST /api/v1/sync`.
#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    /// Service authentication token (legacy or scoped).
    pub service_token: String,
    /// The client's complete list of documents (slug + content hash).
    pub documents: Vec<SyncDocumentEntry>,
    /// If `true`, documents in the server scope that are missing from the
    /// client list will be automatically archived.
    #[serde(default)]
    pub archive_missing: bool,
}

/// Response from a sync operation.
#[derive(Debug, Serialize)]
pub struct SyncResponse {
    /// Slugs the client should upload (new or changed content).
    pub to_upload: Vec<String>,
    /// Slugs that were (or should be) archived (present on server, missing from client).
    pub to_archive: Vec<String>,
    /// Slugs with matching content hash (no action needed).
    pub unchanged: Vec<String>,
}

/// Core sync logic — separated from the HTTP layer for testability.
#[cfg(feature = "ssr")]
pub async fn process_sync(
    repo: &dyn crate::db::repository::DocumentRepository,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    search: Option<&dyn SearchService>,
    legacy_token: Option<&str>,
    request: SyncRequest,
) -> Result<SyncResponse, AppError> {
    use std::collections::HashMap;

    // 1. Validate the service token and determine scopes
    let scopes = validate_sync_token(
        service_token_repo,
        legacy_token,
        &request.service_token,
    )
    .await?;

    // 2. Validate all request slugs fall within the token's scopes
    for entry in &request.documents {
        if !slug_matches_scopes(&entry.slug, &scopes) {
            return Err(AppError::Forbidden(format!(
                "Token does not have access to slug '{}'",
                entry.slug
            )));
        }
    }

    // 3. Fetch all server documents within the token's scopes
    // Value: (content_hash, metadata_hash)
    let mut server_docs: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for scope in &scopes {
        if scope == "*" {
            // Wildcard (legacy token): fetch all non-archived documents
            let docs = repo.find_by_slug_prefix("").await?;
            for doc in docs {
                server_docs.insert(doc.slug, (doc.content_hash, doc.metadata_hash));
            }
        } else if let Some(prefix) = scope.strip_suffix("/*") {
            // Prefix scope
            let docs = repo.find_by_slug_prefix(prefix).await?;
            for doc in docs {
                server_docs.insert(doc.slug, (doc.content_hash, doc.metadata_hash));
            }
        } else {
            // Exact scope — fetch by slug directly
            if let Some(doc) = repo.find_by_slug(scope).await? {
                if !doc.is_archived {
                    server_docs.insert(doc.slug, (doc.content_hash, doc.metadata_hash));
                }
            }
        }
    }

    // 4. Build client lookup
    let client_docs: HashMap<&str, &str> = request
        .documents
        .iter()
        .map(|e| (e.slug.as_str(), e.content_hash.as_str()))
        .collect();

    // 5. Compare
    let mut to_upload = Vec::new();
    let mut unchanged = Vec::new();
    let mut to_archive = Vec::new();

    // Check client docs against server
    for entry in &request.documents {
        match server_docs.get(&entry.slug) {
            Some((server_content_hash, server_metadata_hash)) => {
                let content_ok = server_content_hash.as_deref() == Some(entry.content_hash.as_str());
                // Metadata comparison: only enforced when both sides provide a hash.
                // If the client sends a metadata_hash but the server has None (old
                // document), we treat it as changed so the metadata_hash gets stored.
                let metadata_ok = match (entry.metadata_hash.as_deref(), server_metadata_hash.as_deref()) {
                    (Some(c), Some(s)) => c == s,
                    (Some(_), None) => false, // server has no metadata hash yet → upload to populate it
                    (None, _) => true,         // old CLI without metadata_hash → assume unchanged
                };
                if content_ok && metadata_ok {
                    unchanged.push(entry.slug.clone());
                } else {
                    to_upload.push(entry.slug.clone());
                }
            }
            _ => {
                // Missing from server, or server has no content hash
                to_upload.push(entry.slug.clone());
            }
        }
    }

    // Check server docs not in client list
    for slug in server_docs.keys() {
        if !client_docs.contains_key(slug.as_str()) {
            to_archive.push(slug.clone());
        }
    }

    // 6. Archive missing docs if requested
    if request.archive_missing {
        for slug in &to_archive {
            repo.set_archived(slug, true).await?;
            if let Some(svc) = search {
                if let Err(e) = svc.delete_document(slug).await {
                    tracing::warn!("Failed to deindex archived document '{slug}' from search: {e}");
                }
            }
        }
    }

    // Sort for deterministic output
    to_upload.sort();
    to_archive.sort();
    unchanged.sort();

    Ok(SyncResponse {
        to_upload,
        to_archive,
        unchanged,
    })
}

/// Validate the token for sync and return its scopes.
/// Legacy token gets a wildcard scope ("*").
#[cfg(feature = "ssr")]
async fn validate_sync_token(
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    raw_token: &str,
) -> Result<Vec<String>, AppError> {
    // Legacy token bypass — full access
    if let Some(legacy) = legacy_token {
        if !legacy.is_empty() && raw_token == legacy {
            return Ok(vec!["*".to_string()]);
        }
    }

    // Look up scoped token
    let token_hash = crate::auth::token_service::TokenService::hash_token(raw_token);
    let token = service_token_repo
        .find_by_hash(&token_hash)
        .await?
        .ok_or_else(|| AppError::Auth("Invalid service token".into()))?;

    if !token.is_active {
        return Err(AppError::Auth("Service token is deactivated".into()));
    }

    // Touch last_used (fire-and-forget)
    if let Err(e) = service_token_repo.touch_last_used(&token.id).await {
        tracing::warn!("Failed to update last_used_at for token {}: {e}", token.id);
    }

    Ok(token.allowed_scopes)
}

/// Check if a slug matches any of the given scopes.
/// The wildcard scope "*" matches everything.
#[cfg(feature = "ssr")]
fn slug_matches_scopes(slug: &str, scopes: &[String]) -> bool {
    scopes.iter().any(|scope| {
        if scope == "*" {
            return true;
        }
        if let Some(prefix) = scope.strip_suffix("/*") {
            slug == prefix || slug.starts_with(&format!("{prefix}/"))
        } else {
            scope == slug
        }
    })
}

/// Axum handler for `POST /api/v1/sync`.
#[cfg(feature = "ssr")]
pub async fn sync_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<SyncRequest>,
) -> Result<axum::Json<SyncResponse>, AppError> {
    let response = process_sync(
        state.document_repo.as_ref(),
        state.service_token_repo.as_ref(),
        state.search_service.as_deref(),
        Some(&state.service_token),
        request,
    )
    .await?;
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
    use chrono::Utc;

    // ── Mocks ────────────────────────────────────────────────────────────

    struct MockSearchService {
        deleted: Mutex<Vec<String>>,
    }

    impl MockSearchService {
        fn new() -> Self {
            Self { deleted: Mutex::new(vec![]) }
        }

        fn deleted_slugs(&self) -> Vec<String> {
            self.deleted.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl crate::search::client::SearchService for MockSearchService {
        async fn index_document(&self, _: &crate::search::client::SearchDocument) -> Result<(), AppError> { Ok(()) }
        async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
            self.deleted.lock().unwrap().push(slug.to_string());
            Ok(())
        }
        async fn search(&self, _: &str, _: Option<&[String]>, _: bool) -> Result<Vec<crate::search::client::SearchHit>, AppError> { Ok(vec![]) }
        async fn configure_index(&self) -> Result<(), AppError> { Ok(()) }
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

        fn with_docs(docs: Vec<Document>) -> Self {
            Self {
                documents: Mutex::new(docs),
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
            Ok(self.documents.lock().unwrap().iter().find(|d| d.slug == slug).cloned())
        }
        async fn list_by_access_levels(&self, _: Option<&[String]>, _: bool) -> Result<Vec<Document>, AppError> {
            Ok(self.documents.lock().unwrap().clone())
        }
        async fn update_backlinks(&self, _: &str, _: &[String], _: &[String]) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_slug_prefix(&self, prefix: &str) -> Result<Vec<Document>, AppError> {
            Ok(self
                .documents
                .lock()
                .unwrap()
                .iter()
                .filter(|d| {
                    !d.is_archived
                        && (prefix.is_empty()
                            || d.slug == prefix
                            || d.slug.starts_with(&format!("{prefix}/")))
                })
                .cloned()
                .collect())
        }
        async fn set_archived(&self, slug: &str, archived: bool) -> Result<(), AppError> {
            let mut docs = self.documents.lock().unwrap();
            if let Some(doc) = docs.iter_mut().find(|d| d.slug == slug) {
                doc.is_archived = archived;
            }
            Ok(())
        }
    }

    struct MockServiceTokenRepo;

    #[async_trait]
    impl ServiceTokenRepository for MockServiceTokenRepo {
        async fn create(&self, _: ServiceToken) -> Result<(), AppError> { Ok(()) }
        async fn find_by_hash(&self, _: &str) -> Result<Option<ServiceToken>, AppError> { Ok(None) }
        async fn find_by_name(&self, _: &str) -> Result<Option<ServiceToken>, AppError> { Ok(None) }
        async fn find_by_id(&self, _: &str) -> Result<Option<ServiceToken>, AppError> { Ok(None) }
        async fn list_all(&self) -> Result<Vec<ServiceToken>, AppError> { Ok(vec![]) }
        async fn deactivate(&self, _: &str) -> Result<(), AppError> { Ok(()) }
        async fn touch_last_used(&self, _: &str) -> Result<(), AppError> { Ok(()) }
        async fn check_scope_overlap(&self, _: &[String], _: Option<&str>) -> Result<bool, AppError> { Ok(false) }
    }

    fn make_doc(slug: &str, hash: &str) -> Document {
        Document {
            slug: slug.to_string(),
            title: slug.to_string(),
            s3_key: format!("docs/{}.md", slug.replace('/', "_")),
            access_level: "internal".to_string(),
            is_draft: false,
            service_owner: "test".to_string(),
            last_updated: Utc::now(),
            tags: vec![],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            content_hash: Some(hash.to_string()),
            metadata_hash: None,
            is_archived: false,
        }
    }

    // ── Tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_sync_identifies_uploads_for_new_docs() {
        let repo = MockRepo::new(); // empty server
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/new".to_string(),
                content_hash: "sha256:abc".to_string(),
                metadata_hash: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(result.to_upload, vec!["docs/new"]);
        assert!(result.unchanged.is_empty());
        assert!(result.to_archive.is_empty());
    }

    #[tokio::test]
    async fn test_sync_identifies_unchanged() {
        let repo = MockRepo::with_docs(vec![make_doc("docs/a", "sha256:abc")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:abc".to_string(),
                metadata_hash: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert!(result.to_upload.is_empty());
        assert_eq!(result.unchanged, vec!["docs/a"]);
        assert!(result.to_archive.is_empty());
    }

    #[tokio::test]
    async fn test_sync_identifies_changed_hash() {
        let repo = MockRepo::with_docs(vec![make_doc("docs/a", "sha256:old")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:new".to_string(),
                metadata_hash: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(result.to_upload, vec!["docs/a"]);
        assert!(result.unchanged.is_empty());
    }

    #[tokio::test]
    async fn test_sync_identifies_archives() {
        let repo = MockRepo::with_docs(vec![
            make_doc("docs/a", "sha256:abc"),
            make_doc("docs/old", "sha256:def"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:abc".to_string(),
                metadata_hash: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(result.unchanged, vec!["docs/a"]);
        assert_eq!(result.to_archive, vec!["docs/old"]);
    }

    #[tokio::test]
    async fn test_sync_archive_missing_sets_flag() {
        let repo = MockRepo::with_docs(vec![
            make_doc("docs/a", "sha256:abc"),
            make_doc("docs/old", "sha256:def"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:abc".to_string(),
                metadata_hash: None,
            }],
            archive_missing: true,
        };

        process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();

        // Verify the old doc is now archived
        let doc = repo.find_by_slug("docs/old").await.unwrap().unwrap();
        assert!(doc.is_archived);
        // And the kept doc is not
        let doc = repo.find_by_slug("docs/a").await.unwrap().unwrap();
        assert!(!doc.is_archived);
    }

    #[tokio::test]
    async fn test_sync_scope_validation() {
        // Use a scoped token that only has access to "protocols/*"
        let scoped = ServiceToken {
            id: "st-1".to_string(),
            name: "test".to_string(),
            token_hash: crate::auth::token_service::TokenService::hash_token("scoped-tok"),
            allowed_scopes: vec!["protocols/*".to_string()],
            can_write: true,
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            is_active: true,
        };

        struct ScopedTokenRepo(ServiceToken);
        #[async_trait]
        impl ServiceTokenRepository for ScopedTokenRepo {
            async fn create(&self, _: ServiceToken) -> Result<(), AppError> { Ok(()) }
            async fn find_by_hash(&self, hash: &str) -> Result<Option<ServiceToken>, AppError> {
                if hash == self.0.token_hash { Ok(Some(self.0.clone())) } else { Ok(None) }
            }
            async fn find_by_name(&self, _: &str) -> Result<Option<ServiceToken>, AppError> { Ok(None) }
            async fn find_by_id(&self, _: &str) -> Result<Option<ServiceToken>, AppError> { Ok(None) }
            async fn list_all(&self) -> Result<Vec<ServiceToken>, AppError> { Ok(vec![]) }
            async fn deactivate(&self, _: &str) -> Result<(), AppError> { Ok(()) }
            async fn touch_last_used(&self, _: &str) -> Result<(), AppError> { Ok(()) }
            async fn check_scope_overlap(&self, _: &[String], _: Option<&str>) -> Result<bool, AppError> { Ok(false) }
        }

        let repo = MockRepo::new();
        let token_repo = ScopedTokenRepo(scoped);

        // Request with a slug outside the token's scope
        let request = SyncRequest {
            service_token: "scoped-tok".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/outside".to_string(),
                content_hash: "sha256:abc".to_string(),
                metadata_hash: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("other-legacy"), request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Forbidden(msg) => assert!(msg.contains("docs/outside")),
            other => panic!("Expected Forbidden, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_sync_archive_deindexes_from_search() {
        let repo = MockRepo::with_docs(vec![
            make_doc("docs/a", "sha256:abc"),
            make_doc("docs/old", "sha256:def"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let search = MockSearchService::new();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:abc".to_string(),
                metadata_hash: None,
            }],
            archive_missing: true,
        };

        process_sync(&repo, &token_repo, Some(&search), Some("legacy"), request)
            .await
            .unwrap();

        assert_eq!(search.deleted_slugs(), vec!["docs/old"]);
    }

    #[tokio::test]
    async fn test_sync_no_archive_does_not_deindex() {
        let repo = MockRepo::with_docs(vec![
            make_doc("docs/a", "sha256:abc"),
            make_doc("docs/old", "sha256:def"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let search = MockSearchService::new();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:abc".to_string(),
                metadata_hash: None,
            }],
            archive_missing: false,
        };

        process_sync(&repo, &token_repo, Some(&search), Some("legacy"), request)
            .await
            .unwrap();

        assert!(search.deleted_slugs().is_empty());
    }

    // ── Metadata hash tests ──────────────────────────────────────────────

    fn make_doc_with_meta(slug: &str, content_hash: &str, metadata_hash: &str) -> Document {
        let mut doc = make_doc(slug, content_hash);
        doc.metadata_hash = Some(metadata_hash.to_string());
        doc
    }

    #[tokio::test]
    async fn test_sync_metadata_hash_match_is_unchanged() {
        let repo = MockRepo::with_docs(vec![make_doc_with_meta("docs/a", "sha256:content", "sha256:meta")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:meta".to_string()),
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert!(result.to_upload.is_empty(), "should be unchanged when both hashes match");
        assert_eq!(result.unchanged, vec!["docs/a"]);
    }

    #[tokio::test]
    async fn test_sync_metadata_hash_mismatch_triggers_upload() {
        // content hash matches, but metadata (e.g. access_level) changed
        let repo = MockRepo::with_docs(vec![make_doc_with_meta("docs/a", "sha256:content", "sha256:old-meta")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:new-meta".to_string()),
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(result.to_upload, vec!["docs/a"], "should upload when metadata hash differs");
        assert!(result.unchanged.is_empty());
    }

    #[tokio::test]
    async fn test_sync_metadata_hash_absent_on_server_triggers_upload() {
        // Server has no metadata_hash (old document) → force upload to populate it
        let repo = MockRepo::with_docs(vec![make_doc("docs/a", "sha256:content")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:meta".to_string()),
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(result.to_upload, vec!["docs/a"], "should upload when server has no metadata_hash");
    }

    #[tokio::test]
    async fn test_sync_no_metadata_hash_from_client_is_backwards_compat() {
        // Old CLI without metadata_hash → treat as unchanged if content matches
        let repo = MockRepo::with_docs(vec![make_doc_with_meta("docs/a", "sha256:content", "sha256:meta")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert!(result.to_upload.is_empty(), "old CLI without metadata_hash should be treated as unchanged");
        assert_eq!(result.unchanged, vec!["docs/a"]);
    }
}
