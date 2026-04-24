# ENH-006: Embedding Rate Limiting

## Status
Proposed

## Summary
Add configurable rate limiting (throttling) to the embedding service to prevent HTTP 429 errors from external embedding providers during bulk operations like reindexing.

## Motivation
During a full reindex (`POST /api/v1/admin/rag/reindex`), Lekton iterates all documents, splits them into chunks, and embeds each batch. For a corpus of 500 documents averaging 10 chunks each, this produces ~5000 embedding requests in rapid succession.

External providers (OpenAI, OpenRouter, Cohere) enforce rate limits. Even self-hosted Ollama can be overwhelmed if the batch size exceeds GPU memory. Currently, `src/rag/embedding.rs` sends requests as fast as the async runtime allows, with no throttling.

Cheshire Cat addresses this with a 50ms delay between embedding batches during document ingestion (`src/cat/rabbit_hole.py`). While simple, this prevents rate limit errors in practice.

## Current Behavior
- `OpenAICompatibleEmbedding::embed()` in `src/rag/embedding.rs` sends a single batch request per call.
- The reindex loop in `src/rag/reindex.rs` calls embed for each document's chunks sequentially but with no delay.
- `CachedEmbeddingService` in `src/rag/cached_embedding.rs` reduces calls by skipping cached chunks, but cache misses still produce unbounded request rates.
- No retry logic exists for 429 responses.

## Proposed Behavior
- A configurable delay is inserted between embedding batch requests during bulk operations.
- Optional: a token-bucket rate limiter wraps the embedding service for finer control.
- 429 responses trigger exponential backoff retries.

## Implementation Details

### Approach A: Simple Inter-Batch Delay (Recommended)
The simplest and most effective approach, matching what Cheshire Cat does.

#### 1. Extend RagConfig
Add to `src/config.rs`:

```rust
/// Delay in milliseconds between embedding batch requests during bulk operations
/// (reindex, batch ingestion). Prevents rate limiting by external providers.
/// Set to 0 for no delay (e.g., when using local Ollama with sufficient resources).
/// Default: 50.
#[serde(default = "default_embedding_batch_delay_ms")]
pub embedding_batch_delay_ms: u64,

/// Maximum number of texts to embed in a single API call.
/// Larger batches are more efficient but may exceed provider limits.
/// Default: 32.
#[serde(default = "default_embedding_batch_size")]
pub embedding_batch_size: usize,
```

#### 2. Update Reindex Loop
In `src/rag/reindex.rs`, add a delay between embedding calls:

```rust
use tokio::time::{sleep, Duration};

// After each document's chunks are embedded and upserted:
if batch_delay_ms > 0 {
    sleep(Duration::from_millis(batch_delay_ms)).await;
}
```

#### 3. Batch Size Splitting
In the embedding service or the reindex caller, split large chunk lists into sub-batches of `embedding_batch_size`:

```rust
for batch in chunks.chunks(config.embedding_batch_size) {
    let vectors = embedding.embed(batch).await?;
    // ... upsert vectors ...
    if config.embedding_batch_delay_ms > 0 {
        sleep(Duration::from_millis(config.embedding_batch_delay_ms)).await;
    }
}
```

### Approach B: Retry with Exponential Backoff (Complementary)
Add retry logic to `OpenAICompatibleEmbedding::embed()` for transient failures:

```rust
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1000;

async fn embed_with_retry(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
    let mut attempt = 0;
    loop {
        match self.embed_inner(texts).await {
            Ok(result) => return Ok(result),
            Err(e) if is_rate_limited(&e) && attempt < MAX_RETRIES => {
                let backoff = INITIAL_BACKOFF_MS * 2u64.pow(attempt);
                tracing::warn!(attempt, backoff_ms = backoff, "rate limited, retrying");
                sleep(Duration::from_millis(backoff)).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}
```

This is complementary to Approach A: the delay prevents most 429s, and retry handles edge cases.

## Files to Modify
| File | Change |
|------|--------|
| `src/config.rs` | Add `embedding_batch_delay_ms` and `embedding_batch_size` to `RagConfig` |
| `config/default.toml` | Add default values |
| `src/rag/reindex.rs` | Add inter-batch delay and batch size splitting |
| `src/rag/embedding.rs` | (Optional) Add retry with exponential backoff |
| `src/api/ingest.rs` | Apply delay for single-document ingestion if chunk count is large |

## Benefits
- **Reliability**: Prevents reindex failures due to rate limiting, especially with external providers.
- **Self-healing**: Retry logic handles transient rate limits without operator intervention.
- **Configurability**: Operators using local Ollama can set delay to 0; those using OpenAI can increase it.
- **No new dependencies**: Uses `tokio::time::sleep` which is already available.

## Risks and Mitigations
| Risk | Mitigation |
|------|-----------|
| Delay slows down reindex | 50ms delay on 5000 chunks adds only ~4 minutes. Acceptable for a background operation. |
| Retry masks persistent errors | Cap retries at 3; only retry on 429/5xx, not on 4xx client errors. |

## Dependencies
None. Fully self-contained.

## Effort Estimate
Small. A few config fields, a `sleep` call in the reindex loop, and optional retry logic.
