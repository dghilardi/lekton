//! RAG API endpoints.
//!
//! | Method | Path                               | Description                |
//! |--------|------------------------------------|----------------------------|
//! | POST   | `/api/v1/admin/rag/reindex`        | Trigger full re-embedding  |
//! | GET    | `/api/v1/admin/rag/reindex/status` | Poll re-index progress     |
//! | POST   | `/api/v1/rag/chat`                 | Chat with RAG (SSE stream) |
//! | GET    | `/api/v1/rag/sessions`             | List user's chat sessions  |
//! | DELETE | `/api/v1/rag/sessions/{id}`        | Delete a chat session      |

use std::convert::Infallible;
use std::sync::atomic::Ordering;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::auth::extractor::RequiredAuthUser;
use crate::auth::models::UserContext;
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

// ── Chat endpoints ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChatRequest {
    pub session_id: Option<String>,
    pub message: String,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

/// `POST /api/v1/rag/chat` — stream a RAG chat response (requires auth).
pub async fn chat_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
    Json(request): Json<ChatRequest>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let chat_svc = state
        .chat_service
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("RAG chat is not enabled".into()))?;

    // Build UserContext for access-level filtering
    let user_ctx = build_user_context(&state, &user).await?;

    let event_stream = chat_svc
        .stream_response(&user_ctx, request.session_id, request.message)
        .await?;

    let sse_stream = event_stream.map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().data(data))
    });

    Ok(Sse::new(sse_stream))
}

/// `GET /api/v1/rag/sessions` — list the authenticated user's chat sessions.
pub async fn list_sessions_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionResponse>>, AppError> {
    let chat_repo = state
        .chat_repo
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("RAG is not enabled".into()))?;

    let sessions = chat_repo.list_sessions_for_user(&user.user_id).await?;
    let response: Vec<SessionResponse> = sessions
        .into_iter()
        .map(|s| SessionResponse {
            id: s.id,
            title: s.title,
            created_at: s.created_at.to_rfc3339(),
            updated_at: s.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(response))
}

/// `DELETE /api/v1/rag/sessions/{id}` — delete a chat session (owner only).
pub async fn delete_session_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let chat_repo = state
        .chat_repo
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("RAG is not enabled".into()))?;

    // Verify ownership
    let session = chat_repo
        .get_session(&session_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Chat session not found".into()))?;

    if session.user_id != user.user_id {
        return Err(AppError::NotFound("Chat session not found".into()));
    }

    chat_repo.delete_session(&session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Build a [`UserContext`] from the authenticated user for access-level filtering.
async fn build_user_context(
    state: &AppState,
    user: &crate::auth::models::AuthenticatedUser,
) -> Result<UserContext, AppError> {
    let permissions = state
        .user_repo
        .get_permissions(&user.user_id)
        .await?;
    Ok(UserContext {
        user: user.clone(),
        permissions,
    })
}
