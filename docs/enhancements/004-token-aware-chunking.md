# ENH-004: Token-Aware Document Chunking

## Status
Implemented

## Summary
Replace the old character-based chunking (512 characters) with token-aware chunking measured in `cl100k_base` tokens, add configurable overlap, and preserve richer document structure through heading-aware section metadata.

## Motivation
The current splitter in `src/rag/splitter.rs` uses `text_splitter::MarkdownSplitter` with a hard limit of `CHUNK_SIZE = 512` characters (line 4). This has two problems:

1. **Token/character mismatch**: A 512-character chunk can vary significantly in token count depending on content. English prose averages ~4 characters/token, but code can be 2-3 characters/token and URLs/paths can be 6+. This means some chunks may be too short (wasting embedding capacity) while others may exceed the embedding model's context window.

2. **No chunk overlap**: The current implementation produces non-overlapping chunks. When a relevant piece of information spans a chunk boundary, neither chunk contains enough context for good retrieval. Overlap ensures continuity.

Cheshire Cat uses LangChain's `RecursiveCharacterTextSplitter` with tiktoken, measuring in tokens (default: 256 tokens, 64 token overlap). This produces more consistent, retrieval-friendly chunks.

## Current Behavior
- `src/rag/splitter.rs` performs a two-pass split:
  first by H1/H2 section boundaries, then by token-aware `MarkdownSplitter`.
- Chunk size is configurable via `rag.chunk_size_tokens` (default `256`).
- Overlap is configurable via `rag.chunk_overlap_tokens` (default `64`).
- Small adjacent sections are merged forward to avoid tiny retrieval units.
- Fenced code blocks are kept atomic; oversize code blocks are emitted whole rather than torn apart.
- Markdown tables are detected with the GFM parser rather than line-prefix heuristics. Tables that fit the token budget stay atomic; oversized tables are split by row groups with the original header and delimiter repeated in every table chunk.
- Mermaid code fences are detected as diagram blocks. Small diagrams stay atomic; oversized diagrams are split by diagram family while repeating the Mermaid declaration and required context so each produced chunk remains a valid Mermaid fence.
- Each chunk carries structural metadata: `section_path`, `section_anchor`, `byte_offset`, `char_offset`, `chunk_index`.
- Embeddings are generated from enriched text (`Document Title > Section Path\n\nChunk`), while prompt/UI display uses the clean chunk text.

## Implementation Details

### 1. Tokenizer Selection
The implementation uses `tiktoken-rs` with `cl100k_base`, which is a good default approximation for the embedding models currently supported by Lekton.

```toml
text-splitter = { version = "...", features = ["markdown", "tiktoken-rs"] }
```

### 2. Configuration
`RagConfig` exposes the token-based sizing knobs:

```rust
pub chunk_size_tokens: u32,
pub chunk_overlap_tokens: u32,
```

Unlike the original issue draft, the implementation intentionally does **not** introduce parallel character-based knobs such as `chunk_min_chars` / `chunk_max_chars` / `chunk_overlap_chars`. The project converges on token-based sizing only.

### 3. Splitter Behavior
The shipped implementation goes beyond a plain token-aware `MarkdownSplitter` wrapper:

```rust
pub fn split_document(
    content: &str,
    chunk_size_tokens: usize,
    chunk_overlap_tokens: usize,
) -> Vec<SplitChunk>
```

Key properties:
- H1/H2 headings define raw parent sections.
- Very small adjacent sections are merged before token splitting.
- Token overlap is implemented through `text-splitter`'s overlap support.
- Parser-derived protected ranges prevent splits inside code blocks and GFM tables, including tables without outer pipes and cells containing escaped or inline-code pipes.
- Oversized tables bypass normal overlap splitting and are chunked only at row boundaries. Synthetic table chunks use the first original data row offset and repeat the table header for retrieval context.
- Oversized Mermaid diagrams bypass plain token splitting and use a Mermaid-aware splitter:
  relational diagrams preserve containers such as `subgraph ... end`; schema diagrams keep class/entity blocks atomic; interaction/time diagrams preserve participants, sections, and `alt/loop/par` blocks; hierarchical and chart diagrams repeat root or axis context where needed.
- Output is typed (`SplitChunk`) instead of plain `String`.

### 4. Ingestion and Retrieval Metadata
Indexing stores the display text in Qdrant payload and computes embeddings from an enriched text prefix that includes the document title and section hierarchy. This supports better retrieval without polluting the prompt context shown to the LLM.

The structural metadata introduced by this enhancement is also consumed by later query-time improvements on this branch:
- section-level source references (`slug#section-anchor`)
- optional parent-section expansion via `rag.expand_to_parent`

### 5. Reindex Consideration
Changing chunk size, overlap, table splitting policy, Mermaid splitting policy, or chunk payload structure changes the vectors and metadata stored in Qdrant. After deployment, a full reindex (`POST /api/v1/admin/rag/reindex`) is required.

### 6. Update Tests
`src/rag/splitter.rs` now includes tests covering:
- token-aware splitting
- stable chunk indexing
- section path / anchor extraction
- atomic fenced-code and table handling
- GFM table detection without outer pipes, escaped pipes, inline-code pipes, invalid delimiters, and block/blank-line termination
- oversized table row-group splitting with repeated headers
- Mermaid fixture coverage for supported diagram families across small, medium, and large inputs, including valid fences, repeated declarations/context, stable offsets, and structural blocks that must not be torn apart
- UTF-8-safe offset computation when merged sections contain multibyte characters

## Files to Modify
| File | Change |
|------|--------|
| `Cargo.toml` | Add `tiktoken-rs` feature to `text-splitter` |
| `src/config.rs` | Add `chunk_size_tokens` and `chunk_overlap_tokens` to `RagConfig` |
| `config/default.toml` | Add default values |
| `src/rag/splitter.rs` | Replace char-based splitting with typed, heading-aware token splitting |
| `src/rag/splitter_mermaid.rs` | Split oversized Mermaid diagrams by family while preserving valid fenced chunks |
| `src/rag/service.rs` | Pass token config to `split_document` and enrich embedding text |
| `src/rag/vectorstore.rs` | Persist structural metadata in chunk payload |

## Benefits
- **Consistent chunk density**: Each chunk occupies a predictable amount of the embedding model's context window.
- **Better retrieval**: Overlap prevents information loss at chunk boundaries, improving recall for queries that match content near boundaries.
- **Configurable**: Operators can tune chunk size and overlap per deployment. Smaller chunks = more precise retrieval, larger chunks = more context per result.
- **Model alignment**: Token-based sizing aligns with how embedding models actually process input.
- **Richer structure**: Section metadata enables section-level citations and parent-context expansion.

## Risks and Mitigations
| Risk | Mitigation |
|------|-----------|
| tiktoken adds a new dependency | `tiktoken-rs` is well-maintained, used by the OpenAI ecosystem. Compile-time only cost. |
| Token count mismatch with non-OpenAI models | cl100k_base is a reasonable approximation. Exact counts matter less than consistent sizing. |
| Reindex required after deployment | Document in release notes. The existing reindex endpoint handles this. |
| Overlap increases total chunk count and storage | Modest increase (~25% with 64/256 overlap). Tunable via config. |

## Dependencies
This enhancement is the retrieval baseline for later section-aware RAG work, but it is deployable on its own as long as a reindex is performed.

## Effort Estimate
Completed. The work ended up slightly broader than the original ticket because it bundled the payload and section-metadata changes that naturally belong to the same reindex window.
