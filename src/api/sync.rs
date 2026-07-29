use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::db::asset_repository::AssetRepository;
use crate::error::AppError;
#[cfg(feature = "ssr")]
use crate::rag::service::RagService;
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
    /// Hash of front-matter metadata (title, summary, access_level, …).
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
    /// Stable identifier for the import source (from `.lekton.yml` `id` field).
    /// Only documents from this source are considered when computing archives.
    pub source_id: String,
    /// The client's complete list of documents.
    pub documents: Vec<SyncDocumentEntry>,
    /// If `true`, documents from this source that are missing from the
    /// client list will be automatically archived.
    #[serde(default)]
    pub archive_missing: bool,
    /// The release being synced (from `lekton-sync --version`).
    ///
    /// `None` targets the source's unversioned bucket, which is the behaviour
    /// for sources that never published a release. Once a source *has* published
    /// one, omitting this is an error rather than a silent write to a bucket
    /// nobody resolves.
    #[serde(default)]
    pub release: Option<String>,
    /// When `true`, compute the delta without performing any write.
    ///
    /// Sync is the first call `lekton-sync` makes even under `--dry-run`, and it
    /// archives and registers releases as a side effect, so a preview needs a
    /// way to ask for the plan only.
    #[serde(default)]
    pub dry_run: bool,
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
#[allow(clippy::too_many_arguments)]
pub async fn process_sync(
    repo: &dyn crate::db::repository::DocumentRepository,
    release_repo: &dyn crate::db::release_repository::ReleaseRepository,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    search: Option<&dyn SearchService>,
    rag: Option<&dyn RagService>,
    asset_repo: Option<&dyn AssetRepository>,
    storage: Option<&dyn crate::storage::client::StorageClient>,
    attachment_search: Option<&dyn crate::search::attachment_search::AttachmentSearchService>,
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

    // 2b. A source that has published a release must keep naming one. Writing
    // to the unversioned bucket instead would create documents that nothing
    // resolves (the source's readers go through releases from now on), and
    // writing over `latest` would mutate an already-tagged release.
    if request.release.is_none() && release_repo.is_release_managed(&request.source_id).await? {
        return Err(AppError::BadRequest(format!(
            "source '{}' is release-managed; --version is required",
            request.source_id
        )));
    }

    // 2c. A slug belongs to exactly one source. Until this release the unique
    // `slug` index enforced that implicitly; now that a slug may legitimately
    // repeat across releases, the check has to be explicit.
    let requested_slugs: Vec<String> = request.documents.iter().map(|e| e.slug.clone()).collect();
    for existing in repo.find_by_slugs(&requested_slugs).await? {
        match existing.source_id.as_deref() {
            Some(owner) if owner != request.source_id => {
                return Err(AppError::BadRequest(format!(
                    "slug '{}' is already owned by source '{}'",
                    existing.slug, owner
                )));
            }
            _ => {}
        }
    }

    // 3. Fetch the server documents for this source *within the release being
    // synced*, so the archive computation below cannot touch another release.
    let mut server_by_slug: HashMap<String, ServerDocInfo> = HashMap::new();
    let mut server_by_source_path: HashMap<String, String> = HashMap::new();

    for doc in repo
        .find_all_by_source_id_and_release(&request.source_id, request.release.as_deref())
        .await?
    {
        if let Some(ref sp) = doc.source_path {
            server_by_source_path.insert(sp.clone(), doc.slug.clone());
        }
        server_by_slug.insert(
            doc.slug.clone(),
            (doc.content_hash, doc.metadata_hash, doc.source_path),
        );
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
            if existing_slug == &entry.slug {
                existing_slug.clone()
            } else {
                // Slug renamed — check that the new slug isn't already taken by a different doc
                if server_by_slug.contains_key(entry.slug.as_str()) {
                    return Err(AppError::BadRequest(format!(
                        "Cannot rename '{}' to '{}': target slug is already in use by another document",
                        existing_slug, entry.slug
                    )));
                }
                // Claim the old slug so it isn't mistakenly archived before ingest renames it
                claimed_slugs.insert(existing_slug.clone());
                entry.slug.clone()
            }
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

    // 6. Archive missing docs if requested. Skipped under `dry_run`, which asks
    // for the plan only — `to_archive` is still reported so the caller can show
    // what would happen.
    if request.archive_missing && !request.dry_run {
        for slug in &to_archive {
            repo.set_archived(slug, true).await?;
            if let Some(svc) = search {
                if let Err(e) = svc.delete_document(slug).await {
                    tracing::warn!("Failed to deindex archived document '{slug}' from search: {e}");
                }
            }
            if let Some(rag_svc) = rag {
                if let Err(e) = rag_svc.delete_document(slug).await {
                    tracing::warn!("Failed to remove archived document '{slug}' from RAG: {e}");
                }
                // Drop the archived document from the assets it referenced and
                // recompute their access levels, so an attachment no longer
                // inherits an archived document's visibility.
                if let (Some(asset_repo), Some(storage)) = (asset_repo, storage) {
                    match asset_repo.set_references(slug, &[]).await {
                        Ok(affected) if !affected.is_empty() => {
                            crate::rag::attachment_extraction::recompute_access_levels(
                                rag_svc,
                                asset_repo,
                                repo,
                                storage,
                                attachment_search,
                                &affected,
                            )
                            .await;
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(
                            "Failed to update asset references for archived '{slug}': {e}"
                        ),
                    }
                }
            }
        }
    }

    // 7. Record the release in the catalogue. Done here rather than on ingest so
    // a re-sync with no changes still marks the release as published, which is
    // what the version selector lists.
    if let (Some(release), false) = (request.release.as_deref(), request.dry_run) {
        release_repo.register(&request.source_id, release).await?;
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
///
/// Sync is a precursor to writes (it tells the client what to upload), so the
/// token must have `can_write`; a read-only token is rejected.
#[cfg(feature = "ssr")]
pub(crate) async fn validate_sync_token(
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    raw_token: &str,
) -> Result<Vec<String>, AppError> {
    let resolved = crate::api::token_validation::resolve_service_token(
        service_token_repo,
        legacy_token,
        raw_token,
    )
    .await?;

    if !resolved.can_write {
        return Err(AppError::Forbidden(
            "Token does not have write permission".into(),
        ));
    }

    Ok(resolved.scopes)
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
        state.release_repo.as_ref(),
        state.service_token_repo.as_ref(),
        state.search_service.as_deref(),
        state.rag_service.as_deref(),
        Some(state.asset_repo.as_ref()),
        Some(state.storage_client.as_ref()),
        state.attachment_search_service.as_deref(),
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

    struct MockRagService {
        deleted: Mutex<Vec<String>>,
    }

    impl MockRagService {
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
    impl crate::rag::service::RagService for MockRagService {
        async fn index_document(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: bool,
            _: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
            self.deleted.lock().unwrap().push(slug.to_string());
            Ok(())
        }

        async fn index_attachment(
            &self,
            _: &str,
            _: &str,
            _: &[crate::rag::service::AttachmentPage],
            _: &[String],
            _: &[String],
        ) -> Result<usize, AppError> {
            Ok(0)
        }

        async fn delete_attachment(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }

        async fn update_attachment_access_levels(
            &self,
            _: &str,
            _: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn health_check(&self) -> Result<(), AppError> {
            Ok(())
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
        async fn health_check(&self) -> Result<(), AppError> {
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
        async fn find_by_slugs(&self, slugs: &[String]) -> Result<Vec<Document>, AppError> {
            Ok(self
                .documents
                .lock()
                .unwrap()
                .iter()
                .filter(|d| slugs.iter().any(|slug| slug == &d.slug))
                .cloned()
                .collect())
        }
        async fn list_all(&self) -> Result<Vec<Document>, AppError> {
            Ok(self.documents.lock().unwrap().clone())
        }
        async fn list_by_access_levels(
            &self,
            _: Option<&[String]>,
            _: bool,
            _: &crate::versioning::ReleasePins,
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
        async fn rename_slug(&self, old_slug: &str, new_slug: &str) -> Result<(), AppError> {
            let mut docs = self.documents.lock().unwrap();
            if let Some(doc) = docs.iter_mut().find(|d| d.slug == old_slug) {
                doc.slug = new_slug.to_string();
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
        async fn find_all_by_source_id(&self, source_id: &str) -> Result<Vec<Document>, AppError> {
            Ok(self
                .documents
                .lock()
                .unwrap()
                .iter()
                .filter(|d| !d.is_archived && d.source_id.as_deref() == Some(source_id))
                .cloned()
                .collect())
        }
    }

    #[derive(Default)]
    struct MockReleaseRepo {
        /// Releases already published, which is what makes a source
        /// release-managed.
        published: Vec<(String, String)>,
        /// Registrations performed during the call under test.
        registered: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl MockReleaseRepo {
        fn with_published(pairs: &[(&str, &str)]) -> Self {
            Self {
                published: pairs
                    .iter()
                    .map(|(s, r)| (s.to_string(), r.to_string()))
                    .collect(),
                ..Default::default()
            }
        }

        fn registrations(&self) -> Vec<(String, String)> {
            self.registered.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl crate::db::release_repository::ReleaseRepository for MockReleaseRepo {
        async fn register(&self, source_id: &str, release: &str) -> Result<(), AppError> {
            self.registered
                .lock()
                .unwrap()
                .push((source_id.to_string(), release.to_string()));
            Ok(())
        }
        async fn list_by_source(
            &self,
            _: &str,
        ) -> Result<Vec<crate::db::release_repository::SourceRelease>, AppError> {
            Ok(vec![])
        }
        async fn is_release_managed(&self, source_id: &str) -> Result<bool, AppError> {
            Ok(self.published.iter().any(|(s, _)| s == source_id))
        }
        async fn latest(&self, _: &str) -> Result<Option<String>, AppError> {
            Ok(None)
        }
        async fn set_latest(&self, _: &str, _: &str) -> Result<(), AppError> {
            Ok(())
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

    /// A token repo returning a single configurable token on matching hash.
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

    fn make_service_token(scopes: Vec<&str>, can_write: bool) -> ServiceToken {
        ServiceToken {
            id: "st-1".to_string(),
            name: "test".to_string(),
            token_hash: crate::auth::token_service::TokenService::hash_token("scoped-tok"),
            allowed_scopes: scopes.into_iter().map(String::from).collect(),
            token_type: "service".to_string(),
            user_id: None,
            can_write,
            access_levels: None,
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            is_active: true,
        }
    }

    fn make_doc(slug: &str, hash: &str) -> Document {
        Document {
            slug: slug.to_string(),
            title: slug.to_string(),
            summary: None,
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
            source_id: Some("test-source".to_string()),
            release: None,
            is_latest: true,
            needs_reindex: false,
            skip_rag: false,
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/new", "sha256:abc")],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:new")],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: true,
            release: None,
            dry_run: false,
        };

        process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
        .await
        .unwrap();

        let doc = repo.find_by_slug("docs/old").await.unwrap().unwrap();
        assert!(doc.is_archived);
        let doc = repo.find_by_slug("docs/a").await.unwrap().unwrap();
        assert!(!doc.is_archived);
    }

    #[tokio::test]
    async fn test_sync_scope_validation() {
        let repo = MockRepo::new();
        let token_repo = ScopedTokenRepo(make_service_token(vec!["protocols/*"], true));
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "scoped-tok".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/outside", "sha256:abc")],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("other-legacy"),
            request,
        )
        .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Forbidden(msg) => assert!(msg.contains("docs/outside")),
            other => panic!("Expected Forbidden, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_sync_rejects_read_only_token() {
        // A token without can_write must not be able to drive a sync (which is a
        // precursor to uploads), even when the slug is within its scope.
        let repo = MockRepo::new();
        let token_repo = ScopedTokenRepo(make_service_token(vec!["*"], false));
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "scoped-tok".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("other-legacy"),
            request,
        )
        .await;
        match result.unwrap_err() {
            AppError::Forbidden(msg) => assert!(msg.contains("write permission")),
            other => panic!("Expected Forbidden (write permission), got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_sync_archive_deindexes_from_search() {
        let repo = MockRepo::with_docs(vec![
            make_doc("docs/a", "sha256:abc"),
            make_doc("docs/old", "sha256:def"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();
        let search = MockSearchService::new();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: true,
            release: None,
            dry_run: false,
        };

        process_sync(
            &repo,
            &release_repo,
            &token_repo,
            Some(&search),
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
        .await
        .unwrap();

        assert_eq!(search.deleted_slugs(), vec!["docs/old"]);
    }

    #[tokio::test]
    async fn test_sync_archive_deletes_from_rag() {
        let repo = MockRepo::with_docs(vec![
            make_doc("docs/a", "sha256:abc"),
            make_doc("docs/old", "sha256:def"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();
        let rag = MockRagService::new();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: true,
            release: None,
            dry_run: false,
        };

        process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            Some(&rag),
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
        .await
        .unwrap();

        assert_eq!(rag.deleted_slugs(), vec!["docs/old"]);
    }

    #[tokio::test]
    async fn test_sync_no_archive_does_not_deindex() {
        let repo = MockRepo::with_docs(vec![
            make_doc("docs/a", "sha256:abc"),
            make_doc("docs/old", "sha256:def"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();
        let search = MockSearchService::new();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:abc")],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        process_sync(
            &repo,
            &release_repo,
            &token_repo,
            Some(&search),
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/a.md".to_string(),
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:meta".to_string()),
                legacy_slug: None,
            }],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/a.md".to_string(),
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:new-meta".to_string()),
                legacy_slug: None,
            }],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/a.md".to_string(),
                slug: "docs/a".to_string(),
                content_hash: "sha256:content".to_string(),
                metadata_hash: Some("sha256:meta".to_string()),
                legacy_slug: None,
            }],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![entry("docs/a", "sha256:content")],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/my-guide.md".to_string(),
                slug: "docs/my-cool-guide".to_string(), // title-derived
                content_hash: "sha256:content".to_string(),
                metadata_hash: None,
                legacy_slug: Some("docs/my-guide".to_string()), // path-derived (old)
            }],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
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
    async fn test_sync_source_path_slug_rename_triggers_upload() {
        // Doc has source_path set but the client's desired slug has changed.
        // Sync should detect the rename and return the doc in to_upload with the new slug.
        let repo = MockRepo::with_docs(vec![make_doc("docs/my-guide", "sha256:content")]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/my-guide.md".to_string(),
                slug: "docs/my-cool-guide".to_string(), // new desired slug
                content_hash: "sha256:content".to_string(),
                metadata_hash: None,
                legacy_slug: Some("docs/my-guide".to_string()),
            }],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
        .await
        .unwrap();
        // Rename detected: actual_slug = new slug, old slug is claimed (not archived)
        assert_eq!(
            result.to_upload,
            vec![upload("docs/my-guide.md", "docs/my-cool-guide")],
            "rename should produce to_upload entry with new slug"
        );
        assert!(result.unchanged.is_empty());
        assert!(
            result.to_archive.is_empty(),
            "old slug must not be archived"
        );
    }

    #[tokio::test]
    async fn test_sync_source_path_same_slug_is_unchanged() {
        // After migration, doc has source_path set and slug hasn't changed — nothing to do.
        let repo = MockRepo::with_docs(vec![make_doc("docs/my-guide", "sha256:content")]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/my-guide.md".to_string(),
                slug: "docs/my-guide".to_string(), // same as stored
                content_hash: "sha256:content".to_string(),
                metadata_hash: None,
                legacy_slug: None,
            }],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
        .await
        .unwrap();
        assert!(result.to_upload.is_empty());
        assert_eq!(result.unchanged, vec!["docs/my-guide.md"]);
    }

    #[tokio::test]
    async fn test_sync_rename_to_existing_slug_is_rejected() {
        // Rename target slug is already in use by a different document — must error.
        let repo = MockRepo::with_docs(vec![
            make_doc("docs/old-name", "sha256:aaa"),
            make_doc("docs/taken", "sha256:bbb"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: vec![SyncDocumentEntry {
                source_path: "docs/old-name.md".to_string(),
                slug: "docs/taken".to_string(), // conflict!
                content_hash: "sha256:aaa".to_string(),
                metadata_hash: None,
                legacy_slug: None,
            }],
            archive_missing: false,
            release: None,
            dry_run: false,
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
        .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("already in use")),
            other => panic!("Expected BadRequest, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_sync_does_not_archive_other_source_docs() {
        // source-a owns docs/a and docs/b; source-b owns docs/c.
        // Syncing source-a with only docs/a should archive docs/b but NOT docs/c.
        let mut doc_a = make_doc("docs/a", "sha256:aaa");
        doc_a.source_id = Some("source-a".to_string());
        let mut doc_b = make_doc("docs/b", "sha256:bbb");
        doc_b.source_id = Some("source-a".to_string());
        let mut doc_c = make_doc("docs/c", "sha256:ccc");
        doc_c.source_id = Some("source-b".to_string());

        let repo = MockRepo::with_docs(vec![doc_a, doc_b, doc_c]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();
        let request = SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "source-a".to_string(),
            documents: vec![entry("docs/a", "sha256:aaa")],
            archive_missing: true,
            release: None,
            dry_run: false,
        };

        process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
        .await
        .unwrap();

        // docs/b should be archived (source-a, not in sync)
        let doc_b = repo.find_by_slug("docs/b").await.unwrap().unwrap();
        assert!(doc_b.is_archived, "docs/b should be archived");
        // docs/c must NOT be archived (belongs to source-b)
        let doc_c = repo.find_by_slug("docs/c").await.unwrap().unwrap();
        assert!(!doc_c.is_archived, "docs/c must not be archived");
    }

    // ── Release versioning ───────────────────────────────────────────────

    fn make_doc_in_release(slug: &str, hash: &str, release: &str) -> Document {
        Document {
            release: Some(release.to_string()),
            ..make_doc(slug, hash)
        }
    }

    fn versioned_request(release: Option<&str>, docs: Vec<SyncDocumentEntry>) -> SyncRequest {
        SyncRequest {
            service_token: "legacy".to_string(),
            source_id: "test-source".to_string(),
            documents: docs,
            archive_missing: true,
            release: release.map(str::to_string),
            dry_run: false,
        }
    }

    /// The core promise of versioning: dropping a document in a newer release
    /// must not touch the copy that older releases still ship.
    #[tokio::test]
    async fn archiving_is_scoped_to_the_release_being_synced() {
        let repo = MockRepo::with_docs(vec![
            make_doc_in_release("docs/removed", "sha256:abc", "1.0.0"),
            make_doc_in_release("docs/kept", "sha256:def", "1.2.0"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::with_published(&[("test-source", "1.0.0")]);

        // Syncing 1.2.0 with only docs/kept present.
        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            versioned_request(Some("1.2.0"), vec![entry("docs/kept", "sha256:def")]),
        )
        .await
        .unwrap();

        assert!(
            result.to_archive.is_empty(),
            "1.0.0's document is invisible to a 1.2.0 sync, so it cannot be archived: {:?}",
            result.to_archive
        );
        let survivor = repo.find_by_slug("docs/removed").await.unwrap().unwrap();
        assert!(
            !survivor.is_archived,
            "the 1.0.0 copy must survive a 1.2.0 sync that omits it"
        );
    }

    /// Within one release the archive behaviour is unchanged.
    #[tokio::test]
    async fn archiving_still_applies_inside_the_synced_release() {
        let repo = MockRepo::with_docs(vec![
            make_doc_in_release("docs/gone", "sha256:abc", "1.2.0"),
            make_doc_in_release("docs/kept", "sha256:def", "1.2.0"),
        ]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::with_published(&[("test-source", "1.2.0")]);

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            versioned_request(Some("1.2.0"), vec![entry("docs/kept", "sha256:def")]),
        )
        .await
        .unwrap();

        assert_eq!(result.to_archive, vec!["docs/gone".to_string()]);
        let gone = repo.find_by_slug("docs/gone").await.unwrap().unwrap();
        assert!(
            gone.is_archived,
            "re-syncing a release must drop its removals"
        );
    }

    #[tokio::test]
    async fn a_release_managed_source_must_name_a_release() {
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::with_published(&[("test-source", "1.0.0")]);

        let err = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            versioned_request(None, vec![entry("docs/a", "sha256:abc")]),
        )
        .await
        .expect_err("omitting the release must fail rather than write somewhere invisible");

        match err {
            AppError::BadRequest(msg) => assert!(
                msg.contains("release-managed") && msg.contains("--version"),
                "the error must tell the operator what to do, got: {msg}"
            ),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    /// A source that never published a release keeps working exactly as before.
    #[tokio::test]
    async fn an_unversioned_source_still_syncs_without_a_release() {
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            versioned_request(None, vec![entry("docs/a", "sha256:abc")]),
        )
        .await
        .expect("an unversioned source needs no release");

        assert_eq!(result.to_upload.len(), 1);
        assert!(
            release_repo.registrations().is_empty(),
            "no release named means nothing to register"
        );
    }

    #[tokio::test]
    async fn a_slug_owned_by_another_source_is_rejected() {
        let foreign = Document {
            source_id: Some("other-source".to_string()),
            ..make_doc("docs/shared", "sha256:abc")
        };
        let repo = MockRepo::with_docs(vec![foreign]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();

        let err = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            versioned_request(None, vec![entry("docs/shared", "sha256:new")]),
        )
        .await
        .expect_err("the unique slug index no longer guards this, so sync must");

        match err {
            AppError::BadRequest(msg) => assert!(
                msg.contains("docs/shared") && msg.contains("other-source"),
                "the error must name the slug and its owner, got: {msg}"
            ),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn syncing_a_release_registers_it() {
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::default();

        process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            versioned_request(Some("1.2.0"), vec![entry("docs/a", "sha256:abc")]),
        )
        .await
        .unwrap();

        assert_eq!(
            release_repo.registrations(),
            vec![("test-source".to_string(), "1.2.0".to_string())],
            "the release must be catalogued so the version selector can list it"
        );
    }

    /// `--dry-run` reports the plan and writes nothing: neither the archive nor
    /// the release registration.
    #[tokio::test]
    async fn dry_run_reports_the_plan_without_writing() {
        let repo = MockRepo::with_docs(vec![make_doc_in_release(
            "docs/gone",
            "sha256:abc",
            "1.2.0",
        )]);
        let token_repo = MockServiceTokenRepo;
        let release_repo = MockReleaseRepo::with_published(&[("test-source", "1.2.0")]);

        let request = SyncRequest {
            dry_run: true,
            ..versioned_request(Some("1.2.0"), vec![])
        };

        let result = process_sync(
            &repo,
            &release_repo,
            &token_repo,
            None,
            None,
            None,
            None,
            None,
            Some("legacy"),
            request,
        )
        .await
        .unwrap();

        assert_eq!(
            result.to_archive,
            vec!["docs/gone".to_string()],
            "the plan must still be reported"
        );
        let doc = repo.find_by_slug("docs/gone").await.unwrap().unwrap();
        assert!(!doc.is_archived, "a dry run must not archive anything");
        assert!(
            release_repo.registrations().is_empty(),
            "a dry run must not register the release"
        );
    }
}
