# ENH-002: Configurable Retrieval Parameters

## Status
Proposed

## Summary
Make the vector search parameters `k` (number of chunks retrieved) and `score_threshold` (minimum similarity) configurable via `RagConfig` instead of being hardcoded constants.

## Motivation
Currently in `src/rag/chat.rs`, `MAX_CONTEXT_CHUNKS` is hardcoded to `5` (line 26). There is no similarity score threshold — all top-K results are used regardless of how low their score is. This means:

1. For narrow knowledge bases, 5 chunks may include irrelevant padding.
2. For broad knowledge bases, 5 chunks may be insufficient.
3. Low-scoring chunks (e.g., 0.3 cosine similarity) are injected into the prompt, potentially confusing the LLM.

Cheshire Cat (the reference project) uses per-memory-type configurable `k=3` and `threshold=0.7` defaults, each independently tunable. While Lekton doesn't need per-type memory configs, making these two parameters configurable and adding threshold filtering is a quick quality win.

## Current Behavior
- `MAX_CONTEXT_CHUNKS = 5` is a compile-time constant in `src/rag/chat.rs:26`.
- `VectorStore::search()` accepts a `limit: usize` parameter but the caller always passes `5`.
- No score filtering is applied after retrieval.

## Proposed Behavior
- `k` and `score_threshold` are read from `RagConfig` with sensible defaults.
- `ChatService` uses these values when calling `vectorstore.search()`.
- Results below `score_threshold` are filtered out before building context.

## Implementation Details

### 1. Extend RagConfig
Add two fields to `RagConfig` in `src/config.rs`:

```rust
/// Maximum number of chunks to retrieve from the vector store per query.
/// Default: 5.
#[serde(default = "default_retrieval_k")]
pub retrieval_k: usize,

/// Minimum cosine similarity score for a chunk to be included in context.
/// Chunks below this threshold are discarded even if fewer than `retrieval_k`
/// results remain. Range: 0.0 to 1.0. Default: 0.0 (no filtering).
#[serde(default)]
pub retrieval_score_threshold: f32,
```

Add default function:
```rust
fn default_retrieval_k() -> usize { 5 }
```

### 2. Update default.toml
Add to the `[rag]` section in `config/default.toml`:

```toml
retrieval_k = 5
retrieval_score_threshold = 0.0
```

### 3. Update ChatService
In `ChatService::from_rag_config`, store `retrieval_k` and `retrieval_score_threshold` as struct fields. In `stream_response`:

```rust
// Step 6: search with configurable k
let search_results = self
    .vectorstore
    .search(query_vector, self.retrieval_k, allowed_levels.as_deref(), include_draft)
    .await?;

// Step 6b: filter by score threshold
let search_results: Vec<_> = search_results
    .into_iter()
    .filter(|r| r.score >= self.retrieval_score_threshold)
    .collect();
```

Remove the `MAX_CONTEXT_CHUNKS` constant.

### 4. Update RagConfig Tests
Extend the existing config tests in `src/config.rs` to verify:
- Default values load correctly (k=5, threshold=0.0).
- Environment variable override works: `LKN__RAG__RETRIEVAL_K=10`.

## Files to Modify
| File | Change |
|------|--------|
| `src/config.rs` | Add `retrieval_k` and `retrieval_score_threshold` to `RagConfig` |
| `config/default.toml` | Add default values |
| `src/rag/chat.rs` | Remove `MAX_CONTEXT_CHUNKS`, use config values, add score filtering |

## Benefits
- **Tuning without recompile**: Operators can adjust retrieval quality per deployment via environment variables or config file.
- **Noise reduction**: Score threshold prevents low-relevance chunks from polluting the LLM context, reducing hallucination risk.
- **Flexibility**: Different deployments (small internal wiki vs large doc corpus) can optimize independently.

## Dependencies
None. Fully self-contained.

## Effort Estimate
Small. Two config fields, one filter line, removal of a constant.
