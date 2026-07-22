//! Domain models for Learn mode: teach-style personalized lessons grounded on
//! the internal documentation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a learning path is scoped to — the seed of the "mission". Semantic
/// retrieval finds the relevant documents; the whole document(s) then ground
/// the lesson.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LearningScope {
    /// A single document, addressed by slug.
    Document { slug: String },
    /// All documents carrying a tag.
    Tag { tag: String },
    /// A free-text topic.
    Topic { text: String },
}

/// A learning path: one progressive journey for a user over a scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningPath {
    /// Unique path ID (UUID).
    pub id: String,
    /// ID of the user who owns this path.
    pub user_id: String,
    /// The scope grounding every lesson in this path.
    pub scope: LearningScope,
    /// Short title shown in the dashboard.
    pub title: String,
    /// Section anchors / document slugs already covered, used to calibrate the
    /// next lesson (avoid repeating covered ground).
    #[serde(default)]
    pub covered_anchors: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// A citation linking lesson content back to a source document/section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LessonCitation {
    /// Slug of the cited document, for building navigation links.
    pub document_slug: String,
    /// URL-safe section anchor to append to `document_slug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_anchor: Option<String>,
    /// Short verbatim quote grounding the citation.
    pub quote: String,
}

/// A pointer to the single most valuable source to read/watch next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LessonSource {
    pub document_slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_anchor: Option<String>,
}

/// A single multiple-choice quiz question. Options should be uniform in length
/// so formatting gives no clue to the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuizQuestion {
    pub prompt: String,
    pub options: Vec<String>,
    /// 0-based index of the correct option.
    pub correct_index: usize,
    /// Shown after answering, regardless of correctness.
    pub explanation: String,
}

/// A generated lesson: one tightly-scoped, self-contained teaching unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    /// Unique lesson ID (UUID).
    pub id: String,
    /// The path this lesson belongs to.
    pub path_id: String,
    /// Denormalised owner, for efficient per-user deletion.
    pub user_id: String,
    /// 1-based position within the path.
    pub seq: u32,
    pub title: String,
    /// Sanitized HTML of the lesson body (see `rendering::markdown`).
    pub body_html: String,
    #[serde(default)]
    pub citations: Vec<LessonCitation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_source: Option<LessonSource>,
    #[serde(default)]
    pub quiz: Vec<QuizQuestion>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// A calibration signal recorded as the user progresses — the teaching
/// equivalent of an architectural decision record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LearningRecordKind {
    /// Result of a lesson's quiz. `per_question[i]` is whether question `i` was
    /// answered correctly; `score` is the fraction correct in `0.0..=1.0`.
    QuizResult { per_question: Vec<bool>, score: f32 },
    /// A non-obvious insight worth steering future lessons.
    Insight { text: String },
}

/// A learning record tied to a path (and optionally a specific lesson).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningRecord {
    /// Unique record ID (UUID).
    pub id: String,
    /// The path this record belongs to.
    pub path_id: String,
    /// The lesson that produced this record, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lesson_id: Option<String>,
    /// Denormalised owner, for efficient per-user deletion.
    pub user_id: String,
    pub kind: LearningRecordKind,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_path_roundtrip_preserves_scope() {
        let path = LearningPath {
            id: "p1".into(),
            user_id: "u1".into(),
            scope: LearningScope::Tag {
                tag: "kafka".into(),
            },
            title: "Kafka basics".into(),
            covered_anchors: vec!["docs/kafka#intro".into()],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_string(&path).unwrap();
        let decoded: LearningPath = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.scope,
            LearningScope::Tag {
                tag: "kafka".into()
            }
        );
        assert_eq!(
            decoded.covered_anchors,
            vec!["docs/kafka#intro".to_string()]
        );
    }

    #[test]
    fn lesson_roundtrip_preserves_quiz_and_citations() {
        let lesson = Lesson {
            id: "l1".into(),
            path_id: "p1".into(),
            user_id: "u1".into(),
            seq: 1,
            title: "Topics and partitions".into(),
            body_html: "<p>A topic is split into partitions.</p>".into(),
            citations: vec![LessonCitation {
                document_slug: "docs/kafka".into(),
                section_anchor: Some("partitions".into()),
                quote: "Each topic is divided into partitions.".into(),
            }],
            primary_source: Some(LessonSource {
                document_slug: "docs/kafka".into(),
                section_anchor: Some("partitions".into()),
            }),
            quiz: vec![QuizQuestion {
                prompt: "What is a partition?".into(),
                options: vec!["An ordered log".into(), "A random heap".into()],
                correct_index: 0,
                explanation: "Partitions are append-only ordered logs.".into(),
            }],
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&lesson).unwrap();
        let decoded: Lesson = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.quiz.len(), 1);
        assert_eq!(decoded.quiz[0].correct_index, 0);
        assert_eq!(
            decoded.citations[0].section_anchor.as_deref(),
            Some("partitions")
        );
    }

    #[test]
    fn learning_record_kind_is_tagged() {
        let record = LearningRecord {
            id: "r1".into(),
            path_id: "p1".into(),
            lesson_id: Some("l1".into()),
            user_id: "u1".into(),
            kind: LearningRecordKind::QuizResult {
                per_question: vec![true, false],
                score: 0.5,
            },
            created_at: Utc::now(),
        };

        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["kind"]["kind"], "quiz_result");
        let decoded: LearningRecord = serde_json::from_value(json).unwrap();
        assert_eq!(
            decoded.kind,
            LearningRecordKind::QuizResult {
                per_question: vec![true, false],
                score: 0.5,
            }
        );
    }
}
