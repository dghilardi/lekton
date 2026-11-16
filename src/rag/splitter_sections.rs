use crate::rendering::markdown::heading_id;

/// Minimum section size in characters; sections smaller than this are merged forward
/// into the next section to avoid producing tiny retrieval units.
/// 128 chars is about 32 cl100k_base tokens, a conservative floor that prevents
/// empty chunks.
pub(in crate::rag) const MIN_SECTION_CHARS: usize = 128;

pub(in crate::rag) struct RawSection {
    pub(in crate::rag) byte_offset: usize,
    pub(in crate::rag) heading_path: Vec<String>,
    /// Page id of the deepest heading (see [`heading_id`]); empty before the
    /// first heading.
    pub(in crate::rag) anchor: String,
    pub(in crate::rag) text: String,
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
/// Lines inside code blocks (e.g. `# comment` in a shell fence) are ignored.
pub(in crate::rag) fn split_into_sections(content: &str) -> Vec<RawSection> {
    let code_blocks = super::splitter_blocks::code_block_ranges(content);
    let mut sections: Vec<RawSection> = Vec::new();
    let mut current_byte_offset = 0usize;
    let mut current_text = String::new();
    let mut current_h1: Option<String> = None;
    let mut current_heading_path: Vec<String> = Vec::new();
    let mut current_anchor = String::new();
    let mut line_byte_offset = 0usize;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let in_code_block = code_blocks
            .iter()
            .any(|block| block.contains(&line_byte_offset));

        if !in_code_block {
            if let Some((level, heading_text)) = parse_heading(trimmed) {
                if !current_text.trim().is_empty() || !sections.is_empty() {
                    sections.push(RawSection {
                        byte_offset: current_byte_offset,
                        heading_path: current_heading_path.clone(),
                        anchor: current_anchor.clone(),
                        text: std::mem::take(&mut current_text),
                    });
                }
                current_byte_offset = line_byte_offset;
                current_text = line.to_string();
                current_anchor = heading_id(trimmed);
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
            anchor: current_anchor,
            text: current_text,
        });
    }

    sections
}

/// Merge consecutive sections whose accumulated text is below `min_chars`.
///
/// Small sections are carried forward: their text is prepended to the next
/// section, which contributes its own `heading_path` to the merged result.
/// This preserves the most specific heading metadata available. Any leftover
/// carry at the end is either appended to the last result section or flushed as
/// a standalone chunk.
pub(in crate::rag) fn merge_small_sections(
    sections: Vec<RawSection>,
    min_chars: usize,
) -> Vec<RawSection> {
    let mut result: Vec<RawSection> = Vec::new();
    let mut carry: Option<RawSection> = None;
    let mut byte_offset_override: Option<usize> = None;

    for section in sections {
        let text = match carry.take() {
            Some(c) => {
                byte_offset_override = Some(c.byte_offset);
                format!("{}\n{}", c.text, section.text)
            }
            None => section.text,
        };
        let merged = RawSection {
            byte_offset: byte_offset_override.take().unwrap_or(section.byte_offset),
            heading_path: section.heading_path,
            anchor: section.anchor,
            text,
        };

        if merged.text.len() < min_chars {
            carry = Some(merged);
        } else {
            result.push(merged);
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
