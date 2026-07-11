//! Background orchestration for indexing attachment text into the RAG store.
//!
//! Uploads enqueue an asset key; a single worker drains a bounded queue and runs
//! [`AttachmentExtractionService::process_one`]: fetch bytes from storage,
//! extract per-page text, derive access levels from the referencing documents,
//! and hand the result to [`RagService::index_attachment`]. Progress is recorded
//! on the asset's extraction fields so it can be surfaced in the UI.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc::{self, error::TrySendError};

use crate::db::asset_repository::{AssetRepository, ExtractionUpdate};
use crate::db::models::ExtractionStatus;
use crate::db::repository::DocumentRepository;
use crate::error::AppError;
use crate::rag::attachment_acl::{attachment_access_levels, ReferencingDoc};
use crate::rag::extraction::{AttachmentExtractors, ExtractionOutcome};
use crate::rag::service::RagService;
use crate::search::attachment_search::AttachmentSearchService;
use crate::storage::client::StorageClient;

/// Handle for enqueuing attachment keys for (re)extraction. Cloneable and cheap.
#[derive(Clone)]
pub struct AttachmentQueue {
    tx: mpsc::Sender<String>,
    asset_repo: Arc<dyn AssetRepository>,
}

impl AttachmentQueue {
    /// Enqueue an asset key. When the bounded queue is temporarily full, retry
    /// asynchronously instead of dropping the work item. If the worker is gone,
    /// mark the asset as failed so it does not stay `Pending` forever.
    pub fn enqueue(&self, key: &str) {
        match self.tx.try_send(key.to_string()) {
            Ok(()) => {}
            Err(TrySendError::Full(key)) => {
                tracing::warn!(
                    key = %key,
                    "attachment extraction queue full, deferring send until capacity is available"
                );
                let tx = self.tx.clone();
                let asset_repo = self.asset_repo.clone();
                tokio::spawn(async move {
                    if let Err(e) = tx.send(key.clone()).await {
                        mark_enqueue_failure(
                            asset_repo.as_ref(),
                            &key,
                            format!("attachment extraction queue unavailable: {e}"),
                        )
                        .await;
                    }
                });
            }
            Err(TrySendError::Closed(key)) => {
                let asset_repo = self.asset_repo.clone();
                tokio::spawn(async move {
                    mark_enqueue_failure(
                        asset_repo.as_ref(),
                        &key,
                        "attachment extraction queue unavailable: worker is closed".to_string(),
                    )
                    .await;
                });
            }
        }
    }
}

async fn mark_enqueue_failure(asset_repo: &dyn AssetRepository, key: &str, error: String) {
    tracing::warn!(key, "{error}");
    if let Err(update_err) = asset_repo
        .update_extraction(
            key,
            ExtractionUpdate {
                status: ExtractionStatus::Failed,
                error: Some(error),
                extracted_content_hash: None,
                extracted_at: None,
                indexed_chunks: None,
            },
        )
        .await
    {
        tracing::warn!(
            key,
            "failed to persist attachment queue error: {update_err}"
        );
    }
}

/// Owns the dependencies needed to extract and index a single attachment.
pub struct AttachmentExtractionService {
    storage: Arc<dyn StorageClient>,
    asset_repo: Arc<dyn AssetRepository>,
    document_repo: Arc<dyn DocumentRepository>,
    rag: Arc<dyn RagService>,
    /// Keyword search over attachment page text, indexed alongside RAG.
    /// `None` when Meilisearch is disabled — attachment content then remains
    /// searchable only through RAG (semantic), not keyword search.
    attachment_search: Option<Arc<dyn AttachmentSearchService>>,
    extractors: Arc<AttachmentExtractors>,
}

impl AttachmentExtractionService {
    pub fn new(
        storage: Arc<dyn StorageClient>,
        asset_repo: Arc<dyn AssetRepository>,
        document_repo: Arc<dyn DocumentRepository>,
        rag: Arc<dyn RagService>,
        attachment_search: Option<Arc<dyn AttachmentSearchService>>,
        extractors: Arc<AttachmentExtractors>,
    ) -> Self {
        Self {
            storage,
            asset_repo,
            document_repo,
            rag,
            attachment_search,
            extractors,
        }
    }

    /// Spawn the background worker draining a bounded queue of `capacity`, and
    /// return a handle for enqueuing keys.
    pub fn spawn(self: Arc<Self>, capacity: usize) -> AttachmentQueue {
        let (tx, mut rx) = mpsc::channel::<String>(capacity.max(1));
        let asset_repo = self.asset_repo.clone();
        tokio::spawn(async move {
            while let Some(key) = rx.recv().await {
                if let Err(e) = self.process_one(&key, false).await {
                    tracing::error!(key, "attachment extraction worker error: {e}");
                }
            }
        });
        AttachmentQueue { tx, asset_repo }
    }

    /// Extract and index one attachment, updating its extraction status. Safe to
    /// call repeatedly; skips work when the content is unchanged since the last
    /// successful run, unless `force` is set.
    ///
    /// `force` bypasses the unchanged-content short-circuit so a full re-index
    /// can pick up a changed chunking/embedding/extraction configuration, or an
    /// attachment that was uploaded before extraction was wired up, without
    /// requiring a no-op re-upload of the same file.
    pub async fn process_one(&self, key: &str, force: bool) -> Result<(), AppError> {
        let asset = match self.asset_repo.find_by_key(key).await? {
            Some(a) => a,
            None => return Ok(()), // deleted in the meantime
        };

        // Already processed for this exact content: nothing to do. Covers both
        // successful indexing (Done) and unsupported types (Skipped) so an
        // unchanged re-upload or backfill does not re-extract them.
        let already_processed = matches!(
            asset.extraction_status,
            ExtractionStatus::Done | ExtractionStatus::Skipped
        );
        if !force
            && already_processed
            && asset.content_hash.is_some()
            && asset.content_hash == asset.extracted_content_hash
        {
            return Ok(());
        }

        self.asset_repo
            .update_extraction(
                key,
                ExtractionUpdate {
                    status: ExtractionStatus::InProgress,
                    error: None,
                    extracted_content_hash: None,
                    extracted_at: None,
                    indexed_chunks: None,
                },
            )
            .await?;

        let bytes = match self.storage.get_object(&asset.s3_key).await {
            Ok(Some(b)) => b,
            Ok(None) => return self.fail(key, "asset content not found in storage").await,
            Err(e) => return self.fail(key, &format!("storage error: {e}")).await,
        };

        let pages = match self.extractors.extract(&bytes, &asset.content_type).await {
            Ok(ExtractionOutcome::Extracted(pages)) => pages,
            Ok(ExtractionOutcome::Unsupported) => {
                // Not indexable: drop any stale chunks and record it as skipped.
                let _ = self.rag.delete_attachment(key).await;
                if let Some(search) = &self.attachment_search {
                    let _ = search.delete_attachment(key).await;
                }
                return self
                    .finish(key, ExtractionStatus::Skipped, &asset.content_hash, 0)
                    .await;
            }
            Err(e) => return self.fail(key, &format!("extraction failed: {e}")).await,
        };

        let (access_levels, tags) =
            referencing_acl(self.document_repo.as_ref(), &asset.referenced_by).await;
        let filename = filename_from_key(key);

        match self
            .rag
            .index_attachment(key, &filename, &pages, &access_levels, &tags)
            .await
        {
            Ok(n) => {
                // Keyword search over the page text, alongside RAG. Linked to
                // the first referencing document — attachments are almost
                // always referenced by exactly one (the admin upload flow
                // links a PDF to a single stub document); a shared attachment
                // simply shows up under that first document's page.
                if let Some(search) = &self.attachment_search {
                    if let Some(doc_slug) = asset.referenced_by.first() {
                        let doc_title = self
                            .document_repo
                            .find_by_slug(doc_slug)
                            .await
                            .ok()
                            .flatten()
                            .map(|d| d.title)
                            .unwrap_or_else(|| doc_slug.clone());
                        if let Err(e) = search
                            .index_pages(
                                key,
                                &filename,
                                doc_slug,
                                &doc_title,
                                &pages,
                                &access_levels,
                            )
                            .await
                        {
                            return self
                                .fail(key, &format!("keyword-search indexing failed: {e}"))
                                .await;
                        }
                    }
                }
                self.finish(key, ExtractionStatus::Done, &asset.content_hash, n as u32)
                    .await
            }
            Err(e) => self.fail(key, &format!("indexing failed: {e}")).await,
        }
    }

    /// Record a terminal success/skip state.
    async fn finish(
        &self,
        key: &str,
        status: ExtractionStatus,
        content_hash: &Option<String>,
        indexed_chunks: u32,
    ) -> Result<(), AppError> {
        self.asset_repo
            .update_extraction(
                key,
                ExtractionUpdate {
                    status,
                    error: None,
                    extracted_content_hash: content_hash.clone(),
                    extracted_at: Some(Utc::now()),
                    indexed_chunks: Some(indexed_chunks),
                },
            )
            .await
    }

    /// Record a failure, logging the reason. Never propagates the status-write
    /// error so the worker keeps draining the queue.
    async fn fail(&self, key: &str, msg: &str) -> Result<(), AppError> {
        tracing::warn!(key, "attachment extraction failed: {msg}");
        let _ = self
            .asset_repo
            .update_extraction(
                key,
                ExtractionUpdate {
                    status: ExtractionStatus::Failed,
                    error: Some(msg.to_string()),
                    extracted_content_hash: None,
                    extracted_at: None,
                    indexed_chunks: None,
                },
            )
            .await;
        Ok(())
    }
}

/// Recompute and apply RAG access levels for attachments whose referencing
/// documents changed (e.g. after a document save), or delete the attachment
/// entirely once it has no referencing document left. A no-op for keys with
/// no indexed chunks. Per-key errors are logged, never propagated, so a
/// document save is not failed by RAG bookkeeping.
///
/// This is the single place that reacts to an attachment's `referenced_by`
/// set changing, so it is what keeps orphaned attachments (dropped via a
/// plain markdown edit, `lekton-sync`, or the admin upload form) from
/// accumulating forever in S3/MongoDB/Qdrant — not just the ones dropped
/// through the upload form's own edit flow.
pub async fn recompute_access_levels(
    rag: &dyn RagService,
    asset_repo: &dyn AssetRepository,
    document_repo: &dyn DocumentRepository,
    storage: &dyn StorageClient,
    attachment_search: Option<&dyn AttachmentSearchService>,
    keys: &[String],
) {
    for key in keys {
        let asset = match asset_repo.find_by_key(key).await {
            Ok(Some(a)) => a,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(key, "recompute attachment ACL: find asset failed: {e}");
                continue;
            }
        };

        if asset.referenced_by.is_empty() {
            if let Err(e) = storage.delete_object(&asset.s3_key).await {
                tracing::warn!(
                    key,
                    "delete orphaned attachment: storage delete failed: {e}"
                );
            }
            if let Err(e) = asset_repo.delete(key).await {
                tracing::warn!(key, "delete orphaned attachment: asset delete failed: {e}");
            }
            if let Err(e) = rag.delete_attachment(key).await {
                tracing::warn!(key, "delete orphaned attachment: RAG delete failed: {e}");
            }
            if let Some(search) = attachment_search {
                if let Err(e) = search.delete_attachment(key).await {
                    tracing::warn!(key, "delete orphaned attachment: search delete failed: {e}");
                }
            }
            continue;
        }

        let (levels, _tags) = referencing_acl(document_repo, &asset.referenced_by).await;

        let mut failures = Vec::new();

        if let Err(e) = rag.update_attachment_access_levels(key, &levels).await {
            tracing::warn!(key, "recompute attachment ACL: update failed: {e}");
            failures.push(format!("RAG ACL update failed: {e}"));
        }
        if let Some(search) = attachment_search {
            if let Err(e) = search.update_access_levels(key, &levels).await {
                tracing::warn!(key, "recompute attachment ACL: search update failed: {e}");
                failures.push(format!("search ACL update failed: {e}"));
            }
        }

        if !failures.is_empty() {
            fail_closed_attachment_acl(key, asset_repo, rag, attachment_search, &failures).await;
        }
    }
}

async fn fail_closed_attachment_acl(
    key: &str,
    asset_repo: &dyn AssetRepository,
    rag: &dyn RagService,
    attachment_search: Option<&dyn AttachmentSearchService>,
    failures: &[String],
) {
    let error = format!(
        "{}. Attachment deindexed until a successful reprocess refreshes ACLs.",
        failures.join("; ")
    );

    if let Err(e) = rag.delete_attachment(key).await {
        tracing::warn!(
            key,
            "recompute attachment ACL: fail-closed RAG delete failed: {e}"
        );
    }
    if let Some(search) = attachment_search {
        if let Err(e) = search.delete_attachment(key).await {
            tracing::warn!(
                key,
                "recompute attachment ACL: fail-closed search delete failed: {e}"
            );
        }
    }
    if let Err(e) = asset_repo
        .update_extraction(
            key,
            ExtractionUpdate {
                status: ExtractionStatus::Failed,
                error: Some(error),
                extracted_content_hash: None,
                extracted_at: None,
                indexed_chunks: None,
            },
        )
        .await
    {
        tracing::warn!(
            key,
            "recompute attachment ACL: failed to mark asset for reprocessing: {e}"
        );
    }
}

/// Derive an attachment's access levels and tag union from its referencing
/// documents. Access levels come from published, non-archived referencers (see
/// [`attachment_access_levels`]); tags are the union over the same set. A
/// per-document lookup error skips that document (fail-closed: the attachment
/// ends up no more visible than what could be resolved).
async fn referencing_acl(
    document_repo: &dyn DocumentRepository,
    slugs: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut docs: Vec<(String, bool, bool)> = Vec::new();
    let mut tag_set = BTreeSet::new();
    for slug in slugs {
        if let Ok(Some(doc)) = document_repo.find_by_slug(slug).await {
            if !doc.is_draft && !doc.is_archived {
                for t in &doc.tags {
                    tag_set.insert(t.clone());
                }
            }
            docs.push((doc.access_level, doc.is_draft, doc.is_archived));
        }
    }
    let refs: Vec<ReferencingDoc> = docs
        .iter()
        .map(|(access_level, is_draft, is_archived)| ReferencingDoc {
            access_level,
            is_draft: *is_draft,
            is_archived: *is_archived,
        })
        .collect();
    (
        attachment_access_levels(&refs),
        tag_set.into_iter().collect(),
    )
}

/// The last path segment of an asset key, used as the attachment's display name.
fn filename_from_key(key: &str) -> String {
    key.rsplit('/').next().unwrap_or(key).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::asset_repository::ExtractionUpdate;
    use crate::db::models::{Asset, Document};
    use crate::rag::service::AttachmentPage;
    use crate::test_utils::MockStorage;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use tokio::time::{sleep, Duration};

    #[test]
    fn filename_from_key_takes_last_segment() {
        assert_eq!(
            filename_from_key("project-a/configs/manual.pdf"),
            "manual.pdf"
        );
        assert_eq!(filename_from_key("readme.txt"), "readme.txt");
        assert_eq!(filename_from_key(""), "");
    }

    fn make_asset(key: &str, referenced_by: Vec<String>) -> Asset {
        Asset {
            key: key.to_string(),
            content_type: "application/pdf".to_string(),
            size_bytes: 0,
            s3_key: format!("assets/{key}"),
            uploaded_at: Utc::now(),
            uploaded_by: "test".to_string(),
            referenced_by,
            content_hash: None,
            extraction_status: ExtractionStatus::Done,
            extraction_error: None,
            extracted_content_hash: None,
            extracted_at: None,
            indexed_chunks: None,
        }
    }

    struct FakeAssetRepo {
        assets: Mutex<Vec<Asset>>,
    }

    impl FakeAssetRepo {
        fn new(assets: Vec<Asset>) -> Self {
            Self {
                assets: Mutex::new(assets),
            }
        }

        fn find_local(&self, key: &str) -> Option<Asset> {
            self.assets
                .lock()
                .unwrap()
                .iter()
                .find(|a| a.key == key)
                .cloned()
        }

        fn still_has(&self, key: &str) -> bool {
            self.assets.lock().unwrap().iter().any(|a| a.key == key)
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
        async fn update_extraction(
            &self,
            key: &str,
            update: ExtractionUpdate,
        ) -> Result<(), AppError> {
            if let Some(asset) = self
                .assets
                .lock()
                .unwrap()
                .iter_mut()
                .find(|asset| asset.key == key)
            {
                asset.extraction_status = update.status;
                asset.extraction_error = update.error;
                asset.extracted_content_hash = update.extracted_content_hash;
                asset.extracted_at = update.extracted_at;
                asset.indexed_chunks = update.indexed_chunks;
            }
            Ok(())
        }
        async fn set_references(&self, _: &str, _: &[String]) -> Result<Vec<String>, AppError> {
            Ok(vec![])
        }
    }

    struct NoopDocumentRepo;

    #[async_trait]
    impl DocumentRepository for NoopDocumentRepo {
        async fn create_or_update(&self, _: Document) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_slug(&self, _: &str) -> Result<Option<Document>, AppError> {
            Ok(None)
        }
        async fn find_by_slugs(&self, _: &[String]) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
        async fn list_all(&self) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
        async fn list_by_access_levels(
            &self,
            _: Option<&[String]>,
            _: bool,
        ) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
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
        async fn set_archived(&self, _: &str, _: bool) -> Result<(), AppError> {
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
    }

    #[derive(Default)]
    struct RecordingRagService {
        deleted_attachment: AtomicBool,
        updated_acl: Mutex<Option<Vec<String>>>,
        fail_update: AtomicBool,
    }

    #[async_trait]
    impl RagService for RecordingRagService {
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
        async fn delete_document(&self, _: &str) -> Result<(), AppError> {
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
            self.deleted_attachment.store(true, Ordering::SeqCst);
            Ok(())
        }
        async fn update_attachment_access_levels(
            &self,
            _: &str,
            access_levels: &[String],
        ) -> Result<(), AppError> {
            if self.fail_update.load(Ordering::SeqCst) {
                return Err(AppError::Internal("simulated rag acl failure".to_string()));
            }
            *self.updated_acl.lock().unwrap() = Some(access_levels.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    async fn recompute_access_levels_deletes_orphaned_attachment() {
        let asset_repo = FakeAssetRepo::new(vec![make_asset("pdfs/a.pdf", vec![])]);
        let document_repo = NoopDocumentRepo;
        let rag = RecordingRagService::default();
        let storage = MockStorage::new();
        storage
            .objects
            .lock()
            .unwrap()
            .insert("assets/pdfs/a.pdf".to_string(), vec![1, 2, 3]);

        recompute_access_levels(
            &rag,
            &asset_repo,
            &document_repo,
            &storage,
            None,
            &["pdfs/a.pdf".to_string()],
        )
        .await;

        assert!(
            rag.deleted_attachment.load(Ordering::SeqCst),
            "orphaned attachment should be deleted from RAG"
        );
        assert!(
            !asset_repo.still_has("pdfs/a.pdf"),
            "orphaned asset record should be deleted"
        );
        assert!(
            !storage
                .objects
                .lock()
                .unwrap()
                .contains_key("assets/pdfs/a.pdf"),
            "orphaned asset's S3 object should be deleted"
        );
        assert!(rag.updated_acl.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn recompute_access_levels_updates_acl_for_referenced_attachment() {
        let asset_repo = FakeAssetRepo::new(vec![make_asset(
            "pdfs/b.pdf",
            vec!["docs/still-linking".to_string()],
        )]);
        let document_repo = NoopDocumentRepo;
        let rag = RecordingRagService::default();
        let storage = MockStorage::new();

        recompute_access_levels(
            &rag,
            &asset_repo,
            &document_repo,
            &storage,
            None,
            &["pdfs/b.pdf".to_string()],
        )
        .await;

        assert!(!rag.deleted_attachment.load(Ordering::SeqCst));
        assert!(asset_repo.still_has("pdfs/b.pdf"));
        // referencing_acl finds no matching document (NoopDocumentRepo returns
        // None), so the recomputed access level list is empty — but the
        // attachment itself is not deleted, since it is still referenced.
        assert_eq!(*rag.updated_acl.lock().unwrap(), Some(vec![]));
    }

    #[derive(Default)]
    struct RecordingAttachmentSearchService {
        deleted: Mutex<Vec<String>>,
        updated_acl: Mutex<Option<(String, Vec<String>)>>,
        fail_index: AtomicBool,
        fail_update: AtomicBool,
    }

    #[async_trait]
    impl crate::search::attachment_search::AttachmentSearchService
        for RecordingAttachmentSearchService
    {
        async fn index_pages(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: &[crate::rag::service::AttachmentPage],
            _: &[String],
        ) -> Result<(), AppError> {
            if self.fail_index.load(Ordering::SeqCst) {
                return Err(AppError::Internal(
                    "simulated search indexing failure".to_string(),
                ));
            }
            Ok(())
        }
        async fn delete_attachment(&self, attachment_key: &str) -> Result<(), AppError> {
            self.deleted
                .lock()
                .unwrap()
                .push(attachment_key.to_string());
            Ok(())
        }
        async fn update_access_levels(
            &self,
            attachment_key: &str,
            access_levels: &[String],
        ) -> Result<(), AppError> {
            if self.fail_update.load(Ordering::SeqCst) {
                return Err(AppError::Internal(
                    "simulated search acl failure".to_string(),
                ));
            }
            *self.updated_acl.lock().unwrap() =
                Some((attachment_key.to_string(), access_levels.to_vec()));
            Ok(())
        }
        async fn search(
            &self,
            _: &str,
            _: Option<&[String]>,
        ) -> Result<Vec<crate::search::attachment_search::AttachmentSearchHit>, AppError> {
            Ok(vec![])
        }
        async fn configure_index(&self) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn keyword_index_failure_marks_attachment_for_retry() {
        let key = "notes/retry.txt";
        let mut asset = make_asset(key, vec!["docs/guide".to_string()]);
        asset.content_type = "text/plain".to_string();
        asset.content_hash = Some("sha256:content".to_string());

        let asset_repo = Arc::new(FakeAssetRepo::new(vec![asset]));
        let storage = Arc::new(MockStorage::new());
        storage
            .put_object("assets/notes/retry.txt", b"retryable attachment".to_vec())
            .await
            .unwrap();
        let rag = Arc::new(RecordingRagService::default());
        let search = Arc::new(RecordingAttachmentSearchService::default());
        search.fail_index.store(true, Ordering::SeqCst);

        let service = AttachmentExtractionService::new(
            storage,
            asset_repo.clone(),
            Arc::new(NoopDocumentRepo),
            rag,
            Some(search),
            Arc::new(AttachmentExtractors::new(100, None)),
        );

        service.process_one(key, true).await.unwrap();

        let asset = asset_repo.find_local(key).unwrap();
        assert_eq!(asset.extraction_status, ExtractionStatus::Failed);
        assert!(asset
            .extraction_error
            .as_deref()
            .unwrap_or_default()
            .contains("keyword-search indexing failed"));
        assert!(asset.extracted_content_hash.is_none());
    }

    #[tokio::test]
    async fn recompute_access_levels_also_updates_attachment_search() {
        let asset_repo = FakeAssetRepo::new(vec![make_asset(
            "pdfs/c.pdf",
            vec!["docs/still-linking".to_string()],
        )]);
        let document_repo = NoopDocumentRepo;
        let rag = RecordingRagService::default();
        let storage = MockStorage::new();
        let attachment_search = RecordingAttachmentSearchService::default();

        recompute_access_levels(
            &rag,
            &asset_repo,
            &document_repo,
            &storage,
            Some(&attachment_search),
            &["pdfs/c.pdf".to_string()],
        )
        .await;

        assert_eq!(
            *attachment_search.updated_acl.lock().unwrap(),
            Some(("pdfs/c.pdf".to_string(), vec![]))
        );
        assert!(attachment_search.deleted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn recompute_access_levels_deletes_from_attachment_search_when_orphaned() {
        let asset_repo = FakeAssetRepo::new(vec![make_asset("pdfs/d.pdf", vec![])]);
        let document_repo = NoopDocumentRepo;
        let rag = RecordingRagService::default();
        let storage = MockStorage::new();
        let attachment_search = RecordingAttachmentSearchService::default();

        recompute_access_levels(
            &rag,
            &asset_repo,
            &document_repo,
            &storage,
            Some(&attachment_search),
            &["pdfs/d.pdf".to_string()],
        )
        .await;

        assert_eq!(
            *attachment_search.deleted.lock().unwrap(),
            vec!["pdfs/d.pdf".to_string()]
        );
    }

    #[tokio::test]
    async fn recompute_access_levels_deindexes_attachment_when_rag_acl_update_fails() {
        let asset_repo = FakeAssetRepo::new(vec![make_asset(
            "pdfs/e.pdf",
            vec!["docs/still-linking".to_string()],
        )]);
        let document_repo = NoopDocumentRepo;
        let rag = RecordingRagService::default();
        rag.fail_update.store(true, Ordering::SeqCst);
        let storage = MockStorage::new();
        let attachment_search = RecordingAttachmentSearchService::default();

        recompute_access_levels(
            &rag,
            &asset_repo,
            &document_repo,
            &storage,
            Some(&attachment_search),
            &["pdfs/e.pdf".to_string()],
        )
        .await;

        assert!(
            rag.deleted_attachment.load(Ordering::SeqCst),
            "attachments with uncertain ACLs should be removed from RAG"
        );
        assert_eq!(
            *attachment_search.deleted.lock().unwrap(),
            vec!["pdfs/e.pdf".to_string()]
        );

        let asset = asset_repo.find_local("pdfs/e.pdf").unwrap();
        assert_eq!(asset.extraction_status, ExtractionStatus::Failed);
        assert!(asset
            .extraction_error
            .as_deref()
            .unwrap_or_default()
            .contains("RAG ACL update failed"));
        assert!(asset.extracted_content_hash.is_none());
    }

    #[tokio::test]
    async fn recompute_access_levels_deindexes_attachment_when_search_acl_update_fails() {
        let asset_repo = FakeAssetRepo::new(vec![make_asset(
            "pdfs/f.pdf",
            vec!["docs/still-linking".to_string()],
        )]);
        let document_repo = NoopDocumentRepo;
        let rag = RecordingRagService::default();
        let storage = MockStorage::new();
        let attachment_search = RecordingAttachmentSearchService::default();
        attachment_search.fail_update.store(true, Ordering::SeqCst);

        recompute_access_levels(
            &rag,
            &asset_repo,
            &document_repo,
            &storage,
            Some(&attachment_search),
            &["pdfs/f.pdf".to_string()],
        )
        .await;

        assert!(
            rag.deleted_attachment.load(Ordering::SeqCst),
            "attachments with uncertain search ACLs should be removed from indexes"
        );
        assert_eq!(
            *attachment_search.deleted.lock().unwrap(),
            vec!["pdfs/f.pdf".to_string()]
        );

        let asset = asset_repo.find_local("pdfs/f.pdf").unwrap();
        assert_eq!(asset.extraction_status, ExtractionStatus::Failed);
        assert!(asset
            .extraction_error
            .as_deref()
            .unwrap_or_default()
            .contains("search ACL update failed"));
    }

    #[tokio::test]
    async fn attachment_queue_retries_when_channel_is_temporarily_full() {
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![make_asset("pdfs/g.pdf", vec![])]));
        let (tx, mut rx) = mpsc::channel(1);
        let queue = AttachmentQueue {
            tx,
            asset_repo: asset_repo.clone(),
        };

        queue.enqueue("pdfs/g.pdf");
        queue.enqueue("pdfs/h.pdf");

        assert_eq!(rx.recv().await.as_deref(), Some("pdfs/g.pdf"));
        assert_eq!(rx.recv().await.as_deref(), Some("pdfs/h.pdf"));

        let asset = asset_repo.find_local("pdfs/g.pdf").unwrap();
        assert_ne!(asset.extraction_status, ExtractionStatus::Failed);
    }

    #[tokio::test]
    async fn attachment_queue_marks_asset_failed_when_worker_is_closed() {
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![make_asset("pdfs/i.pdf", vec![])]));
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let queue = AttachmentQueue {
            tx,
            asset_repo: asset_repo.clone(),
        };

        queue.enqueue("pdfs/i.pdf");
        sleep(Duration::from_millis(20)).await;

        let asset = asset_repo.find_local("pdfs/i.pdf").unwrap();
        assert_eq!(asset.extraction_status, ExtractionStatus::Failed);
        assert!(asset
            .extraction_error
            .as_deref()
            .unwrap_or_default()
            .contains("queue unavailable"));
    }
}
