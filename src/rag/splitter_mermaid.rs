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
    context: Vec<MermaidLine>,
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
        context: Vec::new(),
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
    for line in &parsed.context {
        push_line(&mut chunk, &line.text);
    }
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

fn is_timeline_diagram(diagram_type: &MermaidDiagramType) -> bool {
    matches!(
        diagram_type,
        MermaidDiagramType::SequenceDiagram
            | MermaidDiagramType::Zenuml
            | MermaidDiagramType::Journey
            | MermaidDiagramType::Gantt
            | MermaidDiagramType::Timeline
            | MermaidDiagramType::Kanban
    )
}

fn is_hierarchical_diagram(diagram_type: &MermaidDiagramType) -> bool {
    matches!(
        diagram_type,
        MermaidDiagramType::Mindmap
            | MermaidDiagramType::Treemap
            | MermaidDiagramType::Ishikawa
            | MermaidDiagramType::TreeView
    )
}

fn is_chart_diagram(diagram_type: &MermaidDiagramType) -> bool {
    matches!(
        diagram_type,
        MermaidDiagramType::Packet
            | MermaidDiagramType::Pie
            | MermaidDiagramType::QuadrantChart
            | MermaidDiagramType::Sankey
            | MermaidDiagramType::XyChart
            | MermaidDiagramType::Radar
            | MermaidDiagramType::Venn
    )
}

fn is_sequence_context(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.starts_with("participant ")
        || lower.starts_with("actor ")
        || lower == "autonumber"
        || lower.starts_with("autonumber ")
}

fn apply_timeline_context(parsed: &mut ParsedMermaidBlock) {
    if !matches!(
        parsed.diagram_type,
        MermaidDiagramType::SequenceDiagram | MermaidDiagramType::Zenuml
    ) {
        return;
    }

    let mut context = Vec::new();
    let mut body = Vec::new();
    for line in std::mem::take(&mut parsed.body) {
        if is_sequence_context(&line.text) {
            context.push(line);
        } else {
            body.push(line);
        }
    }
    parsed.context = context;
    parsed.body = body;
}

fn apply_hierarchy_context(parsed: &mut ParsedMermaidBlock) {
    if !is_hierarchical_diagram(&parsed.diagram_type) {
        return;
    }

    let mut context = Vec::new();
    let mut body = Vec::new();
    let mut root_taken = false;
    for line in std::mem::take(&mut parsed.body) {
        if !root_taken && !line.text.trim().is_empty() {
            context.push(line);
            root_taken = true;
        } else {
            body.push(line);
        }
    }
    parsed.context.extend(context);
    parsed.body = body;
}

fn is_chart_context(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.starts_with("title ")
        || lower.starts_with("x-axis")
        || lower.starts_with("y-axis")
        || lower.starts_with("quadrant-")
}

fn apply_chart_context(parsed: &mut ParsedMermaidBlock) {
    if !matches!(
        parsed.diagram_type,
        MermaidDiagramType::XyChart | MermaidDiagramType::QuadrantChart | MermaidDiagramType::Radar
    ) {
        return;
    }

    let mut context = Vec::new();
    let mut body = Vec::new();
    for line in std::mem::take(&mut parsed.body) {
        if is_chart_context(&line.text) {
            context.push(line);
        } else {
            body.push(line);
        }
    }
    parsed.context.extend(context);
    parsed.body = body;
}

fn apply_gantt_context(parsed: &mut ParsedMermaidBlock) {
    if !matches!(parsed.diagram_type, MermaidDiagramType::Gantt) {
        return;
    }

    let mut context = Vec::new();
    let mut body = Vec::new();
    let mut seen_section = false;
    for line in std::mem::take(&mut parsed.body) {
        let is_section = line
            .text
            .trim()
            .to_ascii_lowercase()
            .starts_with("section ");
        if !seen_section && !is_section {
            context.push(line);
        } else {
            seen_section = true;
            body.push(line);
        }
    }
    parsed.context.extend(context);
    parsed.body = body;
}

fn is_c4_relation(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.starts_with("rel(")
        || lower.starts_with("rel_")
        || lower.starts_with("birel(")
        || lower.starts_with("birel_")
}

fn apply_c4_context(parsed: &mut ParsedMermaidBlock) {
    if !matches!(parsed.diagram_type, MermaidDiagramType::C4) {
        return;
    }

    let mut context = Vec::new();
    let mut body = Vec::new();
    for line in std::mem::take(&mut parsed.body) {
        if is_c4_relation(&line.text) {
            body.push(line);
        } else {
            context.push(line);
        }
    }
    parsed.context.extend(context);
    parsed.body = body;
}

fn apply_strategy_context(parsed: &mut ParsedMermaidBlock) {
    apply_timeline_context(parsed);
    apply_hierarchy_context(parsed);
    apply_chart_context(parsed);
    apply_gantt_context(parsed);
    apply_c4_context(parsed);
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

fn starts_interaction_block(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.starts_with("alt ")
        || lower.starts_with("opt ")
        || lower.starts_with("loop ")
        || lower.starts_with("par ")
        || lower.starts_with("critical ")
        || lower.starts_with("break ")
        || lower.starts_with("rect ")
        || lower.starts_with("box ")
}

fn interaction_units(lines: &[MermaidLine]) -> Vec<Vec<MermaidLine>> {
    let mut units = Vec::new();
    let mut current: Vec<MermaidLine> = Vec::new();
    let mut depth = 0usize;

    for line in lines {
        let starts = starts_interaction_block(&line.text);
        let ends = line.text.trim() == "end";
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

fn section_units(lines: &[MermaidLine]) -> Vec<Vec<MermaidLine>> {
    let mut units = Vec::new();
    let mut current: Vec<MermaidLine> = Vec::new();

    for line in lines {
        let lower = line.text.trim().to_ascii_lowercase();
        let starts_section = lower.starts_with("section ") || lower.ends_with(':');
        if starts_section && !current.is_empty() {
            units.push(std::mem::take(&mut current));
        }
        current.push(line.clone());
    }

    if !current.is_empty() {
        units.push(current);
    }

    units
}

fn timeline_units(parsed: &ParsedMermaidBlock) -> Vec<Vec<MermaidLine>> {
    match parsed.diagram_type {
        MermaidDiagramType::SequenceDiagram | MermaidDiagramType::Zenuml => {
            interaction_units(&parsed.body)
        }
        MermaidDiagramType::Journey
        | MermaidDiagramType::Gantt
        | MermaidDiagramType::Timeline
        | MermaidDiagramType::Kanban => section_units(&parsed.body),
        _ => line_units(&parsed.body),
    }
}

fn indentation(line: &str) -> usize {
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

fn hierarchy_units(lines: &[MermaidLine]) -> Vec<Vec<MermaidLine>> {
    let group_indent = lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .map(|line| indentation(&line.text))
        .min()
        .unwrap_or(0);
    let mut units = Vec::new();
    let mut current: Vec<MermaidLine> = Vec::new();

    for line in lines {
        let starts_group = !line.text.trim().is_empty() && indentation(&line.text) <= group_indent;
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

fn chart_units(lines: &[MermaidLine]) -> Vec<Vec<MermaidLine>> {
    line_units(lines)
}

fn body_units(parsed: &ParsedMermaidBlock) -> Option<Vec<Vec<MermaidLine>>> {
    if parsed.body.is_empty() {
        return None;
    }
    if is_relational_diagram(&parsed.diagram_type) {
        Some(relational_units(&parsed.body))
    } else if is_schema_diagram(&parsed.diagram_type) {
        Some(schema_units(parsed))
    } else if is_timeline_diagram(&parsed.diagram_type) {
        Some(timeline_units(parsed))
    } else if is_hierarchical_diagram(&parsed.diagram_type) {
        Some(hierarchy_units(&parsed.body))
    } else if is_chart_diagram(&parsed.diagram_type) {
        Some(chart_units(&parsed.body))
    } else if matches!(parsed.diagram_type, MermaidDiagramType::Unknown(_)) {
        None
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

    let Some(mut parsed) = parse_mermaid_block(block) else {
        return vec![(base_offset, block.to_string())];
    };
    apply_strategy_context(&mut parsed);
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
                "C4Context" => body.push_str(&format!(
                    "Person(p{i}, \"User {i}\")\nSystem(s{i}, \"System {i}\")\nRel(p{i}, s{i}, \"uses\")\n"
                )),
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

    fn timeline_fixture(declaration: &str) -> String {
        let mut body = format!("```mermaid\n{declaration}\n");
        match declaration {
            "sequenceDiagram" => {
                body.push_str("participant A\nparticipant B\nautonumber\n");
                for i in 0..54 {
                    body.push_str(&format!("A->>B: message {i}\n"));
                }
            }
            "zenuml" => {
                body.push_str("participant A\nparticipant B\n");
                for i in 0..54 {
                    body.push_str(&format!("A.method{i}()\n"));
                }
            }
            "journey" => {
                for section in 0..12 {
                    body.push_str(&format!("section Phase {section}\n"));
                    body.push_str(&format!("Task {section}: 5: User\n"));
                    body.push_str(&format!("Review {section}: 3: User\n"));
                }
            }
            "gantt" => {
                body.push_str("dateFormat  YYYY-MM-DD\n");
                for section in 0..12 {
                    body.push_str(&format!("section Phase {section}\n"));
                    body.push_str(&format!(
                        "Task {section} :done, t{section}, 2026-01-01, 2d\n"
                    ));
                }
            }
            "timeline" => {
                for section in 0..12 {
                    body.push_str(&format!("section Phase {section}\n"));
                    body.push_str(&format!("2026-01-{section:02} : Event {section}\n"));
                }
            }
            "kanban" => {
                for section in 0..12 {
                    body.push_str(&format!("section Lane {section}\n"));
                    body.push_str(&format!("Task {section}\n"));
                }
            }
            _ => {}
        }
        body.push_str("```\n");
        body
    }

    fn hierarchy_or_chart_fixture(declaration: &str) -> String {
        let mut body = format!("```mermaid\n{declaration}\n");
        match declaration {
            "mindmap" => {
                body.push_str("  root((Root))\n");
                for i in 0..36 {
                    body.push_str(&format!("    Branch {i}\n      Leaf {i}\n"));
                }
            }
            "treemap-beta" | "ishikawa" | "treeView" => {
                body.push_str("  Root\n");
                for i in 0..36 {
                    body.push_str(&format!("    Branch {i}\n      Leaf {i}\n"));
                }
            }
            "packet-beta" => {
                for i in 0..54 {
                    body.push_str(&format!("{i}: field_{i}\n"));
                }
            }
            "pie title Usage" => {
                for i in 0..54 {
                    body.push_str(&format!("\"Slice {i}\" : {i}\n"));
                }
            }
            "quadrantChart" => {
                body.push_str("x-axis Low --> High\ny-axis Low --> High\nquadrant-1 Plan\n");
                for i in 0..54 {
                    body.push_str(&format!("Point {i}: [{}, {}]\n", i % 10, (i + 3) % 10));
                }
            }
            "sankey-beta" => {
                for i in 0..54 {
                    body.push_str(&format!("source{i},target{i},{}\n", i + 1));
                }
            }
            "xychart-beta" => {
                body.push_str("title \"Trend\"\nx-axis [a, b, c]\ny-axis \"Value\" 0 --> 100\n");
                for i in 0..54 {
                    body.push_str(&format!("line [{i}, {}, {}]\n", i + 1, i + 2));
                }
            }
            "radar-beta" => {
                body.push_str("title Skills\n");
                for i in 0..54 {
                    body.push_str(&format!("\"Metric {i}\" : {}\n", i % 10));
                }
            }
            "venn" => {
                for i in 0..54 {
                    body.push_str(&format!("A{i}: {}\n", i + 1));
                }
            }
            _ => {}
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

    #[test]
    fn timeline_mermaid_types_split_large_blocks() {
        for declaration in [
            "sequenceDiagram",
            "zenuml",
            "journey",
            "gantt",
            "timeline",
            "kanban",
        ] {
            let block = timeline_fixture(declaration);
            let chunks = split_mermaid_block(&block, 0, 90, &tokenizer());

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
    fn sequence_participants_are_repeated_in_each_chunk() {
        let block = timeline_fixture("sequenceDiagram");
        let chunks = split_mermaid_block(&block, 0, 80, &tokenizer());

        assert!(chunks.len() > 1);
        for (_, chunk) in chunks {
            assert!(chunk.contains("participant A"));
            assert!(chunk.contains("participant B"));
            assert!(chunk.contains("autonumber"));
        }
    }

    #[test]
    fn sequence_alt_block_is_not_split_across_chunks() {
        let mut block = String::from(
            "```mermaid\nsequenceDiagram\nparticipant A\nparticipant B\nalt success\n",
        );
        for i in 0..30 {
            block.push_str(&format!("A->>B: success {i}\n"));
        }
        block.push_str("else failure\nA->>B: failed\nend\nA->>B: after\n```\n");

        let chunks = split_mermaid_block(&block, 0, 70, &tokenizer());
        assert!(chunks.len() > 1);
        let alt_chunk = chunks
            .iter()
            .map(|(_, chunk)| chunk)
            .find(|chunk| chunk.contains("alt success"))
            .expect("expected a chunk containing the alt block");
        assert!(alt_chunk.contains("else failure"));
        assert!(alt_chunk.contains("end\n"));
    }

    #[test]
    fn hierarchical_and_chart_mermaid_types_split_large_blocks() {
        for declaration in [
            "mindmap",
            "treemap-beta",
            "ishikawa",
            "treeView",
            "packet-beta",
            "pie title Usage",
            "quadrantChart",
            "sankey-beta",
            "xychart-beta",
            "radar-beta",
            "venn",
        ] {
            let block = hierarchy_or_chart_fixture(declaration);
            let chunks = split_mermaid_block(&block, 0, 90, &tokenizer());

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
    fn mindmap_root_is_repeated_in_each_chunk() {
        let block = hierarchy_or_chart_fixture("mindmap");
        let chunks = split_mermaid_block(&block, 0, 70, &tokenizer());

        assert!(chunks.len() > 1);
        for (_, chunk) in chunks {
            assert!(chunk.contains("root((Root))"));
        }
    }

    #[test]
    fn xychart_axes_are_repeated_in_each_chunk() {
        let block = hierarchy_or_chart_fixture("xychart-beta");
        let chunks = split_mermaid_block(&block, 0, 70, &tokenizer());

        assert!(chunks.len() > 1);
        for (_, chunk) in chunks {
            assert!(chunk.contains("title \"Trend\""));
            assert!(chunk.contains("x-axis [a, b, c]"));
            assert!(chunk.contains("y-axis \"Value\" 0 --> 100"));
        }
    }

    #[test]
    fn unknown_mermaid_type_stays_atomic_when_oversized() {
        let mut block = String::from("```mermaid\nexperimentalDiagram\n");
        for i in 0..80 {
            block.push_str(&format!("statement {i}\n"));
        }
        block.push_str("```\n");

        let chunks = split_mermaid_block(&block, 0, 60, &tokenizer());
        assert_eq!(chunks, vec![(0, block)]);
    }

    #[test]
    fn gantt_config_is_repeated_in_each_chunk() {
        let block = timeline_fixture("gantt");
        let chunks = split_mermaid_block(&block, 0, 70, &tokenizer());

        assert!(chunks.len() > 1);
        for (_, chunk) in chunks {
            assert!(chunk.contains("dateFormat  YYYY-MM-DD"));
        }
    }

    #[test]
    fn c4_definition_context_is_repeated_for_relation_chunks() {
        let block = relational_fixture("C4Context");
        let chunks = split_mermaid_block(&block, 0, 100, &tokenizer());

        assert!(chunks.len() > 1);
        for (_, chunk) in chunks {
            assert!(chunk.contains("Person(p0, \"User 0\")"));
            assert!(chunk.contains("System(s0, \"System 0\")"));
            assert!(chunk.contains("Rel("));
        }
    }
}
