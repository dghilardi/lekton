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
    /// Deepest heading title for section-level citations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_title: Option<String>,
    /// URL-safe section anchor to append to `document_slug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_anchor: Option<String>,
    /// Cosine similarity score from retrieval.
    pub score: f32,
    /// Optional short preview snippet for UI display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Asset key when this source is an attachment (links to
    /// `/api/v1/assets/{key}`); `None` for document sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_key: Option<String>,
    /// 1-based page number within the attachment, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_page: Option<u32>,
}

/// Fewest words a source preview needs to say more than a heading does.
const MIN_SNIPPET_WORDS: usize = 4;

/// A short plain-text preview of a retrieved chunk, for the sources list.
///
/// Markdown structure that says nothing on its own — headings, code fences,
/// table and horizontal rules — is dropped, so a chunk starting with
/// `## Usage` previews its first sentence instead. `None` when nothing
/// readable is left.
pub fn source_snippet(text: &str, max_chars: usize) -> Option<String> {
    let prose = text
        .lines()
        .map(str::trim)
        .filter(|line| !is_markup_only(line))
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>();
    if word_count(prose.iter().copied()) < MIN_SNIPPET_WORDS {
        return None;
    }
    let normalized = prose.join(" ");
    let mut chars = normalized.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    Some(if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    })
}

/// Words with at least one letter or digit, so table pipes and list
/// bullets do not count.
fn word_count<'a>(words: impl Iterator<Item = &'a str>) -> usize {
    words
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

fn is_markup_only(line: &str) -> bool {
    line.is_empty()
        || line.starts_with('#')
        || line.starts_with("```")
        || line.starts_with("~~~")
        || line
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | '=' | '*' | '_' | ' '))
}

impl SourceReference {
    /// The preview to show. Also cleans previews saved before
    /// [`source_snippet`] existed, such as a bare `## Usage`.
    pub fn display_snippet(&self) -> Option<String> {
        let snippet = self.snippet.as_deref()?.trim();
        let snippet = snippet.trim_start_matches('#').trim_start();
        (word_count(snippet.split_whitespace()) >= MIN_SNIPPET_WORDS).then(|| snippet.to_string())
    }
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

/// How an assistant message ended.
///
/// Messages stored before this field existed deserialize as [`Complete`],
/// which is what the old code assumed for every saved answer.
///
/// [`Complete`]: ChatMessageStatus::Complete
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatMessageStatus {
    /// The model finished the answer (it reported a finish reason).
    #[default]
    Complete,
    /// The token stream ended before the model finished, e.g. because a
    /// proxy or the provider closed the connection mid-answer.
    Interrupted,
    /// The token stream failed with an error partway through.
    Error,
    /// The reader stopped the answer before the model finished it.
    Stopped,
}

impl ChatMessageStatus {
    pub fn is_complete(self) -> bool {
        self == Self::Complete
    }
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
    /// Whether the answer is complete. Always `Complete` for user messages.
    #[serde(default)]
    pub status: ChatMessageStatus,
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
                section_title: Some("Installation".into()),
                section_anchor: Some("installation".into()),
                score: 0.91,
                snippet: Some("Quick start guide".into()),
                attachment_key: None,
                source_page: None,
            }]),
            status: ChatMessageStatus::Complete,
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&message).unwrap();
        let decoded: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.sources.as_ref().unwrap().len(), 1);
        assert_eq!(
            decoded.sources.as_ref().unwrap()[0].document_slug,
            "docs/getting-started"
        );
        assert_eq!(
            decoded.sources.as_ref().unwrap()[0]
                .section_anchor
                .as_deref(),
            Some("installation")
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

    #[test]
    fn snippet_skips_headings_and_markup() {
        let chunk = "## Usage\n\n```bash\nlekton-sync --dry-run\n```\n\nRun the sync from the CI job after the build.";
        assert_eq!(
            source_snippet(chunk, 180).as_deref(),
            Some("lekton-sync --dry-run Run the sync from the CI job after the build.")
        );
        assert_eq!(source_snippet("## Usage", 180), None);
        assert_eq!(source_snippet("| A | B |\n|---|---|", 180), None);
        assert_eq!(
            source_snippet("one two three four five six", 10).as_deref(),
            Some("one two th...")
        );
    }

    #[test]
    fn stored_heading_only_snippets_are_hidden() {
        let mut source = SourceReference {
            document_slug: "docs/a".into(),
            document_title: "A".into(),
            section_title: None,
            section_anchor: None,
            score: 0.7,
            snippet: Some("## Usage".into()),
            attachment_key: None,
            source_page: None,
        };
        assert_eq!(source.display_snippet(), None);
        source.snippet = Some("## Usage Run the sync from CI.".into());
        assert_eq!(
            source.display_snippet().as_deref(),
            Some("Usage Run the sync from CI.")
        );
    }

    #[test]
    fn chat_message_without_status_is_complete() {
        let json = r#"{
            "id":"m1",
            "session_id":"s1",
            "role":"assistant",
            "content":"Answer",
            "created_at":{"$date":{"$numberLong":"1704067200000"}}
        }"#;

        let decoded: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.status, ChatMessageStatus::Complete);
    }

    #[test]
    fn chat_message_status_roundtrips_as_snake_case() {
        let json = serde_json::to_string(&ChatMessageStatus::Interrupted).unwrap();
        assert_eq!(json, r#""interrupted""#);
        let decoded: ChatMessageStatus = serde_json::from_str(r#""error""#).unwrap();
        assert_eq!(decoded, ChatMessageStatus::Error);
        assert!(!decoded.is_complete());
    }
}
