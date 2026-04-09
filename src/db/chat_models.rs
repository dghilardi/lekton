//! Chat session and message models for RAG conversations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Feedback rating for an AI response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackRating {
    Positive,
    Negative,
}

/// User feedback on a single assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageFeedback {
    /// Unique feedback ID (UUID).
    pub id: String,
    /// The message this feedback refers to.
    pub message_id: String,
    /// The session the message belongs to (for efficient queries).
    pub session_id: String,
    /// The user who submitted the feedback.
    pub user_id: String,
    /// Thumbs up or thumbs down.
    pub rating: FeedbackRating,
    /// Optional free-text comment (typically for negative feedback).
    pub comment: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

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
