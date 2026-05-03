use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(in crate::rag) enum MarkdownBlockKind {
    Code,
    Mermaid,
    Table,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(in crate::rag) struct MarkdownBlock {
    pub(in crate::rag) kind: MarkdownBlockKind,
    pub(in crate::rag) range: Range<usize>,
}

fn markdown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_HEADING_ATTRIBUTES
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(in crate::rag) enum MermaidDiagramType {
    Flowchart,
    Graph,
    SequenceDiagram,
    ClassDiagram,
    StateDiagram,
    ErDiagram,
    Journey,
    Gantt,
    Pie,
    QuadrantChart,
    RequirementDiagram,
    GitGraph,
    C4,
    Mindmap,
    Timeline,
    Zenuml,
    Sankey,
    XyChart,
    Block,
    Packet,
    Kanban,
    Architecture,
    Radar,
    Treemap,
    Venn,
    Ishikawa,
    TreeView,
    Unknown(String),
}

impl MermaidDiagramType {
    pub(in crate::rag) fn from_declaration(declaration: &str) -> Self {
        let token = declaration
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim()
            .trim_end_matches(';');
        let normalized = token.to_ascii_lowercase();
        let normalized = normalized.trim_end_matches("-beta");
        match normalized {
            "flowchart" => Self::Flowchart,
            "graph" => Self::Graph,
            "sequencediagram" => Self::SequenceDiagram,
            "classdiagram" => Self::ClassDiagram,
            "statediagram" | "statediagram-v2" => Self::StateDiagram,
            "erdiagram" => Self::ErDiagram,
            "journey" => Self::Journey,
            "gantt" => Self::Gantt,
            "pie" => Self::Pie,
            "quadrantchart" => Self::QuadrantChart,
            "requirementdiagram" => Self::RequirementDiagram,
            "gitgraph" => Self::GitGraph,
            token if token.starts_with("c4") => Self::C4,
            "mindmap" => Self::Mindmap,
            "timeline" => Self::Timeline,
            "zenuml" => Self::Zenuml,
            "sankey" => Self::Sankey,
            "xychart" => Self::XyChart,
            "block" => Self::Block,
            "packet" => Self::Packet,
            "kanban" => Self::Kanban,
            "architecture" => Self::Architecture,
            "radar" => Self::Radar,
            "treemap" => Self::Treemap,
            "venn" => Self::Venn,
            "ishikawa" => Self::Ishikawa,
            "treeview" => Self::TreeView,
            _ => Self::Unknown(normalized.to_string()),
        }
    }
}

fn is_mermaid_info_string(info: &str) -> bool {
    info.split_whitespace()
        .next()
        .is_some_and(|lang| lang.eq_ignore_ascii_case("mermaid"))
}

#[cfg(test)]
fn is_directive_or_comment(line: &str) -> bool {
    line.trim().starts_with("%%")
}

pub(in crate::rag) fn mermaid_source_from_fence(block: &str) -> Option<&str> {
    let first_line_end = block.find('\n')?;
    let first_line = block[..first_line_end].trim();
    if !first_line.starts_with("```") || !is_mermaid_info_string(first_line.trim_start_matches('`'))
    {
        return None;
    }

    let source = &block[first_line_end + 1..];
    source
        .rsplit_once("\n```")
        .map(|(source, _)| source)
        .or_else(|| source.strip_suffix("```"))
}

#[cfg(test)]
pub(in crate::rag) fn mermaid_diagram_type(block: &str) -> Option<MermaidDiagramType> {
    let source = mermaid_source_from_fence(block)?;
    let mut in_frontmatter = false;
    let mut frontmatter_seen = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_directive_or_comment(trimmed) {
            continue;
        }
        if trimmed == "---" && !frontmatter_seen {
            in_frontmatter = true;
            frontmatter_seen = true;
            continue;
        }
        if trimmed == "---" && in_frontmatter {
            in_frontmatter = false;
            continue;
        }
        if in_frontmatter {
            continue;
        }
        return Some(MermaidDiagramType::from_declaration(trimmed));
    }

    None
}

pub(in crate::rag) fn markdown_blocks(text: &str) -> Vec<MarkdownBlock> {
    let mut blocks: Vec<MarkdownBlock> = Parser::new_ext(text, markdown_options())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if is_mermaid_info_string(&info) =>
            {
                Some(MarkdownBlock {
                    kind: MarkdownBlockKind::Mermaid,
                    range,
                })
            }
            Event::Start(Tag::CodeBlock(_)) => Some(MarkdownBlock {
                kind: MarkdownBlockKind::Code,
                range,
            }),
            Event::Start(Tag::Table(_)) => Some(MarkdownBlock {
                kind: MarkdownBlockKind::Table,
                range,
            }),
            _ => None,
        })
        .collect();
    blocks.sort_by_key(|block| block.range.start);
    blocks.dedup();
    blocks
}

/// Return byte ranges within `text` that must not be split: code blocks and
/// Markdown tables recognized by the same GFM parser used for rendering.
pub(in crate::rag) fn protected_ranges(text: &str) -> Vec<(usize, usize)> {
    markdown_blocks(text)
        .into_iter()
        .map(|block| (block.range.start, block.range.end))
        .collect()
}

#[cfg(test)]
pub(in crate::rag) fn table_ranges(text: &str) -> Vec<Range<usize>> {
    markdown_blocks(text)
        .into_iter()
        .filter_map(|block| (block.kind == MarkdownBlockKind::Table).then_some(block.range))
        .collect()
}

/// Merge consecutive chunks whose split boundary falls inside a protected range.
///
/// A chunk whose start offset is strictly inside a range `(s, e)` is
/// concatenated onto the previous chunk instead of being emitted separately.
/// This produces an oversize but semantically intact chunk when a code fence or
/// table exceeds `CHUNK_SIZE`.
pub(in crate::rag) fn merge_broken_blocks(
    raw: Vec<(usize, String)>,
    protected: &[(usize, usize)],
) -> Vec<(usize, String)> {
    let mut result: Vec<(usize, String)> = Vec::new();
    for (offset, text) in raw {
        let inside = protected.iter().any(|&(s, e)| s < offset && offset < e);
        if inside {
            if let Some((_, last_text)) = result.last_mut() {
                last_text.push_str(&text);
                continue;
            }
        }
        result.push((offset, text));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_ranges_detects_fence_and_table() {
        let text = "Before\n```\ncode\n```\nMiddle\n| A | B |\n| - | - |\n| 1 | 2 |\nAfter\n";
        let ranges = protected_ranges(text);
        assert_eq!(ranges.len(), 2);

        let fence_range = ranges[0];
        assert!(
            text[fence_range.0..fence_range.1].starts_with("```"),
            "first range should be the fence"
        );

        let table_range = ranges[1];
        assert!(
            text[table_range.0..table_range.1]
                .trim_start()
                .starts_with('|'),
            "second range should be the table"
        );
    }

    #[test]
    fn protected_ranges_detects_gfm_table_without_outer_pipes() {
        let text = "Before\n\nColumn A | Column B\n--- | ---\nvalue | data\n\nAfter\n";
        let ranges = protected_ranges(text);
        assert_eq!(ranges.len(), 1);
        let table = &text[ranges[0].0..ranges[0].1];
        assert_eq!(table, "Column A | Column B\n--- | ---\nvalue | data\n");
    }

    #[test]
    fn protected_ranges_ignores_invalid_table_delimiter() {
        let text = "Column A | Column B\nnot a delimiter\nvalue | data\n";
        let ranges = protected_ranges(text);
        assert!(ranges.is_empty());
    }

    #[test]
    fn table_range_stops_before_blank_line_or_block_element() {
        let blank = "A | B\n--- | ---\n1 | 2\n\nNot table\n";
        let blank_ranges = protected_ranges(blank);
        assert_eq!(blank_ranges.len(), 1);
        assert_eq!(
            &blank[blank_ranges[0].0..blank_ranges[0].1],
            "A | B\n--- | ---\n1 | 2\n"
        );

        let block = "A | B\n--- | ---\n1 | 2\n> quote\n";
        let block_ranges = protected_ranges(block);
        assert_eq!(block_ranges.len(), 1);
        assert_eq!(
            &block[block_ranges[0].0..block_ranges[0].1],
            "A | B\n--- | ---\n1 | 2\n"
        );
    }

    #[test]
    fn table_range_stops_before_following_heading() {
        let text = "A | B\n--- | ---\n1 | 2\n# Next section\n";
        let ranges = protected_ranges(text);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&text[ranges[0].0..ranges[0].1], "A | B\n--- | ---\n1 | 2\n");
    }

    #[test]
    fn table_inside_code_fence_is_not_detected_as_table() {
        let text = "```\n| A | B |\n| --- | --- |\n| 1 | 2 |\n```\n";
        let blocks = markdown_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, MarkdownBlockKind::Code);
        assert_eq!(
            &text[blocks[0].range.clone()],
            "```\n| A | B |\n| --- | --- |\n| 1 | 2 |\n```"
        );
        assert!(table_ranges(text).is_empty());
    }

    #[test]
    fn mermaid_code_fence_is_detected_as_mermaid_block() {
        let text = "Before\n\n```mermaid\nflowchart TD\nA --> B\n```\n\nAfter\n";
        let blocks = markdown_blocks(text);
        let mermaid = blocks
            .iter()
            .find(|block| block.kind == MarkdownBlockKind::Mermaid)
            .expect("expected a mermaid block");
        assert_eq!(
            mermaid_diagram_type(&text[mermaid.range.clone()]),
            Some(MermaidDiagramType::Flowchart)
        );
    }

    #[test]
    fn non_mermaid_code_fence_stays_code_block() {
        let text = "```rust\nfn main() {}\n```\n";
        let blocks = markdown_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, MarkdownBlockKind::Code);
        assert_eq!(mermaid_diagram_type(&text[blocks[0].range.clone()]), None);
    }

    #[test]
    fn mermaid_diagram_type_skips_directives_comments_and_frontmatter() {
        let text = "```mermaid\n---\ntitle: Demo\n---\n%%{init: {\"theme\": \"base\"}}%%\n%% comment\nsequenceDiagram\nA->>B: hello\n```\n";
        let blocks = markdown_blocks(text);
        assert_eq!(blocks[0].kind, MarkdownBlockKind::Mermaid);
        assert_eq!(
            mermaid_diagram_type(&text[blocks[0].range.clone()]),
            Some(MermaidDiagramType::SequenceDiagram)
        );
    }

    #[test]
    fn mermaid_diagram_type_falls_back_to_unknown() {
        let text = "```mermaid\nexperimentalDiagram\nA --> B\n```\n";
        let blocks = markdown_blocks(text);
        assert_eq!(
            mermaid_diagram_type(&text[blocks[0].range.clone()]),
            Some(MermaidDiagramType::Unknown("experimentaldiagram".into()))
        );
    }
}
