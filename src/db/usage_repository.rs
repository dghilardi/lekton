//! Repository for the LLM usage event log.

use async_trait::async_trait;

use crate::db::usage_models::LlmUsageEvent;
use crate::error::AppError;

#[async_trait]
pub trait UsageEventRepository: Send + Sync {
    /// Append a batch of events.
    ///
    /// Batched rather than one-at-a-time because events arrive at the rate of
    /// LLM calls and each one is tiny; the writer accumulates them so a busy
    /// chat turn costs one insert instead of six.
    async fn record_events(&self, events: Vec<LlmUsageEvent>) -> Result<(), AppError>;
}

// ── MongoDB implementation ───────────────────────────────────────────────────

#[cfg(feature = "ssr")]
pub struct MongoUsageEventRepository {
    events: mongodb::Collection<LlmUsageEvent>,
}

#[cfg(feature = "ssr")]
impl MongoUsageEventRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            events: db.collection("llm_usage_events"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl UsageEventRepository for MongoUsageEventRepository {
    async fn record_events(&self, events: Vec<LlmUsageEvent>) -> Result<(), AppError> {
        if events.is_empty() {
            return Ok(());
        }
        self.events
            .insert_many(events)
            .await
            .map_err(|e| AppError::Internal(format!("mongo insert llm_usage_events: {e}")))?;
        Ok(())
    }
}
