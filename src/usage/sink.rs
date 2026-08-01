//! Background writer for the LLM usage event log.
//!
//! [`record`](super::record) is called from deep inside the LLM services, on
//! the request path. Writing to Mongo there would add database latency to every
//! LLM call and couple the services to a repository they otherwise don't need,
//! so events go through a bounded channel to a background task instead.
//!
//! The sink is installed once at startup, like the Prometheus recorder. When
//! `usage.event_log` is off nothing is installed and [`emit`] is a no-op.
//!
//! **Backpressure is a drop, not a block.** If the writer falls behind, events
//! are discarded and counted in `lekton_llm_usage_events_dropped_total`. Losing
//! accounting detail is the right trade against stalling a user's chat, and the
//! Prometheus totals — which never go through this path — stay complete.

use std::sync::Arc;
use std::sync::OnceLock;

use tokio::sync::mpsc;

use crate::db::usage_models::LlmUsageEvent;
use crate::db::usage_repository::UsageEventRepository;

/// How many events may queue before new ones are dropped.
const CHANNEL_CAPACITY: usize = 1_024;
/// How many events one insert may carry.
const BATCH_SIZE: usize = 64;

static SINK: OnceLock<mpsc::Sender<LlmUsageEvent>> = OnceLock::new();

/// Start the background writer and register it as the process-wide sink.
///
/// Calling this more than once leaves the first sink in place; the extra
/// writer is dropped. Call it once at startup.
pub fn install(repo: Arc<dyn UsageEventRepository>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    if SINK.set(tx).is_err() {
        tracing::warn!("LLM usage event sink already installed — ignoring");
        return;
    }
    tokio::spawn(write_loop(repo, rx));
    tracing::info!("LLM usage event log enabled");
}

/// Queue an event, dropping it if the writer is behind.
pub fn emit(event: LlmUsageEvent) {
    let Some(sink) = SINK.get() else {
        return; // event log disabled
    };
    if sink.try_send(event).is_err() {
        metrics::counter!("lekton_llm_usage_events_dropped_total").increment(1);
    }
}

async fn write_loop(repo: Arc<dyn UsageEventRepository>, mut rx: mpsc::Receiver<LlmUsageEvent>) {
    let mut batch = Vec::with_capacity(BATCH_SIZE);

    while rx.recv_many(&mut batch, BATCH_SIZE).await > 0 {
        let events = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
        let count = events.len();
        if let Err(e) = repo.record_events(events).await {
            // Never propagate: the accounting log must not take the process
            // down, and the Prometheus totals already recorded these calls.
            tracing::error!(count, error = %e, "failed to persist LLM usage events");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::usage_models::LlmUsageEvent;
    use std::sync::Mutex;

    struct RecordingRepo {
        seen: Arc<Mutex<Vec<LlmUsageEvent>>>,
    }

    #[async_trait::async_trait]
    impl UsageEventRepository for RecordingRepo {
        async fn record_events(
            &self,
            events: Vec<LlmUsageEvent>,
        ) -> Result<(), crate::error::AppError> {
            self.seen.lock().unwrap().extend(events);
            Ok(())
        }
    }

    fn event(feature: &str) -> LlmUsageEvent {
        LlmUsageEvent {
            actor_kind: "user".into(),
            actor_id: Some("u1".into()),
            feature: feature.into(),
            model: "m".into(),
            prompt_tokens: 1,
            completion_tokens: 2,
            estimated: false,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn write_loop_batches_and_persists() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let repo = Arc::new(RecordingRepo { seen: seen.clone() });
        let (tx, rx) = mpsc::channel(8);

        for feature in ["chat", "hyde", "learn"] {
            tx.send(event(feature)).await.expect("send");
        }
        drop(tx); // closes the channel so the loop terminates

        write_loop(repo, rx).await;

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].feature, "chat");
    }

    #[tokio::test]
    async fn emit_without_a_sink_is_a_noop() {
        // The global sink is never installed in tests; emitting must not panic
        // and must not block.
        emit(event("chat"));
    }
}
