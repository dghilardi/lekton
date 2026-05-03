use tiktoken_rs::CoreBPE;

use super::splitter_blocks::mermaid_source_from_fence;

#[derive(Debug, Clone)]
struct MermaidLine {
    offset: usize,
    text: String,
}

#[derive(Debug, Clone)]
struct ParsedMermaidBlock {
    preamble: Vec<MermaidLine>,
    declaration: MermaidLine,
    body: Vec<MermaidLine>,
}

fn token_count(tokenizer: &CoreBPE, text: &str) -> usize {
    tokenizer.encode_ordinary(text).len()
}

fn source_start_offset(block: &str) -> Option<usize> {
    block.find('\n').map(|idx| idx + 1)
}

fn mermaid_source_lines(block: &str) -> Option<Vec<MermaidLine>> {
    let source = mermaid_source_from_fence(block)?;
    let source_start = source_start_offset(block)?;
    let mut offset = source_start;
    Some(
        source
            .split_inclusive('\n')
            .map(|line| {
                let current = offset;
                offset += line.len();
                MermaidLine {
                    offset: current,
                    text: line.to_string(),
                }
            })
            .collect(),
    )
}

fn parse_mermaid_block(block: &str) -> Option<ParsedMermaidBlock> {
    let lines = mermaid_source_lines(block)?;
    let mut preamble = Vec::new();
    let mut body = Vec::new();
    let mut declaration: Option<MermaidLine> = None;
    let mut in_frontmatter = false;
    let mut frontmatter_seen = false;

    for line in lines {
        let trimmed = line.text.trim();
        if declaration.is_none() {
            if trimmed.is_empty() || trimmed.starts_with("%%") {
                preamble.push(line);
                continue;
            }
            if trimmed == "---" && !frontmatter_seen {
                in_frontmatter = true;
                frontmatter_seen = true;
                preamble.push(line);
                continue;
            }
            if trimmed == "---" && in_frontmatter {
                in_frontmatter = false;
                preamble.push(line);
                continue;
            }
            if in_frontmatter {
                preamble.push(line);
                continue;
            }
            declaration = Some(line);
        } else {
            body.push(line);
        }
    }

    Some(ParsedMermaidBlock {
        preamble,
        declaration: declaration?,
        body,
    })
}

fn push_line(output: &mut String, line: &str) {
    output.push_str(line);
    if !line.ends_with('\n') {
        output.push('\n');
    }
}

fn build_chunk(parsed: &ParsedMermaidBlock, body: &[MermaidLine]) -> String {
    let mut chunk = String::from("```mermaid\n");
    for line in &parsed.preamble {
        push_line(&mut chunk, &line.text);
    }
    push_line(&mut chunk, &parsed.declaration.text);
    for line in body {
        push_line(&mut chunk, &line.text);
    }
    chunk.push_str("```\n");
    chunk
}

fn grouped_body_chunks(
    parsed: &ParsedMermaidBlock,
    chunk_size_tokens: usize,
    tokenizer: &CoreBPE,
) -> Option<Vec<Vec<MermaidLine>>> {
    if parsed.body.is_empty() {
        return None;
    }

    let mut groups: Vec<Vec<MermaidLine>> = Vec::new();
    let mut current: Vec<MermaidLine> = Vec::new();

    for line in &parsed.body {
        let mut candidate = current.clone();
        candidate.push(line.clone());
        let candidate_text = build_chunk(parsed, &candidate);
        if token_count(tokenizer, &candidate_text) <= chunk_size_tokens {
            current = candidate;
            continue;
        }

        if !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        current.push(line.clone());
        let single_text = build_chunk(parsed, &current);
        if token_count(tokenizer, &single_text) > chunk_size_tokens {
            groups.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        groups.push(current);
    }

    Some(groups)
}

pub(in crate::rag) fn split_mermaid_block(
    block: &str,
    base_offset: usize,
    chunk_size_tokens: usize,
    tokenizer: &CoreBPE,
) -> Vec<(usize, String)> {
    if token_count(tokenizer, block) <= chunk_size_tokens {
        return vec![(base_offset, block.to_string())];
    }

    let Some(parsed) = parse_mermaid_block(block) else {
        return vec![(base_offset, block.to_string())];
    };
    let Some(groups) = grouped_body_chunks(&parsed, chunk_size_tokens, tokenizer) else {
        return vec![(base_offset, block.to_string())];
    };
    if groups.len() <= 1 {
        return vec![(base_offset, block.to_string())];
    }

    groups
        .into_iter()
        .map(|body| {
            let offset = body
                .first()
                .map(|line| base_offset + line.offset)
                .unwrap_or(base_offset + parsed.declaration.offset);
            (offset, build_chunk(&parsed, &body))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiktoken_rs::cl100k_base;

    fn tokenizer() -> CoreBPE {
        cl100k_base().expect("cl100k_base tokenizer should load")
    }

    fn large_flowchart() -> String {
        let mut body =
            String::from("```mermaid\n%%{init: {\"theme\": \"base\"}}%%\nflowchart TD\n");
        for i in 0..40 {
            body.push_str(&format!("A{i}[Step {i}] --> A{}\n", i + 1));
        }
        body.push_str("```\n");
        body
    }

    #[test]
    fn small_mermaid_block_returns_original_text() {
        let block = "```mermaid\nflowchart TD\nA --> B\n```\n";
        let chunks = split_mermaid_block(block, 7, 256, &tokenizer());
        assert_eq!(chunks, vec![(7, block.to_string())]);
    }

    #[test]
    fn large_mermaid_block_splits_into_valid_fenced_chunks() {
        let block = large_flowchart();
        let chunks = split_mermaid_block(&block, 11, 64, &tokenizer());

        assert!(chunks.len() > 1);
        for (_, chunk) in &chunks {
            assert!(chunk.starts_with("```mermaid\n"));
            assert!(chunk.contains("%%{init: {\"theme\": \"base\"}}%%"));
            assert!(chunk.contains("flowchart TD"));
            assert!(chunk.ends_with("```\n"));
            assert_eq!(chunk.matches("```mermaid").count(), 1);
            assert_eq!(
                chunk
                    .split('\n')
                    .filter(|line| line.trim() == "```")
                    .count(),
                1
            );
        }
    }

    #[test]
    fn large_mermaid_block_offsets_point_to_original_body_lines() {
        let block = large_flowchart();
        let chunks = split_mermaid_block(&block, 11, 64, &tokenizer());
        let first_edge_offset = block.find("A0[Step 0]").unwrap();

        assert_eq!(chunks[0].0, 11 + first_edge_offset);
        for (offset, chunk) in chunks {
            let first_local_line = chunk
                .lines()
                .find(|line| line.trim_start().starts_with('A'))
                .expect("chunk should contain a body line");
            assert!(
                block[offset - 11..].starts_with(first_local_line),
                "offset should point at the first original body line"
            );
        }
    }
}
