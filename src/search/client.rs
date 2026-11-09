use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// A document representation optimized for the search index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchDocument {
    /// Primary key — slug with `/` replaced by `__` (Meilisearch requires alphanumeric/-/_).
    pub id: String,
    /// Document slug (original path, e.g. `incidents/2025-12-13`).
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Access level name (e.g. `"public"`, `"internal"`).
    pub access_level: String,
    /// Whether the document is a draft.
    pub is_draft: bool,
    /// The team/service that owns this document.
    pub service_owner: String,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// First ~200 characters of content, stripped of markup.
    pub content_preview: String,
    /// Last updated as Unix timestamp (seconds).
    pub last_updated: i64,
    /// The import source owning the document. Carried so a future release-scoped
    /// search is a filter rather than a full re-index of the corpus.
    #[serde(default)]
    pub source_id: Option<String>,
    /// The release this document belongs to. Only `latest` is indexed today, so
    /// this records *which* release the entry reflects; adding it later would
    /// have meant re-indexing everything.
    #[serde(default)]
    pub release: Option<String>,
}

/// A search result returned to the client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchHit {
    pub slug: String,
    pub title: String,
    pub tags: Vec<String>,
    pub content_preview: String,
    /// Asset key when this hit came from an attachment page. `None` for
    /// ordinary document hits.
    #[serde(default)]
    pub attachment_key: Option<String>,
    /// Set when this hit came from a PDF attachment's page rather than the
    /// document's own title/summary/tags — the match is inside the linked
    /// PDF, on this page. `None` for ordinary document hits.
    #[serde(default)]
    pub page: Option<u32>,
    /// Character ranges (`[start, end)`) of `title` that match the query.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub title_matches: Vec<[usize; 2]>,
    /// Character ranges (`[start, end)`) of `content_preview` that match the query.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preview_matches: Vec<[usize; 2]>,
}

/// Highlight delimiters asked of the search engine: private-use characters,
/// so matches come back as ranges over plain text rather than as markup.
pub const HIGHLIGHT_START: &str = "\u{E000}";
pub const HIGHLIGHT_END: &str = "\u{E001}";

/// Removes highlight delimiters from `marked`, returning the plain text and the
/// character ranges they enclosed.
pub fn split_highlights(marked: &str) -> (String, Vec<[usize; 2]>) {
    let mut text = String::with_capacity(marked.len());
    let mut ranges = Vec::new();
    let mut open = None;
    let mut chars = 0;
    for c in marked.chars() {
        match c {
            '\u{E000}' => open = Some(chars),
            '\u{E001}' => {
                if let Some(start) = open.take() {
                    if start < chars {
                        ranges.push([start, chars]);
                    }
                }
            }
            _ => {
                text.push(c);
                chars += 1;
            }
        }
    }
    (text, ranges)
}

/// Trait for search operations, enabling mock testing.
#[async_trait]
pub trait SearchService: Send + Sync {
    /// Add or update a document in the search index.
    async fn index_document(&self, doc: &SearchDocument) -> Result<(), AppError>;

    /// Remove a document from the search index.
    async fn delete_document(&self, slug: &str) -> Result<(), AppError>;

    /// Search documents visible to the caller.
    ///
    /// - `allowed_levels`: the access level names the caller can read.
    ///   `None` means admin (no level restriction).
    /// - `include_draft`: whether to include draft documents.
    async fn search(
        &self,
        query: &str,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<SearchHit>, AppError>;

    /// Configure the search index (filterable/searchable attributes).
    /// Should be called once on startup.
    async fn configure_index(&self) -> Result<(), AppError>;

    /// Cheap reachability probe for readiness checks: `Ok(())` only if the
    /// search backend actually answers, not merely because the service was
    /// constructed.
    async fn health_check(&self) -> Result<(), AppError>;
}

/// Meilisearch implementation of the SearchService.
#[cfg(feature = "ssr")]
pub struct MeilisearchService {
    client: meilisearch_sdk::client::Client,
    index_name: String,
}

#[cfg(feature = "ssr")]
impl MeilisearchService {
    /// Create a new MeilisearchService from the application's centralised config.
    ///
    /// Returns `Err` when `search.url` is empty or unset (search is then disabled).
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

    /// Create a new MeilisearchService with explicit URL and optional API key.
    pub fn new(
        url: impl Into<String>,
        api_key: Option<impl Into<String>>,
    ) -> Result<Self, AppError> {
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
            .add_documents(&[doc], Some("id"))
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch index error: {e}")))?;

        Ok(())
    }

    async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
        let id = slug_to_id(slug);
        match self.index().delete_document(&id).await {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("404") => Ok(()),
            Err(e) => Err(AppError::Internal(format!("Meilisearch delete error: {e}"))),
        }
    }

    async fn search(
        &self,
        query: &str,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<SearchHit>, AppError> {
        // Build a Meilisearch filter expression.
        let mut filters: Vec<String> = Vec::new();

        if let Some(levels) = allowed_levels {
            if levels.is_empty() {
                // The caller has no readable levels → return nothing.
                return Ok(vec![]);
            }
            // e.g. access_level IN ["public", "internal"]
            let quoted: Vec<String> = levels.iter().map(|l| format!("\"{}\"", l)).collect();
            filters.push(format!("access_level IN [{}]", quoted.join(", ")));
        }

        if !include_draft {
            filters.push("is_draft = false".to_string());
        }

        let filter_str = filters.join(" AND ");

        let index = self.index();
        let mut search_query = index.search();
        search_query
            .with_query(query)
            .with_limit(20)
            .with_attributes_to_highlight(meilisearch_sdk::search::Selectors::Some(&[
                "title",
                "content_preview",
            ]))
            .with_highlight_pre_tag(HIGHLIGHT_START)
            .with_highlight_post_tag(HIGHLIGHT_END)
            // A window around the first match rather than the opening words.
            .with_attributes_to_crop(meilisearch_sdk::search::Selectors::Some(&[(
                "content_preview",
                Some(24),
            )]))
            .with_crop_marker("…");

        if !filter_str.is_empty() {
            search_query.with_filter(&filter_str);
        }

        let results: meilisearch_sdk::search::SearchResults<SearchDocument> = search_query
            .execute()
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch search error: {e}")))?;

        let hits = results
            .hits
            .into_iter()
            .map(|hit| {
                let formatted = |field: &str| {
                    hit.formatted_result
                        .as_ref()
                        .and_then(|formatted| formatted.get(field))
                        .and_then(serde_json::Value::as_str)
                        .map(split_highlights)
                };
                let (title, title_matches) =
                    formatted("title").unwrap_or_else(|| (hit.result.title.clone(), vec![]));
                let (content_preview, preview_matches) = formatted("content_preview")
                    .unwrap_or_else(|| (hit.result.content_preview.clone(), vec![]));
                SearchHit {
                    slug: hit.result.slug,
                    title,
                    tags: hit.result.tags,
                    content_preview,
                    attachment_key: None,
                    page: None,
                    title_matches,
                    preview_matches,
                }
            })
            .collect();

        Ok(hits)
    }

    async fn configure_index(&self) -> Result<(), AppError> {
        let index = self.index();

        let _: meilisearch_sdk::task_info::TaskInfo = index
            .set_filterable_attributes([
                "access_level",
                "is_draft",
                "service_owner",
                "tags",
                // Filterable now so a release-scoped search later needs no
                // settings change and no re-index.
                "source_id",
                "release",
            ])
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

    async fn health_check(&self) -> Result<(), AppError> {
        self.client
            .health()
            .await
            .map(|_| ())
            .map_err(|e| AppError::Internal(format!("Meilisearch health check failed: {e}")))
    }
}

/// Convert a slug to a valid Meilisearch document ID.
///
/// Meilisearch only allows alphanumeric characters, hyphens, and underscores.
/// Base64 URL-safe (no padding) encoding guarantees no collisions regardless of slug content.
pub fn slug_to_id(slug: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD.encode(slug.as_bytes())
}

/// Build a SearchDocument from a domain Document and its raw markdown content.
pub fn build_search_document(
    doc: &crate::db::models::Document,
    raw_content: &str,
) -> SearchDocument {
    let preview = strip_markdown_for_preview(raw_content, 200);

    SearchDocument {
        id: slug_to_id(&doc.slug),
        slug: doc.slug.clone(),
        title: doc.title.clone(),
        access_level: doc.access_level.clone(),
        is_draft: doc.is_draft,
        service_owner: doc.service_owner.clone(),
        tags: doc.tags.clone(),
        content_preview: preview,
        last_updated: doc.last_updated.timestamp(),
        source_id: doc.source_id.clone(),
        release: doc.release.clone(),
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

    if text.len() > max_len {
        let boundary = text.floor_char_boundary(max_len);
        text.truncate(boundary);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_become_character_ranges_over_plain_text() {
        let (text, ranges) =
            split_highlights("Deploy \u{E000}kafka\u{E001} to \u{E000}prod\u{E001}…");
        assert_eq!(text, "Deploy kafka to prod…");
        assert_eq!(ranges, vec![[7, 12], [16, 20]]);

        let (text, ranges) = split_highlights("città \u{E000}è\u{E001}");
        assert_eq!(text, "città è");
        assert_eq!(ranges, vec![[6, 7]]);

        // Unbalanced or empty markers never produce a range.
        assert_eq!(
            split_highlights("\u{E001}a\u{E000}\u{E001}b"),
            ("ab".to_string(), vec![])
        );
    }

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
            summary: None,
            s3_key: "docs/getting-started.md".to_string(),
            access_level: "public".to_string(),
            is_draft: false,
            service_owner: "platform".to_string(),
            last_updated: Utc::now(),
            tags: vec!["intro".to_string()],
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
        };

        let search_doc = build_search_document(&doc, "# Getting Started\n\nWelcome to Lekton.");
        assert_eq!(search_doc.slug, "getting-started");
        assert_eq!(search_doc.access_level, "public");
        assert!(!search_doc.is_draft);
        assert!(search_doc.content_preview.contains("Getting Started"));
        assert!(search_doc.content_preview.contains("Welcome to Lekton"));
    }

    #[test]
    fn test_build_search_document_preserves_draft_flag() {
        use chrono::Utc;

        let doc = crate::db::models::Document {
            slug: "wip-doc".to_string(),
            title: "WIP".to_string(),
            summary: None,
            s3_key: "docs/wip.md".to_string(),
            access_level: "internal".to_string(),
            is_draft: true,
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
        };

        let search_doc = build_search_document(&doc, "# WIP content");
        assert_eq!(search_doc.access_level, "internal");
        assert!(search_doc.is_draft);
    }
}
