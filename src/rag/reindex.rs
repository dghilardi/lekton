use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::db::repository::DocumentRepository;
use crate::rag::service::RagService;
use crate::storage::client::StorageClient;

/// Shared state for tracking a background re-index operation.
#[derive(Default)]
pub struct ReindexState {
    pub is_running: AtomicBool,
    /// Progress percentage (0–100).
    pub progress: AtomicU32,
}

/// Run a full re-index of all non-archived documents.
///
/// This function is meant to be spawned as a background Tokio task.
/// It updates `state` with progress as it goes and resets `is_running`
/// on completion (or failure).
pub async fn run_reindex(
    reindex: Arc<ReindexState>,
    document_repo: Arc<dyn DocumentRepository>,
    storage: Arc<dyn StorageClient>,
    rag: Arc<dyn RagService>,
) {
    reindex.progress.store(0, Ordering::Relaxed);

    // Load all non-archived documents (None = no access level filter, true = include drafts)
    let documents = match document_repo.list_by_access_levels(None, true).await {
        Ok(docs) => docs,
        Err(e) => {
            tracing::error!("RAG reindex: failed to list documents: {e}");
            reindex.is_running.store(false, Ordering::Release);
            return;
        }
    };

    // Filter out archived documents
    let documents: Vec<_> = documents.into_iter().filter(|d| !d.is_archived).collect();
    let total = documents.len();

    if total == 0 {
        tracing::info!("RAG reindex: no documents to index");
        reindex.progress.store(100, Ordering::Relaxed);
        reindex.is_running.store(false, Ordering::Release);
        return;
    }

    tracing::info!(total, "RAG reindex: starting");

    for (i, doc) in documents.iter().enumerate() {
        // Fetch content from S3
        let content = match storage.get_object(&doc.s3_key).await {
            Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Ok(None) => {
                tracing::warn!(slug = %doc.slug, "RAG reindex: content not found in S3, skipping");
                continue;
            }
            Err(e) => {
                tracing::warn!(slug = %doc.slug, "RAG reindex: failed to read from S3: {e}");
                continue;
            }
        };

        // Index the document
        if let Err(e) = rag
            .index_document(
                &doc.slug,
                &doc.title,
                &content,
                &doc.access_level,
                doc.is_draft,
                &doc.tags,
            )
            .await
        {
            tracing::warn!(slug = %doc.slug, "RAG reindex: failed to index: {e}");
        }

        // Update progress
        let pct = ((i + 1) * 100 / total) as u32;
        reindex.progress.store(pct, Ordering::Relaxed);
    }

    tracing::info!(total, "RAG reindex: complete");
    reindex.progress.store(100, Ordering::Relaxed);
    reindex.is_running.store(false, Ordering::Release);
}
