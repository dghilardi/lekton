use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::db::repository::DocumentRepository;
use crate::search::client::{build_search_document, SearchService};
use crate::storage::client::StorageClient;

/// Shared state for tracking a background full-text search re-index operation.
#[derive(Default)]
pub struct SearchReindexState {
    pub is_running: AtomicBool,
    /// Progress percentage (0-100).
    pub progress: AtomicU32,
    /// Per-run failed/skipped counters and last error.
    pub outcome: crate::jobs::JobOutcome,
}

impl crate::jobs::RunningFlag for SearchReindexState {
    fn is_running(&self) -> &AtomicBool {
        &self.is_running
    }
}

/// Run a full reconciliation of the Meilisearch `documents` index.
///
/// Active, visible documents are indexed from the canonical MongoDB metadata and
/// S3 markdown content. Hidden or archived documents are deleted from the search
/// index so stale results do not remain after metadata-only changes.
pub async fn run_reindex(
    reindex: Arc<SearchReindexState>,
    document_repo: Arc<dyn DocumentRepository>,
    storage: Arc<dyn StorageClient>,
    search: Arc<dyn SearchService>,
) {
    // Reset `is_running` unconditionally when this task ends, even on an early
    // return or panic, so a crashed reindex cannot block all future runs.
    let _guard = crate::jobs::RunningGuard::new(reindex.clone());

    reindex.progress.store(0, Ordering::Relaxed);
    reindex.outcome.reset();

    if let Err(e) = search.configure_index().await {
        tracing::warn!("Search reindex: failed to configure Meilisearch index: {e}");
    }

    let documents = match document_repo.list_all().await {
        Ok(docs) => docs,
        Err(e) => {
            tracing::error!("Search reindex: failed to list documents: {e}");
            return;
        }
    };

    let total = documents.len();
    if total == 0 {
        tracing::info!("Search reindex: no documents to index");
        reindex.progress.store(100, Ordering::Relaxed);
        return;
    }

    tracing::info!(total, "Search reindex: starting");

    for (i, doc) in documents.iter().enumerate() {
        if doc.is_archived || doc.is_hidden {
            if let Err(e) = search.delete_document(&doc.slug).await {
                tracing::warn!(slug = %doc.slug, "Search reindex: failed to delete stale document: {e}");
                reindex
                    .outcome
                    .record_failure(format!("delete {}: {e}", doc.slug));
            }
            update_progress(&reindex, i, total);
            continue;
        }

        let content = match storage.get_object(&doc.s3_key).await {
            Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Ok(None) => {
                tracing::warn!(slug = %doc.slug, "Search reindex: content not found in storage, skipping");
                reindex.outcome.record_skip();
                update_progress(&reindex, i, total);
                continue;
            }
            Err(e) => {
                tracing::warn!(slug = %doc.slug, "Search reindex: failed to read from storage: {e}");
                reindex
                    .outcome
                    .record_failure(format!("read {}: {e}", doc.slug));
                update_progress(&reindex, i, total);
                continue;
            }
        };

        let search_doc = build_search_document(doc, &content);
        if let Err(e) = search.index_document(&search_doc).await {
            tracing::warn!(slug = %doc.slug, "Search reindex: failed to index document: {e}");
            reindex
                .outcome
                .record_failure(format!("index {}: {e}", doc.slug));
        }

        update_progress(&reindex, i, total);
    }

    tracing::info!(total, "Search reindex: complete");
    reindex.progress.store(100, Ordering::Relaxed);
}

fn update_progress(reindex: &SearchReindexState, index: usize, total: usize) {
    let pct = ((index + 1) * 100 / total) as u32;
    reindex.progress.store(pct, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;

    use crate::db::models::Document;
    use crate::error::AppError;
    use crate::search::client::{SearchDocument, SearchHit};

    struct MockDocumentRepo {
        documents: Vec<Document>,
    }

    #[async_trait]
    impl DocumentRepository for MockDocumentRepo {
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
            Ok(self.documents.clone())
        }

        async fn list_by_access_levels(
            &self,
            _: Option<&[String]>,
            _: bool,
            _: &crate::versioning::ReleasePins,
        ) -> Result<Vec<Document>, AppError> {
            Ok(self.documents.clone())
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
            Ok(self.documents.clone())
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
    struct MockStorage {
        objects: Mutex<HashMap<String, Vec<u8>>>,
    }

    #[async_trait]
    impl StorageClient for MockStorage {
        async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), AppError> {
            self.objects
                .lock()
                .unwrap()
                .insert(key.to_string(), content);
            Ok(())
        }

        async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, AppError> {
            Ok(self.objects.lock().unwrap().get(key).cloned())
        }

        async fn delete_object(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingSearch {
        configured: AtomicBool,
        indexed: Mutex<Vec<SearchDocument>>,
        deleted: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SearchService for RecordingSearch {
        async fn index_document(&self, doc: &SearchDocument) -> Result<(), AppError> {
            self.indexed.lock().unwrap().push(doc.clone());
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
        ) -> Result<Vec<SearchHit>, AppError> {
            Ok(vec![])
        }

        async fn configure_index(&self) -> Result<(), AppError> {
            self.configured.store(true, Ordering::Relaxed);
            Ok(())
        }
        async fn health_check(&self) -> Result<(), AppError> {
            Ok(())
        }
    }

    fn make_doc(slug: &str, is_hidden: bool, is_archived: bool) -> Document {
        Document {
            slug: slug.to_string(),
            title: format!("Title {slug}"),
            summary: None,
            s3_key: format!("docs/{}.md", slug.replace('/', "_")),
            access_level: "internal".to_string(),
            is_draft: false,
            service_owner: "platform".to_string(),
            last_updated: Utc::now(),
            tags: vec!["tag".to_string()],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden,
            content_hash: None,
            metadata_hash: None,
            is_archived,
            source_path: None,
            source_id: None,
            release: None,
            is_latest: true,
            needs_reindex: false,
            skip_rag: false,
        }
    }

    #[tokio::test]
    async fn reindex_indexes_active_documents_and_deletes_hidden_or_archived() {
        let active = make_doc("docs/active", false, false);
        let hidden = make_doc("docs/hidden", true, false);
        let archived = make_doc("docs/archived", false, true);

        let repo = Arc::new(MockDocumentRepo {
            documents: vec![active.clone(), hidden.clone(), archived.clone()],
        });
        let storage = Arc::new(MockStorage::default());
        storage
            .put_object(&active.s3_key, b"# Active\n\nVisible content".to_vec())
            .await
            .unwrap();
        let search = Arc::new(RecordingSearch::default());
        let state = Arc::new(SearchReindexState {
            is_running: AtomicBool::new(true),
            progress: AtomicU32::new(0),
            ..Default::default()
        });

        run_reindex(state.clone(), repo, storage, search.clone()).await;

        assert!(!state.is_running.load(Ordering::Acquire));
        assert_eq!(state.progress.load(Ordering::Relaxed), 100);
        assert!(search.configured.load(Ordering::Relaxed));

        let indexed = search.indexed.lock().unwrap();
        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0].slug, active.slug);
        assert!(indexed[0].content_preview.contains("Active"));

        let deleted = search.deleted.lock().unwrap();
        assert_eq!(&*deleted, &vec![hidden.slug.clone(), archived.slug.clone()]);
    }

    #[tokio::test]
    async fn reindex_counts_failed_documents_and_records_last_error() {
        struct FailingSearch;
        #[async_trait]
        impl SearchService for FailingSearch {
            async fn index_document(&self, _: &SearchDocument) -> Result<(), AppError> {
                Err(AppError::Internal("index rejected".to_string()))
            }
            async fn delete_document(&self, _: &str) -> Result<(), AppError> {
                Ok(())
            }
            async fn search(
                &self,
                _: &str,
                _: Option<&[String]>,
                _: bool,
            ) -> Result<Vec<SearchHit>, AppError> {
                Ok(vec![])
            }
            async fn configure_index(&self) -> Result<(), AppError> {
                Ok(())
            }
            async fn health_check(&self) -> Result<(), AppError> {
                Ok(())
            }
        }

        let doc = make_doc("a", false, false);
        let repo = Arc::new(MockDocumentRepo {
            documents: vec![doc.clone()],
        });
        let storage = Arc::new(MockStorage::default());
        storage
            .put_object(&doc.s3_key, b"# A".to_vec())
            .await
            .unwrap();
        let state = Arc::new(SearchReindexState::default());
        state.is_running.store(true, Ordering::Release);

        run_reindex(state.clone(), repo, storage, Arc::new(FailingSearch)).await;

        let (failed, skipped, last_error) = state.outcome.snapshot();
        assert_eq!(failed, 1, "a failed index must be counted");
        assert_eq!(skipped, 0);
        assert!(
            last_error
                .as_deref()
                .unwrap_or_default()
                .contains("index a"),
            "last_error should name the failed document, got: {last_error:?}"
        );
        // Progress still reaches 100, but the failure count reveals it was partial.
        assert_eq!(state.progress.load(Ordering::Relaxed), 100);
    }

    #[tokio::test]
    async fn reindex_resets_running_flag_even_when_it_panics() {
        // A reindex that panics mid-run must not leave `is_running` stuck true,
        // which would permanently block every future reindex.
        struct PanickingSearch;
        #[async_trait]
        impl SearchService for PanickingSearch {
            async fn index_document(&self, _: &SearchDocument) -> Result<(), AppError> {
                panic!("simulated indexing panic");
            }
            async fn delete_document(&self, _: &str) -> Result<(), AppError> {
                Ok(())
            }
            async fn search(
                &self,
                _: &str,
                _: Option<&[String]>,
                _: bool,
            ) -> Result<Vec<SearchHit>, AppError> {
                Ok(vec![])
            }
            async fn configure_index(&self) -> Result<(), AppError> {
                Ok(())
            }
            async fn health_check(&self) -> Result<(), AppError> {
                Ok(())
            }
        }

        let doc = make_doc("a", false, false);
        let repo = Arc::new(MockDocumentRepo {
            documents: vec![doc.clone()],
        });
        let storage = Arc::new(MockStorage::default());
        storage
            .put_object(&doc.s3_key, b"# A".to_vec())
            .await
            .unwrap();
        let state = Arc::new(SearchReindexState {
            is_running: AtomicBool::new(true),
            progress: AtomicU32::new(0),
            ..Default::default()
        });

        let state_in_task = state.clone();
        let handle = tokio::spawn(async move {
            run_reindex(state_in_task, repo, storage, Arc::new(PanickingSearch)).await;
        });
        let _ = handle.await; // JoinError from the panic; ignore it.

        assert!(
            !state.is_running.load(Ordering::Acquire),
            "a panicked reindex must reset is_running"
        );
    }
}
