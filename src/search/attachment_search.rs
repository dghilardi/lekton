//! Keyword search over PDF attachment content, indexed separately from
//! documents (see [`crate::search::client`]) so an exact term that only
//! appears inside a PDF — a part number, an error code, an acronym — can
//! still be found via keyword search, complementing RAG's semantic search.
//!
//! Indexed at page granularity: one Meilisearch document per extracted page
//! (or one document for non-paginated plain-text attachments), carrying
//! enough of the owning document's identity to link a hit back to
//! `/docs/{slug}` and show which page to look at.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::error::AppError;
#[cfg(feature = "ssr")]
use crate::rag::service::AttachmentPage;
#[cfg(feature = "ssr")]
use async_trait::async_trait;

/// One indexed page of an attachment.
#[cfg(feature = "ssr")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentChunkDocument {
    /// Primary key: `slug_to_id(attachment_key)` + the page number (or `"0"`
    /// for non-paginated content), so re-indexing the same page overwrites it.
    pub id: String,
    /// The asset key, e.g. `"project-a/datasheet.pdf"`. Filterable, so a
    /// re-index can delete every page for this attachment before re-adding.
    pub attachment_key: String,
    /// Display filename (last path segment of `attachment_key`).
    pub filename: String,
    /// Slug of the (first) document that references this attachment, used to
    /// link a hit back to `/docs/{slug}`.
    pub document_slug: String,
    /// Title of that document, shown alongside the filename in results.
    pub document_title: String,
    /// 1-based page number, or `None` for non-paginated (plain text) content.
    pub page: Option<u32>,
    /// First ~200 characters of the page's extracted text.
    pub content_preview: String,
    /// Access levels inherited from the referencing document(s); mirrors the
    /// RAG attachment ACL derivation (`referencing_acl`) — no draft gating,
    /// since attachment content is treated as published-or-invisible there
    /// too. An empty list makes the page unreachable by search.
    pub access_levels: Vec<String>,
}

/// A search result returned to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentSearchHit {
    pub document_slug: String,
    pub document_title: String,
    pub attachment_key: String,
    pub filename: String,
    pub page: Option<u32>,
    pub content_preview: String,
}

/// Trait for attachment keyword search, enabling mock testing.
#[cfg(feature = "ssr")]
#[async_trait]
pub trait AttachmentSearchService: Send + Sync {
    /// Replace every indexed page for `attachment_key` with `pages`. Pages
    /// are deleted first, then the fresh set is added — a brief window where
    /// the attachment has no search results, traded for not having to track
    /// which page ids became stale (attachments are re-indexed as a whole on
    /// every content change, unlike RAG's finer-grained upsert-then-delete).
    async fn index_pages(
        &self,
        attachment_key: &str,
        filename: &str,
        document_slug: &str,
        document_title: &str,
        pages: &[AttachmentPage],
        access_levels: &[String],
    ) -> Result<(), AppError>;

    /// Remove every indexed page for an attachment.
    async fn delete_attachment(&self, attachment_key: &str) -> Result<(), AppError>;

    /// Update only the `access_levels` of every indexed page for an
    /// attachment, without re-indexing its text — for when the referencing
    /// document's access level changes but the attachment content did not.
    /// A no-op if the attachment has no indexed pages.
    async fn update_access_levels(
        &self,
        attachment_key: &str,
        access_levels: &[String],
    ) -> Result<(), AppError>;

    /// Search attachment pages visible to the caller.
    ///
    /// `allowed_levels`: the access level names the caller can read. `None`
    /// means admin (no level restriction); `Some(&[])` means nothing is
    /// visible.
    async fn search(
        &self,
        query: &str,
        allowed_levels: Option<&[String]>,
    ) -> Result<Vec<AttachmentSearchHit>, AppError>;

    /// Configure the index (filterable/searchable attributes). Call once on startup.
    async fn configure_index(&self) -> Result<(), AppError>;
}

/// Truncate to at most `max_len` bytes, on a char boundary.
#[cfg(feature = "ssr")]
fn truncate_preview(text: &str, max_len: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= max_len {
        return trimmed.to_string();
    }
    let boundary = (0..=max_len)
        .rev()
        .find(|&i| trimmed.is_char_boundary(i))
        .unwrap_or(0);
    trimmed[..boundary].to_string()
}

/// Build the id for one page: stable across re-indexes of the same
/// attachment/page, distinct across pages, safe for use as a Meilisearch
/// primary key (alphanumeric/-/_ only).
pub fn page_id(attachment_key: &str, page: Option<u32>) -> String {
    format!(
        "{}_{}",
        crate::search::client::slug_to_id(attachment_key),
        page.unwrap_or(0)
    )
}

/// Meilisearch implementation of [`AttachmentSearchService`], targeting a
/// dedicated index (separate from the document index) on the same instance.
#[cfg(feature = "ssr")]
pub struct MeilisearchAttachmentService {
    client: meilisearch_sdk::client::Client,
    index_name: String,
}

#[cfg(feature = "ssr")]
impl MeilisearchAttachmentService {
    /// Create from the application's centralised search config (same
    /// Meilisearch instance/credentials as the document index, different
    /// index name — no separate connection config needed).
    ///
    /// Returns `Err` when `search.url` is empty or unset.
    pub fn from_app_config(search: &crate::config::SearchConfig) -> Result<Self, AppError> {
        if search.url.is_empty() {
            return Err(AppError::Internal("search.url is not configured".into()));
        }
        let api_key = if search.api_key.is_empty() {
            None
        } else {
            Some(search.api_key.as_str())
        };
        Self::new(&search.url, api_key)
    }

    pub fn new(
        url: impl Into<String>,
        api_key: Option<impl Into<String>>,
    ) -> Result<Self, AppError> {
        let client = meilisearch_sdk::client::Client::new(url, api_key)
            .map_err(|e| AppError::Internal(format!("Failed to create Meilisearch client: {e}")))?;

        Ok(Self {
            client,
            index_name: "attachment_chunks".to_string(),
        })
    }

    fn index(&self) -> meilisearch_sdk::indexes::Index {
        self.client.index(&self.index_name)
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl AttachmentSearchService for MeilisearchAttachmentService {
    async fn index_pages(
        &self,
        attachment_key: &str,
        filename: &str,
        document_slug: &str,
        document_title: &str,
        pages: &[AttachmentPage],
        access_levels: &[String],
    ) -> Result<(), AppError> {
        self.delete_attachment(attachment_key).await?;

        if pages.is_empty() {
            return Ok(());
        }

        let docs: Vec<AttachmentChunkDocument> = pages
            .iter()
            .filter(|p| !p.text.trim().is_empty())
            .map(|p| AttachmentChunkDocument {
                id: page_id(attachment_key, p.page_number),
                attachment_key: attachment_key.to_string(),
                filename: filename.to_string(),
                document_slug: document_slug.to_string(),
                document_title: document_title.to_string(),
                page: p.page_number,
                content_preview: truncate_preview(&p.text, 200),
                access_levels: access_levels.to_vec(),
            })
            .collect();

        if docs.is_empty() {
            return Ok(());
        }

        self.index()
            .add_documents(&docs, Some("id"))
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch attachment index error: {e}")))?;

        Ok(())
    }

    async fn delete_attachment(&self, attachment_key: &str) -> Result<(), AppError> {
        use meilisearch_sdk::documents::DocumentDeletionQuery;

        let index = self.index();
        let filter = format!("attachment_key = \"{attachment_key}\"");
        let mut query = DocumentDeletionQuery::new(&index);
        query.with_filter(&filter);

        match index.delete_documents_with(&query).await {
            Ok(_) => Ok(()),
            Err(e) => Err(AppError::Internal(format!(
                "Meilisearch attachment delete error: {e}"
            ))),
        }
    }

    async fn update_access_levels(
        &self,
        attachment_key: &str,
        access_levels: &[String],
    ) -> Result<(), AppError> {
        use meilisearch_sdk::documents::DocumentsQuery;

        const PAGE_SIZE: usize = 1000;

        #[derive(Deserialize)]
        struct IdOnly {
            id: String,
        }

        #[derive(Serialize)]
        struct PartialAcl<'a> {
            id: &'a str,
            access_levels: &'a [String],
        }

        let index = self.index();
        let filter = format!("attachment_key = \"{attachment_key}\"");
        let mut offset = 0usize;

        loop {
            let mut query = DocumentsQuery::new(&index);
            query.with_filter(&filter);
            query.with_limit(PAGE_SIZE);
            query.with_offset(offset);
            query.with_fields(["id"]);

            let results: meilisearch_sdk::documents::DocumentsResults<IdOnly> =
                index.get_documents_with(&query).await.map_err(|e| {
                    AppError::Internal(format!("Meilisearch attachment ACL fetch error: {e}"))
                })?;

            if results.results.is_empty() {
                return Ok(());
            }

            let updates: Vec<PartialAcl> = results
                .results
                .iter()
                .map(|d| PartialAcl {
                    id: &d.id,
                    access_levels,
                })
                .collect();

            // `add_or_update` merges by id (only `access_levels` is overwritten,
            // every other field is left untouched), unlike `add_documents` which
            // replaces the whole document.
            self.index()
                .add_or_update(&updates, Some("id"))
                .await
                .map_err(|e| {
                    AppError::Internal(format!("Meilisearch attachment ACL update error: {e}"))
                })?;

            if results.results.len() < PAGE_SIZE {
                return Ok(());
            }

            offset += results.results.len();
        }
    }

    async fn search(
        &self,
        query: &str,
        allowed_levels: Option<&[String]>,
    ) -> Result<Vec<AttachmentSearchHit>, AppError> {
        let mut filters: Vec<String> = Vec::new();

        if let Some(levels) = allowed_levels {
            if levels.is_empty() {
                return Ok(vec![]);
            }
            let quoted: Vec<String> = levels.iter().map(|l| format!("\"{l}\"")).collect();
            filters.push(format!("access_levels IN [{}]", quoted.join(", ")));
        }

        let filter_str = filters.join(" AND ");

        let index = self.index();
        let mut search_query = index.search();
        search_query.with_query(query).with_limit(10);
        if !filter_str.is_empty() {
            search_query.with_filter(&filter_str);
        }

        let results: meilisearch_sdk::search::SearchResults<AttachmentChunkDocument> = search_query
            .execute()
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch attachment search error: {e}")))?;

        Ok(results
            .hits
            .into_iter()
            .map(|hit| AttachmentSearchHit {
                document_slug: hit.result.document_slug,
                document_title: hit.result.document_title,
                attachment_key: hit.result.attachment_key,
                filename: hit.result.filename,
                page: hit.result.page,
                content_preview: hit.result.content_preview,
            })
            .collect())
    }

    async fn configure_index(&self) -> Result<(), AppError> {
        let index = self.index();

        index
            .set_filterable_attributes(["attachment_key", "access_levels"])
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch attachment config error: {e}")))?;

        index
            .set_searchable_attributes(["content_preview", "filename", "document_title"])
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch attachment config error: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "ssr")]
    fn truncate_preview_keeps_short_text_as_is() {
        assert_eq!(truncate_preview("  hello world  ", 200), "hello world");
    }

    #[test]
    #[cfg(feature = "ssr")]
    fn truncate_preview_cuts_long_text_on_char_boundary() {
        let text = "a".repeat(250);
        let truncated = truncate_preview(&text, 200);
        assert_eq!(truncated.len(), 200);
    }

    #[test]
    fn page_id_differs_per_page_and_matches_across_calls() {
        let a = page_id("pdfs/a.pdf", Some(3));
        let b = page_id("pdfs/a.pdf", Some(4));
        assert_ne!(a, b);
        assert_eq!(a, page_id("pdfs/a.pdf", Some(3)));
    }

    #[test]
    fn page_id_defaults_non_paginated_to_zero() {
        assert!(page_id("files/readme.txt", None).ends_with("_0"));
    }
}
