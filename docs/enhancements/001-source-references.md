# ENH-001: Source References in Chat Responses

## Status
Implemented

## Summary
Expose the retrieved document references used for RAG answers alongside streamed chat responses and persisted chat history, so users can inspect and navigate to the source material behind each assistant answer.

## Motivation
Currently, `ChatService::stream_response` in `src/rag/chat.rs` retrieves chunks from Qdrant, injects them into the system prompt as context, and then discards the retrieval metadata before the response reaches the client. The user receives an answer with no visibility into which documents were used, how strong the retrieval signal was, or where to navigate to verify the answer.

Source references are effectively table-stakes for production RAG systems. They increase trust, improve debugging when retrieval quality is poor, and make the chat experience a navigation surface for the documentation portal instead of a dead-end answer box.

## Current Behavior
1. `VectorStore::search()` returns `Vec<VectorSearchResult>` containing `chunk_text`, `document_slug`, `document_title`, `score`.
2. These results are formatted into a context string and embedded in the system prompt.
3. The SSE stream emits `ChatEvent::Session`, `ChatEvent::Delta`, `ChatEvent::Done`.
4. Assistant messages persisted in MongoDB contain only `id`, `session_id`, `role`, `content`, and `created_at`.
5. No source metadata reaches the live client or the history endpoint.

## Proposed Behavior
1. After vector search completes, collect source references into a structured list.
2. Emit a new SSE event `ChatEvent::Sources` immediately after `ChatEvent::Session` and before any `ChatEvent::Delta`.
3. Persist the emitted references alongside the assistant message in MongoDB for history replay.
4. Re-apply access control when historical messages are read back, so previously stored references do not leak documents the current user can no longer access.
5. When retrieval metadata contains `section_path`/`section_anchor`, emit section-level citations so the UI can link directly to `slug#section-anchor`.

## Implementation Details

### 1. Shared Source Reference Model
Introduce a shared source-reference type in a reusable module, not inside `src/rag/chat.rs`, because it is consumed by chat streaming, MongoDB persistence, API responses, and the frontend chat page.

Recommended shape:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceReference {
    /// Document slug for building navigation links.
    pub document_slug: String,
    /// Human-readable document title.
    pub document_title: String,
    /// Deepest heading title for section-level citations.
    pub section_title: Option<String>,
    /// URL-safe section anchor to append to `document_slug`.
    pub section_anchor: Option<String>,
    /// Cosine similarity score from Qdrant (0.0 to 1.0).
    pub score: f32,
    /// Optional short preview snippet for the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}
```

Notes:
- Do not expose full `chunk_text` by default. It adds bandwidth, duplicates prompt context, and is noisier than necessary for the UI.
- `score` is useful to persist for debugging and admin inspection even if the normal user-facing UI eventually hides it.

### 2. New SSE Event Type
Add a variant to `ChatEvent` in `src/rag/chat.rs`:

```rust
#[serde(rename = "sources")]
Sources {
    sources: Vec<SourceReference>,
}
```

Event ordering should be:
1. `session`
2. `sources`
3. zero or more `delta`
4. `done`

This keeps the contract deterministic for the client and allows the UI to render references before the answer finishes streaming.

### 3. Build Display Sources Separately From Prompt Context
In `stream_response`, after vector search and before building the LLM request:

1. Keep using the raw `search_results` to build the prompt context string.
2. Derive a second collection of `SourceReference` values for the client and persistence layer.
3. Deduplicate by `(document_slug, section_anchor)`, keeping the highest score per cited section.
4. Derive `section_title` from the deepest element of `section_path`.
5. Optionally derive a short snippet from the top chunk for each cited section.
5. Yield `ChatEvent::Sources` as the second event in the `async_stream::stream!` block.

This separation matters because the best prompt representation is chunk-oriented, while the best UX representation is document-oriented.

### 4. Persist Sources with Assistant Messages
Extend `ChatMessage` in `src/db/chat_models.rs` to include an optional `sources` field:

```rust
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    /// Source references used to generate this response (assistant messages only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceReference>>,
}
```

Populate `sources` only for assistant messages. User messages should leave it as `None`.

### 5. Re-Apply RBAC on History Replay
The live retrieval step already respects access filters because vector search runs with the caller's current visibility constraints. Historical replay needs the same care.

When serving `GET /api/v1/rag/sessions/{id}/messages`:
- include `sources` in the response payload for assistant messages;
- filter or drop any stored source references the current caller can no longer access;
- avoid returning clickable references for documents that are now hidden by access-level or draft rules.

This keeps the enhancement aligned with Lekton's requirement that users must not see links or results for documents they cannot access.

### 6. Frontend Rendering
In `src/pages/chat.rs`:

- extend `UiMessage` with an optional `sources` field;
- track `streaming_sources` for the in-progress assistant response;
- parse the new `sources` SSE event in `fetch_chat_stream`;
- render a collapsible "Sources" block below assistant messages;
- link each source to `/docs/{document_slug}#section-anchor` when section metadata is present, otherwise to `/docs/{document_slug}`.

The same component should render persisted sources when a session is reloaded from history.

### 7. MCP Alignment
The MCP `search_documents` tool already returns `score`. No additional feature work is required to expose scores there, but the serialized shape should remain conceptually aligned with the source-reference model used by chat so the system does not drift into multiple similar-but-different result formats.

## Files to Modify
| File | Change |
|------|--------|
| shared model module (`src/db/chat_models.rs` or similar) | Define `SourceReference` in a reusable location |
| `src/rag/chat.rs` | Add `ChatEvent::Sources`, derive display sources, emit them in the SSE stream, persist them with assistant messages |
| `src/db/chat_models.rs` | Add optional `sources` field to `ChatMessage` |
| `src/db/chat_repository.rs` | Ensure `sources` is persisted and loaded from MongoDB |
| `src/api/rag.rs` | Include `sources` in session-message responses and filter them against current RBAC visibility |
| `src/pages/chat.rs` | Render live and persisted sources below assistant messages |

## Benefits
- **Trust**: Users can verify the AI's answer against the original documentation.
- **Navigation**: Direct links to source documents improve documentation discoverability.
- **Debugging**: Scores and retained references make retrieval quality easier to inspect.
- **Feedback loop**: Combined with the existing feedback system (ENH already shipped), users can flag cases where sources were irrelevant.

## Risks and Considerations
- **RBAC leakage**: Persisted references must be filtered again when chat history is loaded.
- **UI noise**: Exposing every chunk would overwhelm the chat interface; prefer document-level references.
- **Bandwidth**: Full chunk payloads should stay internal unless there is a dedicated admin/debug view.
- **Event-contract changes**: The SSE client and persisted-history API must be updated together to avoid live/history mismatches.

## Suggested Follow-Ups
1. Add automated tests for the new SSE event ordering and message serialization.
2. Add API tests covering source filtering when access permissions change after a message is generated.
3. Consider an admin-only debug mode later if full chunk-level provenance becomes necessary.

## Effort Estimate
Small to medium. The retrieval data is already available, but the enhancement touches the streaming contract, persisted chat schema, history endpoint, frontend hydration logic, and RBAC-sensitive replay behavior.
