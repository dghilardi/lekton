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
    /// Full plain text of the body. Searchable, but a search never returns it
    /// whole: hits carry [`RETRIEVED_ATTRIBUTES`] and at most a cropped
    /// snippet. Empty for entries indexed before full-body search.
    #[serde(default)]
    pub content: String,
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

/// Searchable attributes in ranking precedence. Document metadata and the
/// preview come before the full body, so they outrank body-only matches.
#[cfg(feature = "ssr")]
const SEARCHABLE_ATTRIBUTES: [&str; 5] = ["title", "content_preview", "slug", "tags", "content"];

/// Attributes a search returns: everything except the full body.
#[cfg(feature = "ssr")]
const RETRIEVED_ATTRIBUTES: [&str; 11] = [
    "id",
    "slug",
    "title",
    "access_level",
    "is_draft",
    "service_owner",
    "tags",
    "content_preview",
    "last_updated",
    "source_id",
    "release",
];

/// Words kept around a match when cropping the preview or a body snippet.
#[cfg(feature = "ssr")]
const SNIPPET_WORDS: usize = 24;

/// How long to wait for a settings update. Changing searchable attributes makes
/// Meilisearch re-process every stored document.
#[cfg(feature = "ssr")]
const SETTINGS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Length of the stored preview, in bytes.
const PREVIEW_LEN: usize = 200;

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

    /// Configure the search index (filterable/searchable attributes) and wait
    /// until the engine applies it. Called on startup and before a re-index.
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
            // Never the full body; `content` only comes back cropped below.
            .with_attributes_to_retrieve(meilisearch_sdk::search::Selectors::Some(
                &RETRIEVED_ATTRIBUTES,
            ))
            .with_attributes_to_highlight(meilisearch_sdk::search::Selectors::Some(&[
                "title",
                "content_preview",
                "content",
            ]))
            .with_highlight_pre_tag(HIGHLIGHT_START)
            .with_highlight_post_tag(HIGHLIGHT_END)
            // A window around the first match rather than the opening words.
            .with_attributes_to_crop(meilisearch_sdk::search::Selectors::Some(&[
                ("content_preview", Some(SNIPPET_WORDS)),
                ("content", Some(SNIPPET_WORDS)),
            ]))
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
                let (mut content_preview, mut preview_matches) = formatted("content_preview")
                    .unwrap_or_else(|| (hit.result.content_preview.clone(), vec![]));
                // A match only in the body shows a snippet around it instead.
                if preview_matches.is_empty() {
                    if let Some((snippet, matches)) =
                        formatted("content").filter(|(_, matches)| !matches.is_empty())
                    {
                        content_preview = snippet;
                        preview_matches = matches;
                    }
                }
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
        let settings = meilisearch_sdk::settings::Settings::new()
            .with_filterable_attributes([
                "access_level",
                "is_draft",
                "service_owner",
                "tags",
                // Filterable now so a release-scoped search later needs no
                // settings change and no re-index.
                "source_id",
                "release",
            ])
            .with_searchable_attributes(SEARCHABLE_ATTRIBUTES)
            .with_sortable_attributes(["last_updated"]);

        let task = self
            .index()
            .set_settings(&settings)
            .await
            .map_err(|e| AppError::Internal(format!("Meilisearch config error: {e}")))?;
        // Wait, so searches and a following re-index use the new attributes and
        // a rejected update is reported instead of silently ignored.
        let task = task
            .wait_for_completion(&self.client, None, Some(SETTINGS_TIMEOUT))
            .await
            .map_err(|e| {
                AppError::Internal(format!("Meilisearch settings update not applied: {e}"))
            })?;
        if let meilisearch_sdk::tasks::Task::Failed { content } = task {
            return Err(AppError::Internal(format!(
                "Meilisearch settings update failed: {}",
                content.error
            )));
        }

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
    let content = plain_text(raw_content);
    let preview = truncate_on_char_boundary(&content, PREVIEW_LEN);

    SearchDocument {
        id: slug_to_id(&doc.slug),
        slug: doc.slug.clone(),
        title: doc.title.clone(),
        access_level: doc.access_level.clone(),
        is_draft: doc.is_draft,
        service_owner: doc.service_owner.clone(),
        tags: doc.tags.clone(),
        content_preview: preview,
        content,
        last_updated: doc.last_updated.timestamp(),
        source_id: doc.source_id.clone(),
        release: doc.release.clone(),
    }
}

/// Plain text of a stored body: Markdown, or the HTML the editor saves. Code
/// spans and blocks are kept verbatim so identifiers stay searchable.
fn plain_text(raw: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser};

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;

    let parser = Parser::new_ext(raw, options);
    let mut text = String::new();
    let mut html = HtmlText::default();

    for event in parser {
        match event {
            Event::Text(t) | Event::Code(t) => {
                if !text.is_empty() && !text.ends_with(' ') {
                    text.push(' ');
                }
                text.push_str(&t);
            }
            Event::Html(h) | Event::InlineHtml(h) => html.feed(&h, &mut text),
            Event::SoftBreak | Event::HardBreak => {
                text.push(' ');
            }
            _ => {}
        }
    }
    text
}

/// The first `max_len` bytes of `text`, cut on a character boundary.
fn truncate_on_char_boundary(text: &str, max_len: usize) -> String {
    text[..text.floor_char_boundary(max_len)].to_string()
}

/// Visible text of raw HTML, fed one parser event at a time. Tags break words
/// unless they are inline formatting; comments and script/style bodies are
/// dropped; common entities are decoded.
#[derive(Default)]
struct HtmlText {
    /// The tag being read, from after `<`, when inside one.
    tag: Option<String>,
    /// The element whose body is skipped (`script` or `style`).
    skipping: Option<String>,
}

impl HtmlText {
    const INLINE: [&'static str; 14] = [
        "a", "abbr", "b", "code", "em", "i", "kbd", "mark", "s", "span", "strong", "sub", "sup",
        "u",
    ];

    fn feed(&mut self, html: &str, out: &mut String) {
        let mut visible = String::new();
        for ch in html.chars() {
            let Some(tag) = self.tag.as_mut() else {
                if ch == '<' {
                    self.tag = Some(String::new());
                } else if self.skipping.is_none() {
                    visible.push(ch);
                }
                continue;
            };
            if tag.is_empty() && ch.is_whitespace() {
                // A literal `<`, not a tag.
                self.tag = None;
                if self.skipping.is_none() {
                    visible.push('<');
                    visible.push(ch);
                }
            } else if ch == '>' && !(tag.starts_with("!--") && !tag.ends_with("--")) {
                let tag = self.tag.take().unwrap_or_default();
                self.close_tag(&tag, &mut visible);
            } else {
                tag.push(ch);
            }
        }
        for ch in decode_entities(&visible).chars() {
            if ch.is_whitespace() {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
            } else {
                out.push(ch);
            }
        }
    }

    fn close_tag(&mut self, tag: &str, visible: &mut String) {
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if let Some(skipped) = &self.skipping {
            if closing && *skipped == name {
                self.skipping = None;
            }
            return;
        }
        if !closing && (name == "script" || name == "style") {
            self.skipping = Some(name);
        } else if !Self::INLINE.contains(&name.as_str()) {
            visible.push(' ');
        }
    }
}

/// Decodes the named entities editors emit and numeric character references.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let ch = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            };
            ch.map(|ch| (ch, end))
        });
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
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
        let preview = truncate_on_char_boundary(&plain_text(md), 200);
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
        let preview = truncate_on_char_boundary(&plain_text(&md), 50);
        assert!(preview.len() <= 50);
    }

    #[test]
    fn plain_text_keeps_markdown_code_and_text_past_the_preview() {
        let md = format!(
            "# Devices\n\n{}\n\nCall `POST /api/v1/devices` on the TP-200.\n\n\
             ```rust\nfn reset_device(code: &str) -> ErrDevice42 {{}}\n```\n\n\
             | Code | Meaning |\n|---|---|\n| ERR_DEVICE_42 | reboot |\n",
            "filler ".repeat(100)
        );
        let text = plain_text(&md);
        for needle in [
            "POST /api/v1/devices",
            "TP-200",
            "fn reset_device(code: &str) -> ErrDevice42",
            "ERR_DEVICE_42",
        ] {
            assert!(text.contains(needle), "{needle} missing from {text}");
        }
        assert!(!text.contains("```") && !text.contains('|') && !text.contains('#'));
    }

    #[test]
    fn plain_text_reads_the_html_bodies_the_editor_stores() {
        let html = "<h1>Device&nbsp;guide</h1><p>Reset the <strong>TP</strong>-200 \
                    with <code>reset_device()</code>.</p><pre><code>if a &lt; b &amp;&amp; \
                    c &#62; d { ERR_DEVICE_42 }</code></pre><!-- <p>hidden</p> --><script>\
                    alert('x')</script><ul><li>first</li><li>second</li></ul>";
        let text = plain_text(html);
        assert_eq!(
            text,
            "Device guide Reset the TP-200 with reset_device(). \
             if a < b && c > d { ERR_DEVICE_42 } first second "
        );
    }

    #[test]
    fn preview_is_the_unchanged_prefix_of_the_full_text() {
        let md = format!("# Intro\n\n{} tail_identifier", "àèì ".repeat(200));
        let doc = build_search_document(&document("long"), &md);
        assert!(doc.content_preview.len() <= PREVIEW_LEN);
        assert!(doc.content_preview.len() > PREVIEW_LEN - 4);
        assert!(doc.content.starts_with(&doc.content_preview));
        assert!(doc.content.ends_with("tail_identifier"));
        assert!(!doc.content_preview.contains("tail_identifier"));
    }

    #[test]
    fn entries_indexed_before_full_body_search_still_deserialize() {
        let legacy = serde_json::json!({
            "id": "a", "slug": "a", "title": "A", "access_level": "public",
            "is_draft": false, "service_owner": "team", "tags": [],
            "content_preview": "preview", "last_updated": 0
        });
        let doc: SearchDocument = serde_json::from_value(legacy).unwrap();
        assert!(doc.content.is_empty());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn searches_retrieve_every_field_but_the_full_body() {
        let doc = build_search_document(&document("a"), "body");
        let mut fields: Vec<_> = serde_json::to_value(&doc)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| *k != "content")
            .cloned()
            .collect();
        let mut retrieved: Vec<_> = RETRIEVED_ATTRIBUTES.map(String::from).to_vec();
        fields.sort();
        retrieved.sort();
        assert_eq!(fields, retrieved);
        assert_eq!(SEARCHABLE_ATTRIBUTES.last(), Some(&"content"));
    }

    fn document(slug: &str) -> crate::db::models::Document {
        crate::db::models::Document {
            slug: slug.to_string(),
            title: slug.to_string(),
            summary: None,
            s3_key: format!("docs/{slug}.md"),
            access_level: "public".to_string(),
            is_draft: false,
            service_owner: "team".to_string(),
            last_updated: chrono::Utc::now(),
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
        }
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
