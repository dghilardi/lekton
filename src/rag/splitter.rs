use super::splitter_blocks::{
    markdown_blocks, merge_broken_blocks, protected_ranges, MarkdownBlockKind,
};
use super::splitter_sections::{merge_small_sections, split_into_sections, MIN_SECTION_CHARS};
use super::splitter_table::split_table_block;
use text_splitter::{ChunkConfig, MarkdownSplitter};
use tiktoken_rs::cl100k_base;

/// A chunk produced by splitting a Markdown document.
#[derive(Debug, Clone)]
pub struct SplitChunk {
    /// The raw text of this chunk (used as display text for injection into prompts).
    pub text: String,
    /// Heading hierarchy above this chunk (e.g. `["Architecture", "Storage Layer"]`).
    pub section_path: Vec<String>,
    /// URL-safe anchor derived from the full heading path joined with `-`.
    /// e.g. `"architecture-storage-layer"` for `["Architecture", "Storage Layer"]`.
    pub section_anchor: String,
    /// Byte offset of this chunk's start in the original document.
    pub byte_offset: usize,
    /// Character offset of this chunk's start in the original document.
    pub char_offset: usize,
    /// Zero-based index of this chunk within the document.
    pub chunk_index: u32,
}

/// Convert a heading string into a URL-safe anchor slug.
/// `"Storage Layer"` → `"storage-layer"`
pub fn anchor_from_heading(heading: &str) -> String {
    let raw: String = heading
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_lowercase().next().unwrap())
            } else if c == ' ' || c == '-' {
                Some('-')
            } else {
                None
            }
        })
        .collect();
    raw.trim_matches('-').to_string()
}

// Public API

/// Split a Markdown document into semantically meaningful chunks.
///
/// Two-pass approach:
/// 1. Split by H1/H2 headings into sections; merge sections smaller than
///    `MIN_SECTION_CHARS` forward into the next section.
/// 2. Apply a token-aware `MarkdownSplitter` (cl100k_base) with overlap to each section.
///
/// `chunk_size_tokens` and `chunk_overlap_tokens` come from `RagConfig` and are forwarded
/// from `DefaultRagService`. Each chunk carries `section_path` and `section_anchor`
/// derived from the heading hierarchy, enabling section-level metadata in retrieval.
pub fn split_document(
    content: &str,
    chunk_size_tokens: usize,
    chunk_overlap_tokens: usize,
) -> Vec<SplitChunk> {
    if content.is_empty() {
        return Vec::new();
    }

    let sections = split_into_sections(content);
    let sections = merge_small_sections(sections, MIN_SECTION_CHARS);

    let tokenizer = cl100k_base().expect("cl100k_base tokenizer should always load");
    let splitter = MarkdownSplitter::new(
        ChunkConfig::new(chunk_size_tokens)
            .with_sizer(tokenizer.clone())
            .with_overlap(chunk_overlap_tokens)
            .expect("chunk_overlap_tokens must be less than chunk_size_tokens"),
    );
    let mut chunks: Vec<SplitChunk> = Vec::new();

    for section in sections {
        let section_anchor = section
            .heading_path
            .iter()
            .map(|h| anchor_from_heading(h))
            .collect::<Vec<_>>()
            .join("-");

        let split_regular = |segment: &str, base_offset: usize| -> Vec<(usize, String)> {
            let protected = protected_ranges(segment);
            let raw: Vec<(usize, String)> = splitter
                .chunk_indices(segment)
                .filter(|(_, t)| !t.trim().is_empty())
                .map(|(off, t)| (off, t.to_string()))
                .collect();
            merge_broken_blocks(raw, &protected)
                .into_iter()
                .map(|(off, text)| (base_offset + off, text))
                .collect()
        };

        let special_blocks = markdown_blocks(&section.text).into_iter().filter(|block| {
            matches!(
                block.kind,
                MarkdownBlockKind::Table | MarkdownBlockKind::Mermaid
            )
        });
        let mut safe: Vec<(usize, String)> = Vec::new();
        let mut cursor = 0usize;
        for block in special_blocks {
            if cursor < block.range.start {
                safe.extend(split_regular(
                    &section.text[cursor..block.range.start],
                    cursor,
                ));
            }
            match block.kind {
                MarkdownBlockKind::Table => safe.extend(split_table_block(
                    &section.text[block.range.clone()],
                    block.range.start,
                    chunk_size_tokens,
                    &tokenizer,
                )),
                MarkdownBlockKind::Mermaid => {
                    safe.push((
                        block.range.start,
                        section.text[block.range.clone()].to_string(),
                    ));
                }
                MarkdownBlockKind::Code => {}
            }
            cursor = block.range.end;
        }
        if cursor < section.text.len() {
            safe.extend(split_regular(&section.text[cursor..], cursor));
        }

        for (rel_offset, text) in safe {
            let abs_byte_offset = section.byte_offset + rel_offset;
            // Compute char_offset from two known-safe slices instead of
            // content[..abs_byte_offset]: merged sections add a '\n' separator
            // that is not in content, so abs_byte_offset may not land on a
            // char boundary in content (panic for multi-byte chars like '┌').
            let char_offset = content[..section.byte_offset].chars().count()
                + section.text[..rel_offset].chars().count();
            chunks.push(SplitChunk {
                text,
                section_path: section.heading_path.clone(),
                section_anchor: section_anchor.clone(),
                byte_offset: abs_byte_offset,
                char_offset,
                chunk_index: chunks.len() as u32,
            });
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKENS: usize = 256;
    const OVERLAP: usize = 64;

    fn table_chunks<'a>(chunks: &'a [SplitChunk], header: &str) -> Vec<&'a SplitChunk> {
        chunks
            .iter()
            .filter(|chunk| chunk.text.contains(header))
            .collect()
    }

    fn assert_each_table_chunk_has_header(chunks: &[&SplitChunk], header: &str, delimiter: &str) {
        assert!(!chunks.is_empty(), "expected at least one table chunk");
        for chunk in chunks {
            assert!(
                chunk.text.contains(header),
                "table chunk is missing header: {}",
                chunk.text
            );
            assert!(
                chunk.text.contains(delimiter),
                "table chunk is missing delimiter: {}",
                chunk.text
            );
        }
    }

    fn assert_rows_once_in_order(chunks: &[&SplitChunk], rows: &[String]) {
        let combined = chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut cursor = 0usize;
        for row in rows {
            assert_eq!(
                combined.matches(row).count(),
                1,
                "row should appear exactly once: {row}"
            );
            let position = combined[cursor..]
                .find(row)
                .unwrap_or_else(|| panic!("row appears out of order or is missing: {row}"));
            cursor += position + row.len();
        }
    }

    #[test]
    fn empty_content_returns_no_chunks() {
        assert!(split_document("", TOKENS, OVERLAP).is_empty());
    }

    #[test]
    fn short_content_returns_single_chunk() {
        let chunks = split_document("Hello world", TOKENS, OVERLAP);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Hello world");
        assert_eq!(chunks[0].byte_offset, 0);
        assert_eq!(chunks[0].char_offset, 0);
        assert_eq!(chunks[0].chunk_index, 0);
    }

    #[test]
    fn long_content_is_split_into_multiple_chunks() {
        let paragraph = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        let content = paragraph.repeat(50); // ~2850 chars ≈ 712 tokens
        let chunks = split_document(&content, TOKENS, OVERLAP);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
    }

    #[test]
    fn chunk_indices_are_sequential() {
        let paragraph = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        let content = paragraph.repeat(50);
        let chunks = split_document(&content, TOKENS, OVERLAP);
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i as u32);
        }
    }

    #[test]
    fn markdown_structure_is_respected() {
        let content = format!(
            "# Heading 1\n\n{}\n\n# Heading 2\n\n{}",
            "First section content. ".repeat(20),
            "Second section content. ".repeat(20),
        );
        let chunks = split_document(&content, TOKENS, OVERLAP);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn section_path_set_for_h1() {
        let content = format!("# Architecture\n\n{}", "Content. ".repeat(20));
        let chunks = split_document(&content, TOKENS, OVERLAP);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert_eq!(chunk.section_path, vec!["Architecture"]);
            assert_eq!(chunk.section_anchor, "architecture");
        }
    }

    #[test]
    fn section_path_set_for_h2_under_h1() {
        let body = "Storage details. ".repeat(15);
        let content = format!(
            "# Architecture\n\n{}\n\n## Storage Layer\n\n{}",
            "Intro. ".repeat(15),
            body
        );
        let chunks = split_document(&content, TOKENS, OVERLAP);
        let storage_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.section_path.len() == 2)
            .collect();
        assert!(
            !storage_chunks.is_empty(),
            "expected at least one chunk under Storage Layer"
        );
        assert_eq!(
            storage_chunks[0].section_path,
            vec!["Architecture", "Storage Layer"]
        );
        assert_eq!(
            storage_chunks[0].section_anchor,
            "architecture-storage-layer"
        );
    }

    #[test]
    fn small_sections_are_merged() {
        let content = "# A\n\nShort.\n\n# B\n\nAlso short.\n";
        let chunks = split_document(content, TOKENS, OVERLAP);
        assert_eq!(chunks.len(), 1, "expected merged into a single chunk");
        assert!(chunks[0].text.contains("Short."));
        assert!(chunks[0].text.contains("Also short."));
    }

    #[test]
    fn headings_inside_code_blocks_are_ignored() {
        let body = "Real content. ".repeat(5);
        let content = format!(
            "# Real Heading\n\n```\n# Not a heading\n## Also not\n```\n\n{}",
            body
        );
        let chunks = split_document(&content, TOKENS, OVERLAP);
        for chunk in &chunks {
            assert_eq!(
                chunk.section_path,
                vec!["Real Heading"],
                "heading inside code block should not split sections"
            );
        }
    }

    #[test]
    fn large_code_fence_is_not_split() {
        // Code fence body >> chunk_size_tokens should be one oversize chunk.
        let fence_body = "    let x = 1;\n".repeat(250); // ~3750 chars ≈ 937 tokens
        let content = format!(
            "# Section\n\nIntro.\n\n```rust\n{}```\n\nOutro.\n",
            fence_body
        );
        let chunks = split_document(&content, TOKENS, OVERLAP);
        for chunk in &chunks {
            let open_count = chunk.text.matches("```rust").count();
            let close_count = chunk.text.split('\n').filter(|l| l.trim() == "```").count();
            assert_eq!(
                open_count, close_count,
                "fence opened and closed count must match within each chunk"
            );
        }
    }

    #[test]
    fn large_table_is_split_by_rows_with_repeated_header() {
        let header = "| Column A | Column B | Column C |";
        let delimiter = "| --- | --- | --- |";
        let rows: String = (0..60)
            .map(|i| format!("| val-{i} | data-{i} | info-{i} |\n"))
            .collect();
        let table = format!("{header}\n{delimiter}\n{rows}");
        let content = format!("# Section\n\nIntro.\n\n{}\nOutro.\n", table);
        let chunks = split_document(&content, 80, 0);
        let table_chunks = table_chunks(&chunks, header);
        assert!(
            table_chunks.len() > 1,
            "large table should be split into row-group chunks"
        );
        assert_each_table_chunk_has_header(&table_chunks, header, delimiter);
        assert!(table_chunks[0].text.contains("| val-0 |"));
        assert!(table_chunks.last().unwrap().text.contains("| val-59 |"));
        assert_eq!(
            table_chunks[0].byte_offset,
            content.find("| val-0 |").unwrap(),
            "synthetic table chunks should point at their first original data row"
        );
    }

    #[test]
    fn large_table_preserves_each_data_row_once_in_order() {
        let header = "| Row | Description |";
        let delimiter = "| --- | --- |";
        let rows: Vec<String> = (0..36)
            .map(|i| format!("| row-{i:02} | {} |\n", "detail ".repeat(5)))
            .collect();
        let table = format!("{header}\n{delimiter}\n{}", rows.concat());
        let content = format!("# Section\n\n{}\n", table);
        let chunks = split_document(&content, 70, 0);
        let table_chunks = table_chunks(&chunks, header);

        assert!(
            table_chunks.len() > 1,
            "test setup should force row-group splitting"
        );
        assert_each_table_chunk_has_header(&table_chunks, header, delimiter);
        assert_rows_once_in_order(&table_chunks, &rows);
    }

    #[test]
    fn small_table_is_isolated_from_surrounding_text() {
        let header = "| Setting | Value |";
        let delimiter = "| --- | --- |";
        let content = format!(
            "# Section\n\nIntro before.\n\n{header}\n{delimiter}\n| retries | 3 |\n\nOutro after.\n"
        );
        let chunks = split_document(&content, TOKENS, OVERLAP);
        let table_chunks = table_chunks(&chunks, header);

        assert_eq!(table_chunks.len(), 1);
        assert_each_table_chunk_has_header(&table_chunks, header, delimiter);
        assert!(table_chunks[0].text.contains("| retries | 3 |"));
        assert!(!table_chunks[0].text.contains("Intro before."));
        assert!(!table_chunks[0].text.contains("Outro after."));
    }

    #[test]
    fn small_mermaid_block_is_isolated_from_surrounding_text() {
        let content =
            "# Section\n\nIntro before.\n\n```mermaid\nflowchart TD\nA --> B\n```\n\nOutro after.\n";
        let chunks = split_document(content, TOKENS, OVERLAP);
        let mermaid_chunks = mermaid_fenced_chunks(&chunks);

        assert_eq!(mermaid_chunks.len(), 1);
        assert!(mermaid_chunks[0].text.contains("flowchart TD"));
        assert!(!mermaid_chunks[0].text.contains("Intro before."));
        assert!(!mermaid_chunks[0].text.contains("Outro after."));
        assert_mermaid_fence_balanced(mermaid_chunks[0]);
    }

    #[test]
    fn gfm_table_alignment_and_empty_cells_are_preserved() {
        let header = "Name | Left | Center | Right | Empty";
        let delimiter = ":--- | :--- | :---: | ---: | ---";
        let content = format!(
            "# Section\n\n{header}\n{delimiter}\nsvc | value | centered | right | \n\nAfter\n"
        );
        let chunks = split_document(&content, TOKENS, OVERLAP);
        let table_chunks = table_chunks(&chunks, header);

        assert_eq!(table_chunks.len(), 1);
        assert_each_table_chunk_has_header(&table_chunks, header, delimiter);
        assert!(table_chunks[0]
            .text
            .contains("svc | value | centered | right | "));
    }

    #[test]
    fn markdown_table_with_escaped_and_inline_code_pipes_stays_intact() {
        let content = "# Section\n\n| Pattern | Meaning |\n| --- | --- |\n| `a|b` | escaped \\| pipe |\n\nAfter\n";
        let chunks = split_document(content, TOKENS, OVERLAP);
        let table_chunks = table_chunks(&chunks, "| Pattern | Meaning |");
        assert_eq!(table_chunks.len(), 1);
        assert!(table_chunks[0].text.contains("`a|b`"));
        assert!(table_chunks[0].text.contains("escaped \\| pipe"));
    }

    #[test]
    fn oversized_table_row_is_not_split() {
        let long_cell = "long-cell ".repeat(120);
        let content = format!(
            "# Section\n\n| A | B |\n| --- | --- |\n| key | {} |\n",
            long_cell
        );
        let chunks = split_document(&content, 40, 0);
        let table_chunks = table_chunks(&chunks, "| A | B |");
        assert_eq!(table_chunks.len(), 1);
        assert!(table_chunks[0]
            .text
            .contains(&format!("| key | {} |", long_cell)));
        assert_eq!(
            table_chunks[0].byte_offset,
            content.find("| key |").unwrap(),
            "oversized row chunk should point at the original row"
        );
    }

    #[test]
    fn multibyte_chars_in_merged_sections_do_not_panic() {
        // Regression: architecture.md in the demo corpus has box-drawing chars
        // (┌, │, └) inside a code block. A tiny intro before the diagram gets
        // merged by merge_small_sections, adding a '\n' separator that shifts
        // MarkdownSplitter offsets away from content's char boundaries.
        // Slicing content[..abs_byte_offset] used to panic on those boundaries.
        let content = "# Overview\n\nIntro.\n\n## Diagram\n\n```\n┌──────┐\n│ test │\n└──────┘\n```\n\nMore content ".to_string()
            + &"with box ┌─┐└─┘ chars ".repeat(30);
        let chunks = split_document(&content, TOKENS, OVERLAP);
        assert!(!chunks.is_empty());
        // Verify all chunks have valid char_offset (no panic, no overflow)
        let total_chars = content.chars().count();
        for chunk in &chunks {
            assert!(
                chunk.char_offset <= total_chars,
                "char_offset {} exceeds total chars {}",
                chunk.char_offset,
                total_chars
            );
        }
    }

    #[test]
    fn anchor_from_heading_basic() {
        assert_eq!(anchor_from_heading("Storage Layer"), "storage-layer");
        assert_eq!(anchor_from_heading("Hello World"), "hello-world");
        assert_eq!(anchor_from_heading("  Trim  "), "trim");
    }

    #[test]
    fn anchor_from_heading_strips_special_chars() {
        assert_eq!(anchor_from_heading("API (v2)"), "api-v2");
        assert_eq!(anchor_from_heading("C++ Guide"), "c-guide");
    }

    #[derive(Debug, Clone, Copy)]
    enum MermaidFixtureSize {
        Small,
        Medium,
        Large,
    }

    impl MermaidFixtureSize {
        fn filler_lines(self) -> usize {
            match self {
                Self::Small => 2,
                Self::Medium => 24,
                Self::Large => 120,
            }
        }
    }

    const MERMAID_FIXTURE_TYPES: &[(&str, &str)] = &[
        ("flowchart", "flowchart TD"),
        ("graph", "graph TD"),
        ("sequenceDiagram", "sequenceDiagram"),
        ("classDiagram", "classDiagram"),
        ("stateDiagram", "stateDiagram-v2"),
        ("erDiagram", "erDiagram"),
        ("journey", "journey"),
        ("gantt", "gantt"),
        ("pie", "pie title Usage"),
        ("quadrantChart", "quadrantChart"),
        ("requirementDiagram", "requirementDiagram"),
        ("gitGraph", "gitGraph"),
        ("C4", "C4Context"),
        ("mindmap", "mindmap"),
        ("timeline", "timeline"),
        ("zenuml", "zenuml"),
        ("sankey", "sankey-beta"),
        ("xychart", "xychart-beta"),
        ("block", "block-beta"),
        ("packet", "packet-beta"),
        ("kanban", "kanban"),
        ("architecture", "architecture-beta"),
        ("radar", "radar-beta"),
        ("treemap", "treemap-beta"),
        ("venn", "venn"),
        ("ishikawa", "ishikawa"),
        ("treeView", "treeView"),
    ];

    fn mermaid_fixture(declaration: &str, size: MermaidFixtureSize) -> String {
        let mut body = String::new();
        body.push_str(declaration);
        body.push('\n');
        body.push_str("%% Mermaid splitter fixture\n");
        for i in 0..size.filler_lines() {
            body.push_str(&format!("%% fixture-line-{i}\n"));
        }
        format!("# Diagram\n\n```mermaid\n{body}```\n")
    }

    fn mermaid_fenced_chunks(chunks: &[SplitChunk]) -> Vec<&SplitChunk> {
        chunks
            .iter()
            .filter(|chunk| chunk.text.contains("```mermaid"))
            .collect()
    }

    fn assert_mermaid_fence_balanced(chunk: &SplitChunk) {
        let open_count = chunk.text.matches("```mermaid").count();
        let close_count = chunk
            .text
            .split('\n')
            .filter(|line| line.trim() == "```")
            .count();
        assert_eq!(
            open_count, close_count,
            "Mermaid fence should be balanced in chunk: {}",
            chunk.text
        );
    }

    #[test]
    fn mermaid_fixtures_are_currently_kept_atomic() {
        for (name, declaration) in MERMAID_FIXTURE_TYPES {
            for size in [
                MermaidFixtureSize::Small,
                MermaidFixtureSize::Medium,
                MermaidFixtureSize::Large,
            ] {
                let content = mermaid_fixture(declaration, size);
                let chunks = split_document(&content, 80, 0);
                let mermaid_chunks = mermaid_fenced_chunks(&chunks);

                assert_eq!(
                    mermaid_chunks.len(),
                    1,
                    "{name} {size:?} should currently produce one Mermaid chunk"
                );
                assert!(
                    mermaid_chunks[0].text.contains(declaration),
                    "{name} {size:?} chunk should contain its declaration"
                );
                assert_mermaid_fence_balanced(mermaid_chunks[0]);
            }
        }
    }
}
