use crate::db::models::{IngestRequest, IngestResponse};
use crate::error::AppError;

#[cfg(feature = "ssr")]
use chrono::Utc;
#[cfg(feature = "ssr")]
use crate::db::access_level_repository::AccessLevelRepository;
#[cfg(feature = "ssr")]
use crate::db::models::Document;
#[cfg(feature = "ssr")]
use crate::db::document_version_repository::DocumentVersionRepository;
#[cfg(feature = "ssr")]
use crate::db::repository::DocumentRepository;
#[cfg(feature = "ssr")]
use crate::db::service_token_repository::ServiceTokenRepository;
#[cfg(feature = "ssr")]
use crate::rendering::links::extract_internal_links;
#[cfg(feature = "ssr")]
use crate::rag::service::RagService;
#[cfg(feature = "ssr")]
use crate::search::client::SearchService;
#[cfg(feature = "ssr")]
use crate::storage::client::StorageClient;

/// Bundles the service references needed by [`process_ingest`].
#[cfg(feature = "ssr")]
pub struct IngestContext<'a> {
    pub repo: &'a dyn DocumentRepository,
    pub storage: &'a dyn StorageClient,
    pub search: Option<&'a dyn SearchService>,
    pub access_level_repo: &'a dyn AccessLevelRepository,
    pub service_token_repo: &'a dyn ServiceTokenRepository,
    pub version_repo: &'a dyn DocumentVersionRepository,
    pub rag: Option<&'a dyn RagService>,
    /// The legacy global token from the `SERVICE_TOKEN` env var (if set).
    pub legacy_token: Option<&'a str>,
}

/// Core ingestion logic — separated from the HTTP layer for testability.
///
/// Validates the request, uploads content to S3, upserts metadata in MongoDB,
/// and optionally indexes the document in Meilisearch.
#[cfg(feature = "ssr")]
pub async fn process_ingest(
    ctx: &IngestContext<'_>,
    request: IngestRequest,
) -> Result<IngestResponse, AppError> {
    // 1. Validate the service token (legacy or scoped)
    validate_token(ctx, &request.service_token, &request.slug).await?;

    // 2. Validate the slug
    if request.slug.is_empty() {
        return Err(AppError::BadRequest("Slug cannot be empty".into()));
    }
    if request.slug.contains("..") {
        return Err(AppError::BadRequest(
            "Slug must not contain '..'".into(),
        ));
    }
    if request.slug.starts_with('/') {
        return Err(AppError::BadRequest(
            "Slug must not start with '/'".into(),
        ));
    }

    // 3. Validate the access_level name exists in the registry.
    if request.access_level.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Access level cannot be empty".into(),
        ));
    }
    // Normalise to lowercase so "Public" and "public" are the same.
    let access_level = request.access_level.to_lowercase();
    if !ctx.access_level_repo.exists(&access_level).await? {
        return Err(AppError::BadRequest(format!(
            "Unknown access level: '{access_level}'"
        )));
    }

    // 4. Compute content hash (used for S3 upload decision)
    let new_hash = format!(
        "sha256:{}",
        crate::auth::token_service::TokenService::hash_token(&request.content)
    );

    // Compute metadata hash (sent by CLI alongside content_hash; stored separately
    // so that metadata-only changes can be detected during sync without requiring
    // a full content re-upload).
    let new_metadata_hash = compute_metadata_hash(
        &request.title,
        &access_level,
        &request.service_owner,
        &request.tags,
        request.parent_slug.as_deref(),
        request.order,
        request.is_hidden,
    );

    // 5. Extract internal links from content
    let links_out = extract_internal_links(&request.content);

    // 6. Get old document to diff backlinks and detect changes
    let old_doc = ctx.repo.find_by_slug(&request.slug).await?;

    let (old_links, old_backlinks, old_parent_slug, old_order, old_is_hidden, old_hash) =
        match &old_doc {
            Some(d) => (
                d.links_out.clone(),
                d.backlinks.clone(),
                d.parent_slug.clone(),
                d.order,
                d.is_hidden,
                d.content_hash.clone(),
            ),
            None => (vec![], vec![], None, 0, false, None),
        };

    let content_changed = old_hash.as_deref() != Some(&new_hash);

    // Determine effective metadata values
    let effective_parent_slug = if request.parent_slug.is_some() {
        request.parent_slug.clone()
    } else {
        old_parent_slug
    };
    let effective_order = if request.order > 0 {
        request.order
    } else {
        old_order
    };
    let effective_is_hidden = if request.is_hidden {
        true
    } else {
        old_is_hidden
    };

    // Check if metadata changed (compared to existing doc)
    let metadata_changed = old_doc.as_ref().map_or(true, |d| {
        d.title != request.title
            || d.access_level != access_level
            || d.is_draft != request.is_draft
            || d.service_owner != request.service_owner
            || d.tags != request.tags
            || d.parent_slug != effective_parent_slug
            || d.order != effective_order
            || d.is_hidden != effective_is_hidden
            || d.links_out != links_out
    });

    // If nothing changed, return early
    if !content_changed && !metadata_changed {
        let s3_key = format!("docs/{}.md", request.slug.replace('/', "_"));
        return Ok(IngestResponse {
            message: "Document unchanged".to_string(),
            slug: request.slug,
            s3_key,
            changed: false,
        });
    }

    // 7. Build the S3 key
    let s3_key = format!("docs/{}.md", request.slug.replace('/', "_"));

    // Keep raw content for search indexing
    let raw_content = request.content.clone();

    // 8. Create version history before overwriting (only when content changed and old doc exists)
    if content_changed {
        if let Some(ref old) = old_doc {
            if let Some(ref old_content_hash) = old.content_hash {
                // Copy old content to history
                let version_num = ctx.version_repo.next_version_number(&request.slug).await?;
                let history_key = format!(
                    "docs/history/{}/{}.md",
                    request.slug.replace('/', "_"),
                    version_num
                );

                // Read old content from S3 and copy to history
                if let Ok(Some(old_content)) = ctx.storage.get_object(&old.s3_key).await {
                    if let Err(e) = ctx.storage.put_object(&history_key, old_content).await {
                        tracing::warn!("Failed to archive old version to S3: {e}");
                    }
                }

                // Determine who is updating (token name or "legacy")
                let updated_by = resolve_token_name(ctx, &request.service_token).await;

                let version = crate::db::document_version_repository::DocumentVersion {
                    id: uuid::Uuid::new_v4().to_string(),
                    slug: request.slug.clone(),
                    version: version_num,
                    content_hash: old_content_hash.clone(),
                    s3_key: history_key,
                    updated_by,
                    created_at: Utc::now(),
                };

                if let Err(e) = ctx.version_repo.create(version).await {
                    tracing::warn!("Failed to create version record: {e}");
                }
            }
        }

        // 9. Upload new content to S3
        ctx.storage
            .put_object(&s3_key, request.content.into_bytes())
            .await?;
    }

    // 9. Upsert document metadata in MongoDB
    let doc = Document {
        slug: request.slug.clone(),
        title: request.title,
        s3_key: s3_key.clone(),
        access_level,
        is_draft: request.is_draft,
        service_owner: request.service_owner,
        last_updated: Utc::now(),
        tags: request.tags,
        links_out: links_out.clone(),
        backlinks: old_backlinks,
        parent_slug: effective_parent_slug,
        order: effective_order,
        is_hidden: effective_is_hidden,
        content_hash: Some(new_hash),
        metadata_hash: Some(new_metadata_hash),
        is_archived: false,
    };

    // 10. Build search document before ownership transfer
    let search_doc = ctx.search
        .as_ref()
        .map(|_| crate::search::client::build_search_document(&doc, &raw_content));

    // Capture fields for RAG indexing before doc is consumed
    let rag_slug = doc.slug.clone();
    let rag_title = doc.title.clone();
    let rag_access_level = doc.access_level.clone();
    let rag_is_draft = doc.is_draft;
    let rag_tags = doc.tags.clone();
    let rag_is_archived = doc.is_archived;

    ctx.repo.create_or_update(doc).await?;

    // 11. Update backlinks on referenced documents.
    //     Note: this is not atomic with the create_or_update above.
    //     Both operations are idempotent, so partial failure leaves
    //     consistent (if stale) state that self-heals on re-ingest.
    ctx.repo.update_backlinks(&request.slug, &old_links, &links_out)
        .await?;

    // 12. Index in Meilisearch (if available)
    if let (Some(search_svc), Some(search_doc)) = (ctx.search, search_doc) {
        if let Err(e) = search_svc.index_document(&search_doc).await {
            tracing::warn!("Failed to index document in search: {e}");
        }
    }

    // 13. Index in RAG vector store (if available)
    if let Some(rag) = ctx.rag {
        if rag_is_archived {
            if let Err(e) = rag.delete_document(&rag_slug).await {
                tracing::warn!("Failed to remove archived document from RAG: {e}");
            }
        } else {
            if let Err(e) = rag
                .index_document(
                    &rag_slug,
                    &rag_title,
                    &raw_content,
                    &rag_access_level,
                    rag_is_draft,
                    &rag_tags,
                )
                .await
            {
                tracing::warn!("Failed to index document in RAG: {e}");
            }
        }
    }

    Ok(IngestResponse {
        message: "Document ingested successfully".to_string(),
        slug: request.slug,
        s3_key,
        changed: true,
    })
}

/// Build a canonical string from document metadata and hash it.
///
/// The canonical format is identical to what `lekton-sync` (the CLI) computes,
/// so the server and client always agree on what "metadata unchanged" means.
///
/// Fields included: title, access_level (already lowercase), service_owner,
/// tags (sorted), parent_slug, order, is_hidden.
/// `is_draft` is intentionally excluded because the CLI does not expose it yet.
#[cfg(feature = "ssr")]
pub(crate) fn compute_metadata_hash(
    title: &str,
    access_level: &str,
    service_owner: &str,
    tags: &[String],
    parent_slug: Option<&str>,
    order: u32,
    is_hidden: bool,
) -> String {
    let mut sorted_tags: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    sorted_tags.sort_unstable();
    let canonical = format!(
        "title={title}\naccess_level={access_level}\nservice_owner={service_owner}\ntags={}\nparent_slug={}\norder={order}\nis_hidden={is_hidden}",
        sorted_tags.join(","),
        parent_slug.unwrap_or(""),
    );
    format!(
        "sha256:{}",
        crate::auth::token_service::TokenService::hash_token(&canonical)
    )
}

/// Validate the service token — either legacy global token or scoped token.
#[cfg(feature = "ssr")]
async fn validate_token(
    ctx: &IngestContext<'_>,
    raw_token: &str,
    slug: &str,
) -> Result<(), AppError> {
    // 1. Legacy token bypass (full access, no scope check)
    if let Some(legacy) = ctx.legacy_token {
        if !legacy.is_empty() && raw_token == legacy {
            return Ok(());
        }
    }

    // 2. Look up scoped token by hash
    let token_hash = crate::auth::token_service::TokenService::hash_token(raw_token);
    let token = ctx
        .service_token_repo
        .find_by_hash(&token_hash)
        .await?
        .ok_or_else(|| AppError::Auth("Invalid service token".into()))?;

    if !token.is_active {
        return Err(AppError::Auth("Service token is deactivated".into()));
    }

    if !token.can_write {
        return Err(AppError::Forbidden(
            "Token does not have write permission".into(),
        ));
    }

    if !token.matches_slug(slug) {
        return Err(AppError::Forbidden(
            "Token does not have access to this document scope".into(),
        ));
    }

    // Fire-and-forget last_used update
    let id = token.id.clone();
    let repo = ctx.service_token_repo;
    if let Err(e) = repo.touch_last_used(&id).await {
        tracing::warn!("Failed to update last_used_at for token {id}: {e}");
    }

    Ok(())
}

/// Resolve the human-readable name for the token used in this request.
#[cfg(feature = "ssr")]
async fn resolve_token_name(ctx: &IngestContext<'_>, raw_token: &str) -> String {
    if let Some(legacy) = ctx.legacy_token {
        if !legacy.is_empty() && raw_token == legacy {
            return "legacy".to_string();
        }
    }
    let hash = crate::auth::token_service::TokenService::hash_token(raw_token);
    match ctx.service_token_repo.find_by_hash(&hash).await {
        Ok(Some(token)) => token.name,
        _ => "unknown".to_string(),
    }
}

/// Axum handler for `POST /api/v1/ingest`.
///
/// Only available when the `ssr` feature is enabled.
#[cfg(feature = "ssr")]
pub async fn ingest_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<IngestRequest>,
) -> Result<axum::Json<IngestResponse>, AppError> {
    let ctx = IngestContext {
        repo: state.document_repo.as_ref(),
        storage: state.storage_client.as_ref(),
        search: state.search_service.as_deref(),
        access_level_repo: state.access_level_repo.as_ref(),
        service_token_repo: state.service_token_repo.as_ref(),
        version_repo: state.document_version_repo.as_ref(),
        rag: state.rag_service.as_deref(),
        legacy_token: Some(&state.service_token),
    };

    let response = process_ingest(&ctx, request).await?;
    Ok(axum::Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    use crate::db::access_level_repository::AccessLevelRepository;
    use crate::db::auth_models::AccessLevelEntity;
    use crate::db::service_token_models::ServiceToken;
    use crate::db::service_token_repository::ServiceTokenRepository;
    use crate::test_utils::MockStorage;

    /// A mock access level repo that accepts any non-empty level name.
    struct MockAccessLevelRepo;

    #[async_trait]
    impl AccessLevelRepository for MockAccessLevelRepo {
        async fn create(&self, _level: AccessLevelEntity) -> Result<(), AppError> { Ok(()) }
        async fn find_by_name(&self, _name: &str) -> Result<Option<AccessLevelEntity>, AppError> { Ok(None) }
        async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError> { Ok(vec![]) }
        async fn update(&self, _level: AccessLevelEntity) -> Result<(), AppError> { Ok(()) }
        async fn delete(&self, _name: &str) -> Result<(), AppError> { Ok(()) }
        async fn exists(&self, _name: &str) -> Result<bool, AppError> { Ok(true) }
        async fn seed_defaults(&self) -> Result<(), AppError> { Ok(()) }
    }

    /// A mock service token repo for unit tests.
    struct MockServiceTokenRepo {
        tokens: Mutex<Vec<ServiceToken>>,
    }

    impl MockServiceTokenRepo {
        fn new() -> Self {
            Self {
                tokens: Mutex::new(vec![]),
            }
        }

        fn with_token(token: ServiceToken) -> Self {
            Self {
                tokens: Mutex::new(vec![token]),
            }
        }
    }

    #[async_trait]
    impl ServiceTokenRepository for MockServiceTokenRepo {
        async fn create(&self, token: ServiceToken) -> Result<(), AppError> {
            self.tokens.lock().unwrap().push(token);
            Ok(())
        }
        async fn find_by_hash(&self, token_hash: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(self.tokens.lock().unwrap().iter().find(|t| t.token_hash == token_hash).cloned())
        }
        async fn find_by_name(&self, name: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(self.tokens.lock().unwrap().iter().find(|t| t.name == name).cloned())
        }
        async fn find_by_id(&self, id: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(self.tokens.lock().unwrap().iter().find(|t| t.id == id).cloned())
        }
        async fn list_all(&self) -> Result<Vec<ServiceToken>, AppError> {
            Ok(self.tokens.lock().unwrap().clone())
        }
        async fn deactivate(&self, id: &str) -> Result<(), AppError> {
            let mut tokens = self.tokens.lock().unwrap();
            if let Some(t) = tokens.iter_mut().find(|t| t.id == id) {
                t.is_active = false;
            }
            Ok(())
        }
        async fn touch_last_used(&self, _id: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn check_scope_overlap(&self, _scopes: &[String], _exclude_id: Option<&str>) -> Result<bool, AppError> {
            Ok(false)
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

        async fn list_by_access_levels(
            &self,
            allowed_levels: Option<&[String]>,
            include_draft: bool,
        ) -> Result<Vec<Document>, AppError> {
            Ok(self
                .documents
                .lock()
                .unwrap()
                .iter()
                .filter(|d| {
                    let level_ok = allowed_levels
                        .map(|lvls| lvls.contains(&d.access_level))
                        .unwrap_or(true);
                    let draft_ok = include_draft || !d.is_draft;
                    level_ok && draft_ok
                })
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

    fn make_request(token: &str, slug: &str) -> IngestRequest {
        IngestRequest {
            service_token: token.to_string(),
            slug: slug.to_string(),
            title: "Test Doc".to_string(),
            content: "# Hello\nWorld".to_string(),
            access_level: "internal".to_string(),
            is_draft: false,
            service_owner: "test-team".to_string(),
            tags: vec!["test".to_string()],
            parent_slug: None,
            order: 0,
            is_hidden: false,
        }
    }

    struct MockVersionRepo;

    #[async_trait]
    impl crate::db::document_version_repository::DocumentVersionRepository for MockVersionRepo {
        async fn create(&self, _: crate::db::document_version_repository::DocumentVersion) -> Result<(), AppError> { Ok(()) }
        async fn find_latest(&self, _: &str) -> Result<Option<crate::db::document_version_repository::DocumentVersion>, AppError> { Ok(None) }
        async fn list_by_slug(&self, _: &str) -> Result<Vec<crate::db::document_version_repository::DocumentVersion>, AppError> { Ok(vec![]) }
        async fn next_version_number(&self, _: &str) -> Result<u64, AppError> { Ok(1) }
    }

    fn make_ctx<'a>(
        repo: &'a MockRepo,
        storage: &'a MockStorage,
        token_repo: &'a dyn ServiceTokenRepository,
        legacy_token: Option<&'a str>,
    ) -> IngestContext<'a> {
        IngestContext {
            repo,
            storage,
            search: None,
            access_level_repo: &MockAccessLevelRepo,
            service_token_repo: token_repo,
            version_repo: &MockVersionRepo,
            rag: None,
            legacy_token,
        }
    }

    fn make_scoped_token(raw_token: &str, scopes: Vec<&str>) -> ServiceToken {
        use crate::auth::token_service::TokenService;
        ServiceToken {
            id: "st-1".to_string(),
            name: "test-scoped".to_string(),
            token_hash: TokenService::hash_token(raw_token),
            allowed_scopes: scopes.into_iter().map(String::from).collect(),
            can_write: true,
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            is_active: true,
        }
    }

    #[tokio::test]
    async fn test_ingest_success() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let request = make_request("valid-token", "docs/hello");

        let result = process_ingest(&ctx, request).await;
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

        // Verify metadata was saved with access_level normalised to lowercase
        let doc = repo.find_by_slug("docs/hello").await.unwrap();
        assert!(doc.is_some());
        let doc = doc.unwrap();
        assert_eq!(doc.title, "Test Doc");
        assert_eq!(doc.access_level, "internal");
        assert!(!doc.is_draft);
    }

    #[tokio::test]
    async fn test_ingest_draft_flag_preserved() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let mut request = make_request("valid-token", "docs/wip");
        request.is_draft = true;

        process_ingest(&ctx, request).await.unwrap();

        let doc = repo.find_by_slug("docs/wip").await.unwrap().unwrap();
        assert!(doc.is_draft);
    }

    #[tokio::test]
    async fn test_ingest_invalid_token() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let request = make_request("wrong-token", "docs/hello");

        let result = process_ingest(&ctx, request).await;
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
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let request = make_request("valid-token", "");

        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("Slug cannot be empty")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_empty_access_level() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let mut request = make_request("valid-token", "docs/hello");
        request.access_level = "  ".to_string();

        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("Access level cannot be empty")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_normalises_access_level_to_lowercase() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let mut request = make_request("valid-token", "docs/hello");
        request.access_level = "Internal".to_string();

        process_ingest(&ctx, request).await.unwrap();

        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert_eq!(doc.access_level, "internal");
    }

    #[tokio::test]
    async fn test_ingest_upsert() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        // First ingest
        let request1 = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request1).await.unwrap();

        // Second ingest (update)
        let mut request2 = make_request("valid-token", "docs/hello");
        request2.title = "Updated Doc".to_string();
        process_ingest(&ctx, request2).await.unwrap();

        // Should have only one document
        let docs = repo
            .list_by_access_levels(Some(&["internal".to_string()]), false)
            .await
            .unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "Updated Doc");
    }

    #[tokio::test]
    async fn test_ingest_rejects_path_traversal() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let request = make_request("valid-token", "../etc/passwd");
        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ingest_rejects_absolute_slug() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let request = make_request("valid-token", "/absolute/path");
        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
    }

    // ── Scoped token tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_ingest_scoped_token_success() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let scoped = make_scoped_token("scoped-secret", vec!["docs/*"]);
        let token_repo = MockServiceTokenRepo::with_token(scoped);
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("legacy-token"));

        let request = make_request("scoped-secret", "docs/hello");
        let result = process_ingest(&ctx, request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ingest_scoped_token_out_of_scope() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let scoped = make_scoped_token("scoped-secret", vec!["protocols/*"]);
        let token_repo = MockServiceTokenRepo::with_token(scoped);
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("legacy-token"));

        let request = make_request("scoped-secret", "docs/hello");
        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Forbidden(msg) => assert!(msg.contains("scope")),
            other => panic!("Expected Forbidden error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_legacy_token_bypasses_scopes() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new(); // no scoped tokens
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("legacy-token"));

        // Using the legacy token should work for any slug
        let request = make_request("legacy-token", "any/slug/here");
        let result = process_ingest(&ctx, request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ingest_inactive_token_rejected() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let mut scoped = make_scoped_token("inactive-secret", vec!["docs/*"]);
        scoped.is_active = false;
        let token_repo = MockServiceTokenRepo::with_token(scoped);
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("legacy-token"));

        let request = make_request("inactive-secret", "docs/hello");
        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("deactivated")),
            other => panic!("Expected Auth error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_read_only_token_rejected() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let mut scoped = make_scoped_token("readonly-secret", vec!["docs/*"]);
        scoped.can_write = false;
        let token_repo = MockServiceTokenRepo::with_token(scoped);
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("legacy-token"));

        let request = make_request("readonly-secret", "docs/hello");
        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Forbidden(msg) => assert!(msg.contains("write permission")),
            other => panic!("Expected Forbidden error, got: {:?}", other),
        }
    }

    // ── Content hash tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_ingest_unchanged_content_skips_upload() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        // First ingest
        let request1 = make_request("valid-token", "docs/hello");
        let r1 = process_ingest(&ctx, request1).await.unwrap();
        assert!(r1.changed);
        assert_eq!(storage.put_count.load(std::sync::atomic::Ordering::Relaxed), 1);

        // Second ingest with identical content and metadata
        let request2 = make_request("valid-token", "docs/hello");
        let r2 = process_ingest(&ctx, request2).await.unwrap();
        assert!(!r2.changed);
        // S3 upload should NOT have happened again
        assert_eq!(storage.put_count.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_ingest_changed_content_uploads() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        // First ingest
        let request1 = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request1).await.unwrap();

        // Second ingest with different content
        let mut request2 = make_request("valid-token", "docs/hello");
        request2.content = "# Updated\nNew content".to_string();
        let r2 = process_ingest(&ctx, request2).await.unwrap();
        assert!(r2.changed);
        // 3 puts: initial upload + history copy + new upload
        assert_eq!(storage.put_count.load(std::sync::atomic::Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn test_ingest_same_content_different_metadata_updates_db() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        // First ingest
        let request1 = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request1).await.unwrap();

        // Second ingest: same content, different title
        let mut request2 = make_request("valid-token", "docs/hello");
        request2.title = "New Title".to_string();
        let r2 = process_ingest(&ctx, request2).await.unwrap();
        assert!(r2.changed);
        // S3 upload should NOT happen (content is the same)
        assert_eq!(storage.put_count.load(std::sync::atomic::Ordering::Relaxed), 1);

        // But DB should be updated with new title
        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert_eq!(doc.title, "New Title");
    }

    #[tokio::test]
    async fn test_ingest_stores_content_hash() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let request = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request).await.unwrap();

        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert!(doc.content_hash.is_some());
        assert!(doc.content_hash.unwrap().starts_with("sha256:"));
    }

    #[tokio::test]
    async fn test_ingest_stores_metadata_hash() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let request = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request).await.unwrap();

        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert!(doc.metadata_hash.is_some());
        assert!(doc.metadata_hash.as_ref().unwrap().starts_with("sha256:"));

        // metadata_hash must differ from content_hash (they cover different input)
        assert_ne!(doc.metadata_hash, doc.content_hash);
    }

    #[tokio::test]
    async fn test_ingest_metadata_hash_changes_when_access_level_changes() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let request1 = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request1).await.unwrap();
        let hash1 = repo.find_by_slug("docs/hello").await.unwrap().unwrap().metadata_hash.unwrap();

        let mut request2 = make_request("valid-token", "docs/hello");
        request2.access_level = "public".to_string();
        process_ingest(&ctx, request2).await.unwrap();
        let hash2 = repo.find_by_slug("docs/hello").await.unwrap().unwrap().metadata_hash.unwrap();

        assert_ne!(hash1, hash2, "metadata_hash must change when access_level changes");
    }

    #[tokio::test]
    async fn test_ingest_metadata_hash_stable_when_nothing_changes() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let request1 = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request1).await.unwrap();
        let hash1 = repo.find_by_slug("docs/hello").await.unwrap().unwrap().metadata_hash.unwrap();

        // Second ingest with identical data — unchanged, no DB write
        let request2 = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request2).await.unwrap();
        let hash2 = repo.find_by_slug("docs/hello").await.unwrap().unwrap().metadata_hash.unwrap();

        assert_eq!(hash1, hash2, "metadata_hash must be stable when nothing changes");
    }
}
