//! Persisted record of a single LLM call.
//!
//! Written by the event log (`usage.event_log`) to answer questions the
//! Prometheus counters cannot: *who* spent what. Prometheus stays
//! low-cardinality; per-caller detail lives here and expires on a TTL.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Who a call is billed to.
///
/// A machine PAT has no user account behind it, and background indexing has no
/// caller at all — both still cost money, so they get their own kinds rather
/// than being lumped in with a user or dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageKey {
    User(String),
    ServiceToken(String),
    Anonymous,
    /// Background work with no caller: indexing, reindex, attachment extraction.
    System,
}

impl UsageKey {
    /// Low-cardinality discriminant, safe to use as a metric label.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::ServiceToken(_) => "service_token",
            Self::Anonymous => "anonymous",
            Self::System => "system",
        }
    }

    /// The identity within `kind`, if there is one.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::User(id) | Self::ServiceToken(id) => Some(id),
            Self::Anonymous | Self::System => None,
        }
    }
}

/// One LLM call, as stored in `llm_usage_events`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmUsageEvent {
    /// `user`, `service_token`, `anonymous` or `system`.
    pub actor_kind: String,
    /// User or token id; `None` for anonymous and system calls.
    pub actor_id: Option<String>,
    /// Which LLM call this was (`chat`, `learn`, `embedding`, …).
    pub feature: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// `true` when the counts were estimated because the provider reported none.
    pub estimated: bool,
    /// Indexed with a TTL, so events prune themselves.
    ///
    /// Must be a BSON date, not the string chrono serialises to by default:
    /// MongoDB only expires date-typed fields, so without this the TTL index
    /// silently keeps every event forever — and range queries over the window
    /// match nothing.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_key_exposes_kind_and_id() {
        let key = UsageKey::User("u1".into());

        assert_eq!(key.kind(), "user");
        assert_eq!(key.id(), Some("u1"));
    }

    #[test]
    fn callerless_keys_have_no_id() {
        assert_eq!(UsageKey::System.id(), None);
        assert_eq!(UsageKey::Anonymous.id(), None);
        assert_eq!(UsageKey::System.kind(), "system");
    }
}
