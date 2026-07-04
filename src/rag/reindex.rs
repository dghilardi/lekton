use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::db::asset_repository::AssetRepository;
use crate::db::repository::DocumentRepository;
use crate::rag::attachment_extraction::AttachmentExtractionService;
use crate::rag::service::RagService;
use crate::storage::client::StorageClient;

/// Shared state for tracking a background re-index operation.
#[derive(Default)]
pub struct ReindexState {
    pub is_running: AtomicBool,
    /// Progress percentage (0–100).
    pub progress: AtomicU32,
}

/// Run a full re-index of all non-archived documents, plus every referenced
/// PDF attachment (force-reprocessed regardless of content-hash match), so
/// documents uploaded before extraction was wired up — or a changed
/// chunking/embedding/extraction configuration — are picked up without
/// requiring a no-op re-upload of each file.
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
    let documents: Vec<_> = documents.into_iter().filter(|d| !d.is_archived).collect();

    // Only attachments still referenced by at least one document are worth
    // re-indexing; an empty `referenced_by` means it is orphaned and pending
    // cleanup (see `recompute_access_levels`), not something to re-embed.
    let assets: Vec<_> = match asset_repo.list_all().await {
        Ok(assets) => assets
            .into_iter()
            .filter(|a| !a.referenced_by.is_empty())
            .collect(),
        Err(e) => {
            tracing::error!("RAG reindex: failed to list assets: {e}");
            vec![]
        }
    };

    let total = documents.len() + assets.len();
    if total == 0 {
        tracing::info!("RAG reindex: nothing to index");
        reindex.progress.store(100, Ordering::Relaxed);
        reindex.is_running.store(false, Ordering::Release);
        return;
    }

    tracing::info!(
        documents = documents.len(),
        assets = assets.len(),
        "RAG reindex: starting"
    );

    let mut done = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for doc in &documents {
        // A skip_rag document (e.g. a PDF upload stub) is deliberately excluded
        // from RAG; a full reindex must respect that rather than silently
        // re-adding it, and clean up any chunks left by an earlier ingest.
        if doc.skip_rag {
            if let Err(e) = rag.delete_document(&doc.slug).await {
                tracing::warn!(slug = %doc.slug, "RAG reindex: failed to remove skip_rag document: {e}");
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
                done += 1;
                reindex
                    .progress
                    .store((done * 100 / total) as u32, Ordering::Relaxed);
                continue;
            }
            Err(e) => {
                tracing::warn!(slug = %doc.slug, "RAG reindex: failed to read from S3: {e}");
                failed += 1;
                done += 1;
                reindex
                    .progress
                    .store((done * 100 / total) as u32, Ordering::Relaxed);
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
            failed += 1;
        }

        done += 1;
        reindex
            .progress
            .store((done * 100 / total) as u32, Ordering::Relaxed);
    }

    let mut attachments_skipped = 0usize;
    let mut attachments_failed = 0usize;

    if let Some(service) = &attachment_service {
        for asset in &assets {
            if let Err(e) = service.process_one(&asset.key, true).await {
                tracing::warn!(key = %asset.key, "RAG reindex: failed to re-index attachment: {e}");
                attachments_failed += 1;
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
        "RAG reindex: complete"
    );
    reindex.progress.store(100, Ordering::Relaxed);
    reindex.is_running.store(false, Ordering::Release);
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
            referenced_by,
            content_hash: Some("sha256:same".to_string()),
            extraction_status: ExtractionStatus::Done,
            extraction_error: None,
            extracted_content_hash: Some("sha256:same".to_string()),
            extracted_at: None,
            indexed_chunks: Some(1),
        }
    }

    struct FakeDocRepo {
        docs: Vec<Document>,
    }

    #[async_trait]
    impl DocumentRepository for FakeDocRepo {
        async fn create_or_update(&self, _: Document) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_slug(&self, _: &str) -> Result<Option<Document>, AppError> {
            Ok(None)
        }
        async fn list_all(&self) -> Result<Vec<Document>, AppError> {
            Ok(self.docs.clone())
        }
        async fn list_by_access_levels(
            &self,
            _: Option<&[String]>,
            _: bool,
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
        async fn set_references(&self, _: &str, _: &[String]) -> Result<Vec<String>, AppError> {
            Ok(vec![])
        }
    }

    #[derive(Default)]
    struct RecordingRagService {
        indexed_slugs: Mutex<Vec<String>>,
        deleted_slugs: Mutex<Vec<String>>,
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
        ) -> Result<(), AppError> {
            self.indexed_slugs.lock().unwrap().push(slug.to_string());
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
        });
        let rag = Arc::new(RecordingRagService::default());
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![]));

        run_reindex(reindex, doc_repo, storage, rag.clone(), asset_repo, None).await;

        assert_eq!(*rag.indexed_slugs.lock().unwrap(), vec!["normal"]);
        assert_eq!(*rag.deleted_slugs.lock().unwrap(), vec!["upload-stub"]);
    }

    #[tokio::test]
    async fn run_reindex_force_reprocesses_referenced_attachments_only() {
        let reindex = Arc::new(ReindexState::default());
        let storage = Arc::new(MockStorage::new());
        let doc_repo = Arc::new(FakeDocRepo { docs: vec![] });
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
        let doc_repo = Arc::new(FakeDocRepo { docs: vec![] });
        let rag = Arc::new(RecordingRagService::default());
        let asset_repo = Arc::new(FakeAssetRepo::new(vec![make_asset(
            "pdfs/referenced.pdf",
            vec!["docs/a".to_string()],
        )]));

        run_reindex(reindex.clone(), doc_repo, storage, rag, asset_repo, None).await;

        assert!(!reindex.is_running.load(Ordering::Acquire));
        assert_eq!(reindex.progress.load(Ordering::Relaxed), 100);
    }
}
