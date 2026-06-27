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

        // Already indexed for this exact content: nothing to do.
        if asset.extraction_status == ExtractionStatus::Done
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

        let (access_levels, tags) = self.derive_acl_and_tags(&asset.referenced_by).await?;
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

    /// Access levels and tags inherited from the published, non-archived
    /// documents that reference the attachment.
    async fn derive_acl_and_tags(
        &self,
        slugs: &[String],
    ) -> Result<(Vec<String>, Vec<String>), AppError> {
        let mut docs = Vec::new();
        let mut tag_set = BTreeSet::new();
        for slug in slugs {
            if let Some(doc) = self.document_repo.find_by_slug(slug).await? {
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
        Ok((
            attachment_access_levels(&refs),
            tag_set.into_iter().collect(),
        ))
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
/// documents changed (e.g. after a document save). A no-op for keys with no
/// indexed chunks. Per-key errors are logged, never propagated, so a document
/// save is not failed by RAG bookkeeping.
pub async fn recompute_access_levels(
    rag: &dyn RagService,
    asset_repo: &dyn AssetRepository,
    document_repo: &dyn DocumentRepository,
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

        let mut docs = Vec::new();
        for slug in &asset.referenced_by {
            if let Ok(Some(doc)) = document_repo.find_by_slug(slug).await {
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
        let levels = attachment_access_levels(&refs);

        if let Err(e) = rag.update_attachment_access_levels(key, &levels).await {
            tracing::warn!(key, "recompute attachment ACL: update failed: {e}");
        }
    }
}

/// The last path segment of an asset key, used as the attachment's display name.
fn filename_from_key(key: &str) -> String {
    key.rsplit('/').next().unwrap_or(key).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_from_key_takes_last_segment() {
        assert_eq!(
            filename_from_key("project-a/configs/manual.pdf"),
            "manual.pdf"
        );
        assert_eq!(filename_from_key("readme.txt"), "readme.txt");
        assert_eq!(filename_from_key(""), "");
    }
}
