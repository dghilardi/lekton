use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag};
use text_splitter::{ChunkConfig, MarkdownSplitter};
use tiktoken_rs::{cl100k_base, CoreBPE};

/// Minimum section size in characters; sections smaller than this are merged forward
/// into the next section to avoid producing tiny retrieval units.
/// 128 chars ≈ 32 cl100k_base tokens, a conservative floor that prevents empty chunks.
const MIN_SECTION_CHARS: usize = 128;

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

// ── Internal section type ─────────────────────────────────────────────────────

struct RawSection {
    byte_offset: usize,
    heading_path: Vec<String>,
    text: String,
}

/// Detect an H1 or H2 heading line. Returns `(level, heading_text)` or `None`.
fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim();
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 2 {
        return None;
    }
    let rest = &trimmed[hashes..];
    rest.strip_prefix(' ')
        .map(|stripped| (hashes as u8, stripped.trim_end()))
}

/// Split a Markdown document by H1/H2 headings into raw sections.
/// Headings inside fenced code blocks are ignored.
fn split_into_sections(content: &str) -> Vec<RawSection> {
    let mut sections: Vec<RawSection> = Vec::new();
    let mut current_byte_offset = 0usize;
    let mut current_text = String::new();
    let mut current_h1: Option<String> = None;
    let mut current_heading_path: Vec<String> = Vec::new();
    let mut in_code_block = false;
    let mut line_byte_offset = 0usize;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
        }

        if !in_code_block {
            if let Some((level, heading_text)) = parse_heading(trimmed) {
                if !current_text.trim().is_empty() || !sections.is_empty() {
                    sections.push(RawSection {
                        byte_offset: current_byte_offset,
                        heading_path: current_heading_path.clone(),
                        text: std::mem::take(&mut current_text),
                    });
                }
                current_byte_offset = line_byte_offset;
                current_text = line.to_string();
                if level == 1 {
                    current_h1 = Some(heading_text.to_string());
                    current_heading_path = vec![heading_text.to_string()];
                } else {
                    current_heading_path = if let Some(ref h1) = current_h1 {
                        vec![h1.clone(), heading_text.to_string()]
                    } else {
                        vec![heading_text.to_string()]
                    };
                }
                line_byte_offset += line.len();
                continue;
            }
        }

        current_text.push_str(line);
        line_byte_offset += line.len();
    }

    if !current_text.trim().is_empty() {
        sections.push(RawSection {
            byte_offset: current_byte_offset,
            heading_path: current_heading_path,
            text: current_text,
        });
    }

    sections
}

/// Merge consecutive sections whose accumulated text is below `min_chars`.
///
/// Small sections are carried forward: their text is prepended to the next
/// section, which contributes its own `heading_path` to the merged result.
/// This preserves the most specific (deepest) heading metadata available.
/// Any leftover carry at the end is either appended to the last result section
/// or flushed as a standalone chunk.
fn merge_small_sections(sections: Vec<RawSection>, min_chars: usize) -> Vec<RawSection> {
    let mut result: Vec<RawSection> = Vec::new();
    let mut carry: Option<RawSection> = None;

    for section in sections {
        let (byte_offset, text, heading_path) = if let Some(c) = carry.take() {
            (
                c.byte_offset,
                format!("{}\n{}", c.text, section.text),
                section.heading_path,
            )
        } else {
            (section.byte_offset, section.text, section.heading_path)
        };

        if text.len() < min_chars {
            carry = Some(RawSection {
                byte_offset,
                heading_path,
                text,
            });
        } else {
            result.push(RawSection {
                byte_offset,
                heading_path,
                text,
            });
        }
    }

    if let Some(c) = carry {
        if let Some(last) = result.last_mut() {
            last.text.push('\n');
            last.text.push_str(&c.text);
        } else {
            result.push(c);
        }
    }

    result
}

// ── Code-fence / table atomicity ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ProtectedKind {
    Code,
    Table,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ProtectedBlock {
    kind: ProtectedKind,
    range: Range<usize>,
}

fn markdown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_HEADING_ATTRIBUTES
}

fn protected_blocks(text: &str) -> Vec<ProtectedBlock> {
    let mut blocks: Vec<ProtectedBlock> = Parser::new_ext(text, markdown_options())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::CodeBlock(_)) => Some(ProtectedBlock {
                kind: ProtectedKind::Code,
                range,
            }),
            Event::Start(Tag::Table(_)) => Some(ProtectedBlock {
                kind: ProtectedKind::Table,
                range,
            }),
            _ => None,
        })
        .collect();
    blocks.sort_by_key(|block| block.range.start);
    blocks.dedup();
    blocks
}

/// Return byte ranges within `text` that must not be split: code blocks
/// and Markdown tables recognized by the same GFM parser used for rendering.
fn protected_ranges(text: &str) -> Vec<(usize, usize)> {
    protected_blocks(text)
        .into_iter()
        .map(|block| (block.range.start, block.range.end))
        .collect()
}

fn table_ranges(text: &str) -> Vec<Range<usize>> {
    protected_blocks(text)
        .into_iter()
        .filter_map(|block| (block.kind == ProtectedKind::Table).then_some(block.range))
        .collect()
}

/// Merge consecutive chunks whose split boundary falls inside a protected range.
///
/// A chunk whose start offset is strictly inside a range `(s, e)` — meaning
/// the previous chunk contains the opening of the block — is concatenated onto
/// the previous chunk instead of being emitted separately. This produces an
/// oversize but semantically intact chunk when a code fence or table exceeds
/// `CHUNK_SIZE`.
fn merge_broken_blocks(
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

struct TableLine<'a> {
    offset: usize,
    text: &'a str,
}

fn table_lines(table: &str) -> Vec<TableLine<'_>> {
    let mut offset = 0usize;
    table
        .split_inclusive('\n')
        .map(|line| {
            let current = offset;
            offset += line.len();
            TableLine {
                offset: current,
                text: line,
            }
        })
        .collect()
}

fn token_count(tokenizer: &CoreBPE, text: &str) -> usize {
    tokenizer.encode_ordinary(text).len()
}

fn split_table_block(
    table: &str,
    base_offset: usize,
    chunk_size_tokens: usize,
    tokenizer: &CoreBPE,
) -> Vec<(usize, String)> {
    if token_count(tokenizer, table) <= chunk_size_tokens {
        return vec![(base_offset, table.to_string())];
    }

    let lines = table_lines(table);
    if lines.len() <= 2 {
        return vec![(base_offset, table.to_string())];
    }

    let header = format!("{}{}", lines[0].text, lines[1].text);
    let mut chunks: Vec<(usize, String)> = Vec::new();
    let mut current = String::new();
    let mut current_start: Option<usize> = None;

    for row in &lines[2..] {
        let row_offset = base_offset + row.offset;
        if current_start.is_none() {
            current = header.clone();
            current.push_str(row.text);
            current_start = Some(row_offset);
            if token_count(tokenizer, &current) > chunk_size_tokens {
                chunks.push((row_offset, std::mem::take(&mut current)));
                current_start = None;
            }
            continue;
        }

        let mut candidate = current.clone();
        candidate.push_str(row.text);
        if token_count(tokenizer, &candidate) <= chunk_size_tokens {
            current = candidate;
        } else {
            chunks.push((
                current_start.expect("table row chunk must have a start"),
                current,
            ));
            current = header.clone();
            current.push_str(row.text);
            current_start = Some(row_offset);
            if token_count(tokenizer, &current) > chunk_size_tokens {
                chunks.push((row_offset, std::mem::take(&mut current)));
                current_start = None;
            }
        }
    }

    if let Some(start) = current_start {
        chunks.push((start, current));
    }

    chunks
}

// ── Public API ────────────────────────────────────────────────────────────────

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

        let mut safe: Vec<(usize, String)> = Vec::new();
        let mut cursor = 0usize;
        for table_range in table_ranges(&section.text) {
            if cursor < table_range.start {
                safe.extend(split_regular(
                    &section.text[cursor..table_range.start],
                    cursor,
                ));
            }
            safe.extend(split_table_block(
                &section.text[table_range.clone()],
                table_range.start,
                chunk_size_tokens,
                &tokenizer,
            ));
            cursor = table_range.end;
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
        let blocks = protected_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, ProtectedKind::Code);
        assert_eq!(
            &text[blocks[0].range.clone()],
            "```\n| A | B |\n| --- | --- |\n| 1 | 2 |\n```"
        );
        assert!(table_ranges(text).is_empty());
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
    fn protected_ranges_detects_fence_and_table() {
        let text = "Before\n```\ncode\n```\nMiddle\n| A | B |\n| - | - |\n| 1 | 2 |\nAfter\n";
        let ranges = protected_ranges(text);
        assert_eq!(ranges.len(), 2);
        // Fence starts at "```\n" offset (7) and ends after closing "```\n" (20)
        let fence_range = ranges[0];
        assert!(
            text[fence_range.0..fence_range.1].starts_with("```"),
            "first range should be the fence"
        );
        // Table range
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
}
