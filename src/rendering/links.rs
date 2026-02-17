use pulldown_cmark::{Event, Options, Parser, Tag};

/// Extract all internal links from markdown content.
///
/// Internal links are those pointing to other Lekton documents
/// (starting with `/docs/` or relative paths, excluding external URLs and anchors).
///
/// Returns a deduplicated list of normalized slugs.
pub fn extract_internal_links(markdown: &str) -> Vec<String> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION;

    let parser = Parser::new_ext(markdown, options);
    let mut links = Vec::new();

    for event in parser {
        if let Event::Start(Tag::Link { dest_url, .. }) = event {
            let url = dest_url.as_ref();

            if is_internal_link(url) {
                let normalized = normalize_link(url);
                if !normalized.is_empty() && !links.contains(&normalized) {
                    links.push(normalized);
                }
            }
        }
    }

    links
}

/// Determine if a link is internal (points to another Lekton document).
///
/// External links (`http://`, `https://`), anchors (`#heading`),
/// and mailto links are excluded.
fn is_internal_link(url: &str) -> bool {
    !url.starts_with("http://")
        && !url.starts_with("https://")
        && !url.starts_with('#')
        && !url.starts_with("mailto:")
}

/// Normalize a link to a slug format.
///
/// Strips the `/docs/` prefix, removes anchor fragments and trailing slashes.
fn normalize_link(url: &str) -> String {
    let stripped = url
        .trim_start_matches('/')
        .trim_start_matches("docs/");

    // Remove anchor fragment
    let without_anchor = stripped.split('#').next().unwrap_or("");

    without_anchor.trim_end_matches('/').to_string()
}

/// Result of validating internal links in a document.
#[derive(Debug, Clone)]
pub struct LinkValidationResult {
    /// All internal link slugs found in the document.
    pub all_links: Vec<String>,
    /// Links whose target documents do not exist.
    pub broken_links: Vec<String>,
    /// Links whose target documents exist.
    pub valid_links: Vec<String>,
}

/// Validate all internal links in markdown content against the document repository.
///
/// Extracts internal links, checks each against the repository,
/// and returns categorized results.
#[cfg(feature = "ssr")]
pub async fn validate_links(
    markdown: &str,
    repo: &dyn crate::db::repository::DocumentRepository,
) -> Result<LinkValidationResult, crate::error::AppError> {
    let all_links = extract_internal_links(markdown);
    let mut broken_links = Vec::new();
    let mut valid_links = Vec::new();

    for link in &all_links {
        match repo.find_by_slug(link).await? {
            Some(_) => valid_links.push(link.clone()),
            None => broken_links.push(link.clone()),
        }
    }

    Ok(LinkValidationResult {
        all_links,
        broken_links,
        valid_links,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_internal_links_basic() {
        let md = "Check the [deployment guide](/docs/deployment-guide) for details.";
        let links = extract_internal_links(md);
        assert_eq!(links, vec!["deployment-guide"]);
    }

    #[test]
    fn test_extract_multiple_links() {
        let md = r#"
See [getting started](/docs/getting-started) and [architecture](/docs/architecture).
Also check [API reference](/docs/api-reference).
"#;
        let links = extract_internal_links(md);
        assert_eq!(links, vec!["getting-started", "architecture", "api-reference"]);
    }

    #[test]
    fn test_external_links_excluded() {
        let md = r#"
Visit [Rust](https://www.rust-lang.org) and [Google](http://google.com).
Also see [internal](/docs/internal-doc).
"#;
        let links = extract_internal_links(md);
        assert_eq!(links, vec!["internal-doc"]);
    }

    #[test]
    fn test_anchor_only_links_excluded() {
        let md = "Jump to [section](#installation) below.";
        let links = extract_internal_links(md);
        assert!(links.is_empty());
    }

    #[test]
    fn test_anchored_internal_link_normalized() {
        let md = "See [setup](/docs/getting-started#installation).";
        let links = extract_internal_links(md);
        assert_eq!(links, vec!["getting-started"]);
    }

    #[test]
    fn test_relative_links() {
        let md = "See [sibling](architecture) for details.";
        let links = extract_internal_links(md);
        assert_eq!(links, vec!["architecture"]);
    }

    #[test]
    fn test_trailing_slash_stripped() {
        let md = "See [guide](/docs/deployment-guide/).";
        let links = extract_internal_links(md);
        assert_eq!(links, vec!["deployment-guide"]);
    }

    #[test]
    fn test_deduplication() {
        let md = r#"
[first link](/docs/architecture) and [second link](/docs/architecture).
"#;
        let links = extract_internal_links(md);
        assert_eq!(links, vec!["architecture"]);
    }

    #[test]
    fn test_empty_content() {
        let links = extract_internal_links("");
        assert!(links.is_empty());
    }

    #[test]
    fn test_no_links() {
        let md = "This is plain text with **bold** and *italic* but no links.";
        let links = extract_internal_links(md);
        assert!(links.is_empty());
    }

    #[test]
    fn test_mailto_excluded() {
        let md = "Contact [support](mailto:support@example.com).";
        let links = extract_internal_links(md);
        assert!(links.is_empty());
    }

    #[test]
    fn test_nested_path_links() {
        let md = "See [guide](/docs/engineering/deployment-guide).";
        let links = extract_internal_links(md);
        assert_eq!(links, vec!["engineering/deployment-guide"]);
    }

    #[test]
    fn test_normalize_link() {
        assert_eq!(normalize_link("/docs/hello"), "hello");
        assert_eq!(normalize_link("/docs/hello/"), "hello");
        assert_eq!(normalize_link("/docs/hello#anchor"), "hello");
        assert_eq!(normalize_link("docs/hello"), "hello");
        assert_eq!(normalize_link("relative-page"), "relative-page");
        assert_eq!(normalize_link("/docs/a/b/c"), "a/b/c");
    }

    #[test]
    fn test_is_internal_link() {
        assert!(is_internal_link("/docs/hello"));
        assert!(is_internal_link("relative-page"));
        assert!(!is_internal_link("https://example.com"));
        assert!(!is_internal_link("http://example.com"));
        assert!(!is_internal_link("#anchor"));
        assert!(!is_internal_link("mailto:test@example.com"));
    }
}
