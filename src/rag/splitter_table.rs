use tiktoken_rs::CoreBPE;

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

pub(in crate::rag) fn split_table_block(
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
