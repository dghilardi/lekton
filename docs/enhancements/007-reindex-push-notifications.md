# ENH-007: Reindex Progress Push Notifications via SSE

## Status
Proposed

## Summary
Replace the current polling-based reindex progress mechanism with Server-Sent Events (SSE) that push progress updates to the admin UI in real time.

## Motivation
The current reindex system (`src/rag/reindex.rs`) tracks progress via `AtomicU32` in `ReindexState` (line 13). The admin frontend must poll `GET /api/v1/admin/rag/reindex/status` at an interval to read the current percentage. This has several drawbacks:

1. **Latency**: Poll intervals create a delay between actual progress and displayed progress.
2. **Wasted requests**: Most polls return the same percentage (no change).
3. **No granular events**: The admin can't see *which* document is being indexed or if errors occurred — only a percentage.

Cheshire Cat pushes ingestion progress to the client via WebSocket every 10 seconds, including document-level status. Since Lekton already uses SSE for chat streaming (`ChatEvent` in `src/rag/chat.rs`), the same pattern can be reused for reindex.

## Current Behavior
- `ReindexState` in `src/rag/reindex.rs` exposes `is_running: AtomicBool` and `progress: AtomicU32`.
- The admin endpoint `GET /api/v1/admin/rag/reindex/status` reads these atomics and returns JSON `{ is_running, progress }`.
- The frontend polls this endpoint (presumably every few seconds).
- No document-level detail or error reporting is available without checking server logs.

## Proposed Behavior
- `POST /api/v1/admin/rag/reindex` returns immediately with a 202 Accepted and a `stream_url`.
- `GET /api/v1/admin/rag/reindex/stream` returns an SSE stream with progress events.
- Events include document-level progress, errors, and a final completion event.
- The polling endpoint remains available as a fallback.

## Implementation Details

### 1. Define Reindex Events

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum ReindexEvent {
    /// Reindex has started.
    #[serde(rename = "started")]
    Started { total_documents: usize },

    /// A single document has been indexed.
    #[serde(rename = "progress")]
    Progress {
        document_slug: String,
        document_title: String,
        current: usize,
        total: usize,
        percent: u32,
    },

    /// A document failed to index (non-fatal, reindex continues).
    #[serde(rename = "error")]
    Error {
        document_slug: String,
        message: String,
        current: usize,
        total: usize,
    },

    /// Reindex completed.
    #[serde(rename = "done")]
    Done {
        total_indexed: usize,
        total_errors: usize,
        elapsed_seconds: f64,
    },
}
```

### 2. Broadcast Channel
Use a `tokio::sync::broadcast` channel to decouple the reindex task from SSE consumers:

```rust
pub struct ReindexState {
    pub is_running: AtomicBool,
    pub progress: AtomicU32,
    /// Broadcast sender for SSE consumers. Capacity should handle burst of events.
    pub event_tx: broadcast::Sender<ReindexEvent>,
}

impl Default for ReindexState {
    fn default() -> Self {
        let (event_tx, _) = broadcast::channel(128);
        Self {
            is_running: AtomicBool::new(false),
            progress: AtomicU32::new(0),
            event_tx,
        }
    }
}
```

### 3. Emit Events in the Reindex Loop
Update `run_reindex` in `src/rag/reindex.rs` to send events:

```rust
// Before the loop:
let _ = reindex.event_tx.send(ReindexEvent::Started { total_documents: total });

// After each document (success):
let _ = reindex.event_tx.send(ReindexEvent::Progress {
    document_slug: doc.slug.clone(),
    document_title: doc.title.clone(),
    current: i + 1,
    total,
    percent: pct,
});

// On document error:
let _ = reindex.event_tx.send(ReindexEvent::Error {
    document_slug: doc.slug.clone(),
    message: e.to_string(),
    current: i + 1,
    total,
});

// After the loop:
let _ = reindex.event_tx.send(ReindexEvent::Done {
    total_indexed: success_count,
    total_errors: error_count,
    elapsed_seconds: start.elapsed().as_secs_f64(),
});
```

Using `let _ =` because it's fine to silently drop events when no SSE client is connected (the broadcast send fails if there are no receivers).

### 4. SSE Endpoint
Add `GET /api/v1/admin/rag/reindex/stream` that subscribes to the broadcast channel:

```rust
async fn reindex_stream(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.reindex.event_tx.subscribe();

    let stream = async_stream::stream! {
        while let Ok(event) = rx.recv().await {
            let json = serde_json::to_string(&event).unwrap_or_default();
            yield Ok(Event::default().data(json));

            // Close the stream after Done event
            if matches!(event, ReindexEvent::Done { .. }) {
                break;
            }
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}
```

### 5. Frontend Update
Replace the polling mechanism with an `EventSource` connection:

```javascript
const evtSource = new EventSource('/api/v1/admin/rag/reindex/stream');
evtSource.onmessage = (event) => {
    const data = JSON.parse(event.data);
    switch (data.type) {
        case 'started':
            // Show progress bar
            break;
        case 'progress':
            // Update progress bar + show current document
            break;
        case 'error':
            // Show warning for failed document
            break;
        case 'done':
            // Show completion summary
            evtSource.close();
            break;
    }
};
```

(In Leptos, this would use `leptos::create_signal` with a `gloo_net::eventsource::EventSource` or equivalent.)

### 6. Keep Polling Endpoint
Keep the existing `GET /api/v1/admin/rag/reindex/status` endpoint unchanged as a fallback for clients that don't support SSE (e.g., monitoring scripts, health checks).

## Files to Modify
| File | Change |
|------|--------|
| `src/rag/reindex.rs` | Add `broadcast::Sender` to `ReindexState`, emit events in the loop, track error count and elapsed time |
| `src/api/admin.rs` (or equivalent route file) | Add `GET /api/v1/admin/rag/reindex/stream` SSE endpoint |
| Admin frontend component | Replace polling with EventSource |

## Benefits
- **Real-time feedback**: Admins see progress instantly without polling delay.
- **Granular visibility**: Know which document is being indexed and which ones failed.
- **Error reporting**: Inline error events surface problems without log diving.
- **Completion summary**: Total indexed, errors, and elapsed time in one event.
- **Reduced server load**: No repeated polling requests during long reindex operations.

## Risks and Mitigations
| Risk | Mitigation |
|------|-----------|
| Broadcast channel memory if events pile up | Capacity of 128 is sufficient; receivers that lag will get `RecvError::Lagged` and can reconnect |
| SSE connection drops mid-reindex | Client can reconnect and get future events; polling fallback provides current state |
| Multiple admins watching the same reindex | Broadcast channel naturally supports multiple receivers |

## Dependencies
None. Self-contained. However, it pairs well with:
- **ENH-006 (Embedding Rate Limiting)**: The delay between batches makes the progress stream more meaningful (events are spaced out rather than all arriving in a burst).

## Effort Estimate
Small. The `tokio::sync::broadcast` channel is straightforward. The SSE endpoint follows the same pattern as the existing chat SSE stream. Main work is the frontend update.
