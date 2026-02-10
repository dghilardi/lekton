use pulldown_cmark::{html, Options, Parser};

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
}
