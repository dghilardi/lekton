use serde::{Deserialize, Serialize};

use crate::error::AppError;
#[cfg(feature = "ssr")]
use crate::search::client::SearchService;

/// A single document entry in a sync request.
#[derive(Debug, Clone, Deserialize)]
pub struct SyncDocumentEntry {
    /// Relative path of the source file within the repository (e.g. `docs/guides/intro.md`).
    /// Used as the stable document identity for migration lookup.
    pub source_path: String,
    /// Desired slug for the document (title-derived or explicit from front matter).
    pub slug: String,
    pub content_hash: String,
    /// Hash of front-matter metadata (title, access_level, …).
    #[serde(default)]
    pub metadata_hash: Option<String>,
    /// Path-derived slug from the old CLI (e.g. `docs/guides/intro`). Sent when
    /// `slug` differs from the path-based derivation so the server can locate
    /// documents that were indexed before `source_path` was introduced.
    #[serde(default)]
    pub legacy_slug: Option<String>,
}

/// Request payload for `POST /api/v1/sync`.
#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    /// Service authentication token (legacy or scoped).
    pub service_token: String,
    /// The client's complete list of documents.
    pub documents: Vec<SyncDocumentEntry>,
    /// If `true`, documents in the server scope that are missing from the
    /// client list will be automatically archived.
    #[serde(default)]
    pub archive_missing: bool,
}

/// A single entry in the `to_upload` list returned by the sync endpoint.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SyncUploadEntry {
    /// Source file path (echoed from the request).
    pub source_path: String,
    /// The slug the client MUST use when calling the ingest endpoint.
    ///
    /// May differ from the requested slug when the server resolves a
    /// migration case (document already stored under a legacy path-based slug).
    pub actual_slug: String,
}

/// Response from a sync operation.
#[derive(Debug, Serialize)]
pub struct SyncResponse {
    /// Documents the client should upload (new or changed).
    pub to_upload: Vec<SyncUploadEntry>,
    /// Slugs that were (or should be) archived (present on server, missing from client).
    pub to_archive: Vec<String>,
    /// Source paths with no pending changes.
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

    // (content_hash, metadata_hash, source_path)
    type ServerDocInfo = (Option<String>, Option<String>, Option<String>);

    // 1. Validate the service token and determine scopes
    let scopes =
        validate_sync_token(service_token_repo, legacy_token, &request.service_token).await?;

    // 2. Validate all request slugs fall within the token's scopes
    for entry in &request.documents {
        if !scope_matches_any(&entry.slug, &scopes) {
            return Err(AppError::Forbidden(format!(
                "Token does not have access to slug '{}'",
                entry.slug
            )));
        }
    }

    // 3. Fetch all server documents within the token's scopes.
    // Value: (content_hash, metadata_hash, source_path)
    let mut server_by_slug: HashMap<String, ServerDocInfo> = HashMap::new();
    // source_path → slug index for migration lookup
    let mut server_by_source_path: HashMap<String, String> = HashMap::new();

    for scope in &scopes {
        let docs = if scope == "*" {
            repo.find_by_slug_prefix("").await?
        } else if let Some(prefix) = scope.strip_suffix("/*") {
            repo.find_by_slug_prefix(prefix).await?
        } else {
            match repo.find_by_slug(scope).await? {
                Some(doc) if !doc.is_archived => vec![doc],
                _ => vec![],
            }
        };

        for doc in docs {
            if let Some(ref sp) = doc.source_path {
                server_by_source_path.insert(sp.clone(), doc.slug.clone());
            }
            server_by_slug.insert(
                doc.slug.clone(),
                (doc.content_hash, doc.metadata_hash, doc.source_path),
            );
        }
    }

    // 4. Compare — resolve actual_slug for each client entry
    let mut to_upload: Vec<SyncUploadEntry> = Vec::new();
    let mut unchanged = Vec::new();
    let mut to_archive = Vec::new();

    // Slugs in the server that have been "claimed" by a source_path match,
    // so they are excluded from the archive check.
    let mut claimed_slugs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in &request.documents {
        // Resolve the canonical slug using the priority chain:
        // 1. Lookup by source_path (stable identity after first sync)
        // 2. Lookup by desired slug (exact match or first sync of an unchanged doc)
        // 3. Lookup by legacy_slug (migration: doc was indexed with old path-based slug)
        // 4. New document — use the desired slug
        let actual_slug = if let Some(existing_slug) = server_by_source_path.get(&entry.source_path)
        {
            existing_slug.clone()
        } else if server_by_slug.contains_key(&entry.slug) {
            entry.slug.clone()
        } else if let Some(ref legacy) = entry.legacy_slug {
            if server_by_slug.contains_key(legacy.as_str()) {
                legacy.clone()
            } else {
                entry.slug.clone()
            }
        } else {
            entry.slug.clone()
        };

        claimed_slugs.insert(actual_slug.clone());

        match server_by_slug.get(&actual_slug) {
            Some((server_content_hash, server_metadata_hash, server_source_path)) => {
                let content_ok =
                    server_content_hash.as_deref() == Some(entry.content_hash.as_str());
                let metadata_ok = match (
                    entry.metadata_hash.as_deref(),
                    server_metadata_hash.as_deref(),
                ) {
                    (Some(c), Some(s)) => c == s,
                    (Some(_), None) => false,
                    (None, _) => true,
                };
                // Force upload when source_path is not yet stored on the server,
                // so the migration populates it in a single pass.
                let source_path_ok =
                    server_source_path.as_deref() == Some(entry.source_path.as_str());

                if content_ok && metadata_ok && source_path_ok {
                    unchanged.push(entry.source_path.clone());
                } else {
                    to_upload.push(SyncUploadEntry {
                        source_path: entry.source_path.clone(),
                        actual_slug,
                    });
                }
            }
            None => {
                to_upload.push(SyncUploadEntry {
                    source_path: entry.source_path.clone(),
                    actual_slug,
                });
            }
        }
    }

    // Server docs not claimed by any client entry are candidates for archiving.
    for slug in server_by_slug.keys() {
        if !claimed_slugs.contains(slug.as_str()) {
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
    to_upload.sort_by(|a, b| a.source_path.cmp(&b.source_path));
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
pub(crate) async fn validate_sync_token(
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
pub(crate) fn scope_matches_any(slug: &str, scopes: &[String]) -> bool {
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
            Self {
                deleted: Mutex::new(vec![]),
            }
        }

        fn deleted_slugs(&self) -> Vec<String> {
            self.deleted.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl crate::search::client::SearchService for MockSearchService {
        async fn index_document(
            &self,
            _: &crate::search::client::SearchDocument,
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
            self.deleted.lock().unwrap().push(slug.to_string());
            Ok(())
        }
        async fn search(
            &self,
            _: &str,
            _: Option<&[String]>,
            _: bool,
        ) -> Result<Vec<crate::search::client::SearchHit>, AppError> {
            Ok(vec![])
        }
        async fn configure_index(&self) -> Result<(), AppError> {
            Ok(())
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
            Ok(self
                .documents
                .lock()
                .unwrap()
                .iter()
                .find(|d| d.slug == slug)
                .cloned())
        }
        async fn list_all(&self) -> Result<Vec<Document>, AppError> {
            Ok(self.documents.lock().unwrap().clone())
        }
        async fn list_by_access_levels(
            &self,
            _: Option<&[String]>,
            _: bool,
        ) -> Result<Vec<Document>, AppError> {
            Ok(self.documents.lock().unwrap().clone())
        }
        async fn update_backlinks(
            &self,
            _: &str,
            _: &[String],
            _: &[String],
        ) -> Result<(), AppError> {
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
        async fn find_by_source_path(
            &self,
            source_path: &str,
        ) -> Result<Option<Document>, AppError> {
            Ok(self
                .documents
                .lock()
                .unwrap()
                .iter()
                .find(|d| d.source_path.as_deref() == Some(source_path))
                .cloned())
        }
    }

    struct MockServiceTokenRepo;

    #[async_trait]
    impl ServiceTokenRepository for MockServiceTokenRepo {
        async fn create(&self, _: ServiceToken) -> Result<(), AppError> {
            Ok(())
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
            source_path: Some(format!("{slug}.md")),
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn entry(slug: &str, content_hash: &str) -> SyncDocumentEntry {
        SyncDocumentEntry {
            source_path: format!("{slug}.md"),
            slug: slug.to_string(),
            content_hash: content_hash.to_string(),
            metadata_hash: None,
            legacy_slug: None,
        }
    }

    fn upload(source_path: &str, actual_slug: &str) -> SyncUploadEntry {
        SyncUploadEntry {
            source_path: source_path.to_string(),
            actual_slug: actual_slug.to_string(),
        }
    }

    // ── Tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_sync_identifies_uploads_for_new_docs() {
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![entry("docs/new", "sha256:abc")],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(result.to_upload, vec![upload("docs/new.md", "docs/new")]);
        assert!(result.unchanged.is_empty());
        assert!(result.to_archive.is_empty());
    }

    #[tokio::test]
    async fn test_sync_identifies_unchanged() {
        let repo = MockRepo::with_docs(vec![make_doc("docs/a", "sha256:abc")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert!(result.to_upload.is_empty());
        assert_eq!(result.unchanged, vec!["docs/a.md"]);
        assert!(result.to_archive.is_empty());
    }

    #[tokio::test]
    async fn test_sync_identifies_changed_hash() {
        let repo = MockRepo::with_docs(vec![make_doc("docs/a", "sha256:old")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![entry("docs/a", "sha256:new")],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(result.to_upload, vec![upload("docs/a.md", "docs/a")]);
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
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(result.unchanged, vec!["docs/a.md"]);
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
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: true,
        };

        process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();

        let doc = repo.find_by_slug("docs/old").await.unwrap().unwrap();
        assert!(doc.is_archived);
        let doc = repo.find_by_slug("docs/a").await.unwrap().unwrap();
        assert!(!doc.is_archived);
    }

    #[tokio::test]
    async fn test_sync_scope_validation() {
        let scoped = ServiceToken {
            id: "st-1".to_string(),
            name: "test".to_string(),
            token_hash: crate::auth::token_service::TokenService::hash_token("scoped-tok"),
            allowed_scopes: vec!["protocols/*".to_string()],
            token_type: "service".to_string(),
            user_id: None,
            can_write: true,
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            is_active: true,
        };

        struct ScopedTokenRepo(ServiceToken);
        #[async_trait]
        impl ServiceTokenRepository for ScopedTokenRepo {
            async fn create(&self, _: ServiceToken) -> Result<(), AppError> {
                Ok(())
            }
            async fn find_by_hash(&self, hash: &str) -> Result<Option<ServiceToken>, AppError> {
                if hash == self.0.token_hash {
                    Ok(Some(self.0.clone()))
                } else {
                    Ok(None)
                }
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

        let repo = MockRepo::new();
        let token_repo = ScopedTokenRepo(scoped);
        let request = SyncRequest {
            service_token: "scoped-tok".to_string(),
            documents: vec![entry("docs/outside", "sha256:abc")],
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
            documents: vec![entry("docs/a", "sha256:abc")],
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
            documents: vec![entry("docs/a", "sha256:abc")],
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
        let repo = MockRepo::with_docs(vec![make_doc_with_meta(
            "docs/a",
            "sha256:content",
            "sha256:meta",
        )]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/a.md".to_string(),
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:meta".to_string()),
                legacy_slug: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert!(
            result.to_upload.is_empty(),
            "should be unchanged when both hashes match"
        );
        assert_eq!(result.unchanged, vec!["docs/a.md"]);
    }

    #[tokio::test]
    async fn test_sync_metadata_hash_mismatch_triggers_upload() {
        let repo = MockRepo::with_docs(vec![make_doc_with_meta(
            "docs/a",
            "sha256:content",
            "sha256:old-meta",
        )]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/a.md".to_string(),
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:new-meta".to_string()),
                legacy_slug: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(
            result.to_upload,
            vec![upload("docs/a.md", "docs/a")],
            "should upload when metadata hash differs"
        );
        assert!(result.unchanged.is_empty());
    }

    #[tokio::test]
    async fn test_sync_metadata_hash_absent_on_server_triggers_upload() {
        let repo = MockRepo::with_docs(vec![make_doc("docs/a", "sha256:content")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/a.md".to_string(),
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:meta".to_string()),
                legacy_slug: None,
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert_eq!(
            result.to_upload,
            vec![upload("docs/a.md", "docs/a")],
            "should upload when server has no metadata_hash"
        );
    }

    #[tokio::test]
    async fn test_sync_no_metadata_hash_from_client_is_backwards_compat() {
        let repo = MockRepo::with_docs(vec![make_doc_with_meta(
            "docs/a",
            "sha256:content",
            "sha256:meta",
        )]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![entry("docs/a", "sha256:content")],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        assert!(
            result.to_upload.is_empty(),
            "old CLI without metadata_hash should be treated as unchanged"
        );
        assert_eq!(result.unchanged, vec!["docs/a.md"]);
    }

    #[tokio::test]
    async fn test_sync_legacy_slug_migration() {
        // Server has a doc indexed with path-based slug (no source_path).
        // New CLI sends desired title-derived slug + legacy_slug for migration.
        let mut old_doc = make_doc("docs/my-guide", "sha256:content");
        old_doc.source_path = None; // simulate old document without source_path
        let repo = MockRepo::with_docs(vec![old_doc]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/my-guide.md".to_string(),
                slug: "docs/my-cool-guide".to_string(), // title-derived
                content_hash: "sha256:content".to_string(),
                metadata_hash: None,
                legacy_slug: Some("docs/my-guide".to_string()), // path-derived (old)
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        // Server resolves via legacy_slug → actual_slug = "docs/my-guide" (preserve URL)
        // source_path not yet set → force upload
        assert_eq!(
            result.to_upload,
            vec![upload("docs/my-guide.md", "docs/my-guide")],
            "migration should resolve to legacy slug and trigger upload to set source_path"
        );
        assert!(result.unchanged.is_empty());
        // Old slug must NOT appear in to_archive (it was claimed)
        assert!(result.to_archive.is_empty());
    }

    #[tokio::test]
    async fn test_sync_source_path_lookup_after_migration() {
        // After migration, doc has source_path set. Next sync should find it by source_path.
        let repo = MockRepo::with_docs(vec![make_doc("docs/my-guide", "sha256:content")]);
        let token_repo = MockServiceTokenRepo;
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/my-guide.md".to_string(),
                slug: "docs/my-cool-guide".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: None,
                legacy_slug: Some("docs/my-guide".to_string()),
            }],
            archive_missing: false,
        };

        let result = process_sync(&repo, &token_repo, None, Some("legacy"), request)
            .await
            .unwrap();
        // Found by source_path → actual_slug = "docs/my-guide", nothing changed
        assert!(result.to_upload.is_empty());
        assert_eq!(result.unchanged, vec!["docs/my-guide.md"]);
    }
}
