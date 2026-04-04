//! RAG API endpoints.
//!
//! | Method | Path                               | Description                |
//! |--------|------------------------------------|----------------------------|
//! | POST   | `/api/v1/admin/rag/reindex`        | Trigger full re-embedding  |
//! | GET    | `/api/v1/admin/rag/reindex/status` | Poll re-index progress     |

use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::app::AppState;
use crate::auth::extractor::RequiredAuthUser;
use crate::error::AppError;

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ReindexStatusResponse {
    pub is_running: bool,
    pub progress: u32,
    pub rag_enabled: bool,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `POST /api/v1/admin/rag/reindex` — trigger a full re-index (admin only).
pub async fn trigger_reindex_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin privileges required".into()));
    }

    let rag = state
        .rag_service
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("RAG is not enabled".into()))?;

    let reindex = state
        .reindex_state
        .as_ref()
        .ok_or_else(|| AppError::Internal("reindex state not available".into()))?;

    // Prevent concurrent runs (compare-and-swap)
    if reindex
        .is_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "message": "Re-index is already in progress",
                "progress": reindex.progress.load(Ordering::Relaxed),
            })),
        ));
    }

    // Spawn background task
    let reindex_clone = reindex.clone();
    let document_repo = state.document_repo.clone();
    let storage = state.storage_client.clone();
    let rag_clone = rag.clone();

    tokio::spawn(async move {
        crate::rag::reindex::run_reindex(reindex_clone, document_repo, storage, rag_clone).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "message": "Re-index started",
        })),
    ))
}

/// `GET /api/v1/admin/rag/reindex/status` — poll re-index progress (admin only).
pub async fn reindex_status_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
) -> Result<Json<ReindexStatusResponse>, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin privileges required".into()));
    }

    let (is_running, progress) = match &state.reindex_state {
        Some(reindex) => (
            reindex.is_running.load(Ordering::Acquire),
            reindex.progress.load(Ordering::Relaxed),
        ),
        None => (false, 0),
    };

    Ok(Json(ReindexStatusResponse {
        is_running,
        progress,
        rag_enabled: state.rag_service.is_some(),
    }))
}
