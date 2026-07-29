//! `POST /api/v1/releases/promote` — move a source's `latest` alias.
//!
//! Called by `lekton-sync --latest` *after* the documents have been uploaded, so
//! the alias only ever moves onto a fully published release. Promoting first
//! would leave readers on a half-uploaded one if the run failed partway.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::error::AppError;

/// Request payload for `POST /api/v1/releases/promote`.
#[derive(Debug, Deserialize)]
pub struct PromoteReleaseRequest {
    /// Service authentication token (legacy or scoped).
    pub service_token: String,
    /// The source whose alias should move.
    pub source_id: String,
    /// The release to alias as `latest`. Must already be published.
    pub release: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PromoteReleaseResponse {
    pub source_id: String,
    pub release: String,
    /// Documents whose `is_latest` flag changed, and which are therefore marked
    /// stale for search and RAG.
    pub reindex_pending: usize,
}

/// Core promotion logic — separated from the HTTP layer for testability.
#[cfg(feature = "ssr")]
pub async fn process_promote_release(
    repo: &dyn crate::db::repository::DocumentRepository,
    release_repo: &dyn crate::db::release_repository::ReleaseRepository,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    request: PromoteReleaseRequest,
) -> Result<PromoteReleaseResponse, AppError> {
    // Promotion is a write, so reuse the sync token rules (must have can_write).
    crate::api::sync::validate_sync_token(service_token_repo, legacy_token, &request.service_token)
        .await?;

    // Refuse to alias a release nobody published: otherwise a typo would point
    // `latest` at nothing and hide the whole source.
    let published = release_repo.list_by_source(&request.source_id).await?;
    if !published.iter().any(|r| r.release == request.release) {
        return Err(AppError::BadRequest(format!(
            "release '{}' is not published for source '{}'",
            request.release, request.source_id
        )));
    }

    // Alias first (a single atomic write), then bring the denormalized
    // `is_latest` flags in line. If the second step failed, the flags would be
    // repaired by re-running the promotion — whereas flags without an alias
    // would leave the two disagreeing with nothing to reconcile them.
    release_repo
        .set_latest(&request.source_id, &request.release)
        .await?;
    let affected = repo
        .promote_release(&request.source_id, &request.release)
        .await?;

    Ok(PromoteReleaseResponse {
        source_id: request.source_id,
        release: request.release,
        reindex_pending: affected.len(),
    })
}

/// Axum handler for `POST /api/v1/releases/promote`.
#[cfg(feature = "ssr")]
pub async fn promote_release_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<PromoteReleaseRequest>,
) -> Result<axum::Json<PromoteReleaseResponse>, AppError> {
    if !state.features.doc_versioning {
        return Err(AppError::BadRequest(
            "documentation versioning is disabled on this instance".into(),
        ));
    }

    let response = process_promote_release(
        state.document_repo.as_ref(),
        state.release_repo.as_ref(),
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        request,
    )
    .await?;

    Ok(axum::Json(response))
}
