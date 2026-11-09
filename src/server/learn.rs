//! Server functions for Learn mode.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::db::learn_models::{LearningPath, LearningScope, Lesson, QuizGrade};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_user_context;

/// A learning path together with its lessons, ordered by `seq`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathWithLessons {
    pub path: LearningPath,
    pub lessons: Vec<Lesson>,
}

/// Keep only the lesson sources the caller can open now, under the same rule
/// as reading the documents themselves.
#[cfg(feature = "ssr")]
async fn with_openable_sources(
    state: &AppState,
    user_ctx: &crate::auth::models::UserContext,
    mut lessons: Vec<Lesson>,
) -> Result<Vec<Lesson>, ServerFnError> {
    let slugs: Vec<String> = lessons
        .iter()
        .flat_map(Lesson::source_slugs)
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if slugs.is_empty() {
        return Ok(lessons);
    }
    let (allowed_levels, include_draft) = user_ctx.document_visibility();
    let openable: std::collections::HashSet<String> = state
        .document_repo
        .find_by_slugs(&slugs)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .into_iter()
        .filter(|doc| {
            !doc.is_archived
                && crate::app::doc_is_accessible(
                    &doc.access_level,
                    doc.is_draft,
                    allowed_levels.as_deref(),
                    include_draft,
                )
        })
        .map(|doc| doc.slug)
        .collect();
    for lesson in &mut lessons {
        lesson.retain_openable_sources(|slug| openable.contains(slug));
    }
    Ok(lessons)
}

/// Resolve the Learn-mode service or fail with an actionable error.
#[cfg(feature = "ssr")]
fn learn_service(
    state: &AppState,
) -> Result<std::sync::Arc<crate::learn::service::LearnService>, ServerFnError> {
    state
        .learn_service
        .clone()
        .ok_or_else(|| ServerFnError::new("Learn mode is not enabled"))
}

/// Start a new learning path over the given scope.
#[server(StartLearningPath, "/api")]
pub async fn start_learning_path(scope: LearningScope) -> Result<LearningPath, ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    service
        .start_path(&user_ctx, scope)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Generate, persist, and return the next lesson for a path.
#[server(GenerateNextLesson, "/api")]
pub async fn generate_next_lesson(path_id: String) -> Result<Lesson, ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    let lesson = service
        .generate_next(&user_ctx, &path_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(with_openable_sources(&state, &user_ctx, vec![lesson])
        .await?
        .remove(0))
}

/// Grade a quiz submission for a lesson and record the result.
#[server(SubmitQuiz, "/api")]
pub async fn submit_quiz(
    lesson_id: String,
    answers: Vec<usize>,
) -> Result<QuizGrade, ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    service
        .submit_quiz(&user_ctx, &lesson_id, &answers)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Fetch a learning path together with its lessons.
#[server(GetLearningPath, "/api")]
pub async fn get_learning_path(path_id: String) -> Result<PathWithLessons, ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    let (path, lessons) = service
        .get_path_with_lessons(&user_ctx, &path_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let lessons = with_openable_sources(&state, &user_ctx, lessons).await?;
    Ok(PathWithLessons { path, lessons })
}

/// List all of the caller's learning paths, most recent first.
#[server(ListMyLearningPaths, "/api")]
pub async fn list_my_learning_paths() -> Result<Vec<LearningPath>, ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    service
        .list_paths(&user_ctx)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Delete all of the caller's learning data (paths, lessons, records).
#[server(DeleteMyLearningData, "/api")]
pub async fn delete_my_learning_data() -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    service
        .delete_all(&user_ctx)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Whether the caller persists learning data (privacy preference).
#[server(GetLearnPrivacy, "/api")]
pub async fn get_learn_privacy() -> Result<bool, ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    service
        .get_persist(&user_ctx)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Set the caller's persistence preference. When off, learning is ephemeral.
#[server(SetLearnPrivacy, "/api")]
pub async fn set_learn_privacy(persist: bool) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    service
        .set_persist(&user_ctx, persist)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Generate a one-off lesson without persisting anything (privacy opt-out).
#[server(GenerateEphemeralLesson, "/api")]
pub async fn generate_ephemeral_lesson(scope: LearningScope) -> Result<Lesson, ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = require_user_context(&state).await?;
    let service = learn_service(&state)?;
    let lesson = service
        .generate_ephemeral(&user_ctx, &scope)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(with_openable_sources(&state, &user_ctx, vec![lesson])
        .await?
        .remove(0))
}
