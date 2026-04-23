# ENH-003: Episodic Memory (Cross-Session Conversation Recall)

## Status
Proposed

## Summary
Store embeddings of user messages in a dedicated Qdrant collection so the RAG system can recall relevant past conversations across sessions. This gives the assistant long-term memory per user, enabling continuity like "as I explained last week..." or retrieving context from a question asked months ago.

## Motivation
Currently, Lekton's chat has a 20-message in-session history window (`MAX_HISTORY_MESSAGES` in `src/rag/chat.rs:24`). Once a session ends or the window is exceeded, all conversational context is lost. The query rewriter (`src/rag/query_rewriter.rs`) helps within a session but cannot bridge across sessions.

Cheshire Cat implements a three-tier memory architecture:
- **Declarative memory**: uploaded documents (equivalent to Lekton's current Qdrant collection).
- **Episodic memory**: past user messages stored as vectors in a separate Qdrant collection, filtered by `user_id`. Retrieved alongside declarative memory on every query.
- **Procedural memory**: tool descriptions (not relevant to Lekton).

The episodic tier is what gives Cheshire Cat its conversational continuity. A user who asked "how do I deploy to staging?" three weeks ago will have that context available when they later ask "what about production?".

## Current Behavior
1. User messages are saved to MongoDB `chat_messages` (text only, no embeddings).
2. During a chat turn, only the current session's last 20 messages are loaded.
3. Vector search only queries the `documents` Qdrant collection (declarative knowledge).
4. No cross-session context is available.

## Proposed Behavior
1. After saving each user message to MongoDB, also embed it and store the vector in a separate Qdrant collection (`episodic_memory` or configurable name).
2. On each new query, perform two parallel vector searches: one against `documents` (declarative) and one against `episodic_memory` (filtered by `user_id`).
3. Merge results from both collections into the LLM context, with clear labeling.
4. Episodic results are labeled distinctly in the system prompt so the LLM knows they are past conversations, not documentation.

## Implementation Details

### 1. New Qdrant Collection for Episodic Memory
Create a second Qdrant collection (e.g., `episodic_memory`) with the same vector dimensions as the documents collection. Payload per point:

```rust
pub struct EpisodicPayload {
    /// The original user message text.
    pub message_text: String,
    /// User ID for filtering (each user only recalls their own history).
    pub user_id: String,
    /// Session ID for optional grouping.
    pub session_id: String,
    /// Timestamp for recency weighting or TTL.
    pub created_at: String,  // ISO 8601
}
```

### 2. Extend RagConfig
Add configuration fields in `src/config.rs`:

```rust
/// Enable episodic memory (cross-session conversation recall). Default: false.
#[serde(default)]
pub episodic_memory_enabled: bool,

/// Qdrant collection name for episodic memory. Default: "episodic_memory".
#[serde(default = "default_episodic_collection")]
pub episodic_memory_collection: String,

/// Number of episodic memory results to retrieve per query. Default: 3.
#[serde(default = "default_episodic_k")]
pub episodic_memory_k: usize,

/// Minimum similarity score for episodic memory recall. Default: 0.7.
#[serde(default = "default_episodic_threshold")]
pub episodic_memory_score_threshold: f32,
```

Higher default threshold (0.7) than declarative (0.0) because episodic recall should be high-confidence only to avoid confusing the LLM with irrelevant past chatter.

### 3. Store User Messages as Vectors
In `ChatService::stream_response`, after saving the user message to MongoDB (step 3, line ~140), spawn a background task to:
1. Embed the user message (reuse the same `EmbeddingService`).
2. Upsert the vector into the episodic collection with `user_id`, `session_id`, `created_at` payload.

This should be fire-and-forget (non-blocking) to avoid adding latency to the chat response. Use `tokio::spawn`.

### 4. Parallel Retrieval
In `stream_response`, after embedding the retrieval query (step 5), perform two searches in parallel using `tokio::join!`:

```rust
let (declarative_results, episodic_results) = tokio::join!(
    self.vectorstore.search(query_vector.clone(), self.retrieval_k, ...),
    self.episodic_store.search(query_vector, self.episodic_k, user_id_filter),
);
```

### 5. Context Assembly
Build the system prompt context with labeled sections:

```
## Documentation
[Kubernetes Setup] (engineering/kubernetes)
To deploy on Kubernetes...

---

## Previous Conversations
[2024-03-15] You previously asked about staging deployments...
```

Use a Tera template variable `{{episodic_context}}` alongside the existing `{{context}}`.

### 6. Access Control
Episodic memory is inherently per-user: the Qdrant filter always includes `user_id = current_user`. No cross-user leakage is possible. This is simpler than declarative memory's multi-level RBAC.

### 7. Memory Hygiene
- **TTL**: Optionally configure a maximum age for episodic memories (e.g., 90 days). Implement via a periodic cleanup job or Qdrant payload filtering on `created_at`.
- **Session deletion**: When a user deletes a chat session via `DELETE /api/v1/rag/sessions/{id}`, also delete the corresponding episodic vectors (filter by `session_id`).
- **User deletion**: When a user account is removed, delete all their episodic vectors.

### 8. New VectorStore Instance
Create a second `QdrantVectorStore` instance pointing to the episodic collection. The `VectorStore` trait already supports everything needed; the only addition is a user_id filter parameter. Options:
- Add a `user_id: Option<&str>` parameter to the `search` method.
- Or create a thin wrapper that always applies the user_id filter.

The cleaner approach is to add a generic metadata filter to the `search` trait method.

## Files to Modify
| File | Change |
|------|--------|
| `src/config.rs` | Add episodic memory config fields to `RagConfig` |
| `config/default.toml` | Add default values (disabled by default) |
| `src/rag/vectorstore.rs` | Add user_id filter support or generic metadata filter to `VectorStore::search` |
| `src/rag/chat.rs` | Add episodic store field, parallel retrieval, context assembly, background embedding of user messages |
| `src/rag/service.rs` | Initialize episodic vector store when enabled |
| `src/db/chat_repository.rs` | On session deletion, also clean up episodic vectors |
| System prompt template | Add `{{episodic_context}}` variable |

## Benefits
- **Conversational continuity**: The assistant remembers what the user discussed in previous sessions.
- **Personalized answers**: Responses can reference and build on prior interactions.
- **Reduced repetition**: Users don't have to re-explain context they've already provided.
- **User engagement**: Long-term memory creates a more natural, assistant-like experience.

## Risks and Mitigations
| Risk | Mitigation |
|------|-----------|
| Stale episodic memories mislead the LLM | High score threshold (0.7 default) + TTL cleanup + label memories with date |
| Storage growth per user | TTL-based cleanup, configurable retention period |
| Embedding cost increase | One extra embedding per user message (cheap), one extra search per query (parallel, no latency increase) |
| Privacy: user sees AI reference old conversations they forgot about | Label episodic context clearly in the UI (future: let users manage/delete specific memories) |

## Dependencies
- **ENH-002 (Configurable Retrieval Parameters)**: The score threshold mechanism introduced there should be reused for episodic memory filtering. Implement ENH-002 first to establish the pattern.

## Effort Estimate
Medium. Requires a new Qdrant collection, parallel search logic, background embedding task, and template updates. No new external dependencies.
