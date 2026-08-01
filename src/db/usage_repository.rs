//! Repository for the LLM usage event log.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::db::usage_models::LlmUsageEvent;
use crate::error::AppError;

/// One caller's usage of one model over a window.
///
/// Grouped by model rather than pre-summed: tokens from different models are
/// not comparable in price, so the credit figure has to be computed after the
/// aggregation, against the price list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageByModel {
    pub actor_kind: String,
    pub actor_id: Option<String>,
    pub model: String,
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[async_trait]
pub trait UsageEventRepository: Send + Sync {
    /// Append a batch of events.
    ///
    /// Batched rather than one-at-a-time because events arrive at the rate of
    /// LLM calls and each one is tiny; the writer accumulates them so a busy
    /// chat turn costs one insert instead of six.
    async fn record_events(&self, events: Vec<LlmUsageEvent>) -> Result<(), AppError>;

    /// Usage since `since`, grouped by caller and model.
    async fn usage_by_model(&self, since: DateTime<Utc>) -> Result<Vec<UsageByModel>, AppError>;
}

/// Read an aggregated count, whatever width MongoDB chose for it.
///
/// `$sum: 1` yields an Int32 while `$sum: "$field"` over i64 values yields an
/// Int64, so a reader that only accepts one silently reports zero for the other
/// — which is how the report first displayed every caller as having made no
/// calls at all.
#[cfg(feature = "ssr")]
fn count(row: &mongodb::bson::Document, key: &str) -> u64 {
    use mongodb::bson::Bson;

    match row.get(key) {
        Some(Bson::Int32(n)) => (*n).max(0) as u64,
        Some(Bson::Int64(n)) => (*n).max(0) as u64,
        Some(Bson::Double(n)) => n.max(0.0) as u64,
        _ => 0,
    }
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

    async fn usage_by_model(&self, since: DateTime<Utc>) -> Result<Vec<UsageByModel>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, Document};

        let pipeline = vec![
            doc! { "$match": { "created_at": { "$gte": mongodb::bson::DateTime::from_chrono(since) } } },
            doc! { "$group": {
                "_id": { "actor_kind": "$actor_kind", "actor_id": "$actor_id", "model": "$model" },
                "calls": { "$sum": 1 },
                "prompt_tokens": { "$sum": "$prompt_tokens" },
                "completion_tokens": { "$sum": "$completion_tokens" },
            }},
        ];

        let rows: Vec<Document> = self
            .events
            .aggregate(pipeline)
            .await
            .map_err(|e| AppError::Internal(format!("mongo aggregate llm_usage_events: {e}")))?
            .try_collect()
            .await
            .map_err(|e| AppError::Internal(format!("mongo collect llm_usage_events: {e}")))?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let id = row.get_document("_id").ok()?;
                Some(UsageByModel {
                    actor_kind: id.get_str("actor_kind").unwrap_or("unknown").to_string(),
                    actor_id: id.get_str("actor_id").ok().map(ToOwned::to_owned),
                    model: id.get_str("model").unwrap_or("unknown").to_string(),
                    calls: count(&row, "calls"),
                    prompt_tokens: count(&row, "prompt_tokens"),
                    completion_tokens: count(&row, "completion_tokens"),
                })
            })
            .collect())
    }
}
