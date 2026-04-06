use text_splitter::MarkdownSplitter;

/// Target chunk size in characters for document splitting.
const CHUNK_SIZE: usize = 512;

/// Split a Markdown document into semantically meaningful chunks.
///
/// Uses `text-splitter`'s `MarkdownSplitter` which respects Markdown
/// structure (headings, paragraphs, code blocks) when choosing split
/// boundaries.  Each chunk will be at most [`CHUNK_SIZE`] characters.
pub fn split_document(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let splitter = MarkdownSplitter::new(CHUNK_SIZE);
    splitter
        .chunks(content)
        .map(String::from)
        .filter(|c| !c.trim().is_empty())
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
        assert_eq!(chunks[0], "Hello world");
    }

    #[test]
    fn long_content_is_split_into_multiple_chunks() {
        // Generate content well above CHUNK_SIZE
        let paragraph = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        let content = paragraph.repeat(50); // ~2850 chars
        let chunks = split_document(&content);
        assert!(chunks.len() > 1, "expected multiple chunks, got {}", chunks.len());
        for chunk in &chunks {
            assert!(
                chunk.len() <= CHUNK_SIZE + 100, // small tolerance for word boundaries
                "chunk too large: {} chars",
                chunk.len()
            );
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
}
