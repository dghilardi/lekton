//! Per-section calibration: decide what the next lesson should do, based on how
//! the learner has performed on each section they've been taught.
//!
//! This is a Zone-of-Proximal-Development proxy: don't re-teach sections the
//! learner has mastered, revisit the ones they're weak on before moving on, and
//! otherwise advance to new material.

/// What the next lesson should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextFocus {
    /// Move on to new, uncovered material.
    Advance,
    /// Revisit and re-explain; the learner is struggling on a section.
    Reinforce,
}

/// A graded lesson outcome tied to the section it taught. Callers pass these
/// most-recent-first.
#[derive(Debug, Clone)]
pub struct LessonOutcome {
    /// The section this lesson taught (see `LessonSource::key`).
    pub section_key: String,
    /// Quiz score in `0.0..=1.0`.
    pub score: f32,
}

/// The plan for the next lesson.
#[derive(Debug, Clone, PartialEq)]
pub struct NextPlan {
    pub focus: NextFocus,
    /// Section keys the learner has mastered — the tutor should avoid
    /// re-teaching these.
    pub mastered: Vec<String>,
    /// When reinforcing, the specific weak section to revisit.
    pub reinforce_key: Option<String>,
}

/// At or above this (latest) score, a section counts as mastered.
const MASTERY_THRESHOLD: f32 = 0.8;
/// Below this (latest) score, a section is weak and worth reinforcing.
const REINFORCE_THRESHOLD: f32 = 0.6;

impl NextPlan {
    /// A short instruction handed to the tutor prompt.
    pub fn directive(&self) -> String {
        match (self.focus, &self.reinforce_key) {
            (NextFocus::Reinforce, Some(key)) => format!(
                "The learner struggled with the section \"{key}\" — revisit it and \
                 re-explain those fundamentals more simply before introducing \
                 anything new."
            ),
            (NextFocus::Reinforce, None) => "The learner has been struggling — \
                 reinforce the fundamentals before advancing."
                .to_string(),
            (NextFocus::Advance, _) => "The learner is doing well — advance to a new \
                 sub-topic they have not seen yet."
                .to_string(),
        }
    }
}

/// Plan the next lesson from the learner's per-section history (most-recent
/// first). With no history, advance.
///
/// Mastery is judged from each section's **latest** attempt, so a section the
/// learner initially failed but later passed counts as mastered (and is no
/// longer flagged weak). The section to reinforce is the most recently attempted
/// one still below the reinforce threshold.
pub fn plan_next(history: &[LessonOutcome]) -> NextPlan {
    // Latest score per section (history is most-recent-first, so the first time
    // we see a key is its latest attempt), preserving recency order.
    let mut latest: Vec<(&str, f32)> = Vec::new();
    for o in history {
        if !latest.iter().any(|(k, _)| *k == o.section_key) {
            latest.push((&o.section_key, o.score));
        }
    }

    let mastered: Vec<String> = latest
        .iter()
        .filter(|(_, s)| *s >= MASTERY_THRESHOLD)
        .map(|(k, _)| (*k).to_string())
        .collect();

    let reinforce_key = latest
        .iter()
        .find(|(_, s)| *s < REINFORCE_THRESHOLD)
        .map(|(k, _)| (*k).to_string());

    let focus = if reinforce_key.is_some() {
        NextFocus::Reinforce
    } else {
        NextFocus::Advance
    };

    NextPlan {
        focus,
        mastered,
        reinforce_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(key: &str, score: f32) -> LessonOutcome {
        LessonOutcome {
            section_key: key.into(),
            score,
        }
    }

    #[test]
    fn no_history_advances() {
        let plan = plan_next(&[]);
        assert_eq!(plan.focus, NextFocus::Advance);
        assert!(plan.mastered.is_empty());
        assert!(plan.reinforce_key.is_none());
    }

    #[test]
    fn a_weak_section_is_reinforced_by_name() {
        let history = vec![outcome("docs/kafka#partitions", 0.33)];
        let plan = plan_next(&history);
        assert_eq!(plan.focus, NextFocus::Reinforce);
        assert_eq!(plan.reinforce_key.as_deref(), Some("docs/kafka#partitions"));
        assert!(plan.directive().contains("docs/kafka#partitions"));
    }

    #[test]
    fn mastered_sections_are_listed_and_not_reinforced() {
        let history = vec![
            outcome("docs/kafka#offsets", 1.0),
            outcome("docs/kafka#partitions", 0.8),
        ];
        let plan = plan_next(&history);
        assert_eq!(plan.focus, NextFocus::Advance);
        assert!(plan.mastered.contains(&"docs/kafka#offsets".to_string()));
        assert!(plan.mastered.contains(&"docs/kafka#partitions".to_string()));
    }

    #[test]
    fn improving_on_a_section_clears_the_weakness() {
        // Most-recent-first: the learner retried partitions and passed, so the
        // old failure no longer counts.
        let history = vec![
            outcome("docs/kafka#partitions", 1.0), // latest
            outcome("docs/kafka#partitions", 0.0), // earlier
        ];
        let plan = plan_next(&history);
        assert_eq!(plan.focus, NextFocus::Advance);
        assert!(plan.reinforce_key.is_none());
        assert!(plan.mastered.contains(&"docs/kafka#partitions".to_string()));
    }

    #[test]
    fn reinforces_the_most_recent_weak_section() {
        let history = vec![
            outcome("docs/kafka#consumers", 0.5), // most recent weak
            outcome("docs/kafka#brokers", 0.9),
            outcome("docs/kafka#topics", 0.2), // older weak
        ];
        let plan = plan_next(&history);
        assert_eq!(plan.focus, NextFocus::Reinforce);
        assert_eq!(plan.reinforce_key.as_deref(), Some("docs/kafka#consumers"));
    }
}
