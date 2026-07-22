//! Orchestration for Learn mode: turns generator output into persisted paths,
//! lessons, and records, enforcing per-user ownership.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::auth::models::UserContext;
use crate::db::learn_models::{
    LearningPath, LearningRecord, LearningRecordKind, LearningScope, Lesson, LessonSource,
    LessonView, QuizGrade, QuizQuestion,
};
use crate::db::learn_repository::LearnRepository;
use crate::error::AppError;
use crate::learn::calibration::{plan_next, LessonOutcome};
use crate::learn::generator::{GeneratedLesson, LessonGenerator};
use crate::learn::token::{QuizKey, QuizSealer};

pub struct LearnService {
    repo: Arc<dyn LearnRepository>,
    generator: LessonGenerator,
    /// Seals ephemeral quiz answer keys so they can be graded server-side
    /// without the answers ever reaching the client.
    sealer: QuizSealer,
}

impl LearnService {
    pub fn new(repo: Arc<dyn LearnRepository>, generator: LessonGenerator) -> Self {
        Self {
            repo,
            generator,
            sealer: QuizSealer::new(),
        }
    }

    /// Start a new learning path for the user. `mission` is the learner's own
    /// reason for studying this scope; it grounds every lesson in the path.
    pub async fn start_path(
        &self,
        user_ctx: &UserContext,
        scope: LearningScope,
        mission: Option<String>,
    ) -> Result<LearningPath, AppError> {
        let now = Utc::now();
        let path = LearningPath {
            id: Uuid::new_v4().to_string(),
            user_id: user_ctx.user.user_id.clone(),
            title: default_title(&scope),
            scope,
            mission: normalize_mission(mission),
            covered_anchors: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        self.repo.create_path(path.clone()).await?;
        Ok(path)
    }

    /// Generate, persist, and return the next lesson for a path.
    pub async fn generate_next(
        &self,
        user_ctx: &UserContext,
        path_id: &str,
    ) -> Result<LessonView, AppError> {
        let path = self.owned_path(user_ctx, path_id).await?;

        // Per-section calibration: build the learner's history (which section
        // each graded lesson taught, and how they scored) and plan the next
        // lesson — avoid mastered sections, reinforce weak ones, else advance.
        let lessons = self.repo.list_lessons_for_path(path_id).await?;
        let records = self.repo.list_records_for_path(path_id).await?;
        let history = build_history(&lessons, &records);
        let plan = plan_next(&history);

        let generated = self
            .generator
            .generate(
                user_ctx,
                &path.scope,
                &plan.mastered,
                Some(&plan.directive()),
                path.mission.as_deref(),
            )
            .await?;

        let seq = lessons.len() as u32 + 1;
        let lesson = into_lesson(
            Uuid::new_v4().to_string(),
            path_id.to_string(),
            seq,
            &user_ctx.user.user_id,
            generated,
        );
        self.repo.add_lesson(lesson.clone()).await?;

        // Record the section this lesson taught, so coverage reflects what the
        // learner has seen at section granularity.
        let mut covered = path.covered_anchors;
        let taught = lesson
            .primary_source
            .as_ref()
            .map(LessonSource::key)
            .or_else(|| lesson.citations.first().map(|c| c.document_slug.clone()));
        if let Some(key) = taught {
            if !covered.contains(&key) {
                covered.push(key);
            }
        }
        self.repo.update_path_progress(path_id, &covered).await?;

        // Persisted lessons are graded by id; no token needed.
        Ok(LessonView::from_lesson(lesson, None))
    }

    /// Grade a quiz submission and record the result.
    pub async fn submit_quiz(
        &self,
        user_ctx: &UserContext,
        lesson_id: &str,
        answers: &[usize],
    ) -> Result<QuizGrade, AppError> {
        let lesson = self
            .repo
            .get_lesson(lesson_id)
            .await?
            .ok_or_else(|| AppError::NotFound("lesson not found".into()))?;
        if lesson.user_id != user_ctx.user.user_id {
            return Err(AppError::Forbidden("not your lesson".into()));
        }

        let grade = grade_quiz(&lesson.quiz, answers);
        self.repo
            .add_record(LearningRecord {
                id: Uuid::new_v4().to_string(),
                path_id: lesson.path_id,
                lesson_id: Some(lesson.id),
                user_id: user_ctx.user.user_id.clone(),
                kind: LearningRecordKind::QuizResult {
                    per_question: grade.per_question.clone(),
                    score: grade.score,
                },
                created_at: Utc::now(),
            })
            .await?;

        Ok(grade)
    }

    /// A path together with its lessons, ordered by `seq`.
    pub async fn get_path_with_lessons(
        &self,
        user_ctx: &UserContext,
        path_id: &str,
    ) -> Result<(LearningPath, Vec<LessonView>), AppError> {
        let path = self.owned_path(user_ctx, path_id).await?;
        let lessons = self
            .repo
            .list_lessons_for_path(path_id)
            .await?
            .into_iter()
            .map(|l| LessonView::from_lesson(l, None))
            .collect();
        Ok((path, lessons))
    }

    /// All of the user's paths, most recent first.
    pub async fn list_paths(&self, user_ctx: &UserContext) -> Result<Vec<LearningPath>, AppError> {
        self.repo.list_paths_for_user(&user_ctx.user.user_id).await
    }

    /// Privacy: delete all of the user's learning data.
    pub async fn delete_all(&self, user_ctx: &UserContext) -> Result<(), AppError> {
        self.repo.delete_all_for_user(&user_ctx.user.user_id).await
    }

    /// Whether the user persists learning data (privacy preference).
    pub async fn get_persist(&self, user_ctx: &UserContext) -> Result<bool, AppError> {
        self.repo.get_persist(&user_ctx.user.user_id).await
    }

    /// Update the user's persistence preference.
    pub async fn set_persist(&self, user_ctx: &UserContext, persist: bool) -> Result<(), AppError> {
        self.repo.set_persist(&user_ctx.user.user_id, persist).await
    }

    /// Generate a one-off lesson without persisting anything (privacy opt-out).
    /// The returned view has a synthetic id, no path, and a sealed `quiz_token`
    /// carrying the answer key so the quiz can still be graded server-side.
    pub async fn generate_ephemeral(
        &self,
        user_ctx: &UserContext,
        scope: &LearningScope,
        mission: Option<String>,
    ) -> Result<LessonView, AppError> {
        let mission = normalize_mission(mission);
        let generated = self
            .generator
            .generate(user_ctx, scope, &[], None, mission.as_deref())
            .await?;
        let lesson = into_lesson(
            Uuid::new_v4().to_string(),
            String::new(),
            0,
            &user_ctx.user.user_id,
            generated,
        );
        let token = if lesson.quiz.is_empty() {
            None
        } else {
            Some(self.sealer.seal(&quiz_key(&lesson.quiz))?)
        };
        Ok(LessonView::from_lesson(lesson, token))
    }

    /// Grade an ephemeral quiz from its sealed token, without persisting
    /// anything. The token proves the answer key was issued by this server.
    pub fn submit_ephemeral_quiz(
        &self,
        token: &str,
        answers: &[usize],
    ) -> Result<QuizGrade, AppError> {
        let key = self.sealer.open(token)?;
        Ok(grade_against(&key.correct, &key.explanations, answers))
    }

    /// Load a path and verify it belongs to the requesting user.
    async fn owned_path(
        &self,
        user_ctx: &UserContext,
        path_id: &str,
    ) -> Result<LearningPath, AppError> {
        let path = self
            .repo
            .get_path(path_id)
            .await?
            .ok_or_else(|| AppError::NotFound("learning path not found".into()))?;
        if path.user_id != user_ctx.user.user_id {
            return Err(AppError::Forbidden("not your learning path".into()));
        }
        Ok(path)
    }
}

/// Assemble a persisted/ephemeral [`Lesson`] from generator output.
fn into_lesson(
    id: String,
    path_id: String,
    seq: u32,
    user_id: &str,
    generated: GeneratedLesson,
) -> Lesson {
    Lesson {
        id,
        path_id,
        user_id: user_id.to_string(),
        seq,
        title: generated.title,
        body_html: generated.body_html,
        citations: generated.citations,
        primary_source: generated.primary_source,
        quiz: generated.quiz,
        created_at: Utc::now(),
    }
}

/// Grade answers against a quiz. An out-of-range or missing answer counts wrong.
fn grade_quiz(quiz: &[QuizQuestion], answers: &[usize]) -> QuizGrade {
    let correct: Vec<usize> = quiz.iter().map(|q| q.correct_index).collect();
    let explanations: Vec<String> = quiz.iter().map(|q| q.explanation.clone()).collect();
    grade_against(&correct, &explanations, answers)
}

/// Core grading: compare `answers` against the `correct` answer key. A missing
/// or out-of-range answer counts wrong. `explanations` are returned verbatim so
/// the client can reveal them only after grading.
fn grade_against(correct: &[usize], explanations: &[String], answers: &[usize]) -> QuizGrade {
    let per_question: Vec<bool> = correct
        .iter()
        .enumerate()
        .map(|(i, &c)| answers.get(i).is_some_and(|&a| a == c))
        .collect();
    let score = if per_question.is_empty() {
        0.0
    } else {
        per_question.iter().filter(|&&b| b).count() as f32 / per_question.len() as f32
    };
    QuizGrade {
        per_question,
        score,
        explanations: explanations.to_vec(),
    }
}

/// Extract the answer key from a quiz, for sealing into an ephemeral token.
fn quiz_key(quiz: &[QuizQuestion]) -> QuizKey {
    QuizKey {
        correct: quiz.iter().map(|q| q.correct_index).collect(),
        explanations: quiz.iter().map(|q| q.explanation.clone()).collect(),
    }
}

/// Build the per-section calibration history (most-recent-first) by joining
/// quiz records to the section their lesson taught (its `primary_source`).
/// Records the lesson of which has no primary source are skipped — there is no
/// section to attribute the score to.
fn build_history(lessons: &[Lesson], records: &[LearningRecord]) -> Vec<LessonOutcome> {
    let lesson_key: std::collections::HashMap<&str, String> = lessons
        .iter()
        .filter_map(|l| {
            l.primary_source
                .as_ref()
                .map(|ps| (l.id.as_str(), ps.key()))
        })
        .collect();

    records
        .iter()
        .filter_map(|r| match &r.kind {
            LearningRecordKind::QuizResult { score, .. } => {
                let lesson_id = r.lesson_id.as_deref()?;
                let section_key = lesson_key.get(lesson_id)?.clone();
                Some(LessonOutcome {
                    section_key,
                    score: *score,
                })
            }
            LearningRecordKind::Insight { .. } => None,
        })
        .collect()
}

/// Trim a learner-supplied mission, treating blank input as "no mission".
fn normalize_mission(mission: Option<String>) -> Option<String> {
    mission
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
}

/// Derive a short human-readable title for a new path.
fn default_title(scope: &LearningScope) -> String {
    match scope {
        LearningScope::Document { slug } => {
            let leaf = slug.rsplit('/').next().unwrap_or(slug);
            let pretty = leaf.replace(['-', '_'], " ");
            format!("Learning: {pretty}")
        }
        LearningScope::Tag { tag } => format!("Learning: {tag}"),
        LearningScope::Topic { text } => {
            let t = text.trim();
            if t.chars().count() > 60 {
                let truncated: String = t.chars().take(57).collect();
                format!("{truncated}…")
            } else {
                t.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn q(correct: usize) -> QuizQuestion {
        QuizQuestion {
            prompt: "q".into(),
            options: vec!["a".into(), "b".into(), "c".into()],
            correct_index: correct,
            explanation: "e".into(),
        }
    }

    fn lesson_with_source(id: &str, slug: &str, anchor: Option<&str>) -> Lesson {
        Lesson {
            id: id.into(),
            path_id: "p".into(),
            user_id: "u".into(),
            seq: 1,
            title: "T".into(),
            body_html: "<p>x</p>".into(),
            citations: vec![],
            primary_source: Some(LessonSource {
                document_slug: slug.into(),
                section_anchor: anchor.map(Into::into),
            }),
            quiz: vec![],
            created_at: Utc::now(),
        }
    }

    fn quiz_record(lesson_id: &str, score: f32) -> LearningRecord {
        LearningRecord {
            id: "r".into(),
            path_id: "p".into(),
            lesson_id: Some(lesson_id.into()),
            user_id: "u".into(),
            kind: LearningRecordKind::QuizResult {
                per_question: vec![],
                score,
            },
            created_at: Utc::now(),
        }
    }

    #[test]
    fn build_history_joins_records_to_the_section_taught() {
        let lessons = vec![
            lesson_with_source("l1", "docs/kafka", Some("partitions")),
            lesson_with_source("l2", "docs/kafka", Some("offsets")),
        ];
        // Records come most-recent-first from the repo.
        let records = vec![quiz_record("l2", 0.4), quiz_record("l1", 1.0)];
        let history = build_history(&lessons, &records);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].section_key, "docs/kafka#offsets");
        assert_eq!(history[0].score, 0.4);
        assert_eq!(history[1].section_key, "docs/kafka#partitions");
    }

    #[test]
    fn build_history_skips_records_without_a_matching_lesson_source() {
        let lessons = vec![lesson_with_source("l1", "docs/kafka", None)];
        // A record for an unknown lesson id is dropped.
        let records = vec![quiz_record("missing", 0.5), quiz_record("l1", 0.9)];
        let history = build_history(&lessons, &records);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].section_key, "docs/kafka");
        assert_eq!(history[0].score, 0.9);
    }

    #[test]
    fn grade_all_correct() {
        let quiz = vec![q(0), q(1), q(2)];
        let grade = grade_quiz(&quiz, &[0, 1, 2]);
        assert_eq!(grade.per_question, vec![true, true, true]);
        assert_eq!(grade.score, 1.0);
    }

    #[test]
    fn grade_partial_and_missing_answers() {
        let quiz = vec![q(0), q(1), q(2)];
        // Second wrong, third answer missing → both count wrong.
        let grade = grade_quiz(&quiz, &[0, 2]);
        assert_eq!(grade.per_question, vec![true, false, false]);
        assert!((grade.score - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn grade_empty_quiz_is_zero() {
        let grade = grade_quiz(&[], &[]);
        assert!(grade.per_question.is_empty());
        assert_eq!(grade.score, 0.0);
    }

    #[test]
    fn title_from_document_slug_uses_leaf() {
        let t = default_title(&LearningScope::Document {
            slug: "eng/deploy-guide".into(),
        });
        assert_eq!(t, "Learning: deploy guide");
    }

    #[test]
    fn title_from_long_topic_is_truncated() {
        let long = "a".repeat(100);
        let t = default_title(&LearningScope::Topic { text: long });
        assert!(t.ends_with('…'));
        assert!(t.chars().count() <= 58);
    }
}
