use text_splitter::MarkdownSplitter;

/// Target chunk size in characters for document splitting.
const CHUNK_SIZE: usize = 512;
/// Minimum section size in characters; sections smaller than this are merged forward
/// into the next section to avoid producing tiny retrieval units.
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
    if rest.starts_with(' ') {
        Some((hashes as u8, rest[1..].trim_end()))
    } else {
        None
    }
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

// ── Public API ────────────────────────────────────────────────────────────────

/// Split a Markdown document into semantically meaningful chunks.
///
/// Two-pass approach:
/// 1. Split by H1/H2 headings into sections; merge sections smaller than
///    `MIN_SECTION_CHARS` forward into the next section.
/// 2. Apply `MarkdownSplitter` to each section that still exceeds `CHUNK_SIZE`.
///
/// Each chunk carries `section_path` and `section_anchor` derived from the
/// heading hierarchy, enabling section-level metadata in retrieval results.
pub fn split_document(content: &str) -> Vec<SplitChunk> {
    if content.is_empty() {
        return Vec::new();
    }

    let sections = split_into_sections(content);
    let sections = merge_small_sections(sections, MIN_SECTION_CHARS);

    let splitter = MarkdownSplitter::new(CHUNK_SIZE);
    let mut chunks: Vec<SplitChunk> = Vec::new();

    for section in sections {
        let section_anchor = section
            .heading_path
            .iter()
            .map(|h| anchor_from_heading(h))
            .collect::<Vec<_>>()
            .join("-");

        if section.text.len() <= CHUNK_SIZE {
            if !section.text.trim().is_empty() {
                let char_offset = content[..section.byte_offset].chars().count();
                chunks.push(SplitChunk {
                    text: section.text,
                    section_path: section.heading_path,
                    section_anchor,
                    byte_offset: section.byte_offset,
                    char_offset,
                    chunk_index: chunks.len() as u32,
                });
            }
        } else {
            for (rel_offset, text) in splitter.chunk_indices(&section.text) {
                if text.trim().is_empty() {
                    continue;
                }
                let abs_byte_offset = section.byte_offset + rel_offset;
                let char_offset = content[..abs_byte_offset].chars().count();
                chunks.push(SplitChunk {
                    text: text.to_string(),
                    section_path: section.heading_path.clone(),
                    section_anchor: section_anchor.clone(),
                    byte_offset: abs_byte_offset,
                    char_offset,
                    chunk_index: chunks.len() as u32,
                });
            }
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_content_returns_no_chunks() {
        assert!(split_document("").is_empty());
    }

    #[test]
    fn short_content_returns_single_chunk() {
        let chunks = split_document("Hello world");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Hello world");
        assert_eq!(chunks[0].byte_offset, 0);
        assert_eq!(chunks[0].char_offset, 0);
        assert_eq!(chunks[0].chunk_index, 0);
    }

    #[test]
    fn long_content_is_split_into_multiple_chunks() {
        let paragraph = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        let content = paragraph.repeat(50); // ~2850 chars
        let chunks = split_document(&content);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            assert!(
                chunk.text.len() <= CHUNK_SIZE + 100,
                "chunk too large: {} chars",
                chunk.text.len()
            );
        }
    }

    #[test]
    fn chunk_indices_are_sequential() {
        let paragraph = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        let content = paragraph.repeat(50);
        let chunks = split_document(&content);
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
        let chunks = split_document(&content);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn section_path_set_for_h1() {
        let content = format!("# Architecture\n\n{}", "Content. ".repeat(20));
        let chunks = split_document(&content);
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
        let chunks = split_document(&content);
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
        let chunks = split_document(content);
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
        let chunks = split_document(&content);
        for chunk in &chunks {
            assert_eq!(
                chunk.section_path,
                vec!["Real Heading"],
                "heading inside code block should not split sections"
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
