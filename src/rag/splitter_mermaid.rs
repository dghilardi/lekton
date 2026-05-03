use tiktoken_rs::CoreBPE;

use super::splitter_blocks::{mermaid_source_from_fence, MermaidDiagramType};

#[derive(Debug, Clone)]
struct MermaidLine {
    offset: usize,
    text: String,
}

#[derive(Debug, Clone)]
struct ParsedMermaidBlock {
    diagram_type: MermaidDiagramType,
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

    let declaration = declaration?;
    let diagram_type = MermaidDiagramType::from_declaration(&declaration.text);
    Some(ParsedMermaidBlock {
        diagram_type,
        preamble,
        declaration,
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

fn is_relational_diagram(diagram_type: &MermaidDiagramType) -> bool {
    matches!(
        diagram_type,
        MermaidDiagramType::Flowchart
            | MermaidDiagramType::Graph
            | MermaidDiagramType::StateDiagram
            | MermaidDiagramType::RequirementDiagram
            | MermaidDiagramType::Architecture
            | MermaidDiagramType::Block
            | MermaidDiagramType::C4
    )
}

fn is_schema_diagram(diagram_type: &MermaidDiagramType) -> bool {
    matches!(
        diagram_type,
        MermaidDiagramType::ClassDiagram
            | MermaidDiagramType::ErDiagram
            | MermaidDiagramType::GitGraph
    )
}

fn starts_relational_container(line: &str) -> bool {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("subgraph ")
        || lower.ends_with('{')
        || lower.starts_with("group ")
        || lower.starts_with("boundary(")
        || lower.starts_with("system_boundary(")
        || lower.starts_with("enterprise_boundary(")
        || lower.starts_with("container_boundary(")
}

fn ends_relational_container(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed == "end" || trimmed == "}" || trimmed == ");"
}

fn line_units(lines: &[MermaidLine]) -> Vec<Vec<MermaidLine>> {
    lines.iter().cloned().map(|line| vec![line]).collect()
}

fn relational_units(lines: &[MermaidLine]) -> Vec<Vec<MermaidLine>> {
    let mut units = Vec::new();
    let mut current: Vec<MermaidLine> = Vec::new();
    let mut depth = 0usize;

    for line in lines {
        let starts = starts_relational_container(&line.text);
        let ends = ends_relational_container(&line.text);
        current.push(line.clone());
        if starts {
            depth += 1;
        }
        if ends && depth > 0 {
            depth -= 1;
        }
        if depth == 0 {
            units.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        units.push(current);
    }

    units
}

fn brace_units(lines: &[MermaidLine]) -> Vec<Vec<MermaidLine>> {
    let mut units = Vec::new();
    let mut current: Vec<MermaidLine> = Vec::new();
    let mut depth = 0usize;

    for line in lines {
        let trimmed = line.text.trim();
        current.push(line.clone());
        if trimmed.ends_with('{') {
            depth += 1;
        }
        if trimmed == "}" && depth > 0 {
            depth -= 1;
        }
        if depth == 0 {
            units.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        units.push(current);
    }

    units
}

fn git_graph_units(lines: &[MermaidLine]) -> Vec<Vec<MermaidLine>> {
    let mut units = Vec::new();
    let mut current: Vec<MermaidLine> = Vec::new();

    for line in lines {
        let lower = line.text.trim().to_ascii_lowercase();
        let starts_group = lower.starts_with("branch ")
            || lower.starts_with("checkout ")
            || lower.starts_with("merge ");
        if starts_group && !current.is_empty() {
            units.push(std::mem::take(&mut current));
        }
        current.push(line.clone());
    }

    if !current.is_empty() {
        units.push(current);
    }

    units
}

fn schema_units(parsed: &ParsedMermaidBlock) -> Vec<Vec<MermaidLine>> {
    if matches!(parsed.diagram_type, MermaidDiagramType::GitGraph) {
        git_graph_units(&parsed.body)
    } else {
        brace_units(&parsed.body)
    }
}

fn body_units(parsed: &ParsedMermaidBlock) -> Option<Vec<Vec<MermaidLine>>> {
    if parsed.body.is_empty() {
        return None;
    }
    if is_relational_diagram(&parsed.diagram_type) {
        Some(relational_units(&parsed.body))
    } else if is_schema_diagram(&parsed.diagram_type) {
        Some(schema_units(parsed))
    } else {
        Some(line_units(&parsed.body))
    }
}

fn grouped_body_chunks(
    parsed: &ParsedMermaidBlock,
    chunk_size_tokens: usize,
    tokenizer: &CoreBPE,
) -> Option<Vec<Vec<MermaidLine>>> {
    let units = body_units(parsed)?;

    let mut groups: Vec<Vec<MermaidLine>> = Vec::new();
    let mut current: Vec<MermaidLine> = Vec::new();

    for unit in units {
        let mut candidate = current.clone();
        candidate.extend(unit.clone());
        let candidate_text = build_chunk(parsed, &candidate);
        if token_count(tokenizer, &candidate_text) <= chunk_size_tokens {
            current = candidate;
            continue;
        }

        if !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        current = unit;
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

    fn relational_fixture(declaration: &str) -> String {
        let mut body = format!("```mermaid\n{declaration}\n");
        for i in 0..48 {
            match declaration {
                "stateDiagram-v2" => body.push_str(&format!("S{i} --> S{}: next\n", i + 1)),
                "requirementDiagram" => body.push_str(&format!(
                    "functionalRequirement req{i} {{\n  id: {i}\n  text: requirement {i}\n  risk: low\n  verifymethod: test\n}}\n"
                )),
                "architecture-beta" => {
                    body.push_str(&format!("service svc{i}(server)[Service {i}]\n"))
                }
                "block-beta" => body.push_str(&format!("B{i}[Block {i}]\n")),
                "C4Context" => body.push_str(&format!("Person(p{i}, \"User {i}\")\n")),
                _ => body.push_str(&format!("A{i}[Step {i}] --> A{}\n", i + 1)),
            }
        }
        body.push_str("```\n");
        body
    }

    fn schema_fixture(declaration: &str) -> String {
        let mut body = format!("```mermaid\n{declaration}\n");
        for i in 0..36 {
            match declaration {
                "classDiagram" => body.push_str(&format!(
                    "class Service{i} {{\n  +String name{i}\n  +run{i}()\n}}\nService{i} --> Service{}\n",
                    i + 1
                )),
                "erDiagram" => body.push_str(&format!(
                    "ENTITY{i} {{\n  string id\n  string value{i}\n}}\nENTITY{i} ||--o{{ ENTITY{} : links\n",
                    i + 1
                )),
                "gitGraph" => body.push_str(&format!(
                    "branch feature{i}\ncheckout feature{i}\ncommit id: \"f{i}-1\"\ncommit id: \"f{i}-2\"\ncheckout main\nmerge feature{i}\n"
                )),
                _ => {}
            }
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

    #[test]
    fn relational_mermaid_types_split_large_blocks() {
        for declaration in [
            "flowchart TD",
            "graph TD",
            "stateDiagram-v2",
            "requirementDiagram",
            "architecture-beta",
            "block-beta",
            "C4Context",
        ] {
            let block = relational_fixture(declaration);
            let chunks = split_mermaid_block(&block, 0, 80, &tokenizer());

            assert!(
                chunks.len() > 1,
                "{declaration} should split into multiple chunks"
            );
            for (_, chunk) in chunks {
                assert!(chunk.contains(declaration));
                assert!(chunk.starts_with("```mermaid\n"));
                assert!(chunk.ends_with("```\n"));
            }
        }
    }

    #[test]
    fn relational_subgraph_is_not_split_across_chunks() {
        let mut block = String::from("```mermaid\nflowchart TD\nsubgraph Cluster\n");
        for i in 0..30 {
            block.push_str(&format!("A{i} --> A{}\n", i + 1));
        }
        block.push_str("end\nOutside --> Done\n```\n");

        let chunks = split_mermaid_block(&block, 0, 60, &tokenizer());
        assert!(chunks.len() > 1);
        let subgraph_chunk = chunks
            .iter()
            .map(|(_, chunk)| chunk)
            .find(|chunk| chunk.contains("subgraph Cluster"))
            .expect("expected a chunk containing the subgraph");
        assert!(subgraph_chunk.contains("end\n"));
        assert!(!chunks
            .iter()
            .skip_while(|(_, chunk)| !chunk.contains("subgraph Cluster"))
            .skip(1)
            .any(|(_, chunk)| chunk.contains("A0 --> A1")));
    }

    #[test]
    fn schema_mermaid_types_split_large_blocks() {
        for declaration in ["classDiagram", "erDiagram", "gitGraph"] {
            let block = schema_fixture(declaration);
            let chunks = split_mermaid_block(&block, 0, 110, &tokenizer());

            assert!(
                chunks.len() > 1,
                "{declaration} should split into multiple chunks"
            );
            for (_, chunk) in chunks {
                assert!(chunk.contains(declaration));
                assert!(chunk.starts_with("```mermaid\n"));
                assert!(chunk.ends_with("```\n"));
            }
        }
    }

    #[test]
    fn class_block_is_not_split_across_chunks() {
        let mut block = String::from("```mermaid\nclassDiagram\nclass BigService {\n");
        for i in 0..30 {
            block.push_str(&format!("  +field{i}: String\n"));
        }
        block.push_str("}\nBigService --> OtherService\n```\n");

        let chunks = split_mermaid_block(&block, 0, 70, &tokenizer());
        assert!(chunks.len() > 1);
        let class_chunk = chunks
            .iter()
            .map(|(_, chunk)| chunk)
            .find(|chunk| chunk.contains("class BigService"))
            .expect("expected a chunk containing the class block");
        assert!(class_chunk.contains("+field0"));
        assert!(class_chunk.contains("+field29"));
        assert!(class_chunk.contains("}\n"));
    }
}
