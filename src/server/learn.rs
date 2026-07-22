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
    service
        .generate_next(&user_ctx, &path_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
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
