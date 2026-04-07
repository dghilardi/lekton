//! Chat session and message models for RAG conversations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A chat conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    /// Unique session ID (UUID).
    pub id: String,
    /// ID of the user who owns this session.
    pub user_id: String,
    /// Short title (auto-generated from first message, editable).
    pub title: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// A single message in a chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message ID (UUID).
    pub id: String,
    /// The session this message belongs to.
    pub session_id: String,
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Message content (plain text / markdown).
    pub content: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
