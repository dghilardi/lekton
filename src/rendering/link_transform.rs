use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Map from normalized `source_path` → `slug` for every document sharing the same
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
/// Normalize keys just like resolved link paths, including absolute verbatim
/// Windows paths stored by older clients, whose leading `//` would otherwise
/// disappear only from the lookup candidate.
pub fn build_siblings_map(docs: &[crate::db::models::Document]) -> HashMap<String, String> {
    docs.iter()
        .filter_map(|d| {
            d.source_path
                .as_deref()
                .map(|sp| (normalize_path(sp), d.slug.clone()))
        })
        .collect()
}

/// Render a document's Markdown to sanitized HTML for the Web UI, resolving
/// every link `href` with [`transform_url`].
///
/// Targets are resolved inside the sanitizer, on attributes it has already
/// parsed: Markdown links and raw HTML alike, and each exactly once (a resolved
/// `/docs/{slug}` read again as a repo-root path could match a sibling's
/// `source_path`). The sanitizer admits the `lekton` scheme only to hand it to
/// the resolver; a `lekton://` target that does not resolve is dropped as
/// before. Links to `/api/v1/assets/…` also get `external` in their `rel`, so
/// the client router lets the browser download them.
pub fn render_web_html(raw: &str, ctx: &LinkContext<'_>) -> String {
    let source_path = ctx.source_path.map(str::to_owned);
    let siblings = ctx.siblings.clone();
    // The sanitizer visits an `<a>`'s attributes in order, ending with the
    // `rel` it adds itself, so the `href` verdict is known by then.
    let asset_link = AtomicBool::new(false);

    crate::rendering::markdown::render_markdown_with_sanitizer(raw, move |sanitizer| {
        sanitizer.add_url_schemes(&["lekton"]).attribute_filter(
            move |element, attribute, value| {
                let value: Cow<'_, str> = match (element, attribute) {
                    ("a", "href") => {
                        let ctx = LinkContext {
                            source_path: source_path.as_deref(),
                            siblings: &siblings,
                        };
                        let url = transform_url(value, &ctx, TransformTarget::Web);
                        asset_link.store(is_api_asset_link(&url), Ordering::Relaxed);
                        url.into()
                    }
                    ("a", "rel") if asset_link.swap(false, Ordering::Relaxed) => {
                        format!("{value} external").into()
                    }
                    _ => value.into(),
                };
                // A target still in the `lekton` scheme did not resolve.
                let unresolved = matches!(attribute, "href" | "src")
                    && url::Url::parse(&value).is_ok_and(|url| url.scheme() == "lekton");
                (!unresolved).then_some(value)
            },
        );
    })
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

fn is_api_asset_link(url: &str) -> bool {
    let (path, _) = split_anchor(url);
    let path = path.split('?').next().unwrap_or(path);
    path.starts_with("/api/v1/assets/")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn legacy_windows_source_paths_resolve_in_web_and_mcp() {
        // Older Windows clients stored absolute verbatim paths. Exercise the
        // production map builder as well as the HTML and MCP render paths.
        for root in ["//?/C:/workspace/repo", "//?/F:/Jenkins/workspace/repo"] {
            let source_path = format!("{root}/doc/README.md");
            let target_path = format!("{root}/doc/guide.md");
            let document = bson::from_document(bson::doc! {
                "slug": "project/guide",
                "title": "Guide",
                "s3_key": "docs/guide.md",
                "access_level": "public",
                "service_owner": "docs",
                "last_updated": bson::DateTime::now(),
                "tags": [],
                "links_out": [],
                "backlinks": [],
                "source_path": target_path,
            })
            .unwrap();
            let siblings = build_siblings_map(&[document]);
            let ctx = LinkContext {
                source_path: Some(&source_path),
                siblings: &siblings,
            };

            for link in ["guide.md#setup", "./guide.md#setup", "../doc/guide#setup"] {
                let markdown = format!("[Guide]({link})");
                assert!(
                    render_web_html(&markdown, &ctx).contains("href=\"/docs/project/guide#setup\"")
                );
                assert_eq!(
                    rewrite_links_in_markdown(&markdown, &ctx, TransformTarget::Mcp),
                    "[Guide](lekton://docs/project/guide#setup)"
                );
            }
            assert_eq!(
                transform_url("missing.md", &ctx, TransformTarget::Web),
                "missing.md"
            );
        }
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
    fn test_render_web_html_resolves_relative_and_external_links() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/services/user.md".to_string(),
            "cloud/services/user".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        let result = render_web_html(
            "See [user service](./user.md#api) and [external](https://example.com).",
            &ctx,
        );
        assert!(result.contains(r#"href="/docs/cloud/services/user#api""#));
        assert!(!result.contains(r#"href="/docs/cloud/services/user#api" rel="external""#));
        assert!(result.contains(r#"href="https://example.com""#));
    }

    #[test]
    fn test_render_web_html_resolves_lekton_uris_without_source() {
        // Documents without a source_id have no siblings; `lekton://` URIs
        // still resolve, with their anchor, as inline and reference links.
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: None,
            siblings: &siblings,
        };
        let result = render_web_html(
            "[Read on Lekton](lekton://docs/docs/guidelines/review)\n\n\
             [Section](lekton://docs/cloud/design#naming) and [ref][g]\n\n\
             [g]: lekton://docs/cloud/glossary",
            &ctx,
        );
        assert!(result.contains(r#"href="/docs/docs/guidelines/review""#));
        assert!(result.contains(r#"href="/docs/cloud/design#naming""#));
        assert!(result.contains(r#"href="/docs/cloud/glossary""#));
        assert!(!result.contains("lekton://"));
    }

    #[test]
    fn test_render_web_html_resolves_lekton_uri_once() {
        // `/docs/guide` must not be read again as the repo path of a sibling.
        let mut siblings = HashMap::new();
        siblings.insert("docs/guide.md".to_string(), "other/guide".to_string());
        let ctx = LinkContext {
            source_path: Some("docs/index.md"),
            siblings: &siblings,
        };
        let result = render_web_html("[Guide](lekton://docs/guide)", &ctx);
        assert!(result.contains(r#"href="/docs/guide""#));
        assert!(!result.contains("/docs/other/guide"));
    }

    #[test]
    fn test_render_web_html_keeps_sanitizing() {
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: None,
            siblings: &siblings,
        };
        let result = render_web_html(
            "[x](javascript:alert(1)) <script>alert(1)</script> [y](lekton://other/z) \
             <a href=\" LEKTON://docs/z\">z</a> ![i](lekton://assets/i.png)",
            &ctx,
        );
        assert!(!result.contains("javascript:"));
        assert!(!result.contains("<script"));
        // Only `lekton://docs/` is resolvable; anything else is still dropped.
        assert!(!result.to_lowercase().contains("lekton:"));
    }

    #[test]
    fn test_asset_links_force_external_navigation() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "api/v1/assets/docs/arch.png.md".to_string(),
            "notes/architecture".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        let result = render_web_html(
            "[manual](/api/v1/assets/docs/manual.pdf \"see href='/api/v1/assets/x.pdf'\") \
             ![diagram](/api/v1/assets/docs/arch.png) [page](lekton://docs/a)",
            &ctx,
        );
        // One `rel`, keeping the sanitizer's tokens; the title is untouched.
        assert!(result.contains(
            r#"<a href="/api/v1/assets/docs/manual.pdf" title="see href='/api/v1/assets/x.pdf'" rel="noopener noreferrer external">"#
        ));
        assert_eq!(result.matches("rel=").count(), 2);
        // Images keep their target even when it looks like a sibling's path.
        assert!(result.contains(r#"src="/api/v1/assets/docs/arch.png""#));
        assert!(!result.contains("notes/architecture"));
        assert!(result.contains(r#"<a href="/docs/a" rel="noopener noreferrer">"#));
    }

    #[test]
    fn test_render_web_html_resolves_raw_html_and_autolinks() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/services/user.md".to_string(),
            "cloud/services/user".to_string(),
        );
        siblings.insert(
            "docs/services/team@example.com.md".to_string(),
            "cloud/services/team".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        let result = render_web_html(
            "<a href=\"./user.md#api\">Users</a> <a href='lekton://docs/cloud/x'>X</a>\n\n\
             <lekton://docs/cloud/guide#api> <team@example.com> [top](#intro)",
            &ctx,
        );
        assert!(result.contains(r#"href="/docs/cloud/services/user#api""#));
        assert!(result.contains(r#"href="/docs/cloud/x""#));
        assert!(result.contains(r#"href="/docs/cloud/guide#api""#));
        // An e-mail autolink is an address, not a repo path.
        assert!(result.contains(r#"href="mailto:team@example.com""#));
        assert!(!result.contains("/docs/cloud/services/team"));
        assert!(result.contains(r##"href="#intro""##));
    }

    #[test]
    fn test_render_web_html_reads_raw_html_attributes_as_parsed() {
        let mut siblings = HashMap::new();
        siblings.insert(
            "docs/services/user.md".to_string(),
            "cloud/services/user".to_string(),
        );
        let ctx = LinkContext {
            source_path: Some("docs/services/device.md"),
            siblings: &siblings,
        };
        let result = render_web_html(
            "<a HREF = \"./user.md#api\">Users</a>\n\n\
             <div>\n<a\nhref=\"./user.md\"\n>Block</a>\n</div>\n\n\
             <a href=\"#intro\" title='Example: href=\"./user.md#api\"'>Intro</a>\n\n\
             ```html\n<a href=\"./user.md\">code</a>\n```",
            &ctx,
        );
        assert!(result.contains(
            r#"<a href="/docs/cloud/services/user#api" rel="noopener noreferrer">Users</a>"#
        ));
        assert!(result.contains(
            r#"<a href="/docs/cloud/services/user" rel="noopener noreferrer">Block</a>"#
        ));
        // Only URL attributes are resolved: the title keeps its text.
        assert!(result.contains(r#"title="Example: href=&quot;./user.md#api&quot;""#));
        // Code is text, not markup: only the two links above resolve.
        assert_eq!(result.matches("/docs/cloud/services/user").count(), 2);
        assert!(result.contains("./user.md"));
    }

    #[test]
    fn test_asset_rel_is_scoped_to_its_own_link() {
        let siblings = HashMap::new();
        let ctx = LinkContext {
            source_path: None,
            siblings: &siblings,
        };
        let result = render_web_html(
            "[manual](/api/v1/assets/a.pdf \"così \\\"<b>\\\" & più\") <a>bare</a> [next](https://example.com)",
            &ctx,
        );
        assert!(result.contains(
            r#"title="così &quot;&lt;b&gt;&quot; &amp; più" rel="noopener noreferrer external""#
        ));
        assert!(
            result.contains(r#"<a href="https://example.com" rel="noopener noreferrer">next</a>"#)
        );
        assert!(result.contains(r#"<a rel="noopener noreferrer">bare</a>"#));
    }
}
