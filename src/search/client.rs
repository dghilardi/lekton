use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::models::AccessLevel;
use crate::error::AppError;

/// A document representation optimized for the search index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchDocument {
    /// Primary key — the document slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Numeric access level for filtering (Public=0, Developer=1, Architect=2, Admin=3).
    pub access_level: i32,
    /// The team/service that owns this document.
    pub service_owner: String,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// First ~200 characters of content, stripped of markup.
    pub content_preview: String,
    /// Last updated as Unix timestamp (seconds).
    pub last_updated: i64,
}

/// A search result returned to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub slug: String,
    pub title: String,
    pub tags: Vec<String>,
    pub content_preview: String,
}

/// Trait for search operations, enabling mock testing.
#[async_trait]
pub trait SearchService: Send + Sync {
    /// Add or update a document in the search index.
    async fn index_document(&self, doc: &SearchDocument) -> Result<(), AppError>;

    /// Remove a document from the search index.
    async fn delete_document(&self, slug: &str) -> Result<(), AppError>;

    /// Search documents, filtering by access level.
    async fn search(
        &self,
        query: &str,
        max_access_level: AccessLevel,
    ) -> Result<Vec<SearchHit>, AppError>;

    /// Configure the search index (filterable/searchable attributes).
    /// Should be called once on startup.
    async fn configure_index(&self) -> Result<(), AppError>;
}

/// Meilisearch implementation of the SearchService.
#[cfg(feature = "ssr")]
pub struct MeilisearchService {
    client: meilisearch_sdk::client::Client,
    index_name: String,
}

#[cfg(feature = "ssr")]
impl MeilisearchService {
    /// Create a new MeilisearchService from environment variables.
    ///
    /// Reads `MEILISEARCH_URL` and `MEILISEARCH_API_KEY`.
    pub fn from_env() -> Result<Self, AppError> {
        let url = std::env::var("MEILISEARCH_URL")
            .map_err(|_| AppError::Internal("MEILISEARCH_URL not set".into()))?;
        let api_key = std::env::var("MEILISEARCH_API_KEY").ok();

        let client = meilisearch_sdk::client::Client::new(url, api_key)
            .map_err(|e| AppError::Internal(format!("Failed to create Meilisearch client: {e}")))?;

        Ok(Self {
            client,
            index_name: "documents".to_string(),
        })
    }

    fn index(&self) -> meilisearch_sdk::indexes::Index {
        self.client.index(&self.index_name)
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl SearchService for MeilisearchService {
    async fn index_document(&self, doc: &SearchDocument) -> Result<(), AppError> {
        let _task: meilisearch_sdk::task_info::TaskInfo = self
            .index()
            .add_documents(&[doc], Some("slug"))
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch index error: {e}")))?;

        Ok(())
    }

    async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
        let _task: meilisearch_sdk::task_info::TaskInfo = self
            .index()
            .delete_document(slug)
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch delete error: {e}")))?;

        Ok(())
    }

    async fn search(
        &self,
        query: &str,
        max_access_level: AccessLevel,
    ) -> Result<Vec<SearchHit>, AppError> {
        let filter = format!("access_level <= {}", max_access_level as i32);

        let results: meilisearch_sdk::search::SearchResults<SearchDocument> = self
            .index()
            .search()
            .with_query(query)
            .with_filter(&filter)
            .with_limit(20)
            .execute()
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch search error: {e}")))?;

        let hits = results
            .hits
            .into_iter()
            .map(|hit| SearchHit {
                slug: hit.result.slug,
                title: hit.result.title,
                tags: hit.result.tags,
                content_preview: hit.result.content_preview,
            })
            .collect();

        Ok(hits)
    }

    async fn configure_index(&self) -> Result<(), AppError> {
        let index = self.index();

        let _: meilisearch_sdk::task_info::TaskInfo = index
            .set_filterable_attributes(["access_level", "service_owner", "tags"])
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch config error: {e}")))?;

        let _: meilisearch_sdk::task_info::TaskInfo = index
            .set_searchable_attributes(["title", "content_preview", "slug", "tags"])
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch config error: {e}")))?;

        let _: meilisearch_sdk::task_info::TaskInfo = index
            .set_sortable_attributes(["last_updated"])
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch config error: {e}")))?;

        Ok(())
    }
}

/// Build a SearchDocument from a domain Document and its raw markdown content.
pub fn build_search_document(
    doc: &crate::db::models::Document,
    raw_content: &str,
) -> SearchDocument {
    let preview = strip_markdown_for_preview(raw_content, 200);

    SearchDocument {
        slug: doc.slug.clone(),
        title: doc.title.clone(),
        access_level: doc.access_level as i32,
        service_owner: doc.service_owner.clone(),
        tags: doc.tags.clone(),
        content_preview: preview,
        last_updated: doc.last_updated.timestamp(),
    }
}

/// Strip basic markdown syntax for a content preview.
fn strip_markdown_for_preview(raw: &str, max_len: usize) -> String {
    use pulldown_cmark::{Event, Options, Parser};

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;

    let parser = Parser::new_ext(raw, options);
    let mut text = String::new();

    for event in parser {
        match event {
            Event::Text(t) | Event::Code(t) => {
                if !text.is_empty() && !text.ends_with(' ') {
                    text.push(' ');
                }
                text.push_str(&t);
                if text.len() >= max_len {
                    break;
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                text.push(' ');
            }
            _ => {}
        }
    }

    text.truncate(max_len);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_markdown_for_preview() {
        let md = "# Hello World\n\nThis is **bold** and *italic* text.\n\n- Item 1\n- Item 2";
        let preview = strip_markdown_for_preview(md, 200);
        assert!(preview.contains("Hello World"));
        assert!(preview.contains("bold"));
        assert!(preview.contains("italic"));
        assert!(!preview.contains("**"));
        assert!(!preview.contains("*"));
        assert!(!preview.contains("#"));
    }

    #[test]
    fn test_strip_markdown_truncation() {
        let md = "# A very long document\n\n".to_string() + &"word ".repeat(100);
        let preview = strip_markdown_for_preview(&md, 50);
        assert!(preview.len() <= 50);
    }

    #[test]
    fn test_build_search_document() {
        use chrono::Utc;

        let doc = crate::db::models::Document {
            slug: "getting-started".to_string(),
            title: "Getting Started".to_string(),
            s3_key: "docs/getting-started.md".to_string(),
            access_level: AccessLevel::Public,
            service_owner: "platform".to_string(),
            last_updated: Utc::now(),
            tags: vec!["intro".to_string()],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
        };

        let search_doc = build_search_document(&doc, "# Getting Started\n\nWelcome to Lekton.");
        assert_eq!(search_doc.slug, "getting-started");
        assert_eq!(search_doc.access_level, 0);
        assert!(search_doc.content_preview.contains("Getting Started"));
        assert!(search_doc.content_preview.contains("Welcome to Lekton"));
    }
}
