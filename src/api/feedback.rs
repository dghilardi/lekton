//! Service-token authenticated documentation-feedback endpoints.
//!
//! Used by a source repository's CI to resolve the feedback items its merge
//! addressed. A feedback item is only resolved when it is `in_progress` **and**
//! claimed for the caller's `source_id` — so a token can only close items that
//! were claimed as delivering to its own source.

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Request body for `POST /api/v1/feedback/resolve`.
#[derive(Debug, Deserialize)]
pub struct ResolveFeedbackRequest {
    /// Service authentication token (legacy or scoped).
    pub service_token: String,
    /// The source the fix was delivered from — must match each item's
    /// `delivery_source_id` for the resolution to apply.
    pub source_id: String,
    /// Feedback ids the caller confirmed as resolved (e.g. found in mainline
    /// history via the resolving commit trailer).
    pub feedback_ids: Vec<String>,
}

/// Response from a resolve operation.
#[derive(Debug, Serialize, PartialEq)]
pub struct ResolveFeedbackResponse {
    /// Ids that were `in_progress` for this source and are now `resolved`.
    pub resolved: Vec<String>,
    /// Ids that did not match (already resolved, claimed for another source,
    /// or unknown) — reported, not an error, so the call is idempotent-safe.
    pub skipped: Vec<String>,
}

/// Core resolve logic, separated from the HTTP layer for testability.
#[cfg(feature = "ssr")]
pub async fn process_resolve_feedback(
    repo: &dyn crate::db::documentation_feedback_repository::DocumentationFeedbackRepository,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    request: ResolveFeedbackRequest,
) -> Result<ResolveFeedbackResponse, AppError> {
    // Resolving is a mutation; require a writable token (mirrors sync).
    let resolved_token = crate::api::token_validation::resolve_service_token(
        service_token_repo,
        legacy_token,
        &request.service_token,
    )
    .await?;
    if !resolved_token.can_write {
        return Err(AppError::Forbidden(
            "Token does not have write permission".into(),
        ));
    }

    let source_id = request.source_id.trim();
    if source_id.is_empty() {
        return Err(AppError::BadRequest("source_id is required".into()));
    }

    let mut resolved = Vec::new();
    let mut skipped = Vec::new();
    for id in &request.feedback_ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if repo.resolve_claimed(id, source_id).await? {
            resolved.push(id.to_string());
        } else {
            skipped.push(id.to_string());
        }
    }

    Ok(ResolveFeedbackResponse { resolved, skipped })
}

/// Axum handler for `POST /api/v1/feedback/resolve`.
#[cfg(feature = "ssr")]
pub async fn resolve_feedback_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<ResolveFeedbackRequest>,
) -> Result<axum::Json<ResolveFeedbackResponse>, AppError> {
    let response = process_resolve_feedback(
        state.documentation_feedback_repo.as_ref(),
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        request,
    )
    .await?;
    Ok(axum::Json(response))
}
