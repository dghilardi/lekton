use pulldown_cmark::{html, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
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

/// Render a raw Markdown string to sanitized HTML.
///
/// Supports GitHub Flavored Markdown (GFM) features: tables,
/// footnotes, strikethrough, task lists, and smart punctuation.
pub fn render_markdown(raw: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION;

    let parser = Parser::new_ext(raw, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
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
            Event::End(TagEnd::Heading(level)) => {
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
                    if level_num >= 2 && level_num <= 3 {
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
            } else if c.is_whitespace() || c == '-' || c == '.' {
                '-'
            } else {
                '-' // Convert all other special chars to dash
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
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
        assert!(result.contains("fn main()"));
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
        assert!(result.contains("checked"));
        assert!(result.contains("type=\"checkbox\""));
    }

    #[test]
    fn test_links() {
        let result = render_markdown("[Lekton](https://example.com)");
        assert!(result.contains("<a href=\"https://example.com\">Lekton</a>"));
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
        assert!(result.contains("<h2>Installation</h2>"));
        assert!(result.contains("<strong>Lekton</strong>"));
        assert!(result.contains("<li>Fast</li>"));
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
    fn test_slugify() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("API Reference"), "api-reference");
        assert_eq!(slugify("Getting Started!"), "getting-started");
        assert_eq!(slugify("Using `cargo`"), "using-cargo");
        assert_eq!(slugify("Section 2.1"), "section-2-1");
    }
}
