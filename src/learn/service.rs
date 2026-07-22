//! Orchestration for Learn mode: turns generator output into persisted paths,
//! lessons, and records, enforcing per-user ownership.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::auth::models::UserContext;
use crate::db::learn_models::{
    LearningPath, LearningRecord, LearningRecordKind, LearningScope, Lesson, QuizGrade,
    QuizQuestion,
};
use crate::db::learn_repository::LearnRepository;
use crate::error::AppError;
use crate::learn::generator::LessonGenerator;

pub struct LearnService {
    repo: Arc<dyn LearnRepository>,
    generator: LessonGenerator,
}

impl LearnService {
    pub fn new(repo: Arc<dyn LearnRepository>, generator: LessonGenerator) -> Self {
        Self { repo, generator }
    }

    /// Start a new learning path for the user.
    pub async fn start_path(
        &self,
        user_ctx: &UserContext,
        scope: LearningScope,
    ) -> Result<LearningPath, AppError> {
        let now = Utc::now();
        let path = LearningPath {
            id: Uuid::new_v4().to_string(),
            user_id: user_ctx.user.user_id.clone(),
            title: default_title(&scope),
            scope,
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
    ) -> Result<Lesson, AppError> {
        let path = self.owned_path(user_ctx, path_id).await?;

        let generated = self
            .generator
            .generate(user_ctx, &path.scope, &path.covered_anchors)
            .await?;

        let seq = self.repo.list_lessons_for_path(path_id).await?.len() as u32 + 1;
        let lesson = Lesson {
            id: Uuid::new_v4().to_string(),
            path_id: path_id.to_string(),
            user_id: user_ctx.user.user_id.clone(),
            seq,
            title: generated.title,
            body_html: generated.body_html,
            citations: generated.citations,
            primary_source: generated.primary_source,
            quiz: generated.quiz,
            created_at: Utc::now(),
        };
        self.repo.add_lesson(lesson.clone()).await?;

        // Record the documents this lesson drew on, so the next lesson avoids
        // re-teaching the same ground (basic calibration; refined in a later step).
        let mut covered = path.covered_anchors;
        for slug in generated.source_slugs {
            if !covered.contains(&slug) {
                covered.push(slug);
            }
        }
        self.repo.update_path_progress(path_id, &covered).await?;

        Ok(lesson)
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
    ) -> Result<(LearningPath, Vec<Lesson>), AppError> {
        let path = self.owned_path(user_ctx, path_id).await?;
        let lessons = self.repo.list_lessons_for_path(path_id).await?;
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

/// Grade answers against a quiz. An out-of-range or missing answer counts wrong.
fn grade_quiz(quiz: &[QuizQuestion], answers: &[usize]) -> QuizGrade {
    let per_question: Vec<bool> = quiz
        .iter()
        .enumerate()
        .map(|(i, q)| answers.get(i).is_some_and(|&a| a == q.correct_index))
        .collect();
    let score = if per_question.is_empty() {
        0.0
    } else {
        per_question.iter().filter(|&&b| b).count() as f32 / per_question.len() as f32
    };
    QuizGrade {
        per_question,
        score,
    }
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

    fn q(correct: usize) -> QuizQuestion {
        QuizQuestion {
            prompt: "q".into(),
            options: vec!["a".into(), "b".into(), "c".into()],
            correct_index: correct,
            explanation: "e".into(),
        }
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
