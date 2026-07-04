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
use tokio::sync::mpsc;

use crate::db::asset_repository::{AssetRepository, ExtractionUpdate};
use crate::db::models::ExtractionStatus;
use crate::db::repository::DocumentRepository;
use crate::error::AppError;
use crate::rag::attachment_acl::{attachment_access_levels, ReferencingDoc};
use crate::rag::extraction::{AttachmentExtractors, ExtractionOutcome};
use crate::rag::service::RagService;
use crate::storage::client::StorageClient;

/// Handle for enqueuing attachment keys for (re)extraction. Cloneable and cheap.
#[derive(Clone)]
pub struct AttachmentQueue {
    tx: mpsc::Sender<String>,
}

impl AttachmentQueue {
    /// Enqueue an asset key. Non-blocking: if the bounded queue is full (or the
    /// worker is gone) the key is dropped — it stays `Pending` and a later
    /// upload or backfill will pick it up.
    pub fn enqueue(&self, key: &str) {
        if let Err(e) = self.tx.try_send(key.to_string()) {
            tracing::warn!(
                key,
                "attachment extraction queue full/closed, skipping: {e}"
            );
        }
    }
}

/// Owns the dependencies needed to extract and index a single attachment.
pub struct AttachmentExtractionService {
    storage: Arc<dyn StorageClient>,
    asset_repo: Arc<dyn AssetRepository>,
    document_repo: Arc<dyn DocumentRepository>,
    rag: Arc<dyn RagService>,
    extractors: Arc<AttachmentExtractors>,
}

impl AttachmentExtractionService {
    pub fn new(
        storage: Arc<dyn StorageClient>,
        asset_repo: Arc<dyn AssetRepository>,
        document_repo: Arc<dyn DocumentRepository>,
        rag: Arc<dyn RagService>,
        extractors: Arc<AttachmentExtractors>,
    ) -> Self {
        Self {
            storage,
            asset_repo,
            document_repo,
            rag,
            extractors,
        }
    }

    /// Spawn the background worker draining a bounded queue of `capacity`, and
    /// return a handle for enqueuing keys.
    pub fn spawn(self: Arc<Self>, capacity: usize) -> AttachmentQueue {
        let (tx, mut rx) = mpsc::channel::<String>(capacity.max(1));
        tokio::spawn(async move {
            while let Some(key) = rx.recv().await {
                if let Err(e) = self.process_one(&key).await {
                    tracing::error!(key, "attachment extraction worker error: {e}");
                }
            }
        });
        AttachmentQueue { tx }
    }

    /// Extract and index one attachment, updating its extraction status. Safe to
    /// call repeatedly; skips work when the content is unchanged since the last
    /// successful run.
    pub async fn process_one(&self, key: &str) -> Result<(), AppError> {
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
        if already_processed
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
            continue;
        }

        let (levels, _tags) = referencing_acl(document_repo, &asset.referenced_by).await;

        if let Err(e) = rag.update_attachment_access_levels(key, &levels).await {
            tracing::warn!(key, "recompute attachment ACL: update failed: {e}");
        }
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
        async fn update_extraction(&self, _: &str, _: ExtractionUpdate) -> Result<(), AppError> {
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
}
