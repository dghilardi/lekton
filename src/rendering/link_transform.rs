use std::collections::HashMap;

/// Target rendering context for link transformation.
#[derive(Copy, Clone)]
pub enum TransformTarget {
    /// Web UI: links become `/docs/{slug}`.
    Web,
    /// MCP resource: links become `lekton://docs/{slug}`.
    Mcp,
}

/// Context needed to resolve relative links inside a document.
pub struct LinkContext<'a> {
    /// `source_path` of the document being rendered (e.g. `"docs/services/device.md"`).
    pub source_path: Option<&'a str>,
    /// Map from `source_path` → `slug` for every document sharing the same
    /// `source_id`.  Built once per render call by fetching all siblings.
    pub siblings: &'a HashMap<String, String>,
}

/// Transform a single URL according to the rendering target and link context.
///
/// Rules:
/// - `http://`, `https://`, `mailto:` — returned unchanged.
/// - `lekton://docs/{slug}` — converted to `/docs/{slug}` (Web) or kept as-is (Mcp).
/// - Relative links (`./foo`, `../bar`) — resolved against the current document's
///   `source_path` directory, then looked up in `siblings`.
/// - Repo-root absolute links (`/foo/bar`) — treated as relative to repo root,
///   looked up in `siblings`.
/// - Anything that cannot be resolved — returned unchanged.
///
/// Anchor fragments (`#section`) are preserved on the resolved URL.
pub fn transform_url(url: &str, ctx: &LinkContext<'_>, target: TransformTarget) -> String {
    // External links pass through unchanged.
    if url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("mailto:")
        || url.starts_with('#')
    {
        return url.to_string();
    }

    // lekton://docs/{slug}[#anchor] — already Lekton-native.
    if let Some(rest) = url.strip_prefix("lekton://docs/") {
        return match target {
            TransformTarget::Web => format!("/docs/{}", rest),
            TransformTarget::Mcp => url.to_string(),
        };
    }

    // For relative / repo-absolute links we need source_path context.
    let Some(source_path) = ctx.source_path else {
        return url.to_string();
    };
    if ctx.siblings.is_empty() {
        return url.to_string();
    }

    // Split off any anchor fragment before resolving the path.
    let (path_part, anchor) = split_anchor(url);

    // Resolve to a candidate source_path within the repo.
    let resolved_source = if path_part.starts_with('/') {
        // Repo-root absolute: strip the leading slash.
        path_part.trim_start_matches('/').to_string()
    } else {
        // Relative: resolve against the directory containing source_path.
        let source_dir = source_path
            .rfind('/')
            .map(|i| &source_path[..=i])
            .unwrap_or("");
        normalize_path(&format!("{}{}", source_dir, path_part))
    };

    // Try exact match, then with .md extension appended.
    let slug = ctx
        .siblings
        .get(&resolved_source)
        .or_else(|| ctx.siblings.get(&format!("{}.md", resolved_source)));

    let Some(slug) = slug else {
        return url.to_string();
    };

    let anchor_suffix = anchor.map(|a| format!("#{}", a)).unwrap_or_default();

    match target {
        TransformTarget::Web => format!("/docs/{}{}", slug, anchor_suffix),
        TransformTarget::Mcp => format!("lekton://docs/{}{}", slug, anchor_suffix),
    }
}

/// Build a `source_path → slug` map suitable for passing to [`LinkContext`].
///
/// Documents without a `source_path` are excluded (they cannot participate in
/// relative link resolution).
pub fn build_siblings_map(docs: &[crate::db::models::Document]) -> HashMap<String, String> {
    docs.iter()
        .filter_map(|d| {
            d.source_path
                .as_deref()
                .map(|sp| (sp.to_string(), d.slug.clone()))
        })
        .collect()
}

/// Rewrite all link `href` attributes in a rendered HTML string.
///
/// This is a lightweight post-processor; it uses a simple state machine rather
/// than a full HTML parser, which is sufficient because the HTML has already
/// been produced by pulldown-cmark and sanitized by ammonia.
pub fn rewrite_links_in_html(html: &str, ctx: &LinkContext<'_>, target: TransformTarget) -> String {
    let mut result = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(href_start) = find_href(rest) {
        // Append everything before `href="`
        result.push_str(&rest[..href_start]);
        rest = &rest[href_start..];

        // Layout: rest = "href=" + quote + url + quote + ...
        //         idx:    0123    5       6         ?
        let quote = rest.as_bytes()[5]; // ' or "
        let content_start = 6; // URL starts here

        if let Some(end) = rest[content_start..].find(quote as char) {
            let url = &rest[content_start..content_start + end];
            let transformed = transform_url(url, ctx, target);
            let external_navigation = should_force_external_navigation(&transformed, target);

            result.push_str(&format!(
                "href={}{}{}",
                quote as char, transformed, quote as char
            ));
            if external_navigation {
                result.push_str(" rel=\"external\"");
            }
            rest = &rest[content_start + end + 1..]; // +1 skips closing quote
        } else {
            // Malformed attribute: copy as-is and continue past `href=`.
            result.push_str("href=");
            rest = &rest[5..];
        }
    }

    result.push_str(rest);
    result
}

/// Rewrite inline markdown link and image URLs in a raw markdown string.
///
/// Handles `[text](url)` and `![alt](url)` patterns.  Reference-style links
/// are not transformed (they are rarely used in imported documentation and would
/// require a two-pass approach).
pub fn rewrite_links_in_markdown(
    markdown: &str,
    ctx: &LinkContext<'_>,
    target: TransformTarget,
) -> String {
    let mut result = String::with_capacity(markdown.len());
    let bytes = markdown.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Look for `](` — the start of an inline link URL.
        if i + 1 < len && bytes[i] == b']' && bytes[i + 1] == b'(' {
            result.push(']');
            result.push('(');
            i += 2;

            // Find the closing `)`, respecting nested parens.
            let url_start = i;
            let mut depth = 1usize;
            while i < len {
                match bytes[i] {
                    b'(' => {
                        depth += 1;
                        i += 1;
                    }
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        i += 1;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }

            let url = &markdown[url_start..i];
            let transformed = transform_url(url, ctx, target);
            result.push_str(&transformed);
            if i < len {
                result.push(')');
                i += 1; // consume `)`
            }
        } else {
            let ch = markdown[i..].chars().next().unwrap();
            result.push(ch);
            i += ch.len_utf8();
        }
    }

    result
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Find the byte offset of the next `href="` or `href='` pattern in `s`.
fn find_href(s: &str) -> Option<usize> {
    // Look for `href=` followed by a quote character.
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..i + 5] == *b"href=" {
            let quote = bytes[i + 5];
            if quote == b'"' || quote == b'\'' {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Split a URL into `(path, anchor)` at the first `#`.
fn split_anchor(url: &str) -> (&str, Option<&str>) {
    match url.find('#') {
        Some(pos) => (&url[..pos], Some(&url[pos + 1..])),
        None => (url, None),
    }
}

/// Normalize a path by resolving `.` and `..` segments.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn should_force_external_navigation(url: &str, target: TransformTarget) -> bool {
    matches!(target, TransformTarget::Web) && is_api_asset_link(url)
}

fn is_api_asset_link(url: &str) -> bool {
    let (path, _) = split_anchor(url);
    let path = path.split('?').next().unwrap_or(path);
    path.starts_with("/api/v1/assets/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_siblings(
        source_path: &str,
        siblings: HashMap<String, String>,
    ) -> (String, HashMap<String, String>) {
        (source_path.to_string(), siblings)
    }

    #[test]
    fn test_external_links_unchanged() {
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: Some("docs/a.md"),
            siblings: &siblings,
        };
        assert_eq!(
            transform_url("https://example.com", &ctx, TransformTarget::Web),
            "https://example.com"
        );
        assert_eq!(
            transform_url("mailto:foo@bar.com", &ctx, TransformTarget::Web),
            "mailto:foo@bar.com"
        );
    }

    #[test]
    fn test_lekton_uri_to_web() {
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: None,
            siblings: &siblings,
        };
        assert_eq!(
            transform_url(
                "lekton://docs/cloud/services/user",
                &ctx,
                TransformTarget::Web
            ),
            "/docs/cloud/services/user"
        );
    }

    #[test]
    fn test_lekton_uri_unchanged_for_mcp() {
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: None,
            siblings: &siblings,
        };
        assert_eq!(
            transform_url(
                "lekton://docs/cloud/services/user",
                &ctx,
                TransformTarget::Mcp
            ),
            "lekton://docs/cloud/services/user"
        );
    }

    #[test]
    fn test_lekton_uri_with_anchor() {
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: None,
            siblings: &siblings,
        };
        assert_eq!(
            transform_url(
                "lekton://docs/cloud/user#section",
                &ctx,
                TransformTarget::Web
            ),
            "/docs/cloud/user#section"
        );
    }

    #[test]
    fn test_relative_link_resolved() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/services/user.md".to_string(),
            "cloud/services/user".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        assert_eq!(
            transform_url("./user", &ctx, TransformTarget::Web),
            "/docs/cloud/services/user"
        );
    }

    #[test]
    fn test_relative_link_parent_dir() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/guidelines/coding.md".to_string(),
            "cloud/guidelines/coding".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        assert_eq!(
            transform_url("../guidelines/coding", &ctx, TransformTarget::Web),
            "/docs/cloud/guidelines/coding"
        );
    }

    #[test]
    fn test_relative_link_mcp() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/services/user.md".to_string(),
            "cloud/services/user".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        assert_eq!(
            transform_url("./user", &ctx, TransformTarget::Mcp),
            "lekton://docs/cloud/services/user"
        );
    }

    #[test]
    fn test_relative_link_with_anchor() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/services/user.md".to_string(),
            "cloud/services/user".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        assert_eq!(
            transform_url("./user#api", &ctx, TransformTarget::Web),
            "/docs/cloud/services/user#api"
        );
    }

    #[test]
    fn test_repo_absolute_link() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/services/user.md".to_string(),
            "cloud/services/user".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        assert_eq!(
            transform_url("/docs/services/user", &ctx, TransformTarget::Web),
            "/docs/cloud/services/user"
        );
    }

    #[test]
    fn test_unknown_link_unchanged() {
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: Some("docs/a.md"),
            siblings: &siblings,
        };
        assert_eq!(
            transform_url("./unknown-doc", &ctx, TransformTarget::Web),
            "./unknown-doc"
        );
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path("docs/services/../guidelines/coding"),
            "docs/guidelines/coding"
        );
        assert_eq!(normalize_path("docs/./services/user"), "docs/services/user");
        assert_eq!(normalize_path("a/b/c"), "a/b/c");
    }

    #[test]
    fn test_rewrite_links_in_html() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/services/user.md".to_string(),
            "cloud/services/user".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        let html = r#"<p>See <a href="./user">user service</a> and <a href="https://example.com">external</a>.</p>"#;
        let result = rewrite_links_in_html(html, &ctx, TransformTarget::Web);
        assert!(result.contains(r#"href="/docs/cloud/services/user""#));
        assert!(!result.contains(r#"href="/docs/cloud/services/user" rel="external""#));
        assert!(result.contains(r#"href="https://example.com""#));
    }

    #[test]
    fn test_asset_links_force_external_navigation() {
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        let html = r#"<p><a href="/api/v1/assets/docs/manual.pdf">manual</a></p>"#;
        let result = rewrite_links_in_html(html, &ctx, TransformTarget::Web);
        assert!(result.contains(r#"href="/api/v1/assets/docs/manual.pdf" rel="external""#));
    }
}
