//! Chat session and message models for RAG conversations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A document reference used to ground an assistant response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceReference {
    /// Document slug for building navigation links.
    pub document_slug: String,
    /// Human-readable document title.
    pub document_title: String,
    /// Cosine similarity score from retrieval.
    pub score: f32,
    /// Optional short preview snippet for UI display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

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
    /// Source references used to generate this response (assistant messages only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceReference>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_sources_roundtrip() {
        let message = ChatMessage {
            id: "m1".into(),
            session_id: "s1".into(),
            role: "assistant".into(),
            content: "Answer".into(),
            sources: Some(vec![SourceReference {
                document_slug: "docs/getting-started".into(),
                document_title: "Getting Started".into(),
                score: 0.91,
                snippet: Some("Quick start guide".into()),
            }]),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&message).unwrap();
        let decoded: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.sources.as_ref().unwrap().len(), 1);
        assert_eq!(
            decoded.sources.as_ref().unwrap()[0].document_slug,
            "docs/getting-started"
        );
    }

    #[test]
    fn chat_message_without_sources_defaults_to_none() {
        let json = r#"{
            "id":"m1",
            "session_id":"s1",
            "role":"assistant",
            "content":"Answer",
            "created_at":{"$date":{"$numberLong":"1704067200000"}}
        }"#;

        let decoded: ChatMessage = serde_json::from_str(json).unwrap();
        assert!(decoded.sources.is_none());
    }
}
