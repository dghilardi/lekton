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
use crate::db::models::{Asset, DocumentReference, ExtractionStatus};
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

/// Re-enqueue attachments whose extraction was left unfinished (`Pending` or
/// `InProgress`) so a process restart does not silently lose queued work.
///
/// The extraction queue is in-memory, so without this sweep any asset that was
/// persisted but not yet processed before a restart would never be indexed.
/// Safe to run on startup: `process_one` skips already-indexed content.
pub async fn resume_unfinished_extractions(
    asset_repo: &dyn AssetRepository,
    queue: &AttachmentQueue,
) {
    match asset_repo.list_unfinished_extractions().await {
        Ok(assets) => {
            if !assets.is_empty() {
                tracing::info!(
                    count = assets.len(),
                    "re-enqueuing unfinished attachment extractions at startup"
                );
            }
            for asset in assets {
                queue.enqueue(&asset.key);
            }
        }
        Err(e) => {
            tracing::warn!("startup attachment extraction sweep failed: {e}");
        }
    }
}

/// How often to re-check the spend ceiling while the queue is held back.
///
/// The ceiling only clears at UTC midnight, so this is a slow poll: the point
/// is to resume without a restart, not to react quickly.
const HEADROOM_POLL: std::time::Duration = std::time::Duration::from_secs(300);

/// Block until the daily ceiling would admit background work again.
///
/// Returns immediately when no ceiling is configured, which is the default.
async fn wait_for_spend_headroom() {
    use crate::db::usage_models::UsageKey;

    wait_until(
        || crate::usage::guard::has_headroom(&UsageKey::System),
        HEADROOM_POLL,
    )
    .await;
}

/// Poll `ready` until it holds, announcing the pause and the resume.
///
/// The predicate is a parameter so the wait can be exercised without the
/// process-wide guard, which a test cannot install more than once.
async fn wait_until(ready: impl Fn() -> bool, poll: std::time::Duration) {
    if ready() {
        return;
    }

    tracing::warn!(
        retry_in_secs = poll.as_secs(),
        "attachment extraction paused: the daily AI spend reserve for background work is used \
         up. Queued attachments wait rather than fail, and resume once it clears."
    );
    while !ready() {
        tokio::time::sleep(poll).await;
    }
    tracing::info!("attachment extraction resuming: spend headroom is available again");
}

/// One async lock per attachment key with an ACL write in flight; entries
/// whose lock nobody holds any more are pruned on the next acquisition.
static ACL_LOCKS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(Default::default);

/// Serialize the ACL writes for one attachment key.
///
/// Both the ACL recompute and the extraction worker's post-index repair read
/// the attachment's references and then write the ACL they imply. Run
/// concurrently, the older read could land last and overwrite a newer,
/// narrower ACL with a wider one. Each holds this guard from the read through
/// the write, so the last write is always derived from the latest read. The
/// worker also holds it while replacing an attachment's keyword-search pages,
/// so a writer updating those pages in place never interleaves with that.
///
/// Process-wide: Lekton runs a single replica, and every ACL write path lives
/// in this module, so no handle needs threading through the callers. The
/// guard is held across store I/O (reads, ACL writes, deletes), so writers of
/// one attachment queue behind each other; accepted, since only writers of the
/// same key contend.
pub(crate) async fn lock_attachment_acl(key: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = ACL_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        match locks.get(key).and_then(std::sync::Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key.to_string(), Arc::downgrade(&lock));
                lock
            }
        }
    };
    lock.lock_owned().await
}

/// The running worker's queue, for the ACL recompute to re-process an
/// attachment it cannot update in place (see [`recompute_access_levels`]).
/// Process-wide for the same reason as [`ACL_LOCKS`].
static ATTACHMENT_QUEUE: std::sync::RwLock<Option<AttachmentQueue>> = std::sync::RwLock::new(None);

/// Register the queue the ACL recompute uses to re-process attachments.
pub(crate) fn register_attachment_queue(queue: AttachmentQueue) {
    *ATTACHMENT_QUEUE.write().unwrap_or_else(|e| e.into_inner()) = Some(queue);
}

/// A queue backed by a plain channel, for tests outside this module.
#[cfg(test)]
pub(crate) fn test_queue(
    asset_repo: Arc<dyn AssetRepository>,
) -> (AttachmentQueue, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel(64);
    (AttachmentQueue { tx, asset_repo }, rx)
}

fn registered_attachment_queue() -> Option<AttachmentQueue> {
    ATTACHMENT_QUEUE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
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

    /// The keyword-search index the worker writes attachment pages to.
    pub fn attachment_search(&self) -> Option<&dyn AttachmentSearchService> {
        self.attachment_search.as_deref()
    }

    /// Spawn the background worker draining a bounded queue of `capacity`, and
    /// return a handle for enqueuing keys.
    pub fn spawn(self: Arc<Self>, capacity: usize) -> AttachmentQueue {
        let (tx, mut rx) = mpsc::channel::<String>(capacity.max(1));
        let asset_repo = self.asset_repo.clone();
        tokio::spawn(async move {
            while let Some(key) = rx.recv().await {
                // Wait *after* taking the item, not before. The worker sits in
                // `recv` for as long as the queue is empty, so a check made
                // before it is stale by the time work arrives — which is how
                // the first version still failed an upload against an
                // exhausted ceiling.
                //
                // Holding the key across the wait is safe: it is only lost if
                // the process dies, and the asset is still `Pending` then, so
                // the startup sweep re-enqueues it. Failing it would not be
                // recoverable — the sweep skips `Failed`.
                wait_for_spend_headroom().await;

                // Failures are recorded on the asset by `process_one`; the
                // worker just moves on to the next key.
                if let Err(e) = self.process_one(&key, false).await {
                    tracing::warn!(key, "attachment extraction worker: {e}");
                }
            }
        });
        let queue = AttachmentQueue { tx, asset_repo };
        register_attachment_queue(queue.clone());
        queue
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

        // Linked only by hidden documents: keep it out of both indexes, without
        // spending an extraction on it.
        if referencing_view(self.document_repo.as_ref(), &asset.referenced_by)
            .await
            .ineligible
        {
            let _acl_guard = lock_attachment_acl(key).await;
            let references = match self.asset_repo.find_by_key(key).await? {
                Some(current) => current.referenced_by,
                None => return Ok(()),
            };
            if referencing_view(self.document_repo.as_ref(), &references)
                .await
                .ineligible
            {
                return self.withdraw(key).await;
            }
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
                // Under the lock, like every other keyword-search page write.
                let _acl_guard = lock_attachment_acl(key).await;
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

        // Extraction can take long enough for the references read above to be
        // stale; start from what they are now.
        let references = match self.asset_repo.find_by_key(key).await {
            Ok(Some(current)) => current.referenced_by,
            Ok(None) => return Ok(()), // deleted in the meantime
            Err(_) => asset.referenced_by.clone(),
        };
        let asset = Asset {
            referenced_by: references,
            ..asset
        };
        let view = referencing_view(self.document_repo.as_ref(), &asset.referenced_by).await;
        let access_levels = view.access_levels;
        let tags = view.tags;
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
                // simply shows up under that first document's page. Indexed
                // even while no document references it yet (the upload is
                // extracted before the linking document is ingested): the
                // recompute that follows that ingest only updates pages that
                // exist, so skipping them here would lose them for good.
                let (document_slug, document_title) = view.link;
                // Held from the page write through the final status write:
                // the pages are replaced as a whole, so a writer updating them
                // in place (an ACL update, the search re-index's page-text
                // backfill) must land entirely before or after.
                let acl_guard = lock_attachment_acl(key).await;
                if let Some(search) = &self.attachment_search {
                    if let Err(e) = search
                        .index_pages(
                            key,
                            &filename,
                            &document_slug,
                            &document_title,
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
                let Some(_acl_guard) = self
                    .reconcile_acl_after_index(key, &access_levels, &document_slug, acl_guard)
                    .await?
                else {
                    return Ok(());
                };
                // Still under the lock: a withdrawal waiting for it must land
                // after this write, never be overwritten by it.
                self.finish(key, ExtractionStatus::Done, &asset.content_hash, n as u32)
                    .await
            }
            Err(e) => self.fail(key, &format!("indexing failed: {e}")).await,
        }
    }

    /// Apply the ACL and document link the attachment's references imply
    /// *now*, if they differ from the ones just indexed (`indexed`,
    /// `indexed_slug`). Takes the attachment's ACL guard, acquired before the
    /// pages were written, and returns it, still held, so the
    /// caller records the final status before anyone else can withdraw it;
    /// `None` when the run must stop here (the asset is gone or was
    /// withdrawn), and an error when the update failed and the attachment was
    /// deindexed and marked failed.
    ///
    /// A document linking the attachment is typically ingested while the
    /// worker is still extracting it: the upload enqueues extraction first.
    /// That ingest's ACL recompute runs before these chunks exist and changes
    /// nothing, so without this check they keep the ACL of the earlier read —
    /// empty, for a freshly uploaded file — and stay invisible. Any recompute
    /// started after this read finds the chunks, so no change slips through.
    async fn reconcile_acl_after_index(
        &self,
        key: &str,
        indexed: &[String],
        indexed_slug: &str,
        acl_guard: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, AppError> {
        // Held from the read through the write; see `lock_attachment_acl`.
        let references = match self.asset_repo.find_by_key(key).await {
            Ok(Some(asset)) => asset.referenced_by,
            Ok(None) => {
                // Cleaned up as an orphan while we indexed: nothing will ever
                // clean these chunks up but us.
                let _ = self.rag.delete_attachment(key).await;
                if let Some(search) = &self.attachment_search {
                    let _ = search.delete_attachment(key).await;
                }
                return Ok(None);
            }
            Err(e) => {
                // The indexed ACL cannot be confirmed against the current
                // references, so fail closed like a failed repair.
                let failure = format!("cannot re-read references after indexing: {e}");
                fail_closed_attachment_acl(
                    key,
                    self.asset_repo.as_ref(),
                    Some(self.rag.as_ref()),
                    self.attachment_search.as_deref(),
                    std::slice::from_ref(&failure),
                )
                .await;
                return Err(AppError::Internal(format!(
                    "attachment deindexed: {failure}"
                )));
            }
        };
        let view = referencing_view(self.document_repo.as_ref(), &references).await;
        if view.ineligible {
            // Only hidden documents link it now.
            self.withdraw(key).await?;
            return Ok(None);
        }
        let current = view.access_levels;
        let (slug, title) = view.link;
        if current == indexed && slug == indexed_slug {
            return Ok(Some(acl_guard));
        }

        let mut failures = Vec::new();
        if let Err(e) = self
            .rag
            .update_attachment_access_levels(key, &current)
            .await
        {
            failures.push(format!("RAG ACL update failed: {e}"));
        }
        if let Some(search) = &self.attachment_search {
            if let Err(e) = search.update_reference(key, &slug, &title, &current).await {
                failures.push(format!("search ACL update failed: {e}"));
            }
        }
        if failures.is_empty() {
            return Ok(Some(acl_guard));
        }
        fail_closed_attachment_acl(
            key,
            self.asset_repo.as_ref(),
            Some(self.rag.as_ref()),
            self.attachment_search.as_deref(),
            &failures,
        )
        .await;
        Err(AppError::Internal(format!(
            "attachment deindexed: {}",
            failures.join("; ")
        )))
    }

    /// Withdraw an attachment only hidden documents link; see
    /// [`withdraw_attachment`].
    async fn withdraw(&self, key: &str) -> Result<(), AppError> {
        withdraw_attachment(
            key,
            self.asset_repo.as_ref(),
            Some(self.rag.as_ref()),
            self.attachment_search.as_deref(),
        )
        .await
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

    /// Record a failure and report it: the RAG reindex counts the error and
    /// keeps the key for "retry failed"; the queue worker logs it and moves
    /// on. A failure to write the status itself is ignored, so the reported
    /// error stays the processing one.
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
        Err(AppError::Internal(format!("attachment {key}: {msg}")))
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
        // Held from the read through the write; see `lock_attachment_acl`.
        let _acl_guard = lock_attachment_acl(key).await;
        let asset = match asset_repo.find_by_key(key).await {
            Ok(Some(a)) => a,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(key, "recompute attachment ACL: find asset failed: {e}");
                continue;
            }
        };

        if asset.referenced_by.is_empty() {
            // Deindex first, delete the blob, and remove the canonical Mongo
            // record only once every dependent store is confirmed clear. The
            // record is the sole anchor a future reconcile could use, so if any
            // step fails we keep it (marked failed) and retry later instead of
            // stranding indexed chunks with stale ACLs and no asset to drive
            // their cleanup. All deletes are idempotent, so a retry is safe.
            let mut failures = Vec::new();
            if let Err(e) = rag.delete_attachment(key).await {
                tracing::warn!(key, "delete orphaned attachment: RAG delete failed: {e}");
                failures.push(format!("RAG delete failed: {e}"));
            }
            if let Some(search) = attachment_search {
                if let Err(e) = search.delete_attachment(key).await {
                    tracing::warn!(key, "delete orphaned attachment: search delete failed: {e}");
                    failures.push(format!("search delete failed: {e}"));
                }
            }
            if let Err(e) = storage.delete_object(&asset.s3_key).await {
                tracing::warn!(
                    key,
                    "delete orphaned attachment: storage delete failed: {e}"
                );
                failures.push(format!("storage delete failed: {e}"));
            }

            if failures.is_empty() {
                if let Err(e) = asset_repo.delete(key).await {
                    tracing::warn!(key, "delete orphaned attachment: asset delete failed: {e}");
                }
            } else {
                mark_orphan_cleanup_failure(key, asset_repo, &failures).await;
            }
            continue;
        }

        let view = referencing_view(document_repo, &asset.referenced_by).await;
        if view.ineligible {
            // Only hidden documents link it: out of both indexes. The record
            // and blob stay, since it is still referenced.
            if let Err(e) = withdraw_attachment(key, asset_repo, Some(rag), attachment_search).await
            {
                tracing::warn!(key, "recompute attachment ACL: {e}");
            }
            continue;
        }
        if asset.extraction_status == ExtractionStatus::Pending {
            // Withdrawn earlier, or not processed yet: there may be no chunks
            // to update, so have the worker (re)process it.
            if let Some(queue) = registered_attachment_queue() {
                queue.enqueue(key);
            }
        }
        let levels = view.access_levels;
        let (slug, title) = view.link;

        let mut failures = Vec::new();

        if let Err(e) = rag.update_attachment_access_levels(key, &levels).await {
            tracing::warn!(key, "recompute attachment ACL: update failed: {e}");
            failures.push(format!("RAG ACL update failed: {e}"));
        }
        if let Some(search) = attachment_search {
            if let Err(e) = search.update_reference(key, &slug, &title, &levels).await {
                tracing::warn!(key, "recompute attachment ACL: search update failed: {e}");
                failures.push(format!("search ACL update failed: {e}"));
            }
        }

        if !failures.is_empty() {
            fail_closed_attachment_acl(key, asset_repo, Some(rag), attachment_search, &failures)
                .await;
        }
    }
}

/// Why an attachment linked only by hidden documents is not indexed.
const NO_VISIBLE_REFERENCER: &str =
    "no visible document links this attachment; it is indexed again once one does";

/// Remove an attachment that only hidden documents link from the given
/// indexes, leaving it `Pending` with no extracted hash: the ACL recompute
/// queues it again once a visible document links it, and the worker then
/// re-extracts it. The status is written first, so an interruption never
/// leaves a `Done` attachment missing from the indexes. Callers hold the
/// attachment's ACL lock. A failed removal fails closed (see
/// [`fail_closed_attachment_acl`]) and is reported.
async fn withdraw_attachment(
    key: &str,
    asset_repo: &dyn AssetRepository,
    rag: Option<&dyn RagService>,
    attachment_search: Option<&dyn AttachmentSearchService>,
) -> Result<(), AppError> {
    asset_repo
        .update_extraction(
            key,
            ExtractionUpdate {
                status: ExtractionStatus::Pending,
                error: Some(NO_VISIBLE_REFERENCER.to_string()),
                extracted_content_hash: None,
                extracted_at: None,
                indexed_chunks: Some(0),
            },
        )
        .await?;
    let mut failures = Vec::new();
    if let Some(rag) = rag {
        if let Err(e) = rag.delete_attachment(key).await {
            failures.push(format!("RAG delete failed: {e}"));
        }
    }
    if let Some(search) = attachment_search {
        if let Err(e) = search.delete_attachment(key).await {
            failures.push(format!("search delete failed: {e}"));
        }
    }
    if !failures.is_empty() {
        fail_closed_attachment_acl(key, asset_repo, rag, attachment_search, &failures).await;
        return Err(AppError::Internal(format!(
            "attachment {key} could not be withdrawn: {}",
            failures.join("; ")
        )));
    }
    Ok(())
}

/// What an orphan sweep found about an attachment it was about to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweptAttachment {
    /// Live after all (see [`crate::indexing::attachment_is_live`]): keep it.
    Live,
    /// Only hidden documents link it: withdrawn through
    /// [`withdraw_attachment`], so it comes back once a visible one does.
    Withdrawn,
    /// No asset record: the caller removes its entries.
    Gone,
}

/// Settle an attachment a sweep found in an index but not among the live
/// ones, under its ACL lock and against current MongoDB. A hidden-only
/// attachment is withdrawn rather than merely deleted, so it is left
/// `Pending` and re-queued when relinked; deleting its entries directly would
/// leave it `Done` and lost.
pub async fn settle_swept_attachment(
    key: &str,
    asset_repo: &dyn AssetRepository,
    document_repo: &dyn DocumentRepository,
    rag: Option<&dyn RagService>,
    attachment_search: Option<&dyn AttachmentSearchService>,
) -> Result<SweptAttachment, AppError> {
    let _acl_guard = lock_attachment_acl(key).await;
    let Some(asset) = asset_repo.find_by_key(key).await? else {
        return Ok(SweptAttachment::Gone);
    };
    if !referencing_view(document_repo, &asset.referenced_by)
        .await
        .ineligible
    {
        return Ok(SweptAttachment::Live);
    }
    withdraw_attachment(key, asset_repo, rag, attachment_search).await?;
    Ok(SweptAttachment::Withdrawn)
}

async fn fail_closed_attachment_acl(
    key: &str,
    asset_repo: &dyn AssetRepository,
    rag: Option<&dyn RagService>,
    attachment_search: Option<&dyn AttachmentSearchService>,
    failures: &[String],
) {
    let error = format!(
        "{}. Attachment deindexed until a successful reprocess refreshes ACLs.",
        failures.join("; ")
    );

    if let Some(rag) = rag {
        if let Err(e) = rag.delete_attachment(key).await {
            tracing::warn!(
                key,
                "recompute attachment ACL: fail-closed RAG delete failed: {e}"
            );
        }
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

/// Mark an orphaned attachment whose cleanup partially failed as `Failed`,
/// keeping the record so a later reconcile retries the delete. Without this the
/// record would either vanish (stranding indexed chunks) or stay silently
/// `Done`.
async fn mark_orphan_cleanup_failure(
    key: &str,
    asset_repo: &dyn AssetRepository,
    failures: &[String],
) {
    let error = format!(
        "orphaned attachment cleanup incomplete: {}. Record kept for retry.",
        failures.join("; ")
    );
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
            "delete orphaned attachment: failed to mark record for retry: {e}"
        );
    }
}

/// What an attachment's referencing documents grant it.
struct ReferencingView {
    /// From the visible, published, non-archived referencers (see
    /// [`attachment_access_levels`]).
    access_levels: Vec<String>,
    /// Union of the same referencers' tags.
    tags: Vec<String>,
    /// Slug and title of the first visible referencer, which keyword-search
    /// hits link to. Empty when there is none.
    link: (String, String),
    /// Referenced, and every reference resolves to a hidden document. Such an
    /// attachment belongs in neither index.
    ineligible: bool,
}

/// Derive an attachment's [`ReferencingView`] from its references.
///
/// Hidden documents are reachable by slug only, so they grant nothing: no
/// access level, no tags, no link. A failed document lookup fails closed on
/// visibility (empty ACL and link) but never marks the attachment ineligible,
/// so a transient error cannot withdraw it.
async fn referencing_view(
    document_repo: &dyn DocumentRepository,
    references: &[DocumentReference],
) -> ReferencingView {
    let slugs: Vec<_> = references
        .iter()
        .map(|reference| reference.slug.clone())
        .collect();
    let resolved = match document_repo.find_by_slugs(&slugs).await {
        Ok(resolved) => resolved,
        Err(e) => {
            tracing::warn!("attachment ACL: cannot resolve referencing documents: {e}");
            return ReferencingView {
                access_levels: Vec::new(),
                tags: Vec::new(),
                link: (String::new(), String::new()),
                ineligible: false,
            };
        }
    };
    let referencing: Vec<Option<&crate::db::models::Document>> = references
        .iter()
        .map(|reference| {
            resolved.iter().find(|doc| {
                doc.slug == reference.slug && doc.release.as_deref() == reference.release.as_deref()
            })
        })
        .collect();
    let visible: Vec<&crate::db::models::Document> = referencing
        .iter()
        .flatten()
        .copied()
        .filter(|doc| !doc.is_hidden)
        .collect();

    let mut tag_set = BTreeSet::new();
    for doc in visible.iter().filter(|d| !d.is_draft && !d.is_archived) {
        tag_set.extend(doc.tags.iter().cloned());
    }
    let refs: Vec<ReferencingDoc> = visible
        .iter()
        .map(|doc| ReferencingDoc {
            access_level: &doc.access_level,
            is_draft: doc.is_draft,
            is_archived: doc.is_archived,
        })
        .collect();
    ReferencingView {
        access_levels: attachment_access_levels(&refs),
        tags: tag_set.into_iter().collect(),
        link: visible
            .first()
            .map(|doc| (doc.slug.clone(), doc.title.clone()))
            .unwrap_or_default(),
        // An unresolved reference grants nothing but does not withdraw the
        // attachment either: only references known to be hidden do.
        ineligible: !referencing.is_empty()
            && referencing
                .iter()
                .all(|doc| doc.is_some_and(|doc| doc.is_hidden)),
    }
}

/// The last path segment of an asset key, used as the attachment's display name.
fn filename_from_key(key: &str) -> String {
    key.rsplit('/').next().unwrap_or(key).to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    #[tokio::test]
    async fn the_queue_does_not_wait_when_there_is_headroom() {
        // The default: no ceiling configured, so nothing is held back and the
        // worker must not pay a poll interval per item.
        let polls = AtomicUsize::new(0);

        super::wait_until(
            || {
                polls.fetch_add(1, AtomicOrdering::Relaxed);
                true
            },
            std::time::Duration::from_secs(3_600),
        )
        .await;

        assert_eq!(
            polls.load(AtomicOrdering::Relaxed),
            1,
            "asked once, went on"
        );
    }

    #[tokio::test]
    async fn the_queue_resumes_once_the_reserve_clears() {
        // Held back, then released: the attachment is still there to process,
        // which is the whole point of waiting rather than failing.
        let attempts = AtomicUsize::new(0);

        super::wait_until(
            || attempts.fetch_add(1, AtomicOrdering::Relaxed) >= 2,
            std::time::Duration::from_millis(1),
        )
        .await;

        assert!(
            attempts.load(AtomicOrdering::Relaxed) >= 3,
            "should have polled until the reserve cleared"
        );
    }

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
            referenced_by: referenced_by.into_iter().map(Into::into).collect(),
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
        /// A reference that lands right after the given (1-based) `find_by_key`
        /// call, the way a document ingest lands while the worker is busy.
        arriving_reference: Mutex<Option<(usize, crate::db::models::DocumentReference)>>,
        find_calls: std::sync::atomic::AtomicUsize,
        /// The (1-based) `find_by_key` call that fails.
        fail_find_at: Mutex<Option<usize>>,
        /// References that replace the stored ones right after the given
        /// (1-based) `find_by_key` call.
        replacing_references: Mutex<Option<(usize, Vec<crate::db::models::DocumentReference>)>>,
        /// Makes writes of a `Done` status slow, like a busy database.
        slow_done_write: AtomicBool,
    }

    impl FakeAssetRepo {
        fn new(assets: Vec<Asset>) -> Self {
            Self {
                assets: Mutex::new(assets),
                arriving_reference: Mutex::new(None),
                find_calls: Default::default(),
                fail_find_at: Mutex::new(None),
                replacing_references: Mutex::new(None),
                slow_done_write: AtomicBool::new(false),
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
            let mut assets = self.assets.lock().unwrap();
            let found = assets.iter().find(|a| a.key == key).cloned();
            let call = self.find_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if *self.fail_find_at.lock().unwrap() == Some(call) {
                return Err(AppError::Internal("asset lookup failed".to_string()));
            }
            let mut replacing = self.replacing_references.lock().unwrap();
            if replacing.as_ref().is_some_and(|(after, _)| *after == call) {
                let (_, references) = replacing.take().unwrap();
                if let Some(asset) = assets.iter_mut().find(|a| a.key == key) {
                    asset.referenced_by = references;
                }
            }
            let mut arriving = self.arriving_reference.lock().unwrap();
            if arriving.as_ref().is_some_and(|(after, _)| *after == call) {
                let (_, reference) = arriving.take().unwrap();
                if let Some(asset) = assets.iter_mut().find(|a| a.key == key) {
                    asset.referenced_by.push(reference);
                }
            }
            Ok(found)
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
        async fn update_extraction(
            &self,
            key: &str,
            update: ExtractionUpdate,
        ) -> Result<(), AppError> {
            if update.status == ExtractionStatus::Done
                && self.slow_done_write.load(Ordering::SeqCst)
            {
                sleep(Duration::from_millis(100)).await;
            }
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
        async fn set_release_references(
            &self,
            _: &crate::db::models::DocumentReference,
            _: &[String],
        ) -> Result<Vec<String>, AppError> {
            Ok(vec![])
        }
        async fn list_unfinished_extractions(&self) -> Result<Vec<Asset>, AppError> {
            Ok(self
                .assets
                .lock()
                .unwrap()
                .iter()
                .filter(|a| {
                    matches!(
                        a.extraction_status,
                        ExtractionStatus::Pending | ExtractionStatus::InProgress
                    )
                })
                .cloned()
                .collect())
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
            _: &crate::versioning::ReleasePins,
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
    }

    #[derive(Default)]
    struct RecordingRagService {
        deleted_attachment: AtomicBool,
        /// The access levels the last `index_attachment` wrote.
        indexed_acl: Mutex<Option<Vec<String>>>,
        updated_acl: Mutex<Option<Vec<String>>>,
        fail_update: AtomicBool,
        fail_delete: AtomicBool,
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
            _: Option<&str>,
            _: Option<&str>,
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
            access_levels: &[String],
            _: &[String],
        ) -> Result<usize, AppError> {
            *self.indexed_acl.lock().unwrap() = Some(access_levels.to_vec());
            *self.updated_acl.lock().unwrap() = None;
            Ok(1)
        }
        async fn delete_attachment(&self, _: &str) -> Result<(), AppError> {
            if self.fail_delete.load(Ordering::SeqCst) {
                return Err(AppError::Internal(
                    "simulated rag delete failure".to_string(),
                ));
            }
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
        async fn health_check(&self) -> Result<(), AppError> {
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
    async fn startup_sweep_reenqueues_only_unfinished_extractions() {
        let mut pending = make_asset("pdfs/pending.pdf", vec!["docs/x".to_string()]);
        pending.extraction_status = ExtractionStatus::Pending;
        let mut in_progress = make_asset("pdfs/inprogress.pdf", vec!["docs/y".to_string()]);
        in_progress.extraction_status = ExtractionStatus::InProgress;
        let mut done = make_asset("pdfs/done.pdf", vec!["docs/z".to_string()]);
        done.extraction_status = ExtractionStatus::Done;

        let asset_repo = Arc::new(FakeAssetRepo::new(vec![pending, in_progress, done]));
        let (tx, mut rx) = mpsc::channel(16);
        let queue = AttachmentQueue {
            tx,
            asset_repo: asset_repo.clone(),
        };

        resume_unfinished_extractions(asset_repo.as_ref(), &queue).await;

        let mut got = Vec::new();
        while let Ok(key) = rx.try_recv() {
            got.push(key);
        }
        got.sort();
        assert_eq!(
            got,
            vec![
                "pdfs/inprogress.pdf".to_string(),
                "pdfs/pending.pdf".to_string(),
            ],
            "only Pending/InProgress assets should be re-enqueued at startup"
        );
    }

    #[tokio::test]
    async fn recompute_access_levels_keeps_orphan_record_when_deindex_fails() {
        // When deindexing an orphaned attachment fails, the canonical Mongo
        // record must survive as the only anchor a future reconcile can use;
        // deleting it would strand the indexed chunks with stale ACLs forever.
        let asset_repo = FakeAssetRepo::new(vec![make_asset("pdfs/orphan.pdf", vec![])]);
        let document_repo = NoopDocumentRepo;
        let rag = RecordingRagService::default();
        rag.fail_delete.store(true, Ordering::SeqCst);
        let storage = MockStorage::new();
        storage
            .objects
            .lock()
            .unwrap()
            .insert("assets/pdfs/orphan.pdf".to_string(), vec![1, 2, 3]);
        let attachment_search = RecordingAttachmentSearchService::default();

        recompute_access_levels(
            &rag,
            &asset_repo,
            &document_repo,
            &storage,
            Some(&attachment_search),
            &["pdfs/orphan.pdf".to_string()],
        )
        .await;

        assert!(
            asset_repo.still_has("pdfs/orphan.pdf"),
            "orphan record must be kept as a retry anchor when deindexing fails"
        );
        let asset = asset_repo.find_local("pdfs/orphan.pdf").unwrap();
        assert_eq!(asset.extraction_status, ExtractionStatus::Failed);
        assert!(asset
            .extraction_error
            .as_deref()
            .unwrap_or_default()
            .contains("RAG delete failed"));
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
        /// (key, document slug) of every `index_pages` call.
        indexed: Mutex<Vec<(String, String)>>,
        /// (key, document slug, document title) of the last reference update.
        updated_link: Mutex<Option<(String, String, String)>>,
        fail_index: AtomicBool,
        fail_update: AtomicBool,
    }

    #[async_trait]
    impl crate::search::attachment_search::AttachmentSearchService
        for RecordingAttachmentSearchService
    {
        async fn index_pages(
            &self,
            key: &str,
            _: &str,
            document_slug: &str,
            _: &str,
            _: &[crate::rag::service::AttachmentPage],
            _: &[String],
        ) -> Result<(), AppError> {
            if self.fail_index.load(Ordering::SeqCst) {
                return Err(AppError::Internal(
                    "simulated search indexing failure".to_string(),
                ));
            }
            self.indexed
                .lock()
                .unwrap()
                .push((key.to_string(), document_slug.to_string()));
            Ok(())
        }
        async fn delete_attachment(&self, attachment_key: &str) -> Result<(), AppError> {
            self.deleted
                .lock()
                .unwrap()
                .push(attachment_key.to_string());
            Ok(())
        }
        async fn update_reference(
            &self,
            attachment_key: &str,
            document_slug: &str,
            document_title: &str,
            access_levels: &[String],
        ) -> Result<(), AppError> {
            if self.fail_update.load(Ordering::SeqCst) {
                return Err(AppError::Internal(
                    "simulated search acl failure".to_string(),
                ));
            }
            *self.updated_acl.lock().unwrap() =
                Some((attachment_key.to_string(), access_levels.to_vec()));
            *self.updated_link.lock().unwrap() = Some((
                attachment_key.to_string(),
                document_slug.to_string(),
                document_title.to_string(),
            ));
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

    impl RecordingRagService {
        /// The access levels the attachment's chunks carry now.
        fn effective_acl(&self) -> Option<Vec<String>> {
            self.updated_acl
                .lock()
                .unwrap()
                .clone()
                .or_else(|| self.indexed_acl.lock().unwrap().clone())
        }
    }

    /// Resolves a fixed set of documents.
    struct StaticDocumentRepo(Vec<Document>);

    #[async_trait]
    impl DocumentRepository for StaticDocumentRepo {
        async fn create_or_update(&self, _: Document) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_slug(&self, _: &str) -> Result<Option<Document>, AppError> {
            Ok(None)
        }
        async fn find_by_slugs(&self, slugs: &[String]) -> Result<Vec<Document>, AppError> {
            Ok(self
                .0
                .iter()
                .filter(|d| slugs.contains(&d.slug))
                .cloned()
                .collect())
        }
        async fn list_all(&self) -> Result<Vec<Document>, AppError> {
            Ok(self.0.clone())
        }
        async fn list_by_access_levels(
            &self,
            _: Option<&[String]>,
            _: bool,
            _: &crate::versioning::ReleasePins,
        ) -> Result<Vec<Document>, AppError> {
            Ok(self.0.clone())
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
    }

    fn internal_doc(slug: &str) -> Document {
        Document {
            slug: slug.to_string(),
            title: "Guide".to_string(),
            summary: None,
            s3_key: format!("docs/{slug}.md"),
            access_level: "internal".to_string(),
            is_draft: false,
            service_owner: "team".to_string(),
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
            skip_rag: false,
        }
    }

    /// A text attachment `key` stored in a fresh asset repo and storage.
    async fn text_attachment(
        key: &str,
        referenced_by: Vec<String>,
    ) -> (Arc<FakeAssetRepo>, Arc<MockStorage>) {
        let mut asset = make_asset(key, referenced_by);
        asset.content_type = "text/plain".to_string();
        asset.content_hash = Some("sha256:content".to_string());
        asset.extraction_status = ExtractionStatus::Pending;
        let storage = Arc::new(MockStorage::new());
        storage
            .put_object(&format!("assets/{key}"), b"attachment text".to_vec())
            .await
            .unwrap();
        (Arc::new(FakeAssetRepo::new(vec![asset])), storage)
    }

    /// The upload enqueues extraction before the document linking the file is
    /// ingested. When the ingest lands while the worker is extracting, its ACL
    /// recompute finds no chunks yet, and the worker must not then index the
    /// chunks with the ACL of the reference-less snapshot it started from.
    #[tokio::test]
    async fn a_reference_landing_during_extraction_reaches_the_indexed_acl() {
        // After the first read: lands while extracting. After the second:
        // lands while indexing, after the ACL was computed.
        for lands_after_read in [1, 2] {
            let key = "attachments/docs/guide/spec.txt";
            let (asset_repo, storage) = text_attachment(key, vec![]).await;
            *asset_repo.arriving_reference.lock().unwrap() =
                Some((lands_after_read, "docs/guide".into()));
            let rag = Arc::new(RecordingRagService::default());
            let search = Arc::new(RecordingAttachmentSearchService::default());

            let service = AttachmentExtractionService::new(
                storage,
                asset_repo.clone(),
                Arc::new(StaticDocumentRepo(vec![internal_doc("docs/guide")])),
                rag.clone(),
                Some(search.clone()),
                Arc::new(AttachmentExtractors::new(100, None)),
            );
            service.process_one(key, false).await.unwrap();

            assert_eq!(
                rag.effective_acl(),
                Some(vec!["internal".to_string()]),
                "reference landing after read {lands_after_read}"
            );
            assert_eq!(
                asset_repo.find_local(key).unwrap().extraction_status,
                ExtractionStatus::Done
            );
        }
    }

    /// The document linking the attachment is ingested only after the worker
    /// finished. The pages must still reach keyword search, and the ingest's
    /// recompute must then point them at the document with its ACL.
    #[tokio::test]
    async fn pages_reach_keyword_search_before_any_document_references_them() {
        let key = "attachments/docs/guide/spec.txt";
        let (asset_repo, storage) = text_attachment(key, vec![]).await;
        let rag = Arc::new(RecordingRagService::default());
        let search = Arc::new(RecordingAttachmentSearchService::default());
        let document_repo = Arc::new(StaticDocumentRepo(vec![internal_doc("docs/guide")]));

        let service = AttachmentExtractionService::new(
            storage.clone(),
            asset_repo.clone(),
            document_repo.clone(),
            rag.clone(),
            Some(search.clone()),
            Arc::new(AttachmentExtractors::new(100, None)),
        );
        service.process_one(key, false).await.unwrap();
        assert_eq!(
            *search.indexed.lock().unwrap(),
            vec![(key.to_string(), String::new())],
            "pages are indexed, linked to no document yet"
        );

        // The document is ingested and its recompute runs.
        asset_repo.assets.lock().unwrap()[0]
            .referenced_by
            .push("docs/guide".into());
        recompute_access_levels(
            rag.as_ref(),
            asset_repo.as_ref(),
            document_repo.as_ref(),
            storage.as_ref(),
            Some(search.as_ref()),
            &[key.to_string()],
        )
        .await;

        assert_eq!(
            *search.updated_link.lock().unwrap(),
            Some((
                key.to_string(),
                "docs/guide".to_string(),
                "Guide".to_string()
            ))
        );
        assert_eq!(
            *search.updated_acl.lock().unwrap(),
            Some((key.to_string(), vec!["internal".to_string()]))
        );
    }

    /// Keyword-search page writes happen under the attachment's lock, so a
    /// writer holding it (an ACL update, the search re-index's page-text
    /// backfill) never interleaves with a re-extraction replacing the pages.
    #[tokio::test]
    async fn pages_are_written_to_keyword_search_under_the_attachment_lock() {
        let key = "attachments/docs/guide/locked.txt";
        let (asset_repo, storage) = text_attachment(key, vec!["docs/guide".into()]).await;
        let search = Arc::new(RecordingAttachmentSearchService::default());
        let service = Arc::new(AttachmentExtractionService::new(
            storage,
            asset_repo.clone(),
            Arc::new(StaticDocumentRepo(vec![internal_doc("docs/guide")])),
            Arc::new(RecordingRagService::default()),
            Some(search.clone()),
            Arc::new(AttachmentExtractors::new(100, None)),
        ));

        let held = lock_attachment_acl(key).await;
        let worker = {
            let service = service.clone();
            tokio::spawn(async move { service.process_one(key, false).await })
        };
        sleep(Duration::from_millis(100)).await;
        assert!(
            search.indexed.lock().unwrap().is_empty(),
            "no page write while another writer holds the lock"
        );
        drop(held);
        worker.await.unwrap().unwrap();

        assert_eq!(search.indexed.lock().unwrap().len(), 1);
        assert_eq!(
            asset_repo.find_local(key).unwrap().extraction_status,
            ExtractionStatus::Done
        );
    }

    /// The worker's final `Done` write must not land after a withdrawal that
    /// ran once its post-index check released the lock: the attachment would
    /// read `Done` while out of both indexes, and nothing would re-queue it.
    #[tokio::test]
    async fn a_withdrawal_during_the_final_status_write_is_not_overwritten() {
        let key = "attachments/race/finish.txt";
        let (asset_repo, storage) = text_attachment(key, vec!["docs/visible".into()]).await;
        // Hidden-only right after the worker's post-index read (the third).
        *asset_repo.replacing_references.lock().unwrap() = Some((3, vec!["docs/hidden".into()]));
        asset_repo.slow_done_write.store(true, Ordering::SeqCst);
        let docs = Arc::new(StaticDocumentRepo(vec![
            internal_doc("docs/visible"),
            hidden_doc("docs/hidden"),
        ]));
        let rag = Arc::new(RecordingRagService::default());
        let service = Arc::new(AttachmentExtractionService::new(
            storage.clone(),
            asset_repo.clone(),
            docs.clone(),
            rag.clone(),
            None,
            Arc::new(AttachmentExtractors::new(100, None)),
        ));

        // Queue the worker first, then the recompute, behind the lock.
        let held = lock_attachment_acl(key).await;
        let worker = {
            let service = service.clone();
            tokio::spawn(async move { service.process_one(key, false).await })
        };
        sleep(Duration::from_millis(50)).await;
        let recompute = {
            let (rag, asset_repo, docs, storage) = (
                rag.clone(),
                asset_repo.clone(),
                docs.clone(),
                storage.clone(),
            );
            tokio::spawn(async move {
                recompute_access_levels(
                    rag.as_ref(),
                    asset_repo.as_ref(),
                    docs.as_ref(),
                    storage.as_ref(),
                    None,
                    &[key.to_string()],
                )
                .await;
            })
        };
        sleep(Duration::from_millis(50)).await;
        drop(held);
        worker.await.unwrap().unwrap();
        recompute.await.unwrap();

        assert!(rag.deleted_attachment.load(Ordering::SeqCst));
        assert_eq!(
            asset_repo.find_local(key).unwrap().extraction_status,
            ExtractionStatus::Pending
        );
    }

    /// The references cannot be re-read after indexing: the indexed ACL cannot
    /// be confirmed, so the attachment fails closed and the run reports it.
    #[tokio::test]
    async fn a_failed_reference_reread_after_indexing_fails_closed() {
        let key = "attachments/docs/guide/reread.txt";
        let (asset_repo, storage) = text_attachment(key, vec!["docs/guide".into()]).await;
        // Reads: on start, after extraction, then the post-index one.
        *asset_repo.fail_find_at.lock().unwrap() = Some(3);
        let rag = Arc::new(RecordingRagService::default());

        let service = AttachmentExtractionService::new(
            storage,
            asset_repo.clone(),
            Arc::new(StaticDocumentRepo(vec![internal_doc("docs/guide")])),
            rag.clone(),
            None,
            Arc::new(AttachmentExtractors::new(100, None)),
        );
        assert!(service.process_one(key, false).await.is_err());
        assert!(rag.deleted_attachment.load(Ordering::SeqCst));
        assert_eq!(
            asset_repo.find_local(key).unwrap().extraction_status,
            ExtractionStatus::Failed
        );
    }

    /// The ACL repair after indexing fails: the attachment is deindexed and
    /// marked failed, and the run reports it rather than succeeding.
    #[tokio::test]
    async fn a_failed_acl_repair_after_indexing_is_reported() {
        let key = "attachments/docs/guide/spec.txt";
        let (asset_repo, storage) = text_attachment(key, vec![]).await;
        // Lands while indexing, so the post-index check has to repair the ACL.
        *asset_repo.arriving_reference.lock().unwrap() = Some((2, "docs/guide".into()));
        let rag = Arc::new(RecordingRagService::default());
        rag.fail_update.store(true, Ordering::SeqCst);

        let service = AttachmentExtractionService::new(
            storage,
            asset_repo.clone(),
            Arc::new(StaticDocumentRepo(vec![internal_doc("docs/guide")])),
            rag.clone(),
            None,
            Arc::new(AttachmentExtractors::new(100, None)),
        );
        assert!(service.process_one(key, false).await.is_err());
        assert!(rag.deleted_attachment.load(Ordering::SeqCst));
        assert_eq!(
            asset_repo.find_local(key).unwrap().extraction_status,
            ExtractionStatus::Failed
        );
    }

    /// The recompute and the worker's post-index repair both read references
    /// and then write the ACL. Interleaved, the older read could overwrite a
    /// newer, narrower ACL, so both hold the attachment's lock across the pair.
    #[tokio::test]
    async fn acl_writers_wait_for_the_attachment_lock() {
        let key = "attachments/locked/spec.txt";
        let held = lock_attachment_acl(key).await;

        // The recompute.
        let (asset_repo, storage) = text_attachment(key, vec!["docs/guide".to_string()]).await;
        let rag = Arc::new(RecordingRagService::default());
        let docs = Arc::new(StaticDocumentRepo(vec![internal_doc("docs/guide")]));
        let recompute = {
            let (rag, asset_repo, docs, storage) = (
                rag.clone(),
                asset_repo.clone(),
                docs.clone(),
                storage.clone(),
            );
            tokio::spawn(async move {
                recompute_access_levels(
                    rag.as_ref(),
                    asset_repo.as_ref(),
                    docs.as_ref(),
                    storage.as_ref(),
                    None,
                    &[key.to_string()],
                )
                .await;
            })
        };

        // The worker, with a reference landing while it indexes, so its
        // post-index repair has an ACL to write.
        let (worker_assets, worker_storage) = text_attachment(key, vec![]).await;
        *worker_assets.arriving_reference.lock().unwrap() = Some((2, "docs/guide".into()));
        let worker_rag = Arc::new(RecordingRagService::default());
        let service = Arc::new(AttachmentExtractionService::new(
            worker_storage,
            worker_assets.clone(),
            docs.clone(),
            worker_rag.clone(),
            None,
            Arc::new(AttachmentExtractors::new(100, None)),
        ));
        let worker = {
            let service = service.clone();
            tokio::spawn(async move { service.process_one(key, false).await })
        };

        sleep(Duration::from_millis(100)).await;
        assert!(
            rag.updated_acl.lock().unwrap().is_none(),
            "the recompute wrote while the lock was held"
        );
        assert_eq!(
            worker_rag.indexed_acl.lock().unwrap().clone(),
            Some(vec![]),
            "indexing itself does not wait"
        );
        assert!(
            worker_rag.updated_acl.lock().unwrap().is_none(),
            "the post-index repair wrote while the lock was held"
        );

        drop(held);
        recompute.await.unwrap();
        worker.await.unwrap().unwrap();
        assert_eq!(
            *rag.updated_acl.lock().unwrap(),
            Some(vec!["internal".to_string()])
        );
        assert_eq!(
            worker_rag.effective_acl(),
            Some(vec!["internal".to_string()])
        );
    }

    fn hidden_doc(slug: &str) -> Document {
        Document {
            is_hidden: true,
            access_level: "public".to_string(),
            ..internal_doc(slug)
        }
    }

    /// A hidden document is reachable by slug only; it must not make the
    /// attachments it links retrievable, nor be what their hits link to.
    #[tokio::test]
    async fn hidden_referencers_grant_an_attachment_nothing() {
        let docs = Arc::new(StaticDocumentRepo(vec![
            hidden_doc("docs/hidden"),
            internal_doc("docs/visible"),
        ]));

        // Also referenced by a visible document: indexed with that one's ACL
        // and linked to it, although the hidden one comes first.
        let key = "attachments/shared/spec.txt";
        let (asset_repo, storage) =
            text_attachment(key, vec!["docs/hidden".into(), "docs/visible".into()]).await;
        let rag = Arc::new(RecordingRagService::default());
        let search = Arc::new(RecordingAttachmentSearchService::default());
        recompute_access_levels(
            rag.as_ref(),
            asset_repo.as_ref(),
            docs.as_ref(),
            storage.as_ref(),
            Some(search.as_ref()),
            &[key.to_string()],
        )
        .await;
        assert_eq!(
            *rag.updated_acl.lock().unwrap(),
            Some(vec!["internal".to_string()])
        );
        assert_eq!(
            search
                .updated_link
                .lock()
                .unwrap()
                .clone()
                .map(|(_, slug, _)| slug),
            Some("docs/visible".to_string())
        );

        // Referenced by the hidden document only: withdrawn from both indexes,
        // and left pending for when a visible document links it again.
        let key = "attachments/hidden-only/spec.txt";
        let (asset_repo, storage) = text_attachment(key, vec!["docs/hidden".into()]).await;
        asset_repo.assets.lock().unwrap()[0].extraction_status = ExtractionStatus::Done;
        let rag = Arc::new(RecordingRagService::default());
        let search = Arc::new(RecordingAttachmentSearchService::default());
        recompute_access_levels(
            rag.as_ref(),
            asset_repo.as_ref(),
            docs.as_ref(),
            storage.as_ref(),
            Some(search.as_ref()),
            &[key.to_string()],
        )
        .await;
        assert!(rag.deleted_attachment.load(Ordering::SeqCst));
        assert_eq!(*search.deleted.lock().unwrap(), vec![key.to_string()]);
        assert!(rag.updated_acl.lock().unwrap().is_none());
        let asset = asset_repo.find_local(key).unwrap();
        assert_eq!(asset.extraction_status, ExtractionStatus::Pending);
        assert!(asset.extracted_content_hash.is_none());
        assert!(
            asset_repo.still_has(key),
            "still referenced: the record stays"
        );
    }

    #[tokio::test]
    async fn extraction_does_not_index_an_attachment_only_hidden_documents_link() {
        let key = "attachments/hidden-only/extract.txt";
        let (asset_repo, storage) = text_attachment(key, vec!["docs/hidden".into()]).await;
        let rag = Arc::new(RecordingRagService::default());
        let service = AttachmentExtractionService::new(
            storage,
            asset_repo.clone(),
            Arc::new(StaticDocumentRepo(vec![hidden_doc("docs/hidden")])),
            rag.clone(),
            None,
            Arc::new(AttachmentExtractors::new(100, None)),
        );
        service.process_one(key, true).await.unwrap();

        assert!(rag.indexed_acl.lock().unwrap().is_none());
        assert!(rag.deleted_attachment.load(Ordering::SeqCst));
        assert_eq!(
            asset_repo.find_local(key).unwrap().extraction_status,
            ExtractionStatus::Pending
        );
    }

    /// Withdrawn while only hidden documents linked it; once a visible one
    /// does, the recompute queues it, since there are no chunks to update.
    #[tokio::test]
    async fn a_withdrawn_attachment_is_requeued_once_a_visible_document_links_it() {
        let key = "attachments/requeue/spec.txt";
        let (asset_repo, storage) = text_attachment(key, vec!["docs/visible".into()]).await;
        let (tx, mut rx) = mpsc::channel(16);
        register_attachment_queue(AttachmentQueue {
            tx,
            asset_repo: asset_repo.clone(),
        });

        recompute_access_levels(
            &RecordingRagService::default(),
            asset_repo.as_ref(),
            &StaticDocumentRepo(vec![internal_doc("docs/visible")]),
            storage.as_ref(),
            None,
            &[key.to_string()],
        )
        .await;

        let mut queued = Vec::new();
        while let Ok(k) = rx.try_recv() {
            queued.push(k);
        }
        assert!(queued.contains(&key.to_string()), "queued: {queued:?}");
    }

    /// Chunks already stored with an empty ACL are repaired by a forced
    /// re-process, which is what the RAG reindex runs for every attachment.
    #[tokio::test]
    async fn a_forced_reprocess_repairs_an_empty_acl() {
        let key = "attachments/docs/guide/spec.txt";
        let (asset_repo, storage) = text_attachment(key, vec!["docs/guide".to_string()]).await;
        {
            let mut assets = asset_repo.assets.lock().unwrap();
            assets[0].extraction_status = ExtractionStatus::Done;
            assets[0].extracted_content_hash = Some("sha256:content".to_string());
        }
        let rag = Arc::new(RecordingRagService::default());
        *rag.indexed_acl.lock().unwrap() = Some(vec![]);

        let service = AttachmentExtractionService::new(
            storage,
            asset_repo,
            Arc::new(StaticDocumentRepo(vec![internal_doc("docs/guide")])),
            rag.clone(),
            None,
            Arc::new(AttachmentExtractors::new(100, None)),
        );
        service.process_one(key, true).await.unwrap();

        assert_eq!(rag.effective_acl(), Some(vec!["internal".to_string()]));
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

        // Reported, so a reindex counts it and "retry failed" picks it up.
        let err = service.process_one(key, true).await.unwrap_err();
        assert!(err.to_string().contains("keyword-search indexing failed"));

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
