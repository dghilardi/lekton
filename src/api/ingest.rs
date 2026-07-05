#[cfg(feature = "ssr")]
use crate::db::models::{IngestRequest, IngestResponse};
#[cfg(feature = "ssr")]
use crate::error::AppError;

#[cfg(feature = "ssr")]
use crate::db::access_level_repository::AccessLevelRepository;
#[cfg(feature = "ssr")]
use crate::db::asset_repository::AssetRepository;
#[cfg(feature = "ssr")]
use crate::db::document_version_repository::DocumentVersionRepository;
#[cfg(feature = "ssr")]
use crate::db::models::Document;
#[cfg(feature = "ssr")]
use crate::db::repository::DocumentRepository;
#[cfg(feature = "ssr")]
use crate::db::service_token_repository::ServiceTokenRepository;
#[cfg(feature = "ssr")]
use crate::rag::service::RagService;
#[cfg(feature = "ssr")]
use crate::rendering::links::{extract_asset_keys, extract_internal_links};
#[cfg(feature = "ssr")]
use crate::search::attachment_search::AttachmentSearchService;
#[cfg(feature = "ssr")]
use crate::search::client::SearchService;
#[cfg(feature = "ssr")]
use crate::storage::client::StorageClient;
#[cfg(feature = "ssr")]
use chrono::Utc;
#[cfg(feature = "ssr")]
use std::sync::Arc;

#[cfg(feature = "ssr")]
pub const SUMMARY_RECOMMENDED_MIN_CHARS: usize = 50;
#[cfg(feature = "ssr")]
pub const SUMMARY_RECOMMENDED_MAX_CHARS: usize = 200;

/// Bundles the service references needed by [`process_ingest`].
#[cfg(feature = "ssr")]
pub struct IngestContext<'a> {
    pub repo: &'a dyn DocumentRepository,
    pub asset_repo: &'a dyn AssetRepository,
    pub storage: &'a dyn StorageClient,
    pub search: Option<&'a dyn SearchService>,
    pub access_level_repo: &'a dyn AccessLevelRepository,
    pub service_token_repo: &'a dyn ServiceTokenRepository,
    pub version_repo: &'a dyn DocumentVersionRepository,
    pub rag: Option<&'a dyn RagService>,
    /// The legacy global token from the `SERVICE_TOKEN` env var (if set).
    pub legacy_token: Option<&'a str>,
}

/// Outcome returned by [`process_ingest`].
///
/// The caller decides whether to recompute RAG access levels synchronously or
/// via a background task, depending on the timeout constraints of the caller
/// (e.g. HTTP vs. a Leptos server function inside the app process).
#[cfg(feature = "ssr")]
#[derive(Debug)]
pub struct ProcessIngestOutcome {
    pub response: IngestResponse,
    /// Asset keys whose RAG access levels need recomputing. May be empty when
    /// the document references no assets or RAG is disabled.
    pub assets_to_recompute: Vec<String>,
}

#[cfg(feature = "ssr")]
fn spawn_attachment_acl_recompute(
    rag: Option<Arc<dyn RagService>>,
    asset_repo: Arc<dyn AssetRepository>,
    document_repo: Arc<dyn DocumentRepository>,
    storage: Arc<dyn StorageClient>,
    attachment_search: Option<Arc<dyn AttachmentSearchService>>,
    keys: Vec<String>,
) {
    if keys.is_empty() {
        return;
    }
    let Some(rag) = rag else {
        return;
    };

    tokio::spawn(async move {
        crate::rag::attachment_extraction::recompute_access_levels(
            rag.as_ref(),
            asset_repo.as_ref(),
            document_repo.as_ref(),
            storage.as_ref(),
            attachment_search.as_deref(),
            &keys,
        )
        .await;
    });
}

/// Core ingestion logic — separated from the HTTP layer for testability.
///
/// Validates the request, uploads content to S3, upserts metadata in MongoDB,
/// and optionally indexes the document in Meilisearch.
///
/// Access-level recomputation for referenced assets is NOT performed here;
/// instead the list of keys to recompute is returned in [`ProcessIngestOutcome`]
/// so the caller can choose sync vs. async execution.
#[cfg(feature = "ssr")]
pub async fn process_ingest(
    ctx: &IngestContext<'_>,
    request: IngestRequest,
) -> Result<ProcessIngestOutcome, AppError> {
    // 1. Validate the service token (legacy or scoped)
    validate_token(ctx, &request.service_token, &request.slug).await?;

    // 2. Validate the slug
    if request.slug.is_empty() {
        return Err(AppError::BadRequest("Slug cannot be empty".into()));
    }
    if request.slug.contains("..") {
        return Err(AppError::BadRequest("Slug must not contain '..'".into()));
    }
    if request.slug.starts_with('/') {
        return Err(AppError::BadRequest("Slug must not start with '/'".into()));
    }
    let normalized_parent_slug = normalize_parent_slug(request.parent_slug.as_deref())?;
    if normalized_parent_slug.as_deref() == Some(request.slug.as_str()) {
        return Err(AppError::BadRequest(
            "Parent slug must not equal document slug".into(),
        ));
    }
    let summary = normalize_summary(request.summary.as_deref());
    warn_about_summary(&request.slug, summary.as_deref());

    // 3. Validate the access_level name exists in the registry.
    if request.access_level.trim().is_empty() {
        return Err(AppError::BadRequest("Access level cannot be empty".into()));
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
    let new_metadata_hash = compute_metadata_hash(MetadataHashInput {
        title: &request.title,
        summary: summary.as_deref(),
        access_level: &access_level,
        service_owner: &request.service_owner,
        tags: &request.tags,
        parent_slug: normalized_parent_slug.as_deref(),
        order: request.order,
        is_hidden: request.is_hidden,
    });

    // 5. Extract internal links from content
    let links_out = extract_internal_links(&request.content);

    // 6. Get old document to diff backlinks and detect changes.
    //    If the slug is new or archived, check for an in-place rename via source_path:
    //    a doc with the same source_path + source_id but a different slug was renamed.
    let by_slug = ctx.repo.find_by_slug(&request.slug).await?;
    let (old_doc, old_s3_key_before_rename) =
        if by_slug.as_ref().map(|d| d.is_archived).unwrap_or(true) {
            let by_source = ctx.repo.find_by_source_path(&request.source_path).await?;
            if let Some(found) = by_source {
                if found.slug != request.slug
                    && found.source_id.as_deref() == Some(request.source_id.as_str())
                    && !found.is_archived
                {
                    ctx.repo.rename_slug(&found.slug, &request.slug).await?;
                    let old_key = found.s3_key.clone();
                    let renamed = Document {
                        slug: request.slug.clone(),
                        ..found
                    };
                    (Some(renamed), Some(old_key))
                } else {
                    (by_slug, None)
                }
            } else {
                (by_slug, None)
            }
        } else {
            (by_slug, None)
        };

    let (old_links, old_backlinks, old_hash) = match &old_doc {
        Some(d) => (
            d.links_out.clone(),
            d.backlinks.clone(),
            d.content_hash.clone(),
        ),
        None => (vec![], vec![], None),
    };

    // Reject cross-source overwrites of non-archived documents.
    if let Some(ref existing) = old_doc {
        if !existing.is_archived {
            if let Some(ref existing_source_id) = existing.source_id {
                if existing_source_id != &request.source_id {
                    return Err(AppError::Forbidden(format!(
                        "Document '{}' belongs to source '{}' and cannot be overwritten by source '{}'",
                        request.slug, existing_source_id, request.source_id
                    )));
                }
            }
        }
    }

    let source_path_changed = old_doc
        .as_ref()
        .is_none_or(|d| d.source_path.as_deref() != Some(&request.source_path));

    let source_id_changed = old_doc
        .as_ref()
        .is_none_or(|d| d.source_id.as_deref() != Some(&request.source_id));

    let content_changed = old_hash.as_deref() != Some(&new_hash);

    // A document whose previous ingest failed to index is re-processed even when
    // nothing else changed, so the retry re-runs indexing and clears the flag.
    let old_doc_needs_reindex = old_doc.as_ref().is_some_and(|d| d.needs_reindex);

    // The request is authoritative: use its values directly.
    // This allows clearing is_hidden, order=0, and parent_slug=None via the sync path.
    let effective_parent_slug = normalized_parent_slug;
    let effective_order = request.order;
    let effective_is_hidden = request.is_hidden;

    // Check if metadata changed (compared to existing doc)
    let metadata_changed = old_doc.as_ref().is_none_or(|d| {
        d.title != request.title
            || d.summary != summary
            || d.access_level != access_level
            || d.is_draft != request.is_draft
            || d.service_owner != request.service_owner
            || d.tags != request.tags
            || d.parent_slug != effective_parent_slug
            || d.order != effective_order
            || d.is_hidden != effective_is_hidden
            || d.links_out != links_out
    });

    // If nothing changed, return early (but not if we just renamed the slug)
    if !content_changed
        && !metadata_changed
        && !source_path_changed
        && !source_id_changed
        && old_s3_key_before_rename.is_none()
        && !old_doc_needs_reindex
    {
        let s3_key = format!("docs/{}.md", request.slug.replace('/', "_"));
        return Ok(ProcessIngestOutcome {
            response: IngestResponse {
                message: "Document unchanged".to_string(),
                slug: request.slug,
                s3_key,
                changed: false,
                indexed: true,
            },
            assets_to_recompute: vec![],
        });
    }

    // 7. Build the S3 key.
    //    On a pure slug rename with no content change, reuse the existing S3 key
    //    so we don't reference a non-existent object.
    let s3_key = if let Some(ref old_key) = old_s3_key_before_rename {
        if content_changed {
            format!("docs/{}.md", request.slug.replace('/', "_"))
        } else {
            old_key.clone()
        }
    } else {
        format!("docs/{}.md", request.slug.replace('/', "_"))
    };

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

    // 9. Build the document. `needs_reindex` is set below from the indexing outcome.
    let mut doc = Document {
        slug: request.slug.clone(),
        title: request.title,
        summary,
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
        source_path: Some(request.source_path.clone()),
        source_id: Some(request.source_id.clone()),
        needs_reindex: false,
        skip_rag: request.skip_rag,
    };

    // 10. (Re)index in Meilisearch + RAG *before* persisting, so the stored
    //     document records whether it is in sync with the indexes. A transient
    //     search/embedding outage now leaves a durable `needs_reindex` flag and
    //     an `indexed: false` response instead of silently drifting from MongoDB.
    let search_doc = ctx
        .search
        .as_ref()
        .map(|_| crate::search::client::build_search_document(&doc, &raw_content));

    let mut indexed_ok = true;

    if let (Some(search_svc), Some(search_doc)) = (ctx.search, search_doc) {
        if let Err(e) = search_svc.index_document(&search_doc).await {
            tracing::warn!(slug = %doc.slug, "Failed to index document in search: {e}");
            indexed_ok = false;
        }
    }

    if let Some(rag) = ctx.rag {
        if doc.skip_rag {
            // Deliberately excluded from RAG (e.g. PDF upload stub — the linked
            // attachment is indexed instead). Delete any chunks a previous
            // ingest may have left, so opting out cleans up stale vectors.
            if let Err(e) = rag.delete_document(&doc.slug).await {
                tracing::warn!(slug = %doc.slug, "Failed to remove RAG chunks for skip_rag document: {e}");
                indexed_ok = false;
            }
        } else if let Err(e) = rag
            .index_document(
                &doc.slug,
                &doc.title,
                &raw_content,
                &doc.access_level,
                doc.is_draft,
                &doc.tags,
            )
            .await
        {
            tracing::warn!(slug = %doc.slug, "Failed to index document in RAG: {e}");
            indexed_ok = false;
        }
    }

    doc.needs_reindex = !indexed_ok;

    // 11. Upsert document metadata in MongoDB (records `needs_reindex`).
    ctx.repo.create_or_update(doc).await?;

    // 12. Update backlinks on referenced documents.
    //     Note: this is not atomic with the create_or_update above.
    //     Both operations are idempotent, so partial failure leaves
    //     consistent (if stale) state that self-heals on re-ingest.
    ctx.repo
        .update_backlinks(&request.slug, &old_links, &links_out)
        .await?;

    // 13. Reconcile asset references so each referenced asset records this
    //     document in its `referenced_by` (and drops it where no longer linked).
    //     This drives attachment access levels for RAG and asset-serve access.
    let asset_keys = extract_asset_keys(&raw_content);
    let assets_to_recompute = match ctx
        .asset_repo
        .set_references(&request.slug, &asset_keys)
        .await
    {
        Ok(affected) => {
            // Collect the full set of asset keys to recompute: changed references
            // plus all currently-referenced assets (so an access_level change on
            // the document propagates to existing attachment chunks even when the
            // referenced set did not change). Only populate when RAG is enabled.
            if ctx.rag.is_some() {
                let mut to_recompute = affected;
                for key in &asset_keys {
                    if !to_recompute.contains(key) {
                        to_recompute.push(key.clone());
                    }
                }
                to_recompute
            } else {
                vec![]
            }
        }
        Err(e) => {
            tracing::warn!(slug = %request.slug, "Failed to update asset references: {e}");
            vec![]
        }
    };

    Ok(ProcessIngestOutcome {
        response: IngestResponse {
            message: "Document ingested successfully".to_string(),
            slug: request.slug,
            s3_key,
            changed: true,
            indexed: indexed_ok,
        },
        assets_to_recompute,
    })
}

/// Input for [`compute_metadata_hash`].
#[cfg(feature = "ssr")]
pub(crate) struct MetadataHashInput<'a> {
    pub title: &'a str,
    pub summary: Option<&'a str>,
    pub access_level: &'a str,
    pub service_owner: &'a str,
    pub tags: &'a [String],
    pub parent_slug: Option<&'a str>,
    pub order: u32,
    pub is_hidden: bool,
}

/// Build a canonical JSON object from document metadata and hash it.
///
/// Uses a BTreeMap so keys are always alphabetically sorted, ensuring a
/// deterministic representation identical to what `lekton-sync` (the CLI) computes.
///
/// Fields included: title, summary, access_level (already lowercase), service_owner,
/// tags (comma-joined, sorted), parent_slug, order, is_hidden.
/// `is_draft` and `source_id` are intentionally excluded (not sent by the CLI).
#[cfg(feature = "ssr")]
pub(crate) fn compute_metadata_hash(input: MetadataHashInput<'_>) -> String {
    use std::collections::BTreeMap;
    let mut sorted_tags: Vec<&str> = input.tags.iter().map(|s| s.as_str()).collect();
    sorted_tags.sort_unstable();
    let tags_str = sorted_tags.join(",");
    let order_str = input.order.to_string();
    let is_hidden_str = input.is_hidden.to_string();
    let mut map = BTreeMap::new();
    map.insert("access_level", input.access_level);
    map.insert("is_hidden", is_hidden_str.as_str());
    map.insert("order", order_str.as_str());
    map.insert("parent_slug", input.parent_slug.unwrap_or(""));
    map.insert("service_owner", input.service_owner);
    map.insert("summary", input.summary.unwrap_or(""));
    map.insert("tags", tags_str.as_str());
    map.insert("title", input.title);
    let canonical =
        serde_json::to_string(&map).expect("BTreeMap<&str,&str> serialization is infallible");
    format!(
        "sha256:{}",
        crate::auth::token_service::TokenService::hash_token(&canonical)
    )
}

#[cfg(feature = "ssr")]
fn normalize_summary(summary: Option<&str>) -> Option<String> {
    summary
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(feature = "ssr")]
fn normalize_parent_slug(parent_slug: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(parent_slug) = parent_slug
        .map(str::trim)
        .filter(|parent| !parent.is_empty())
    else {
        return Ok(None);
    };

    if parent_slug.contains("..") {
        return Err(AppError::BadRequest(
            "Parent slug must not contain '..'".into(),
        ));
    }
    if parent_slug.starts_with('/') {
        return Err(AppError::BadRequest(
            "Parent slug must not start with '/'".into(),
        ));
    }
    if parent_slug.ends_with('/') {
        return Err(AppError::BadRequest(
            "Parent slug must not end with '/'".into(),
        ));
    }

    Ok(Some(parent_slug.to_owned()))
}

#[cfg(feature = "ssr")]
fn warn_about_summary(slug: &str, summary: Option<&str>) {
    match summary {
        None => tracing::warn!(
            slug,
            min = SUMMARY_RECOMMENDED_MIN_CHARS,
            max = SUMMARY_RECOMMENDED_MAX_CHARS,
            "Ingesting document without summary"
        ),
        Some(summary) => {
            let len = summary.chars().count();
            if !(SUMMARY_RECOMMENDED_MIN_CHARS..=SUMMARY_RECOMMENDED_MAX_CHARS).contains(&len) {
                tracing::warn!(
                    slug,
                    len,
                    min = SUMMARY_RECOMMENDED_MIN_CHARS,
                    max = SUMMARY_RECOMMENDED_MAX_CHARS,
                    "Document summary length is outside the recommended range"
                );
            }
        }
    }
}

/// Validate the service token — either legacy global token or scoped token.
#[cfg(feature = "ssr")]
async fn validate_token(
    ctx: &IngestContext<'_>,
    raw_token: &str,
    slug: &str,
) -> Result<(), AppError> {
    let resolved = crate::api::token_validation::resolve_service_token(
        ctx.service_token_repo,
        ctx.legacy_token,
        raw_token,
    )
    .await?;

    if !resolved.can_write {
        return Err(AppError::Forbidden(
            "Token does not have write permission".into(),
        ));
    }

    if !crate::api::sync::scope_matches_any(slug, &resolved.scopes) {
        return Err(AppError::Forbidden(
            "Token does not have access to this document scope".into(),
        ));
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
        asset_repo: state.asset_repo.as_ref(),
        storage: state.storage_client.as_ref(),
        search: state.search_service.as_deref(),
        access_level_repo: state.access_level_repo.as_ref(),
        service_token_repo: state.service_token_repo.as_ref(),
        version_repo: state.document_version_repo.as_ref(),
        rag: state.rag_service.as_deref(),
        legacy_token: Some(&state.service_token),
    };

    let outcome = process_ingest(&ctx, request).await?;

    spawn_attachment_acl_recompute(
        state.rag_service.clone(),
        state.asset_repo.clone(),
        state.document_repo.clone(),
        state.storage_client.clone(),
        state.attachment_search_service.clone(),
        outcome.assets_to_recompute,
    );

    Ok(axum::Json(outcome.response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Notify;
    use tokio::time::{timeout, Duration};

    use crate::db::access_level_repository::AccessLevelRepository;
    use crate::db::asset_repository::ExtractionUpdate;
    use crate::db::auth_models::AccessLevelEntity;
    use crate::db::models::{Asset, ExtractionStatus};
    use crate::db::service_token_models::ServiceToken;
    use crate::db::service_token_repository::ServiceTokenRepository;
    use crate::test_utils::MockStorage;

    /// A mock access level repo that accepts any non-empty level name.
    struct MockAccessLevelRepo;

    #[async_trait]
    impl AccessLevelRepository for MockAccessLevelRepo {
        async fn create(&self, _level: AccessLevelEntity) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_name(&self, _name: &str) -> Result<Option<AccessLevelEntity>, AppError> {
            Ok(None)
        }
        async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError> {
            Ok(vec![])
        }
        async fn update(&self, _level: AccessLevelEntity) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete(&self, _name: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn exists(&self, _name: &str) -> Result<bool, AppError> {
            Ok(true)
        }
        async fn seed_defaults(&self) -> Result<(), AppError> {
            Ok(())
        }
        async fn compute_effective_levels(
            &self,
            roots: &[String],
        ) -> Result<Vec<String>, AppError> {
            Ok(roots.to_vec())
        }
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
            Ok(self
                .tokens
                .lock()
                .unwrap()
                .iter()
                .find(|t| t.token_hash == token_hash)
                .cloned())
        }
        async fn find_by_name(&self, name: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(self
                .tokens
                .lock()
                .unwrap()
                .iter()
                .find(|t| t.name == name)
                .cloned())
        }
        async fn find_by_id(&self, id: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(self
                .tokens
                .lock()
                .unwrap()
                .iter()
                .find(|t| t.id == id)
                .cloned())
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
        async fn check_scope_overlap(
            &self,
            _scopes: &[String],
            _exclude_id: Option<&str>,
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

    fn make_request(token: &str, slug: &str) -> IngestRequest {
        IngestRequest {
            service_token: token.to_string(),
            source_path: format!("{slug}.md"),
            source_id: "test-source-id".to_string(),
            slug: slug.to_string(),
            title: "Test Doc".to_string(),
            summary: Some("A test document used to exercise ingestion behavior.".to_string()),
            content: "# Hello\nWorld".to_string(),
            access_level: "internal".to_string(),
            is_draft: false,
            service_owner: "test-team".to_string(),
            tags: vec!["test".to_string()],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            skip_rag: false,
        }
    }

    struct MockVersionRepo;

    #[async_trait]
    impl crate::db::document_version_repository::DocumentVersionRepository for MockVersionRepo {
        async fn create(
            &self,
            _: crate::db::document_version_repository::DocumentVersion,
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_latest(
            &self,
            _: &str,
        ) -> Result<Option<crate::db::document_version_repository::DocumentVersion>, AppError>
        {
            Ok(None)
        }
        async fn list_by_slug(
            &self,
            _: &str,
        ) -> Result<Vec<crate::db::document_version_repository::DocumentVersion>, AppError>
        {
            Ok(vec![])
        }
        async fn next_version_number(&self, _: &str) -> Result<u64, AppError> {
            Ok(1)
        }
    }

    /// Stateless no-op asset repository for ingest tests that don't assert on
    /// asset reference maintenance.
    struct NoopAssetRepo;

    #[async_trait]
    impl crate::db::asset_repository::AssetRepository for NoopAssetRepo {
        async fn create_or_update(&self, _: crate::db::models::Asset) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_key(&self, _: &str) -> Result<Option<crate::db::models::Asset>, AppError> {
            Ok(None)
        }
        async fn list_all(&self) -> Result<Vec<crate::db::models::Asset>, AppError> {
            Ok(vec![])
        }
        async fn list_by_prefix(&self, _: &str) -> Result<Vec<crate::db::models::Asset>, AppError> {
            Ok(vec![])
        }
        async fn delete(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn update_extraction(
            &self,
            _: &str,
            _: crate::db::asset_repository::ExtractionUpdate,
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn set_references(&self, _: &str, _: &[String]) -> Result<Vec<String>, AppError> {
            Ok(vec![])
        }
    }

    struct StaticAssetRepo {
        asset: Mutex<Option<Asset>>,
    }

    impl StaticAssetRepo {
        fn new(asset: Asset) -> Self {
            Self {
                asset: Mutex::new(Some(asset)),
            }
        }
    }

    #[async_trait]
    impl crate::db::asset_repository::AssetRepository for StaticAssetRepo {
        async fn create_or_update(&self, _: Asset) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_key(&self, key: &str) -> Result<Option<Asset>, AppError> {
            Ok(self
                .asset
                .lock()
                .unwrap()
                .clone()
                .filter(|asset| asset.key == key))
        }
        async fn list_all(&self) -> Result<Vec<Asset>, AppError> {
            Ok(self.asset.lock().unwrap().clone().into_iter().collect())
        }
        async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Asset>, AppError> {
            Ok(self
                .asset
                .lock()
                .unwrap()
                .clone()
                .into_iter()
                .filter(|asset| asset.key.starts_with(prefix))
                .collect())
        }
        async fn delete(&self, key: &str) -> Result<(), AppError> {
            let mut guard = self.asset.lock().unwrap();
            if guard.as_ref().map(|asset| asset.key.as_str()) == Some(key) {
                *guard = None;
            }
            Ok(())
        }
        async fn update_extraction(
            &self,
            _key: &str,
            _update: ExtractionUpdate,
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn set_references(&self, _: &str, _: &[String]) -> Result<Vec<String>, AppError> {
            Ok(vec![])
        }
    }

    fn make_ctx<'a>(
        repo: &'a MockRepo,
        storage: &'a MockStorage,
        token_repo: &'a dyn ServiceTokenRepository,
        legacy_token: Option<&'a str>,
    ) -> IngestContext<'a> {
        IngestContext {
            repo,
            asset_repo: &NoopAssetRepo,
            storage,
            search: None,
            access_level_repo: &MockAccessLevelRepo,
            service_token_repo: token_repo,
            version_repo: &MockVersionRepo,
            rag: None,
            legacy_token,
        }
    }

    /// A RAG service whose `index_document` always fails, to exercise the
    /// partial-indexing path (BUG-6).
    struct FailingRagService;

    #[async_trait]
    impl crate::rag::service::RagService for FailingRagService {
        async fn index_document(
            &self,
            _slug: &str,
            _title: &str,
            _content: &str,
            _access_level: &str,
            _is_draft: bool,
            _tags: &[String],
        ) -> Result<(), AppError> {
            Err(AppError::Internal("rag down".to_string()))
        }
        async fn delete_document(&self, _slug: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn index_attachment(
            &self,
            _attachment_key: &str,
            _filename: &str,
            _pages: &[crate::rag::service::AttachmentPage],
            _access_levels: &[String],
            _tags: &[String],
        ) -> Result<usize, AppError> {
            Ok(0)
        }
        async fn delete_attachment(&self, _attachment_key: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn update_attachment_access_levels(
            &self,
            _attachment_key: &str,
            _access_levels: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_ingest_index_failure_flags_needs_reindex() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let mut ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let failing_rag = FailingRagService;
        ctx.rag = Some(&failing_rag);

        let outcome = process_ingest(&ctx, make_request("valid-token", "docs/hello"))
            .await
            .unwrap();

        // The document is persisted but the caller is told indexing did not succeed.
        assert!(!outcome.response.indexed);
        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert!(doc.needs_reindex);
    }

    /// A RAG service that records whether each method was invoked, so tests can
    /// assert that `skip_rag` documents bypass indexing.
    #[derive(Default)]
    struct RecordingRagService {
        indexed: std::sync::atomic::AtomicBool,
        deleted: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl crate::rag::service::RagService for RecordingRagService {
        async fn index_document(
            &self,
            _slug: &str,
            _title: &str,
            _content: &str,
            _access_level: &str,
            _is_draft: bool,
            _tags: &[String],
        ) -> Result<(), AppError> {
            self.indexed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn delete_document(&self, _slug: &str) -> Result<(), AppError> {
            self.deleted
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn index_attachment(
            &self,
            _attachment_key: &str,
            _filename: &str,
            _pages: &[crate::rag::service::AttachmentPage],
            _access_levels: &[String],
            _tags: &[String],
        ) -> Result<usize, AppError> {
            Ok(0)
        }
        async fn delete_attachment(&self, _attachment_key: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn update_attachment_access_levels(
            &self,
            _attachment_key: &str,
            _access_levels: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
    }

    struct BlockingAclRagService {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl crate::rag::service::RagService for BlockingAclRagService {
        async fn index_document(
            &self,
            _slug: &str,
            _title: &str,
            _content: &str,
            _access_level: &str,
            _is_draft: bool,
            _tags: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_document(&self, _slug: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn index_attachment(
            &self,
            _attachment_key: &str,
            _filename: &str,
            _pages: &[crate::rag::service::AttachmentPage],
            _access_levels: &[String],
            _tags: &[String],
        ) -> Result<usize, AppError> {
            Ok(0)
        }
        async fn delete_attachment(&self, _attachment_key: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn update_attachment_access_levels(
            &self,
            _attachment_key: &str,
            _access_levels: &[String],
        ) -> Result<(), AppError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_ingest_skip_rag_bypasses_rag_indexing() {
        use std::sync::atomic::Ordering::SeqCst;

        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let mut ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let rag = RecordingRagService::default();
        ctx.rag = Some(&rag);

        let mut request = make_request("valid-token", "docs/hello");
        request.skip_rag = true;
        let outcome = process_ingest(&ctx, request).await.unwrap();

        // RAG indexing is skipped; any pre-existing chunks are cleaned up instead.
        assert!(
            !rag.indexed.load(SeqCst),
            "skip_rag must not index the document in RAG"
        );
        assert!(
            rag.deleted.load(SeqCst),
            "skip_rag should delete any pre-existing RAG chunks"
        );
        // Skipping RAG is intentional, not a failure, so the doc stays in sync.
        assert!(outcome.response.indexed);
        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert!(!doc.needs_reindex);
        assert!(doc.skip_rag);
    }

    #[tokio::test]
    async fn test_unchanged_doc_is_reprocessed_when_needs_reindex() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();

        // First ingest fails to index → doc flagged needs_reindex.
        {
            let mut ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
            let failing_rag = FailingRagService;
            ctx.rag = Some(&failing_rag);
            process_ingest(&ctx, make_request("valid-token", "docs/hello"))
                .await
                .unwrap();
        }

        // Second ingest with identical content, RAG now healthy (absent): the
        // unchanged doc is re-processed (not skipped) and the flag is cleared.
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));
        let outcome = process_ingest(&ctx, make_request("valid-token", "docs/hello"))
            .await
            .unwrap();

        assert!(outcome.response.changed);
        assert!(outcome.response.indexed);
        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert!(!doc.needs_reindex);
    }

    fn make_scoped_token(raw_token: &str, scopes: Vec<&str>) -> ServiceToken {
        use crate::auth::token_service::TokenService;
        ServiceToken {
            id: "st-1".to_string(),
            name: "test-scoped".to_string(),
            token_hash: TokenService::hash_token(raw_token),
            allowed_scopes: scopes.into_iter().map(String::from).collect(),
            token_type: "service".to_string(),
            user_id: None,
            can_write: true,
            access_levels: None,
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

        let outcome = result.unwrap();
        assert_eq!(outcome.response.slug, "docs/hello");
        assert!(outcome.response.s3_key.contains("docs_hello"));

        // Verify content was stored
        let stored = storage
            .objects
            .lock()
            .unwrap()
            .get(&outcome.response.s3_key)
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
    async fn spawn_attachment_acl_recompute_returns_before_blocking_acl_update_finishes() {
        let repo = Arc::new(MockRepo::new());
        repo.create_or_update(Document {
            slug: "docs/hello".to_string(),
            title: "Hello".to_string(),
            summary: None,
            s3_key: "docs_hello.md".to_string(),
            access_level: "internal".to_string(),
            is_draft: false,
            service_owner: "test-team".to_string(),
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
            source_path: Some("docs/hello.md".to_string()),
            source_id: Some("test-source-id".to_string()),
            needs_reindex: false,
            skip_rag: false,
        })
        .await
        .unwrap();
        let asset_repo = Arc::new(StaticAssetRepo::new(Asset {
            key: "pdfs/hello.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            size_bytes: 42,
            s3_key: "assets/pdfs/hello.pdf".to_string(),
            uploaded_at: Utc::now(),
            uploaded_by: "tester@example.com".to_string(),
            referenced_by: vec!["docs/hello".to_string()],
            content_hash: None,
            extraction_status: ExtractionStatus::Done,
            extraction_error: None,
            extracted_content_hash: None,
            extracted_at: None,
            indexed_chunks: Some(1),
        }));
        let storage = Arc::new(MockStorage::new());
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let rag = Arc::new(BlockingAclRagService {
            started: started.clone(),
            release: release.clone(),
        });

        spawn_attachment_acl_recompute(
            Some(rag),
            asset_repo,
            repo,
            storage,
            None,
            vec!["pdfs/hello.pdf".to_string()],
        );

        timeout(Duration::from_millis(50), started.notified())
            .await
            .expect("background recompute should start promptly");
        release.notify_waiters();
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

    #[tokio::test]
    async fn test_ingest_rejects_self_referential_parent_slug() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let mut request = make_request("valid-token", "docs/hello");
        request.parent_slug = Some("docs/hello".to_string());

        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => {
                assert!(msg.contains("Parent slug must not equal document slug"))
            }
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_rejects_parent_slug_path_traversal() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let mut request = make_request("valid-token", "docs/hello");
        request.parent_slug = Some("../parent".to_string());

        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => {
                assert!(msg.contains("Parent slug must not contain '..'"))
            }
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_rejects_absolute_parent_slug() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let mut request = make_request("valid-token", "docs/hello");
        request.parent_slug = Some("/parent".to_string());

        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => {
                assert!(msg.contains("Parent slug must not start with '/'"))
            }
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_rejects_parent_slug_with_trailing_slash() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let mut request = make_request("valid-token", "docs/hello");
        request.parent_slug = Some("parent/".to_string());

        let result = process_ingest(&ctx, request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => {
                assert!(msg.contains("Parent slug must not end with '/'"))
            }
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_trims_parent_slug() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let mut request = make_request("valid-token", "docs/hello");
        request.parent_slug = Some("  parent-doc  ".to_string());

        process_ingest(&ctx, request).await.unwrap();

        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert_eq!(doc.parent_slug.as_deref(), Some("parent-doc"));
    }

    #[tokio::test]
    async fn test_ingest_blank_parent_slug_is_treated_as_absent() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let mut request = make_request("valid-token", "docs/hello");
        request.parent_slug = Some("   ".to_string());

        process_ingest(&ctx, request).await.unwrap();

        let doc = repo.find_by_slug("docs/hello").await.unwrap().unwrap();
        assert_eq!(doc.parent_slug, None);
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
        assert!(r1.response.changed);
        assert_eq!(
            storage.put_count.load(std::sync::atomic::Ordering::Relaxed),
            1
        );

        // Second ingest with identical content and metadata
        let request2 = make_request("valid-token", "docs/hello");
        let r2 = process_ingest(&ctx, request2).await.unwrap();
        assert!(!r2.response.changed);
        // S3 upload should NOT have happened again
        assert_eq!(
            storage.put_count.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
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
        assert!(r2.response.changed);
        // 3 puts: initial upload + history copy + new upload
        assert_eq!(
            storage.put_count.load(std::sync::atomic::Ordering::Relaxed),
            3
        );
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
        assert!(r2.response.changed);
        // S3 upload should NOT happen (content is the same)
        assert_eq!(
            storage.put_count.load(std::sync::atomic::Ordering::Relaxed),
            1
        );

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
        let hash1 = repo
            .find_by_slug("docs/hello")
            .await
            .unwrap()
            .unwrap()
            .metadata_hash
            .unwrap();

        let mut request2 = make_request("valid-token", "docs/hello");
        request2.access_level = "public".to_string();
        process_ingest(&ctx, request2).await.unwrap();
        let hash2 = repo
            .find_by_slug("docs/hello")
            .await
            .unwrap()
            .unwrap()
            .metadata_hash
            .unwrap();

        assert_ne!(
            hash1, hash2,
            "metadata_hash must change when access_level changes"
        );
    }

    #[tokio::test]
    async fn test_ingest_metadata_hash_stable_when_nothing_changes() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let request1 = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request1).await.unwrap();
        let hash1 = repo
            .find_by_slug("docs/hello")
            .await
            .unwrap()
            .unwrap()
            .metadata_hash
            .unwrap();

        // Second ingest with identical data — unchanged, no DB write
        let request2 = make_request("valid-token", "docs/hello");
        process_ingest(&ctx, request2).await.unwrap();
        let hash2 = repo
            .find_by_slug("docs/hello")
            .await
            .unwrap()
            .unwrap()
            .metadata_hash
            .unwrap();

        assert_eq!(
            hash1, hash2,
            "metadata_hash must be stable when nothing changes"
        );
    }

    #[tokio::test]
    async fn test_ingest_cross_source_overwrite_is_rejected() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        // First ingest: source-a claims docs/shared
        let mut req_a = make_request("valid-token", "docs/shared");
        req_a.source_id = "source-a".to_string();
        process_ingest(&ctx, req_a).await.unwrap();

        // Second ingest: source-b tries to overwrite docs/shared → must be rejected
        let mut req_b = make_request("valid-token", "docs/shared");
        req_b.source_id = "source-b".to_string();
        let result = process_ingest(&ctx, req_b).await;
        match result.unwrap_err() {
            AppError::Forbidden(msg) => assert!(msg.contains("source-a")),
            other => panic!("Expected Forbidden, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_cross_source_allowed_after_archive() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        // source-a creates docs/migrated, then it gets archived
        let mut req_a = make_request("valid-token", "docs/migrated");
        req_a.source_id = "source-a".to_string();
        process_ingest(&ctx, req_a).await.unwrap();
        repo.set_archived("docs/migrated", true).await.unwrap();

        // source-b can now claim the slug (the document was archived by source-a)
        let mut req_b = make_request("valid-token", "docs/migrated");
        req_b.source_id = "source-b".to_string();
        let result = process_ingest(&ctx, req_b).await;
        assert!(
            result.is_ok(),
            "source-b should be able to claim an archived slug"
        );
    }

    /// Regression tests for BUG-4: is_hidden, order, and parent_slug must be clearable via ingest.
    /// Previously a "keep old value" fallback made these fields impossible to unset.
    #[tokio::test]
    async fn test_ingest_clears_is_hidden_when_request_says_false() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        // First ingest: hidden doc
        let mut req = make_request("valid-token", "docs/hidden");
        req.is_hidden = true;
        process_ingest(&ctx, req).await.unwrap();
        let doc = repo.find_by_slug("docs/hidden").await.unwrap().unwrap();
        assert!(
            doc.is_hidden,
            "document should be hidden after first ingest"
        );

        // Second ingest: un-hide
        let mut req2 = make_request("valid-token", "docs/hidden");
        req2.is_hidden = false;
        process_ingest(&ctx, req2).await.unwrap();
        let doc2 = repo.find_by_slug("docs/hidden").await.unwrap().unwrap();
        assert!(!doc2.is_hidden, "is_hidden must be clearable via ingest");
    }

    #[tokio::test]
    async fn test_ingest_clears_order_when_request_sends_zero() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let mut req = make_request("valid-token", "docs/ordered");
        req.order = 5;
        process_ingest(&ctx, req).await.unwrap();
        let doc = repo.find_by_slug("docs/ordered").await.unwrap().unwrap();
        assert_eq!(doc.order, 5);

        // Reset order to 0
        let mut req2 = make_request("valid-token", "docs/ordered");
        req2.order = 0;
        process_ingest(&ctx, req2).await.unwrap();
        let doc2 = repo.find_by_slug("docs/ordered").await.unwrap().unwrap();
        assert_eq!(doc2.order, 0, "order must be clearable to 0 via ingest");
    }

    #[tokio::test]
    async fn test_ingest_clears_parent_slug_when_request_sends_none() {
        let storage = MockStorage::new();
        let repo = MockRepo::new();
        let token_repo = MockServiceTokenRepo::new();
        let ctx = make_ctx(&repo, &storage, &token_repo, Some("valid-token"));

        let mut req = make_request("valid-token", "docs/child");
        req.parent_slug = Some("docs/parent".to_string());
        process_ingest(&ctx, req).await.unwrap();
        let doc = repo.find_by_slug("docs/child").await.unwrap().unwrap();
        assert_eq!(doc.parent_slug.as_deref(), Some("docs/parent"));

        // Remove parent
        let mut req2 = make_request("valid-token", "docs/child");
        req2.parent_slug = None;
        process_ingest(&ctx, req2).await.unwrap();
        let doc2 = repo.find_by_slug("docs/child").await.unwrap().unwrap();
        assert!(
            doc2.parent_slug.is_none(),
            "parent_slug must be clearable via ingest"
        );
    }

    /// Shared wire vector with `cli/src/hash.rs::document_metadata_hash_wire_vector`.
    /// If the canonical string format ever changes on one side, this test catches it.
    #[cfg(feature = "ssr")]
    #[test]
    fn document_metadata_hash_wire_vector() {
        let got = compute_metadata_hash(MetadataHashInput {
            title: "Test Doc",
            summary: Some("A test document"),
            access_level: "internal",
            service_owner: "platform",
            tags: &["rust".to_string(), "web".to_string()],
            parent_slug: Some("guides"),
            order: 5,
            is_hidden: false,
        });
        assert_eq!(
            got, "sha256:zwiTusSDUfQZa8E3I2cGxlQ21XSoiQW4u3R8GgXT0bc",
            "document metadata hash wire contract with CLI"
        );
    }
}
