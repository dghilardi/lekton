//! Coarse calibration: decide whether the next lesson should advance to new
//! material or reinforce, based on recent quiz performance.

use crate::db::learn_models::{LearningRecord, LearningRecordKind};

/// What the next lesson should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextFocus {
    /// Move on to new, uncovered material.
    Advance,
    /// Revisit and re-explain; the learner is struggling.
    Reinforce,
}

/// Below this average recent score, reinforce instead of advancing.
const REINFORCE_THRESHOLD: f32 = 0.6;
/// How many recent quiz results to average over.
const RECENT_WINDOW: usize = 3;

impl NextFocus {
    /// A short instruction handed to the tutor prompt.
    pub fn directive(self) -> &'static str {
        match self {
            NextFocus::Advance => {
                "The learner is doing well — advance to a new sub-topic they have not seen yet."
            }
            NextFocus::Reinforce => {
                "The learner struggled on recent quizzes — reinforce the fundamentals and \
                 re-explain the last concepts more simply before introducing anything new."
            }
        }
    }
}

/// Decide the next focus from a path's records (expected most-recent-first).
/// With no quiz history, advance.
pub fn calibrate(records: &[LearningRecord]) -> NextFocus {
    let recent: Vec<f32> = records
        .iter()
        .filter_map(|r| match &r.kind {
            LearningRecordKind::QuizResult { score, .. } => Some(*score),
            _ => None,
        })
        .take(RECENT_WINDOW)
        .collect();

    if recent.is_empty() {
        return NextFocus::Advance;
    }
    let avg = recent.iter().sum::<f32>() / recent.len() as f32;
    if avg < REINFORCE_THRESHOLD {
        NextFocus::Reinforce
    } else {
        NextFocus::Advance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn quiz_record(score: f32) -> LearningRecord {
        LearningRecord {
            id: "r".into(),
            path_id: "p".into(),
            lesson_id: None,
            user_id: "u".into(),
            kind: LearningRecordKind::QuizResult {
                per_question: vec![],
                score,
            },
            created_at: Utc::now(),
        }
    }

    #[test]
    fn no_history_advances() {
        assert_eq!(calibrate(&[]), NextFocus::Advance);
    }

    #[test]
    fn low_recent_scores_reinforce() {
        let records = vec![quiz_record(0.3), quiz_record(0.5)];
        assert_eq!(calibrate(&records), NextFocus::Reinforce);
    }

    #[test]
    fn high_recent_scores_advance() {
        let records = vec![quiz_record(1.0), quiz_record(0.8)];
        assert_eq!(calibrate(&records), NextFocus::Advance);
    }

    #[test]
    fn only_recent_window_counts() {
        // Most-recent-first: three perfect recent scores outweigh older failures.
        let records = vec![
            quiz_record(1.0),
            quiz_record(1.0),
            quiz_record(1.0),
            quiz_record(0.0),
            quiz_record(0.0),
        ];
        assert_eq!(calibrate(&records), NextFocus::Advance);
    }

    #[test]
    fn insights_are_ignored() {
        let records = vec![LearningRecord {
            id: "r".into(),
            path_id: "p".into(),
            lesson_id: None,
            user_id: "u".into(),
            kind: LearningRecordKind::Insight { text: "x".into() },
            created_at: Utc::now(),
        }];
        assert_eq!(calibrate(&records), NextFocus::Advance);
    }
}
