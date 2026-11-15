use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use std::collections::HashSet;

use crate::db::asset_repository::AssetRepository;
use crate::db::repository::DocumentRepository;
use crate::indexing::{
    currently_known_attachment_keys, currently_live_slugs, index_everywhere, remove_everywhere,
    IndexTargets,
};
use crate::rag::service::RagService;
use crate::search::attachment_search::AttachmentSearchService;
use crate::search::client::SearchService;
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

/// Run a full reconciliation of the Meilisearch indexes.
///
/// Active, visible documents are indexed from the canonical MongoDB metadata and
/// S3 markdown content. Hidden or archived documents are deleted from the search
/// index so stale results do not remain after metadata-only changes. Entries
/// MongoDB no longer knows at all — a renamed or deleted slug, the pages of an
/// attachment nothing references — are swept from the `documents` and
/// `attachment_chunks` indexes.
///
/// A document flagged `needs_reindex` is brought into RAG as well (when
/// enabled) and its flag cleared once every enabled index took it; see
/// [`crate::indexing`]. Unflagged documents only touch search.
#[allow(clippy::too_many_arguments)]
pub async fn run_reindex(
    reindex: Arc<SearchReindexState>,
    document_repo: Arc<dyn DocumentRepository>,
    storage: Arc<dyn StorageClient>,
    search: Arc<dyn SearchService>,
    asset_repo: Arc<dyn AssetRepository>,
    attachment_search: Option<Arc<dyn AttachmentSearchService>>,
    rag: Option<Arc<dyn RagService>>,
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

    let mut documents_by_slug = std::collections::BTreeMap::<String, Vec<_>>::new();
    for document in documents {
        documents_by_slug
            .entry(document.slug.clone())
            .or_default()
            .push(document);
    }

    let total = documents_by_slug.len();
    if total == 0 {
        tracing::info!("Search reindex: no documents to index");
        reindex.progress.store(100, Ordering::Relaxed);
        return;
    }

    tracing::info!(total, "Search reindex: starting");

    // The slugs that belong in the index; anything else found there is swept.
    let mut live_slugs = HashSet::new();

    for (i, (slug, candidates)) in documents_by_slug.into_iter().enumerate() {
        let Some(doc) = crate::db::repository::resolve_by_release(
            candidates,
            &crate::versioning::ReleasePins::default(),
        ) else {
            if let Err(e) = search.delete_document(&slug).await {
                tracing::warn!(
                    slug,
                    "Search reindex: failed to delete document without latest release: {e}"
                );
                reindex
                    .outcome
                    .record_failure(format!("delete {slug}: {e}"));
            }
            update_progress(&reindex, i, total);
            continue;
        };

        // A flagged document is reconciled in every enabled index, so the
        // flag can be cleared; an unflagged one only needs search.
        let targets = IndexTargets {
            search: Some(search.as_ref()),
            rag: rag.as_deref().filter(|_| doc.needs_reindex),
        };

        if doc.is_archived || doc.is_hidden {
            let failures = remove_everywhere(targets, &doc.slug).await;
            record_document_outcome(&reindex, document_repo.as_ref(), &doc, "delete", failures)
                .await;
            update_progress(&reindex, i, total);
            continue;
        }

        live_slugs.insert(doc.slug.clone());

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

        let failures = index_everywhere(targets, &doc, &content).await;
        record_document_outcome(&reindex, document_repo.as_ref(), &doc, "index", failures).await;

        update_progress(&reindex, i, total);
    }

    sweep_orphans(
        &reindex,
        search.as_ref(),
        document_repo.as_ref(),
        asset_repo.as_ref(),
        attachment_search.as_deref(),
        &live_slugs,
    )
    .await;

    tracing::info!(total, "Search reindex: complete");
    reindex.progress.store(100, Ordering::Relaxed);
}

/// Delete index entries MongoDB does not account for: documents whose slug is
/// not live, and attachment pages whose asset is gone.
///
/// Only called after a complete listing of the documents. Candidates found
/// against that (possibly old) snapshot are rechecked against MongoDB right
/// before deleting, so nothing written during the run is lost; when that
/// recheck fails, nothing is deleted.
///
/// An empty live set skips the sweep entirely, deliberately: a misconfigured
/// database name looks exactly like that, and wiping both indexes on its word
/// is far worse than leaving orphans behind on a genuinely empty instance.
/// Attachments are left alone when the assets cannot be listed.
async fn sweep_orphans(
    reindex: &SearchReindexState,
    search: &dyn SearchService,
    document_repo: &dyn DocumentRepository,
    asset_repo: &dyn AssetRepository,
    attachment_search: Option<&dyn AttachmentSearchService>,
    live_slugs: &HashSet<String>,
) {
    if live_slugs.is_empty() {
        tracing::warn!("Search reindex: no live documents, skipping the orphan sweep");
        return;
    }

    let orphans = match search.list_indexed_slugs().await {
        Ok(indexed) => {
            let candidates: Vec<String> = indexed
                .into_iter()
                .filter(|slug| !live_slugs.contains(slug))
                .collect();
            match currently_live_slugs(document_repo, &candidates).await {
                Ok(live_now) => candidates
                    .into_iter()
                    .filter(|slug| !live_now.contains(slug))
                    .collect(),
                Err(e) => {
                    tracing::warn!("Search reindex: cannot recheck orphan candidates: {e}");
                    reindex
                        .outcome
                        .record_failure(format!("recheck orphans: {e}"));
                    Vec::new()
                }
            }
        }
        Err(e) => {
            tracing::warn!("Search reindex: cannot list indexed documents: {e}");
            reindex.outcome.record_failure(format!("list index: {e}"));
            Vec::new()
        }
    };
    {
        {
            for slug in &orphans {
                tracing::info!(slug, "Search reindex: removing orphaned document");
                if let Err(e) = search.delete_document(slug).await {
                    tracing::warn!(slug, "Search reindex: failed to remove orphan: {e}");
                    reindex
                        .outcome
                        .record_failure(format!("delete orphan {slug}: {e}"));
                }
            }
        }
    }

    let Some(attachment_search) = attachment_search else {
        return;
    };
    // Every asset record counts, referenced yet or not; see
    // `currently_known_attachment_keys`.
    let live_keys: HashSet<String> = match asset_repo.list_all().await {
        Ok(assets) => assets.into_iter().map(|asset| asset.key).collect(),
        Err(e) => {
            tracing::warn!("Search reindex: cannot list assets, skipping attachment sweep: {e}");
            reindex.outcome.record_failure(format!("list assets: {e}"));
            return;
        }
    };
    match attachment_search.list_indexed_attachment_keys().await {
        Ok(indexed) => {
            let candidates: Vec<String> = indexed
                .into_iter()
                .filter(|key| !live_keys.contains(key))
                .collect();
            let known_now = match currently_known_attachment_keys(asset_repo, &candidates).await {
                Ok(known) => known,
                Err(e) => {
                    tracing::warn!("Search reindex: cannot recheck orphan attachments: {e}");
                    reindex
                        .outcome
                        .record_failure(format!("recheck orphan attachments: {e}"));
                    return;
                }
            };
            for key in candidates.iter().filter(|key| !known_now.contains(*key)) {
                tracing::info!(key, "Search reindex: removing orphaned attachment pages");
                if let Err(e) = attachment_search.delete_attachment(key).await {
                    tracing::warn!(key, "Search reindex: failed to remove attachment: {e}");
                    reindex
                        .outcome
                        .record_failure(format!("delete orphan attachment {key}: {e}"));
                }
            }
        }
        Err(e) => {
            tracing::warn!("Search reindex: cannot list indexed attachments: {e}");
            reindex
                .outcome
                .record_failure(format!("list attachment index: {e}"));
        }
    }
}

/// Record a document's reconciliation outcome, clearing its `needs_reindex`
/// flag when it was reconciled in every enabled index without a failure.
async fn record_document_outcome(
    reindex: &SearchReindexState,
    document_repo: &dyn DocumentRepository,
    doc: &crate::db::models::Document,
    action: &str,
    failures: Vec<String>,
) {
    if !failures.is_empty() {
        let failures = failures.join("; ");
        tracing::warn!(slug = %doc.slug, "Search reindex: failed to {action} document: {failures}");
        reindex
            .outcome
            .record_failure(format!("{action} {}: {failures}", doc.slug));
        return;
    }
    if doc.needs_reindex {
        if let Err(e) = document_repo
            .clear_needs_reindex_if_unchanged(&doc.slug, doc.release.as_deref(), doc.last_updated)
            .await
        {
            tracing::warn!(slug = %doc.slug, "Search reindex: cannot clear needs_reindex: {e}");
        }
    }
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

    #[derive(Default)]
    struct MockDocumentRepo {
        documents: Vec<Document>,
        /// Slugs whose `needs_reindex` was cleared.
        cleared: Mutex<Vec<String>>,
        /// Written after `list_all` took its snapshot: lookups by slug see
        /// them, the listing does not.
        late: Vec<Document>,
    }

    #[async_trait]
    impl DocumentRepository for MockDocumentRepo {
        async fn create_or_update(&self, _: Document) -> Result<(), AppError> {
            Ok(())
        }

        async fn find_by_slug(&self, _: &str) -> Result<Option<Document>, AppError> {
            Ok(None)
        }
        async fn find_by_slugs(&self, slugs: &[String]) -> Result<Vec<Document>, AppError> {
            Ok(self
                .documents
                .iter()
                .chain(&self.late)
                .filter(|d| slugs.contains(&d.slug))
                .cloned()
                .collect())
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
        /// What `list_indexed_slugs` reports as already in the index.
        listed: Vec<String>,
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
        async fn list_indexed_slugs(&self) -> Result<Vec<String>, AppError> {
            Ok(self.listed.clone())
        }
    }

    fn no_assets() -> Arc<dyn AssetRepository> {
        Arc::new(crate::test_utils::InMemoryAssetRepo::default())
    }

    /// Lists a fixed set of attachment keys and records which get deleted.
    #[derive(Default)]
    struct RecordingAttachmentSearch {
        listed: Vec<String>,
        deleted: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl AttachmentSearchService for RecordingAttachmentSearch {
        async fn index_pages(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: &[crate::rag::service::AttachmentPage],
            _: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_attachment(&self, key: &str) -> Result<(), AppError> {
            self.deleted.lock().unwrap().push(key.to_string());
            Ok(())
        }
        async fn update_reference(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &[String],
        ) -> Result<(), AppError> {
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
        async fn list_indexed_attachment_keys(&self) -> Result<Vec<String>, AppError> {
            Ok(self.listed.clone())
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
            ..Default::default()
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

        run_reindex(
            state.clone(),
            repo,
            storage,
            search.clone(),
            no_assets(),
            None,
            None,
        )
        .await;

        assert!(!state.is_running.load(Ordering::Acquire));
        assert_eq!(state.progress.load(Ordering::Relaxed), 100);
        assert!(search.configured.load(Ordering::Relaxed));

        let indexed = search.indexed.lock().unwrap();
        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0].slug, active.slug);
        assert!(indexed[0].content_preview.contains("Active"));

        let deleted = search.deleted.lock().unwrap();
        assert_eq!(&*deleted, &vec![archived.slug.clone(), hidden.slug.clone()]);
    }

    #[tokio::test]
    async fn reindex_reconciles_one_latest_copy_per_slug() {
        let mut latest = make_doc("docs/guide", false, false);
        latest.release = Some("2.0.0".to_string());
        latest.s3_key = "docs/2.0.0/guide.md".to_string();

        let mut archived_old = make_doc("docs/guide", false, true);
        archived_old.release = Some("1.0.0".to_string());
        archived_old.is_latest = false;
        archived_old.s3_key = "docs/1.0.0/guide.md".to_string();

        let repo = Arc::new(MockDocumentRepo {
            documents: vec![archived_old, latest.clone()],
            ..Default::default()
        });
        let storage = Arc::new(MockStorage::default());
        storage
            .put_object(&latest.s3_key, b"# Latest release".to_vec())
            .await
            .unwrap();
        let search = Arc::new(RecordingSearch::default());
        let state = Arc::new(SearchReindexState {
            is_running: AtomicBool::new(true),
            ..Default::default()
        });

        run_reindex(
            state,
            repo,
            storage,
            search.clone(),
            no_assets(),
            None,
            None,
        )
        .await;

        let indexed = search.indexed.lock().unwrap();
        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0].slug, "docs/guide");
        assert!(indexed[0].content_preview.contains("Latest release"));
        assert!(search.deleted.lock().unwrap().is_empty());
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
            ..Default::default()
        });
        let storage = Arc::new(MockStorage::default());
        storage
            .put_object(&doc.s3_key, b"# A".to_vec())
            .await
            .unwrap();
        let state = Arc::new(SearchReindexState::default());
        state.is_running.store(true, Ordering::Release);

        run_reindex(
            state.clone(),
            repo,
            storage,
            Arc::new(FailingSearch),
            no_assets(),
            None,
            None,
        )
        .await;

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
            ..Default::default()
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
            run_reindex(
                state_in_task,
                repo,
                storage,
                Arc::new(PanickingSearch),
                no_assets(),
                None,
                None,
            )
            .await;
        });
        let _ = handle.await; // JoinError from the panic; ignore it.

        assert!(
            !state.is_running.load(Ordering::Acquire),
            "a panicked reindex must reset is_running"
        );
    }

    #[tokio::test]
    async fn reindex_sweeps_entries_mongo_no_longer_knows() {
        let active = make_doc("docs/active", false, false);
        let hidden = make_doc("docs/hidden", true, false);
        let repo = Arc::new(MockDocumentRepo {
            documents: vec![active.clone(), hidden.clone()],
            ..Default::default()
        });
        let storage = Arc::new(MockStorage::default());
        storage
            .put_object(&active.s3_key, b"# Active".to_vec())
            .await
            .unwrap();
        let search = Arc::new(RecordingSearch {
            // `docs/renamed-away` is what a slug rename leaves behind.
            listed: vec![
                "docs/active".to_string(),
                "docs/hidden".to_string(),
                "docs/renamed-away".to_string(),
            ],
            ..Default::default()
        });
        let assets = Arc::new(crate::test_utils::InMemoryAssetRepo::with_assets(vec![
            crate::test_utils::make_asset("live.pdf", &["docs/active"]),
            crate::test_utils::make_asset("unreferenced.pdf", &[]),
        ]));
        let attachments = Arc::new(RecordingAttachmentSearch {
            listed: vec![
                "live.pdf".to_string(),
                "unreferenced.pdf".to_string(),
                "deleted.pdf".to_string(),
            ],
            ..Default::default()
        });
        let state = Arc::new(SearchReindexState::default());
        state.is_running.store(true, Ordering::Release);

        run_reindex(
            state,
            repo,
            storage,
            search.clone(),
            assets,
            Some(attachments.clone()),
            None,
        )
        .await;

        let deleted = search.deleted.lock().unwrap().clone();
        assert!(deleted.contains(&"docs/renamed-away".to_string()));
        assert!(!deleted.contains(&"docs/active".to_string()));
        // An unreferenced asset record keeps its pages: it may be an upload
        // whose document is still to be ingested.
        assert_eq!(
            *attachments.deleted.lock().unwrap(),
            vec!["deleted.pdf".to_string()]
        );
    }

    #[tokio::test]
    async fn reindex_never_sweeps_without_a_complete_listing() {
        struct FailingListRepo;
        #[async_trait]
        impl DocumentRepository for FailingListRepo {
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
                Err(AppError::Internal("mongo down".into()))
            }
            async fn list_by_access_levels(
                &self,
                _: Option<&[String]>,
                _: bool,
                _: &crate::versioning::ReleasePins,
            ) -> Result<Vec<Document>, AppError> {
                Err(AppError::Internal("mongo down".into()))
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
            async fn set_archived(
                &self,
                _: &str,
                _: Option<&str>,
                _: bool,
            ) -> Result<(), AppError> {
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

        let search = Arc::new(RecordingSearch {
            listed: vec!["docs/a".to_string()],
            ..Default::default()
        });
        let attachments = Arc::new(RecordingAttachmentSearch {
            listed: vec!["a.pdf".to_string()],
            ..Default::default()
        });
        let state = Arc::new(SearchReindexState::default());
        state.is_running.store(true, Ordering::Release);
        run_reindex(
            state,
            Arc::new(FailingListRepo),
            Arc::new(MockStorage::default()),
            search.clone(),
            no_assets(),
            Some(attachments.clone()),
            None,
        )
        .await;
        assert!(search.deleted.lock().unwrap().is_empty());
        assert!(attachments.deleted.lock().unwrap().is_empty());

        // Documents listed, assets not: documents are swept, attachments kept.
        let doc = make_doc("docs/live", false, false);
        let storage = Arc::new(MockStorage::default());
        storage
            .put_object(&doc.s3_key, b"# Live".to_vec())
            .await
            .unwrap();
        let assets = crate::test_utils::InMemoryAssetRepo::default();
        assets.fail_list.store(true, Ordering::SeqCst);
        let state = Arc::new(SearchReindexState::default());
        state.is_running.store(true, Ordering::Release);
        run_reindex(
            state,
            Arc::new(MockDocumentRepo {
                documents: vec![doc],
                ..Default::default()
            }),
            storage,
            search.clone(),
            Arc::new(assets),
            Some(attachments.clone()),
            None,
        )
        .await;
        assert_eq!(*search.deleted.lock().unwrap(), vec!["docs/a".to_string()]);
        assert!(attachments.deleted.lock().unwrap().is_empty());
    }

    /// Runs a search reindex over `documents` (bodies stored for each) with
    /// the given RAG service, returning the search mock and the repo.
    async fn reindex_with_rag(
        documents: Vec<Document>,
        rag: Option<Arc<dyn crate::rag::service::RagService>>,
    ) -> (Arc<RecordingSearch>, Arc<MockDocumentRepo>) {
        let storage = Arc::new(MockStorage::default());
        for doc in &documents {
            storage
                .put_object(&doc.s3_key, b"# Body".to_vec())
                .await
                .unwrap();
        }
        let repo = Arc::new(MockDocumentRepo {
            documents,
            ..Default::default()
        });
        let search = Arc::new(RecordingSearch::default());
        let state = Arc::new(SearchReindexState::default());
        state.is_running.store(true, Ordering::Release);
        run_reindex(
            state,
            repo.clone(),
            storage,
            search.clone(),
            no_assets(),
            None,
            rag,
        )
        .await;
        (search, repo)
    }

    #[tokio::test]
    async fn reindex_clears_needs_reindex_only_after_every_enabled_index_took_it() {
        let mut flagged = make_doc("docs/flagged", false, false);
        flagged.needs_reindex = true;
        let clean = make_doc("docs/clean", false, false);
        let mut flagged_hidden = make_doc("docs/flagged-hidden", true, false);
        flagged_hidden.needs_reindex = true;

        // RAG enabled: a flagged document goes into RAG too, then is cleared;
        // an unflagged one is left to the RAG reindex.
        let rag = Arc::new(crate::test_utils::RecordingRag::default());
        let (search, repo) = reindex_with_rag(
            vec![flagged.clone(), clean.clone(), flagged_hidden.clone()],
            Some(rag.clone()),
        )
        .await;
        assert_eq!(rag.indexed(), vec!["docs/flagged"]);
        assert_eq!(rag.deleted(), vec!["docs/flagged-hidden"]);
        let mut indexed: Vec<_> = search
            .indexed
            .lock()
            .unwrap()
            .iter()
            .map(|d| d.slug.clone())
            .collect();
        indexed.sort();
        assert_eq!(indexed, vec!["docs/clean", "docs/flagged"]);
        let mut cleared = repo.cleared.lock().unwrap().clone();
        cleared.sort();
        assert_eq!(cleared, vec!["docs/flagged", "docs/flagged-hidden"]);

        // RAG failing: the flag stays.
        let rag = Arc::new(crate::test_utils::RecordingRag::default());
        rag.fail_index.store(true, Ordering::SeqCst);
        let (_, repo) = reindex_with_rag(vec![flagged.clone()], Some(rag)).await;
        assert!(repo.cleared.lock().unwrap().is_empty());

        // RAG disabled: search alone is every enabled index.
        let (_, repo) = reindex_with_rag(vec![flagged], None).await;
        assert_eq!(*repo.cleared.lock().unwrap(), vec!["docs/flagged"]);
    }

    #[tokio::test]
    async fn reindex_never_sweeps_what_mongo_learned_during_the_run() {
        let live = make_doc("docs/live", false, false);
        // Ingested while the reindex ran: after its snapshot, before the sweep.
        let late = make_doc("docs/ingested-meanwhile", false, false);
        let late_hidden = make_doc("docs/hidden-meanwhile", true, false);
        let repo = Arc::new(MockDocumentRepo {
            documents: vec![live.clone()],
            late: vec![late.clone(), late_hidden.clone()],
            ..Default::default()
        });
        let storage = Arc::new(MockStorage::default());
        storage
            .put_object(&live.s3_key, b"# Live".to_vec())
            .await
            .unwrap();
        let search = Arc::new(RecordingSearch {
            listed: vec![
                live.slug.clone(),
                late.slug.clone(),
                late_hidden.slug.clone(),
                "docs/gone".to_string(),
            ],
            ..Default::default()
        });
        let assets = crate::test_utils::InMemoryAssetRepo::default();
        assets
            .late_assets
            .lock()
            .unwrap()
            .push(crate::test_utils::make_asset("uploaded-meanwhile.pdf", &[]));
        let attachments = Arc::new(RecordingAttachmentSearch {
            listed: vec!["uploaded-meanwhile.pdf".to_string(), "gone.pdf".to_string()],
            ..Default::default()
        });
        let state = Arc::new(SearchReindexState::default());
        state.is_running.store(true, Ordering::Release);

        run_reindex(
            state,
            repo,
            storage,
            search.clone(),
            Arc::new(assets),
            Some(attachments.clone()),
            None,
        )
        .await;

        let mut deleted = search.deleted.lock().unwrap().clone();
        deleted.sort();
        assert_eq!(deleted, vec!["docs/gone", "docs/hidden-meanwhile"]);
        assert_eq!(
            *attachments.deleted.lock().unwrap(),
            vec!["gone.pdf".to_string()]
        );
    }
}
