use text_splitter::MarkdownSplitter;

/// Target chunk size in characters for document splitting.
const CHUNK_SIZE: usize = 512;

/// A chunk produced by splitting a Markdown document.
#[derive(Debug, Clone)]
pub struct SplitChunk {
    /// The raw text of this chunk (used as display text for injection into prompts).
    pub text: String,
    /// Heading hierarchy above this chunk (e.g. `["Architecture", "Storage Layer"]`).
    /// Populated by the two-pass heading-aware splitter; empty when not yet computed.
    pub section_path: Vec<String>,
    /// URL-safe anchor derived from the deepest heading (e.g. `"storage-layer"`).
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

/// Split a Markdown document into semantically meaningful chunks.
///
/// Returns a `Vec<SplitChunk>` with byte/char offsets. The `section_path` and
/// `section_anchor` fields are empty at this stage; they are populated by the
/// two-pass heading-aware splitter (see the `split_document_headed` function).
pub fn split_document(content: &str) -> Vec<SplitChunk> {
    if content.is_empty() {
        return Vec::new();
    }
    let splitter = MarkdownSplitter::new(CHUNK_SIZE);
    splitter
        .chunk_indices(content)
        .filter(|(_, text)| !text.trim().is_empty())
        .enumerate()
        .map(|(idx, (byte_offset, text))| {
            let char_offset = content[..byte_offset].chars().count();
            SplitChunk {
                text: text.to_string(),
                section_path: Vec::new(),
                section_anchor: String::new(),
                byte_offset,
                char_offset,
                chunk_index: idx as u32,
            }
        })
        .collect()
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
    fn byte_offsets_point_to_chunk_start() {
        let paragraph = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        let content = paragraph.repeat(50);
        let chunks = split_document(&content);
        for chunk in &chunks {
            assert!(content[chunk.byte_offset..].starts_with(chunk.text.trim_start()));
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
