//! Backfill of the full page text (`content`) of attachment pages indexed
//! before full-text attachment search, which stored only a preview.
//!
//! No canonical page transcript is kept after indexing (Qdrant holds split
//! chunks), so the page text has to be extracted again. Only the free native
//! path runs: plain-text files are decoded and PDFs read through their text
//! layer, never rendered for VLM
//! transcription. A page whose native text does not reproduce the stored
//! preview — typically a scanned page the worker had transcribed — cannot be
//! recovered that way: it is marked settled with an empty `content`, keeps
//! being found by its preview, and gains its full text the next time the
//! attachment is extracted (a new upload or a RAG re-index).

use crate::db::asset_repository::AssetRepository;
use crate::db::models::{Asset, ExtractionStatus};
use crate::error::AppError;
use crate::rag::attachment_extraction::lock_attachment_acl;
use crate::rag::extraction::{AttachmentExtractors, ExtractionOutcome};
use crate::search::attachment_search::{page_preview, AttachmentSearchService};
use crate::storage::client::StorageClient;

/// What a backfill of one attachment did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PageTextBackfill {
    /// Pages that gained their full text.
    pub filled: usize,
    /// Pages whose text native extraction could not reproduce, marked settled
    /// with an empty `content` so later runs do not extract them again.
    pub unrecoverable: usize,
}

impl PageTextBackfill {
    pub fn add(&mut self, other: Self) {
        self.filled += other.filled;
        self.unrecoverable += other.unrecoverable;
    }
}

/// Whether the asset's indexed pages reflect its current content: extracted
/// successfully, from the bytes it holds now.
fn settled(asset: &Asset) -> bool {
    asset.extraction_status == ExtractionStatus::Done
        && asset.content_hash.is_some()
        && asset.content_hash == asset.extracted_content_hash
}

/// Fill the full text of every indexed page of `key` that has none, from the
/// attachment's native text (see the module docs). A no-op for an attachment
/// whose pages all have their text, or whose extraction is not settled — an
/// attachment being re-extracted gets its full text from the worker.
///
/// The bytes are read and extracted without holding the attachment's lock;
/// the pages are then re-read and written under it, after checking the asset
/// still holds the content that was extracted, so the write never lands
/// between the worker's replacement of the pages or an ACL update.
pub async fn backfill_page_text(
    key: &str,
    asset_repo: &dyn AssetRepository,
    storage: &dyn StorageClient,
    attachment_search: &dyn AttachmentSearchService,
) -> Result<PageTextBackfill, AppError> {
    let Some(asset) = asset_repo.find_by_key(key).await? else {
        return Ok(PageTextBackfill::default());
    };
    if !settled(&asset) || attachment_search.pages_without_text(key).await?.is_empty() {
        return Ok(PageTextBackfill::default());
    }

    let Some(bytes) = storage.get_object(&asset.s3_key).await? else {
        return Err(AppError::Internal(
            "attachment content not found in storage".into(),
        ));
    };
    // No VLM: native text only, which costs nothing.
    let pages = match AttachmentExtractors::new(0, None)
        .extract(&bytes, &asset.content_type)
        .await?
    {
        ExtractionOutcome::Extracted(pages) => pages,
        ExtractionOutcome::Unsupported => return Ok(PageTextBackfill::default()),
    };

    let _acl_guard = lock_attachment_acl(key).await;
    match asset_repo.find_by_key(key).await? {
        Some(current) if settled(&current) && current.content_hash == asset.content_hash => {}
        // Changed or being re-extracted meanwhile: the worker writes the text.
        _ => return Ok(PageTextBackfill::default()),
    }
    let mut outcome = PageTextBackfill::default();
    let mut updates = attachment_search.pages_without_text(key).await?;
    for indexed in &mut updates {
        let text = pages
            .iter()
            .find(|page| page.page_number == indexed.page)
            .map(|page| page.text.trim())
            .filter(|text| page_preview(text) == indexed.content_preview);
        match text {
            Some(text) => {
                indexed.content = Some(text.to_string());
                outcome.filled += 1;
            }
            None => {
                indexed.content = Some(String::new());
                outcome.unrecoverable += 1;
            }
        }
    }
    if !updates.is_empty() {
        attachment_search.update_pages(&updates).await?;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use crate::rag::service::AttachmentPage;
    use crate::search::attachment_search::{page_id, AttachmentChunkDocument, AttachmentSearchHit};
    use crate::test_utils::{InMemoryAssetRepo, MockStorage};

    /// Holds attachment pages in memory, like the index would.
    #[derive(Default)]
    struct InMemoryPages {
        pages: Mutex<Vec<AttachmentChunkDocument>>,
        /// Number of `update_pages` calls.
        updates: Mutex<usize>,
    }

    impl InMemoryPages {
        /// A page indexed before full-text search: preview, no `content`.
        fn legacy(&self, key: &str, page: Option<u32>, text: &str) {
            self.pages.lock().unwrap().push(AttachmentChunkDocument {
                id: page_id(key, page),
                attachment_key: key.to_string(),
                filename: key.to_string(),
                document_slug: "docs/guide".to_string(),
                document_title: "Guide".to_string(),
                page,
                content_preview: page_preview(text),
                content: None,
                access_levels: vec!["internal".to_string()],
            });
        }

        fn content(&self, key: &str, page: Option<u32>) -> Option<String> {
            self.pages
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.id == page_id(key, page))
                .and_then(|p| p.content.clone())
        }
    }

    #[async_trait]
    impl AttachmentSearchService for InMemoryPages {
        async fn index_pages(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: &[AttachmentPage],
            _: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_attachment(&self, _: &str) -> Result<(), AppError> {
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
        ) -> Result<Vec<AttachmentSearchHit>, AppError> {
            Ok(vec![])
        }
        async fn configure_index(&self) -> Result<(), AppError> {
            Ok(())
        }
        async fn pages_without_text(
            &self,
            attachment_key: &str,
        ) -> Result<Vec<AttachmentChunkDocument>, AppError> {
            Ok(self
                .pages
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.attachment_key == attachment_key && p.content.is_none())
                .cloned()
                .collect())
        }
        async fn update_pages(&self, pages: &[AttachmentChunkDocument]) -> Result<(), AppError> {
            *self.updates.lock().unwrap() += 1;
            let mut stored = self.pages.lock().unwrap();
            for page in pages {
                stored.retain(|p| p.id != page.id);
                stored.push(page.clone());
            }
            Ok(())
        }
    }

    fn asset(key: &str, content_type: &str) -> Asset {
        Asset {
            key: key.to_string(),
            content_type: content_type.to_string(),
            size_bytes: 0,
            s3_key: format!("assets/{key}"),
            uploaded_at: Utc::now(),
            uploaded_by: "test".to_string(),
            referenced_by: vec!["docs/guide".into()],
            content_hash: Some("sha256:v1".to_string()),
            extraction_status: ExtractionStatus::Done,
            extraction_error: None,
            extracted_content_hash: Some("sha256:v1".to_string()),
            extracted_at: Some(Utc::now()),
            indexed_chunks: Some(1),
        }
    }

    /// A text attachment `key` holding `text`, indexed with a preview only.
    async fn legacy_text_attachment(
        key: &str,
        text: &str,
    ) -> (Arc<InMemoryAssetRepo>, Arc<MockStorage>, Arc<InMemoryPages>) {
        let assets = Arc::new(InMemoryAssetRepo::with_assets(vec![asset(
            key,
            "text/plain; charset=utf-8",
        )]));
        let storage = Arc::new(MockStorage::new());
        storage
            .put_object(&format!("assets/{key}"), text.as_bytes().to_vec())
            .await
            .unwrap();
        let pages = Arc::new(InMemoryPages::default());
        pages.legacy(key, None, text);
        (assets, storage, pages)
    }

    fn long_text() -> String {
        format!(
            "{}\nThe deep identifier E4711 appears here.\n",
            "Opening filler about the device. ".repeat(20)
        )
    }

    #[tokio::test]
    async fn fills_the_full_text_of_preview_only_pages() {
        let key = "attachments/docs/guide/notes.txt";
        let text = long_text();
        let (assets, storage, pages) = legacy_text_attachment(key, &text).await;

        let outcome = backfill_page_text(key, assets.as_ref(), storage.as_ref(), pages.as_ref())
            .await
            .unwrap();

        assert_eq!(
            outcome,
            PageTextBackfill {
                filled: 1,
                unrecoverable: 0
            }
        );
        let content = pages.content(key, None).unwrap();
        assert_eq!(content, text.trim());
        assert!(content.contains("E4711"));
        // The rest of the entry is unchanged.
        let stored = pages.pages.lock().unwrap()[0].clone();
        assert_eq!(stored.access_levels, ["internal"]);
        assert_eq!(stored.document_slug, "docs/guide");
    }

    #[tokio::test]
    async fn a_second_run_has_nothing_left_to_do() {
        let key = "attachments/docs/guide/notes.txt";
        let (assets, storage, pages) = legacy_text_attachment(key, &long_text()).await;
        backfill_page_text(key, assets.as_ref(), storage.as_ref(), pages.as_ref())
            .await
            .unwrap();
        // Gone from storage: a second run must not even read it.
        storage.objects.lock().unwrap().clear();

        let outcome = backfill_page_text(key, assets.as_ref(), storage.as_ref(), pages.as_ref())
            .await
            .unwrap();

        assert_eq!(outcome, PageTextBackfill::default());
        assert_eq!(*pages.updates.lock().unwrap(), 1);
    }

    /// A page whose stored preview native extraction does not reproduce (here
    /// a transcribed page, simulated by a different preview) is settled with
    /// no text rather than given text that was never indexed.
    #[tokio::test]
    async fn a_page_native_text_does_not_reproduce_is_settled_without_text() {
        let key = "attachments/docs/guide/scan.txt";
        let (assets, storage, pages) = legacy_text_attachment(key, &long_text()).await;
        pages.pages.lock().unwrap()[0].content_preview = "Transcribed by a VLM".to_string();

        let outcome = backfill_page_text(key, assets.as_ref(), storage.as_ref(), pages.as_ref())
            .await
            .unwrap();

        assert_eq!(
            outcome,
            PageTextBackfill {
                filled: 0,
                unrecoverable: 1
            }
        );
        assert_eq!(pages.content(key, None).as_deref(), Some(""));
        assert!(pages.pages_without_text(key).await.unwrap().is_empty());
    }

    /// Pages of an attachment whose extraction is not settled are left to the
    /// worker, which writes their full text itself.
    #[tokio::test]
    async fn unsettled_extractions_are_left_to_the_worker() {
        let key = "attachments/docs/guide/notes.txt";
        for change in [
            |a: &mut Asset| a.extraction_status = ExtractionStatus::InProgress,
            |a: &mut Asset| a.content_hash = Some("sha256:v2".to_string()),
        ] {
            let (assets, storage, pages) = legacy_text_attachment(key, &long_text()).await;
            change(&mut assets.assets.lock().unwrap()[0]);

            let outcome =
                backfill_page_text(key, assets.as_ref(), storage.as_ref(), pages.as_ref())
                    .await
                    .unwrap();

            assert_eq!(outcome, PageTextBackfill::default());
            assert_eq!(pages.content(key, None), None);
        }
    }

    /// The backfill writes under the attachment's lock, like the worker's
    /// page replacement, so neither interleaves with the other.
    #[tokio::test]
    async fn the_write_waits_for_the_attachment_lock() {
        let key = "attachments/docs/guide/locked-backfill.txt";
        let (assets, storage, pages) = legacy_text_attachment(key, &long_text()).await;

        let held = lock_attachment_acl(key).await;
        let backfill = {
            let (assets, storage, pages) = (assets.clone(), storage.clone(), pages.clone());
            tokio::spawn(async move {
                backfill_page_text(key, assets.as_ref(), storage.as_ref(), pages.as_ref()).await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(*pages.updates.lock().unwrap(), 0);
        drop(held);

        assert_eq!(backfill.await.unwrap().unwrap().filled, 1);
    }

    #[tokio::test]
    async fn a_missing_object_is_reported() {
        let key = "attachments/docs/guide/notes.txt";
        let (assets, storage, pages) = legacy_text_attachment(key, &long_text()).await;
        storage.objects.lock().unwrap().clear();

        assert!(
            backfill_page_text(key, assets.as_ref(), storage.as_ref(), pages.as_ref())
                .await
                .is_err()
        );
        assert_eq!(pages.content(key, None), None);
    }
}
