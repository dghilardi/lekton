use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::db::asset_repository::AssetRepository;
use crate::db::models::{Asset, Document};
use crate::db::repository::DocumentRepository;
use crate::indexing::{index_everywhere, remove_everywhere, IndexTargets};
use crate::rag::attachment_extraction::AttachmentExtractionService;
use crate::rag::service::RagService;
use crate::search::client::SearchService;
use crate::storage::client::StorageClient;

/// Shared state for tracking a background re-index operation.
#[derive(Default)]
pub struct ReindexState {
    pub is_running: AtomicBool,
    /// Progress percentage (0–100).
    pub progress: AtomicU32,
    /// Per-run failed/skipped counters and last error.
    pub outcome: crate::jobs::JobOutcome,
    /// Slugs of documents that failed in the last run, for a targeted retry.
    pub failed_docs: Mutex<Vec<String>>,
    /// Keys of attachments that failed in the last run, for a targeted retry.
    pub failed_assets: Mutex<Vec<String>>,
}

impl crate::jobs::RunningFlag for ReindexState {
    fn is_running(&self) -> &AtomicBool {
        &self.is_running
    }
}

/// Run a full re-index of all non-archived documents, plus every referenced
/// PDF attachment (force-reprocessed regardless of content-hash match), so
/// documents uploaded before extraction was wired up — or a changed
/// chunking/embedding/extraction configuration — are picked up without
/// requiring a no-op re-upload of each file.
///
/// Afterwards, chunks of documents and attachments MongoDB no longer accounts
/// for (renamed or deleted slugs, unreferenced attachments, points written
/// before `source_kind` existed) are swept from the vector store.
///
/// This function is meant to be spawned as a background Tokio task.
/// It updates `state` with progress as it goes and resets `is_running`
/// on completion (or failure).
pub async fn run_reindex(
    reindex: Arc<ReindexState>,
    document_repo: Arc<dyn DocumentRepository>,
    storage: Arc<dyn StorageClient>,
    rag: Arc<dyn RagService>,
    asset_repo: Arc<dyn AssetRepository>,
    attachment_service: Option<Arc<AttachmentExtractionService>>,
    search: Option<Arc<dyn SearchService>>,
) {
    // Reset `is_running` unconditionally when this task ends, even on an early
    // return or panic, so a crashed reindex cannot block all future runs.
    let _guard = crate::jobs::RunningGuard::new(reindex.clone());

    // Load all non-archived documents (None = no access level filter, true = include drafts)
    // Indexing always follows `latest`: older releases mostly repeat content
    // that is already indexed (identical bodies are deduplicated by hash), so
    // embedding them would spend vectors on duplicates.
    // A sweep is only safe against a complete listing of both collections.
    let mut listed = true;
    // The latest copy of every slug, hidden ones included: they are removed
    // from RAG explicitly (and, when flagged, from search too, so the flag can
    // be cleared). Archived latest copies have no entries to keep and are
    // left to the sweep.
    let documents: Vec<Document> = match document_repo.list_all().await {
        Ok(docs) => latest_copies(docs)
            .into_iter()
            .filter(|d| !d.is_archived)
            .collect(),
        Err(e) => {
            tracing::error!("RAG reindex: failed to list documents: {e}");
            listed = false;
            Vec::new()
        }
    };

    // Only attachments still referenced by at least one document are worth
    // re-indexing; an empty `referenced_by` means it is orphaned and pending
    // cleanup (see `recompute_access_levels`), not something to re-embed.
    let assets: Vec<Asset> = match asset_repo.list_all().await {
        Ok(assets) => assets
            .into_iter()
            .filter(|a| !a.referenced_by.is_empty())
            .collect(),
        Err(e) => {
            tracing::error!("RAG reindex: failed to list assets: {e}");
            listed = false;
            Vec::new()
        }
    };

    let live_slugs: HashSet<String> = documents
        .iter()
        .filter(|d| !d.skip_rag && !d.is_hidden)
        .map(|d| d.slug.clone())
        .collect();
    let live_attachment_keys: HashSet<String> = assets.iter().map(|a| a.key.clone()).collect();

    reindex_items(
        &reindex,
        document_repo.as_ref(),
        storage.as_ref(),
        rag.as_ref(),
        search.as_deref(),
        attachment_service.as_deref(),
        documents,
        assets,
    )
    .await;

    if !listed {
        tracing::warn!("RAG reindex: listing was incomplete, skipping the orphan sweep");
    } else if live_slugs.is_empty() {
        // A misconfigured database looks exactly like this; never empty the
        // store on its word.
        tracing::warn!("RAG reindex: no live documents, skipping the orphan sweep");
    } else {
        match rag.sweep_orphans(&live_slugs, &live_attachment_keys).await {
            Ok(0) => {}
            Ok(deleted) => tracing::info!(deleted, "RAG reindex: removed orphaned chunks"),
            Err(e) => {
                tracing::warn!("RAG reindex: orphan sweep failed: {e}");
                reindex.outcome.record_failure(format!("orphan sweep: {e}"));
            }
        }
    }
}

/// The copy of each slug that search and RAG carry: the one resolving to its
/// source's `latest` release. Keeps the listing's order of first appearance.
fn latest_copies(documents: Vec<Document>) -> Vec<Document> {
    let mut order = Vec::new();
    let mut by_slug = std::collections::HashMap::<String, Vec<Document>>::new();
    for document in documents {
        if !by_slug.contains_key(&document.slug) {
            order.push(document.slug.clone());
        }
        by_slug
            .entry(document.slug.clone())
            .or_default()
            .push(document);
    }
    order
        .into_iter()
        .filter_map(|slug| by_slug.remove(&slug))
        .filter_map(|candidates| {
            crate::db::repository::resolve_by_release(
                candidates,
                &crate::versioning::ReleasePins::default(),
            )
        })
        .collect()
}

/// Re-index only the documents/attachments that failed in the previous run,
/// recorded on the [`ReindexState`]. This avoids re-embedding the entire corpus
/// to recover from a partial RAG failure.
pub async fn run_reindex_failed(
    reindex: Arc<ReindexState>,
    document_repo: Arc<dyn DocumentRepository>,
    storage: Arc<dyn StorageClient>,
    rag: Arc<dyn RagService>,
    asset_repo: Arc<dyn AssetRepository>,
    attachment_service: Option<Arc<AttachmentExtractionService>>,
    search: Option<Arc<dyn SearchService>>,
) {
    let _guard = crate::jobs::RunningGuard::new(reindex.clone());

    // Snapshot the previous run's failures before `reindex_items` clears them.
    let failed_docs = reindex.failed_docs.lock().unwrap().clone();
    let failed_assets = reindex.failed_assets.lock().unwrap().clone();

    let documents: Vec<Document> = if failed_docs.is_empty() {
        Vec::new()
    } else {
        match document_repo.find_by_slugs(&failed_docs).await {
            Ok(docs) => docs,
            Err(e) => {
                tracing::error!("RAG retry: failed to load documents: {e}");
                Vec::new()
            }
        }
    };
    let assets: Vec<Asset> = if failed_assets.is_empty() {
        Vec::new()
    } else {
        match asset_repo.list_all().await {
            Ok(assets) => assets
                .into_iter()
                .filter(|a| failed_assets.contains(&a.key))
                .collect(),
            Err(e) => {
                tracing::error!("RAG retry: failed to load assets: {e}");
                Vec::new()
            }
        }
    };

    reindex_items(
        &reindex,
        document_repo.as_ref(),
        storage.as_ref(),
        rag.as_ref(),
        search.as_deref(),
        attachment_service.as_deref(),
        documents,
        assets,
    )
    .await;
}

/// Record a document's reconciliation outcome. On failure, count it and keep
/// the slug for a targeted retry; on success, clear its `needs_reindex` flag
/// when it was reconciled in every enabled index. Returns whether it failed.
async fn record_document_outcome(
    reindex: &ReindexState,
    document_repo: &dyn DocumentRepository,
    doc: &Document,
    action: &str,
    failures: Vec<String>,
) -> bool {
    if !failures.is_empty() {
        let failures = failures.join("; ");
        tracing::warn!(slug = %doc.slug, "RAG reindex: failed to {action}: {failures}");
        reindex
            .outcome
            .record_failure(format!("{action} {}: {failures}", doc.slug));
        reindex.failed_docs.lock().unwrap().push(doc.slug.clone());
        return true;
    }
    if doc.needs_reindex {
        if let Err(e) = document_repo
            .clear_needs_reindex_if_unchanged(&doc.slug, doc.release.as_deref(), doc.last_updated)
            .await
        {
            tracing::warn!(slug = %doc.slug, "RAG reindex: cannot clear needs_reindex: {e}");
        }
    }
    false
}

/// Shared worker: (re)index the given documents and attachments, updating
/// progress and recording per-item failures so they can be retried in isolation.
///
/// A document flagged `needs_reindex` is brought into search as well (when
/// enabled) and its flag cleared once every enabled index took it; see
/// [`crate::indexing`].
#[allow(clippy::too_many_arguments)]
async fn reindex_items(
    reindex: &ReindexState,
    document_repo: &dyn DocumentRepository,
    storage: &dyn StorageClient,
    rag: &dyn RagService,
    search: Option<&dyn SearchService>,
    attachment_service: Option<&AttachmentExtractionService>,
    documents: Vec<Document>,
    assets: Vec<Asset>,
) {
    reindex.progress.store(0, Ordering::Relaxed);
    reindex.outcome.reset();
    reindex.failed_docs.lock().unwrap().clear();
    reindex.failed_assets.lock().unwrap().clear();

    let total = documents.len() + assets.len();
    if total == 0 {
        tracing::info!("RAG reindex: nothing to index");
        reindex.progress.store(100, Ordering::Relaxed);
        return;
    }

    tracing::info!(
        documents = documents.len(),
        assets = assets.len(),
        "RAG reindex: starting"
    );

    // What a full reindex costs is not knowable up front — it depends on how
    // the corpus chunks, which is only settled while indexing it. So the run
    // measures itself instead of guessing: once enough items are through, the
    // average projects to the whole set, and the operator learns the size of
    // the bill early enough to act on it.
    let spend_at_start = crate::usage::guard::system_spend_today();
    let mut last_projection_at = 0usize;

    let mut done = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for doc in &documents {
        // Checked at the top of the iteration so it sees a settled count,
        // whichever branch the previous item took.
        if done >= last_projection_at + PROJECTION_EVERY {
            last_projection_at = done;
            log_cost_projection(spend_at_start, done - skipped - failed, total);
        }

        // A flagged document is reconciled in every enabled index, so its
        // flag can be cleared; an unflagged one only needs RAG.
        let targets = IndexTargets {
            search: search.filter(|_| doc.needs_reindex),
            rag: Some(rag),
        };

        // A hidden document must not be in RAG (nor search) at all. A skip_rag
        // document (e.g. a PDF upload stub) is deliberately excluded from RAG;
        // a full reindex must respect that rather than silently re-adding it,
        // and clean up any chunks left by an earlier ingest. `index_everywhere`
        // does the same for skip_rag, but needs the body for search.
        if doc.is_hidden || (doc.skip_rag && targets.search.is_none()) {
            let failures = remove_everywhere(targets, &doc.slug).await;
            if record_document_outcome(reindex, document_repo, doc, "remove", failures).await {
                failed += 1;
            }
            done += 1;
            reindex
                .progress
                .store((done * 100 / total) as u32, Ordering::Relaxed);
            continue;
        }

        // Fetch content from S3
        let content = match storage.get_object(&doc.s3_key).await {
            Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Ok(None) => {
                tracing::warn!(slug = %doc.slug, "RAG reindex: content not found in S3, skipping");
                skipped += 1;
                reindex.outcome.record_skip();
                done += 1;
                reindex
                    .progress
                    .store((done * 100 / total) as u32, Ordering::Relaxed);
                continue;
            }
            Err(e) => {
                tracing::warn!(slug = %doc.slug, "RAG reindex: failed to read from S3: {e}");
                failed += 1;
                reindex
                    .outcome
                    .record_failure(format!("read {}: {e}", doc.slug));
                reindex.failed_docs.lock().unwrap().push(doc.slug.clone());
                done += 1;
                reindex
                    .progress
                    .store((done * 100 / total) as u32, Ordering::Relaxed);
                continue;
            }
        };

        let failures = index_everywhere(targets, doc, &content).await;
        if record_document_outcome(reindex, document_repo, doc, "index", failures).await {
            failed += 1;
        }

        done += 1;
        reindex
            .progress
            .store((done * 100 / total) as u32, Ordering::Relaxed);
    }

    let mut attachments_skipped = 0usize;
    let mut attachments_failed = 0usize;

    if let Some(service) = attachment_service {
        for asset in &assets {
            // Checked at the top of the iteration so it sees a settled count,
            // whichever branch the previous item took.
            if done >= last_projection_at + PROJECTION_EVERY {
                last_projection_at = done;
                log_cost_projection(
                    spend_at_start,
                    done - skipped - failed - attachments_failed - attachments_skipped,
                    total,
                );
            }
            if let Err(e) = service.process_one(&asset.key, true).await {
                tracing::warn!(key = %asset.key, "RAG reindex: failed to re-index attachment: {e}");
                attachments_failed += 1;
                reindex
                    .outcome
                    .record_failure(format!("attachment {}: {e}", asset.key));
                reindex
                    .failed_assets
                    .lock()
                    .unwrap()
                    .push(asset.key.clone());
            }
            done += 1;
            reindex
                .progress
                .store((done * 100 / total) as u32, Ordering::Relaxed);
        }
    } else if !assets.is_empty() {
        tracing::warn!(
            assets = assets.len(),
            "RAG reindex: attachment indexing is disabled, skipping referenced PDFs"
        );
        attachments_skipped = assets.len();
        for _ in &assets {
            reindex.outcome.record_skip();
        }
        done += assets.len();
        reindex
            .progress
            .store((done * 100 / total) as u32, Ordering::Relaxed);
    }

    let indexed = documents.len() - failed - skipped;
    tracing::info!(
        documents = documents.len(),
        indexed,
        skipped,
        failed,
        attachments = assets.len(),
        attachments_failed,
        attachments_skipped,
        credits = round_credits(crate::usage::guard::system_spend_today() - spend_at_start),
        "RAG reindex: complete"
    );
    reindex.progress.store(100, Ordering::Relaxed);
}

/// How often, in completed items, to project the run's total cost.
///
/// Early enough to be actionable, sparse enough not to bury the log.
const PROJECTION_EVERY: usize = 25;

/// Log what the run has spent and what that projects to for the whole set.
///
/// Extrapolates from the items that actually indexed, not from everything
/// attempted. Skipped and failed items cost little or nothing, so counting them
/// deflates the average — and once a spend ceiling starts refusing work the
/// projection would fall towards zero while the run indexes nothing at all,
/// reading as reassuring precisely when it should not.
fn log_cost_projection(spend_at_start: f64, indexed: usize, total: usize) {
    if indexed == 0 || total == 0 {
        return; // nothing succeeded yet; there is nothing to extrapolate from
    }
    let spent = crate::usage::guard::system_spend_today() - spend_at_start;
    if spent <= 0.0 {
        return; // no ceiling configured, or nothing priced yet
    }

    tracing::info!(
        indexed,
        total,
        credits_so_far = round_credits(spent),
        credits_per_item = round_credits(spent / indexed as f64),
        projected_credits = round_credits(spent / indexed as f64 * total as f64),
        "RAG reindex: cost so far"
    );
}

/// Credits are only ever read by a person; three decimals is plenty and keeps
/// the log from carrying float noise.
fn round_credits(credits: f64) -> f64 {
    (credits * 1_000.0).round() / 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::asset_repository::ExtractionUpdate;
    use crate::db::models::{Asset, Document, ExtractionStatus};
    use crate::error::AppError;
    use crate::rag::extraction::AttachmentExtractors;
    use crate::rag::service::AttachmentPage;
    use crate::test_utils::MockStorage;
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::Mutex;

    fn make_doc(slug: &str, skip_rag: bool) -> Document {
        Document {
            slug: slug.to_string(),
            title: slug.to_string(),
            summary: None,
            s3_key: format!("docs/{slug}.md"),
            access_level: "public".to_string(),
            is_draft: false,
            service_owner: "test".to_string(),
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
            release: None,
            is_latest: true,
            needs_reindex: false,
            skip_rag,
        }
    }

    fn make_asset(key: &str, referenced_by: Vec<String>) -> Asset {
        Asset {
            key: key.to_string(),
            content_type: "application/octet-stream".to_string(),
            size_bytes: 3,
            s3_key: format!("assets/{key}"),
            uploaded_at: Utc::now(),
            uploaded_by: "test".to_string(),
            referenced_by: referenced_by.into_iter().map(Into::into).collect(),
            content_hash: Some("sha256:same".to_string()),
            extraction_status: ExtractionStatus::Done,
            extraction_error: None,
            extracted_content_hash: Some("sha256:same".to_string()),
            extracted_at: None,
            indexed_chunks: Some(1),
        }
    }

    #[derive(Default)]
    struct FakeDocRepo {
        docs: Vec<Document>,
        /// Slugs whose `needs_reindex` was cleared.
        cleared: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl DocumentRepository for FakeDocRepo {
        async fn create_or_update(&self, _: Document) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_slug(&self, _: &str) -> Result<Option<Document>, AppError> {
            Ok(None)
        }
        async fn find_by_slugs(&self, slugs: &[String]) -> Result<Vec<Document>, AppError> {
            Ok(self
                .docs
                .iter()
                .filter(|d| slugs.contains(&d.slug))
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
            _: &crate::versioning::ReleasePins,
        ) -> Result<Vec<Document>, AppError> {
            Ok(self.docs.clone())
        }
        async fn update_backlinks(
            &self,
            _: &str,
            _: &[String],
            _: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_slug_prefix(&self, _: &str) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
        async fn set_archived(&self, _: &str, _: Option<&str>, _: bool) -> Result<(), AppError> {
            Ok(())
        }
        async fn rename_slug(&self, _: &str, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_source_path(&self, _: &str) -> Result<Option<Document>, AppError> {
            Ok(None)
        }
        async fn find_all_by_source_id(&self, _: &str) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
        async fn clear_needs_reindex_if_unchanged(
            &self,
            slug: &str,
            _: Option<&str>,
            _: chrono::DateTime<Utc>,
        ) -> Result<(), AppError> {
            self.cleared.lock().unwrap().push(slug.to_string());
            Ok(())
        }
    }

    struct FakeAssetRepo {
        assets: Mutex<Vec<Asset>>,
        update_extraction_calls: Mutex<Vec<String>>,
    }

    impl FakeAssetRepo {
        fn new(assets: Vec<Asset>) -> Self {
            Self {
                assets: Mutex::new(assets),
                update_extraction_calls: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl AssetRepository for FakeAssetRepo {
        async fn create_or_update(&self, _: Asset) -> Result<(), AppError> {
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
        async fn find_by_keys(&self, keys: &[String]) -> Result<Vec<Asset>, AppError> {
            Ok(self
                .assets
                .lock()
                .unwrap()
                .iter()
                .filter(|a| keys.contains(&a.key))
                .cloned()
                .collect())
        }
        async fn list_all(&self) -> Result<Vec<Asset>, AppError> {
            Ok(self.assets.lock().unwrap().clone())
        }
        async fn list_by_prefix(&self, _: &str) -> Result<Vec<Asset>, AppError> {
            Ok(vec![])
        }
        async fn delete(&self, key: &str) -> Result<(), AppError> {
            self.assets.lock().unwrap().retain(|a| a.key != key);
            Ok(())
        }
        async fn update_extraction(&self, key: &str, _: ExtractionUpdate) -> Result<(), AppError> {
            self.update_extraction_calls
                .lock()
                .unwrap()
                .push(key.to_string());
            Ok(())
        }
        async fn set_release_references(
            &self,
            _: &crate::db::models::DocumentReference,
            _: &[String],
        ) -> Result<Vec<String>, AppError> {
            Ok(vec![])
        }
        async fn list_unfinished_extractions(&self) -> Result<Vec<Asset>, AppError> {
            Ok(vec![])
        }
    }

    #[derive(Default)]
    struct RecordingRagService {
        indexed_slugs: Mutex<Vec<String>>,
        deleted_slugs: Mutex<Vec<String>>,
        /// Slugs whose indexing should fail (to exercise failure/retry paths).
        fail_slugs: Mutex<Vec<String>>,
        /// The live sets each `sweep_orphans` call received.
        sweeps: Mutex<Vec<(Vec<String>, Vec<String>)>>,
    }

    #[async_trait]
    impl RagService for RecordingRagService {
        async fn index_document(
            &self,
            slug: &str,
            _: &str,
            _: &str,
            _: &str,
            _: bool,
            _: &[String],
            _: Option<&str>,
            _: Option<&str>,
        ) -> Result<(), AppError> {
            // Record every attempt, then fail the ones marked to fail.
            self.indexed_slugs.lock().unwrap().push(slug.to_string());
            if self.fail_slugs.lock().unwrap().iter().any(|s| s == slug) {
                return Err(AppError::Internal(format!("index {slug} rejected")));
            }
            Ok(())
        }
        async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
            self.deleted_slugs.lock().unwrap().push(slug.to_string());
            Ok(())
        }
        async fn index_attachment(
            &self,
            _: &str,
            _: &str,
            _: &[AttachmentPage],
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
        async fn sweep_orphans(
            &self,
            live_slugs: &std::collections::HashSet<String>,
            live_attachment_keys: &std::collections::HashSet<String>,
        ) -> Result<usize, AppError> {
            let mut slugs: Vec<_> = live_slugs.iter().cloned().collect();
            slugs.sort();
            let mut keys: Vec<_> = live_attachment_keys.iter().cloned().collect();
            keys.sort();
            self.sweeps.lock().unwrap().push((slugs, keys));
            Ok(0)
        }
    }

    #[tokio::test]
    async fn run_reindex_sweeps_orphans_against_live_documents_and_attachments() {
        let reindex = Arc::new(ReindexState::default());
        let storage = Arc::new(MockStorage::new());
        storage
            .objects
            .lock()
            .unwrap()
            .insert("docs/normal.md".to_string(), b"# Hello".to_vec());
        let doc_repo = Arc::new(FakeDocRepo {
            docs: vec![make_doc("normal", false), make_doc("upload-stub", true)],
            ..Default::default()
        });
        let rag = Arc::new(RecordingRagService::default());
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![
            make_asset("pdfs/referenced.pdf", vec!["upload-stub".to_string()]),
            make_asset("pdfs/orphaned.pdf", vec![]),
        ]));

        run_reindex(
            reindex,
            doc_repo,
            storage,
            rag.clone(),
            asset_repo,
            None,
            None,
        )
        .await;

        // A skip_rag document has no chunks to keep.
        assert_eq!(
            *rag.sweeps.lock().unwrap(),
            vec![(
                vec!["normal".to_string()],
                vec!["pdfs/referenced.pdf".to_string()]
            )]
        );
    }

    #[tokio::test]
    async fn run_reindex_never_sweeps_without_a_complete_listing() {
        let storage = Arc::new(MockStorage::new());
        let rag = Arc::new(RecordingRagService::default());

        // Assets cannot be listed.
        let assets = Arc::new(crate::test_utils::InMemoryAssetRepo::default());
        assets
            .fail_list
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let doc_repo = Arc::new(FakeDocRepo {
            docs: vec![make_doc("normal", false)],
            ..Default::default()
        });
        run_reindex(
            Arc::new(ReindexState::default()),
            doc_repo,
            storage.clone(),
            rag.clone(),
            assets,
            None,
            None,
        )
        .await;

        // No live document at all: treated as a listing problem.
        run_reindex(
            Arc::new(ReindexState::default()),
            Arc::new(FakeDocRepo::default()),
            storage.clone(),
            rag.clone(),
            Arc::new(FakeAssetRepo::new(vec![])),
            None,
            None,
        )
        .await;

        // A retry of failed items is partial by design.
        let reindex = Arc::new(ReindexState::default());
        reindex
            .failed_docs
            .lock()
            .unwrap()
            .push("normal".to_string());
        run_reindex_failed(
            reindex,
            Arc::new(FakeDocRepo {
                docs: vec![make_doc("normal", false)],
                ..Default::default()
            }),
            storage,
            rag.clone(),
            Arc::new(FakeAssetRepo::new(vec![])),
            None,
            None,
        )
        .await;

        assert!(rag.sweeps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn run_reindex_skips_rag_for_skip_rag_documents() {
        let reindex = Arc::new(ReindexState::default());
        let storage = Arc::new(MockStorage::new());
        storage
            .objects
            .lock()
            .unwrap()
            .insert("docs/normal.md".to_string(), b"# Hello".to_vec());
        let doc_repo = Arc::new(FakeDocRepo {
            docs: vec![make_doc("normal", false), make_doc("upload-stub", true)],
            ..Default::default()
        });
        let rag = Arc::new(RecordingRagService::default());
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![]));

        run_reindex(
            reindex,
            doc_repo,
            storage,
            rag.clone(),
            asset_repo,
            None,
            None,
        )
        .await;

        assert_eq!(*rag.indexed_slugs.lock().unwrap(), vec!["normal"]);
        assert_eq!(*rag.deleted_slugs.lock().unwrap(), vec!["upload-stub"]);
    }

    #[tokio::test]
    async fn run_reindex_failed_reprocesses_only_previously_failed_documents() {
        let reindex = Arc::new(ReindexState::default());
        let storage = Arc::new(MockStorage::new());
        storage
            .objects
            .lock()
            .unwrap()
            .insert("docs/good.md".to_string(), b"# Good".to_vec());
        storage
            .objects
            .lock()
            .unwrap()
            .insert("docs/bad.md".to_string(), b"# Bad".to_vec());
        let doc_repo = Arc::new(FakeDocRepo {
            docs: vec![make_doc("good", false), make_doc("bad", false)],
            ..Default::default()
        });
        let rag = Arc::new(RecordingRagService::default());
        rag.fail_slugs.lock().unwrap().push("bad".to_string());
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![]));

        // Full run: both attempted, "bad" fails and is recorded for retry.
        run_reindex(
            reindex.clone(),
            doc_repo.clone(),
            storage.clone(),
            rag.clone(),
            asset_repo.clone(),
            None,
            None,
        )
        .await;
        assert_eq!(*rag.indexed_slugs.lock().unwrap(), vec!["good", "bad"]);
        assert_eq!(
            *reindex.failed_docs.lock().unwrap(),
            vec!["bad".to_string()]
        );

        // Fix the failure and retry only the failed items: "good" is not touched.
        rag.fail_slugs.lock().unwrap().clear();
        rag.indexed_slugs.lock().unwrap().clear();
        // The trigger sets is_running; the guard resets it. Mimic the trigger.
        reindex
            .is_running
            .store(true, std::sync::atomic::Ordering::Release);

        run_reindex_failed(
            reindex.clone(),
            doc_repo,
            storage,
            rag.clone(),
            asset_repo,
            None,
            None,
        )
        .await;

        assert_eq!(
            *rag.indexed_slugs.lock().unwrap(),
            vec!["bad"],
            "retry must reprocess only the previously-failed document"
        );
        assert!(
            reindex.failed_docs.lock().unwrap().is_empty(),
            "a successful retry clears the failed list"
        );
        assert_eq!(reindex.outcome.snapshot().0, 0, "no failures after retry");
    }

    #[tokio::test]
    async fn run_reindex_force_reprocesses_referenced_attachments_only() {
        let reindex = Arc::new(ReindexState::default());
        let storage = Arc::new(MockStorage::new());
        let doc_repo = Arc::new(FakeDocRepo::default());
        let rag = Arc::new(RecordingRagService::default());
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![
            make_asset("pdfs/referenced.pdf", vec!["docs/a".to_string()]),
            make_asset("pdfs/orphaned.pdf", vec![]),
        ]));
        let extractors = Arc::new(AttachmentExtractors::new(100, None));
        let service = Arc::new(AttachmentExtractionService::new(
            storage.clone(),
            asset_repo.clone(),
            doc_repo.clone(),
            rag.clone(),
            None,
            extractors,
        ));

        run_reindex(
            reindex,
            doc_repo,
            storage,
            rag,
            asset_repo.clone(),
            Some(service),
            None,
        )
        .await;

        // Already Done with a matching content hash: without `force` these
        // would be skipped entirely (no update_extraction calls at all).
        let calls = asset_repo.update_extraction_calls.lock().unwrap();
        assert!(
            calls.contains(&"pdfs/referenced.pdf".to_string()),
            "referenced attachment should be force-reprocessed: {calls:?}"
        );
        assert!(
            !calls.contains(&"pdfs/orphaned.pdf".to_string()),
            "orphaned attachment should not be reprocessed: {calls:?}"
        );
    }

    #[tokio::test]
    async fn run_reindex_without_attachment_service_does_not_fail() {
        let reindex = Arc::new(ReindexState::default());
        let storage = Arc::new(MockStorage::new());
        let doc_repo = Arc::new(FakeDocRepo::default());
        let rag = Arc::new(RecordingRagService::default());
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![make_asset(
            "pdfs/referenced.pdf",
            vec!["docs/a".to_string()],
        )]));

        run_reindex(
            reindex.clone(),
            doc_repo,
            storage,
            rag,
            asset_repo,
            None,
            None,
        )
        .await;

        assert!(!reindex.is_running.load(Ordering::Acquire));
        assert_eq!(reindex.progress.load(Ordering::Relaxed), 100);
    }

    #[tokio::test]
    async fn run_reindex_clears_needs_reindex_only_after_every_enabled_index_took_it() {
        let storage = Arc::new(MockStorage::new());
        for slug in ["flagged", "clean"] {
            storage
                .objects
                .lock()
                .unwrap()
                .insert(format!("docs/{slug}.md"), b"# Body".to_vec());
        }
        let mut flagged = make_doc("flagged", false);
        flagged.needs_reindex = true;
        let clean = make_doc("clean", false);

        // Search enabled: a flagged document goes into search too, then is
        // cleared; an unflagged one is left to the search reindex.
        let doc_repo = Arc::new(FakeDocRepo {
            docs: vec![flagged.clone(), clean.clone()],
            ..Default::default()
        });
        let search = Arc::new(crate::test_utils::RecordingSearch::default());
        run_reindex(
            Arc::new(ReindexState::default()),
            doc_repo.clone(),
            storage.clone(),
            Arc::new(RecordingRagService::default()),
            Arc::new(FakeAssetRepo::new(vec![])),
            None,
            Some(search.clone()),
        )
        .await;
        assert_eq!(search.indexed(), vec!["flagged"]);
        assert_eq!(*doc_repo.cleared.lock().unwrap(), vec!["flagged"]);

        // Search failing: the flag stays.
        let doc_repo = Arc::new(FakeDocRepo {
            docs: vec![flagged.clone()],
            ..Default::default()
        });
        let search = Arc::new(crate::test_utils::RecordingSearch::default());
        search
            .fail_index
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let reindex = Arc::new(ReindexState::default());
        run_reindex(
            reindex.clone(),
            doc_repo.clone(),
            storage.clone(),
            Arc::new(RecordingRagService::default()),
            Arc::new(FakeAssetRepo::new(vec![])),
            None,
            Some(search),
        )
        .await;
        assert!(doc_repo.cleared.lock().unwrap().is_empty());
        assert_eq!(*reindex.failed_docs.lock().unwrap(), vec!["flagged"]);

        // Search disabled: RAG alone is every enabled index.
        let doc_repo = Arc::new(FakeDocRepo {
            docs: vec![flagged],
            ..Default::default()
        });
        run_reindex(
            Arc::new(ReindexState::default()),
            doc_repo.clone(),
            storage,
            Arc::new(RecordingRagService::default()),
            Arc::new(FakeAssetRepo::new(vec![])),
            None,
            None,
        )
        .await;
        assert_eq!(*doc_repo.cleared.lock().unwrap(), vec!["flagged"]);
    }

    #[tokio::test]
    async fn run_reindex_removes_hidden_documents_instead_of_indexing_them() {
        let storage = Arc::new(MockStorage::new());
        storage
            .objects
            .lock()
            .unwrap()
            .insert("docs/visible.md".to_string(), b"# Body".to_vec());
        let mut hidden = make_doc("hidden", false);
        hidden.is_hidden = true;
        let mut flagged_hidden = make_doc("flagged-hidden", false);
        flagged_hidden.is_hidden = true;
        flagged_hidden.needs_reindex = true;
        let doc_repo = Arc::new(FakeDocRepo {
            docs: vec![make_doc("visible", false), hidden, flagged_hidden],
            ..Default::default()
        });
        let rag = Arc::new(RecordingRagService::default());
        let search = Arc::new(crate::test_utils::RecordingSearch::default());

        run_reindex(
            Arc::new(ReindexState::default()),
            doc_repo.clone(),
            storage,
            rag.clone(),
            Arc::new(FakeAssetRepo::new(vec![])),
            None,
            Some(search.clone()),
        )
        .await;

        assert_eq!(*rag.indexed_slugs.lock().unwrap(), vec!["visible"]);
        assert_eq!(
            *rag.deleted_slugs.lock().unwrap(),
            vec!["hidden", "flagged-hidden"]
        );
        // Flagged: removed from search too, then cleared.
        assert_eq!(search.deleted(), vec!["flagged-hidden"]);
        assert_eq!(*doc_repo.cleared.lock().unwrap(), vec!["flagged-hidden"]);
        // Hidden documents are not live: their chunks are swept.
        assert_eq!(rag.sweeps.lock().unwrap()[0].0, vec!["visible".to_string()]);
    }
}
