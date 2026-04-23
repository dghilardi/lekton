# ENH-005: HyDE Query Expansion

## Status
Proposed

## Summary
Add an optional Hypothetical Document Embedding (HyDE) step to the retrieval pipeline. Before vector search, the LLM generates a hypothetical answer to the user's question, and that answer is embedded instead of (or alongside) the raw query. This significantly improves retrieval recall for short, ambiguous, or keyword-sparse queries.

## Motivation
Vector search works by finding chunks whose embeddings are close to the query embedding. But user queries and document chunks have a fundamental asymmetry: queries are short questions ("how do I deploy?") while chunks are declarative statements ("To deploy the application, run `kubectl apply`..."). The embedding spaces of questions and answers don't always align well.

HyDE (Gao et al., 2022) bridges this gap: the LLM generates a hypothetical document that *would* answer the question, then that document is embedded. Since the hypothetical answer is in the same "style" as the actual chunks, cosine similarity works much better.

Cheshire Cat implements this via its `cat_recall_query` hook, where plugins can transform the query before embedding. The most common use is HyDE.

Lekton already has a query rewriter (`src/rag/query_rewriter.rs`) for multi-turn context resolution. HyDE is complementary: the rewriter makes follow-up questions self-contained, then HyDE transforms the self-contained question into a hypothetical answer for better retrieval.

## Current Behavior
1. Query rewriter (if enabled) rewrites follow-up questions into standalone queries.
2. The (rewritten) query is embedded directly via `EmbeddingService::embed()`.
3. The query embedding is used for Qdrant vector search.

## Proposed Behavior
1. Query rewriter runs as before.
2. If HyDE is enabled, the rewritten query is sent to an LLM to generate a hypothetical answer.
3. The hypothetical answer is embedded (not the original query).
4. The hypothetical answer embedding is used for vector search.
5. The original user question is still used in the system prompt template (not the hypothetical answer).

## Implementation Details

### 1. New Module: `src/rag/hyde.rs`

```rust
pub struct HydeExpander {
    llm_provider: Arc<LlmProvider>,
    model: String,
    max_tokens: u32,
    headers: HashMap<String, String>,
}

const HYDE_SYSTEM: &str = "\
You are a technical documentation assistant. \
Given a question, write a short paragraph that directly answers it \
as if it were part of an internal documentation page. \
Do NOT say \"I don't know\" or ask for clarification. \
Write a plausible, detailed answer even if you are not certain. \
Output ONLY the answer paragraph — no prefixes, no explanations.";

impl HydeExpander {
    pub fn from_rag_config(config: &RagConfig, llm_provider: Arc<LlmProvider>) -> Option<Self> {
        if config.hyde_model.is_empty() {
            return None;
        }
        Some(Self {
            llm_provider,
            model: config.hyde_model.clone(),
            max_tokens: config.hyde_max_tokens,
            headers: config.chat_headers.clone(),
        })
    }

    /// Generate a hypothetical answer to the query.
    /// Falls back to the original query if the LLM returns empty.
    pub async fn expand(&self, query: &str) -> Result<String, AppError> {
        // Single LLM call with HYDE_SYSTEM + user query
        // Return the generated text, or fall back to `query`
    }
}
```

### 2. Extend RagConfig
Add to `src/config.rs`:

```rust
/// Model used for HyDE (Hypothetical Document Embedding) query expansion.
/// Uses the same `chat_url` / `chat_api_key` endpoint.
/// Empty string disables HyDE (default).
#[serde(default)]
pub hyde_model: String,

/// Maximum tokens for the HyDE hypothetical answer generation.
/// Default: 150.
#[serde(default = "default_hyde_max_tokens")]
pub hyde_max_tokens: u32,
```

### 3. Integrate into ChatService
In `ChatService::stream_response`, after query rewriting (step 4) and before embedding (step 5):

```rust
// Step 4b: HyDE expansion (optional)
let embedding_query = match &self.hyde_expander {
    Some(expander) => expander.expand(&retrieval_query).await?,
    None => retrieval_query.clone(),
};

// Step 5: embed the (possibly HyDE-expanded) query
let query_vectors = self.embedding.embed(&[embedding_query]).await?;
```

The pipeline becomes: `user_message -> rewrite -> hyde -> embed -> search`.

### 4. Cost Optimization
HyDE adds one LLM call per chat turn. To control costs:
- Use a small/fast model (e.g., `meta-llama/llama-3-8b` or a local Ollama model).
- Keep `max_tokens` low (150 is usually sufficient).
- The model can be the same as the rewrite model to reuse the connection.
- Consider caching: if the query rewriter produced the same output as a recent query, skip HyDE.

### 5. Observability
Log the hypothetical answer at `debug` level for diagnostics:
```rust
tracing::debug!(
    original = retrieval_query,
    hypothetical = %expanded,
    "HyDE expansion"
);
```

If ENH-001 (Source References) is implemented, consider including a flag indicating HyDE was used in the response metadata.

## Files to Modify
| File | Change |
|------|--------|
| `src/rag/hyde.rs` | New module: HyDE expander |
| `src/rag/mod.rs` | Register `hyde` module |
| `src/config.rs` | Add `hyde_model` and `hyde_max_tokens` to `RagConfig` |
| `config/default.toml` | Add default values (empty = disabled) |
| `src/rag/chat.rs` | Add `hyde_expander` field to `ChatService`, call expand before embedding |

## Benefits
- **Better recall for short queries**: "deployment" as a query matches poorly against documentation chunks. A hypothetical answer like "To deploy the application, use kubectl apply -f deployment.yaml in the staging namespace" matches much better.
- **Better recall for keyword-sparse queries**: Questions phrased differently from the documentation ("how do I ship code?") generate hypothetical answers using the same vocabulary as the docs ("deploy", "CI/CD pipeline").
- **Complementary to query rewriting**: Rewriting fixes multi-turn ambiguity; HyDE fixes query-document vocabulary mismatch.

## Risks and Mitigations
| Risk | Mitigation |
|------|-----------|
| Additional LLM call per turn (cost + latency) | Use a small/fast model; disable by default; max_tokens capped at 150 |
| Hypothetical answer may be wrong, leading to bad retrieval | The embedding captures semantic similarity, not factual accuracy. Even a wrong but topically relevant answer retrieves the right chunks. |
| Adds complexity to the retrieval pipeline | Follows the same pattern as query rewriter: optional, config-gated, graceful fallback |

## Dependencies
None strictly, but this enhancement is most effective when combined with:
- **ENH-002 (Configurable Retrieval)**: Score threshold helps filter out HyDE-retrieved chunks that are still low quality.
- **ENH-004 (Token-Aware Chunking)**: Better chunks mean HyDE's improved recall actually finds good content.

## Effort Estimate
Small-Medium. Follows the exact same pattern as the existing `QueryRewriter` — an optional LLM call gated by config. Most of the code can be modeled after `src/rag/query_rewriter.rs`.
