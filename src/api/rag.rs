//! RAG API endpoints.
//!
//! | Method | Path                                      | Description                          |
//! |--------|-------------------------------------------|--------------------------------------|
//! | POST   | `/api/v1/admin/rag/reindex`               | Trigger full re-embedding            |
//! | GET    | `/api/v1/admin/rag/reindex/status`        | Poll re-index progress               |
//! | GET    | `/api/v1/admin/rag/feedback`              | Export feedback (paginated, filtered)|
//! | POST   | `/api/v1/rag/chat`                        | Chat with RAG (SSE stream)           |
//! | GET    | `/api/v1/rag/sessions`                    | List user's chat sessions            |
//! | DELETE | `/api/v1/rag/sessions/{id}`               | Delete a chat session                |
//! | GET    | `/api/v1/rag/sessions/{id}/messages`      | Get messages for a session           |
//! | POST   | `/api/v1/rag/messages/{id}/feedback`      | Submit or update message feedback    |
//! | DELETE | `/api/v1/rag/messages/{id}/feedback`      | Remove message feedback              |

use std::convert::Infallible;
use std::sync::atomic::Ordering;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::auth::extractor::RequiredAuthUser;
use crate::auth::models::UserContext;
use crate::db::chat_models::{FeedbackRating, SourceReference};
use crate::db::feedback_repository::FeedbackListParams;
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

#[derive(Serialize)]
pub struct SessionMessageResponse {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceReference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<serde_json::Value>,
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

/// `GET /api/v1/rag/sessions/{id}/messages` — get messages for a chat session (owner only).
///
/// Each assistant message includes an `id` field and an optional `feedback` object so
/// the frontend can render the current thumbs-up/down state without a second request.
pub async fn get_session_messages_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<SessionMessageResponse>>, AppError> {
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

    let messages = chat_repo.get_messages(&session_id, 500).await?;
    let user_ctx = build_user_context(&state, &user).await?;

    // Load all feedback for this session in one query, then build a lookup map.
    let feedback_map: std::collections::HashMap<String, serde_json::Value> =
        if let Some(fb_repo) = state.feedback_repo.as_ref() {
            fb_repo
                .get_session_feedback(&session_id, &user.user_id)
                .await?
                .into_iter()
                .map(|fb| {
                    let rating_str = match fb.rating {
                        FeedbackRating::Positive => "positive",
                        FeedbackRating::Negative => "negative",
                    };
                    let val = serde_json::json!({
                        "rating": rating_str,
                        "comment": fb.comment,
                    });
                    (fb.message_id, val)
                })
                .collect()
        } else {
            std::collections::HashMap::new()
        };

    let mut response = Vec::with_capacity(messages.len());
    for m in messages {
        let sources = match m.sources {
            Some(sources) => {
                let filtered = filter_source_references(&state, &user_ctx, sources).await?;
                Some(filtered)
            }
            None => None,
        };
        let feedback = feedback_map.get(&m.id).cloned();
        response.push(SessionMessageResponse {
            id: m.id,
            role: m.role,
            content: m.content,
            sources,
            feedback,
        });
    }

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

// ── Feedback endpoints ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SubmitFeedbackRequest {
    pub rating: String,
    pub comment: Option<String>,
}

/// `POST /api/v1/rag/messages/{id}/feedback` — submit or update feedback on an assistant message.
pub async fn submit_feedback_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
    Path(message_id): Path<String>,
    Json(body): Json<SubmitFeedbackRequest>,
) -> Result<StatusCode, AppError> {
    let chat_repo = state
        .chat_repo
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("RAG is not enabled".into()))?;

    let fb_repo = state
        .feedback_repo
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("RAG is not enabled".into()))?;

    // Resolve message → verify it exists and belongs to this user's session.
    let message = chat_repo
        .get_message_by_id(&message_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Message not found".into()))?;

    if message.role != "assistant" {
        return Err(AppError::BadRequest(
            "Feedback can only be given on assistant messages".into(),
        ));
    }

    let session = chat_repo
        .get_session(&message.session_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Chat session not found".into()))?;

    if session.user_id != user.user_id {
        return Err(AppError::NotFound("Message not found".into()));
    }

    let rating = match body.rating.as_str() {
        "positive" => FeedbackRating::Positive,
        "negative" => FeedbackRating::Negative,
        _ => {
            return Err(AppError::BadRequest(
                "rating must be 'positive' or 'negative'".into(),
            ))
        }
    };

    let now = chrono::Utc::now();
    let feedback = crate::db::chat_models::MessageFeedback {
        id: uuid::Uuid::new_v4().to_string(),
        message_id: message_id.clone(),
        session_id: message.session_id,
        user_id: user.user_id.clone(),
        rating,
        comment: body.comment,
        created_at: now,
        updated_at: now,
    };

    fb_repo.upsert_feedback(feedback).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/rag/messages/{id}/feedback` — remove the user's feedback on a message.
pub async fn delete_feedback_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
    Path(message_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let fb_repo = state
        .feedback_repo
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("RAG is not enabled".into()))?;

    fb_repo.delete_feedback(&message_id, &user.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Admin feedback export ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AdminFeedbackQuery {
    pub rating: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub user_id: Option<String>,
    #[serde(default)]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

fn default_per_page() -> u64 {
    50
}

#[derive(Serialize)]
pub struct FeedbackItemResponse {
    pub id: String,
    pub message_id: String,
    pub session_id: String,
    pub user_id: String,
    pub rating: String,
    pub comment: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct FeedbackPageResponse {
    pub items: Vec<FeedbackItemResponse>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

/// `GET /api/v1/admin/rag/feedback` — paginated feedback export (admin only).
///
/// Query params: `rating`, `date_from` (RFC 3339), `date_to` (RFC 3339),
/// `user_id`, `page` (0-based), `per_page` (max 200, default 50).
pub async fn admin_list_feedback_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
    Query(q): Query<AdminFeedbackQuery>,
) -> Result<Json<FeedbackPageResponse>, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin privileges required".into()));
    }

    let fb_repo = state
        .feedback_repo
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("RAG is not enabled".into()))?;

    let date_from = q
        .date_from
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| AppError::BadRequest(format!("invalid date_from: {s}")))
        })
        .transpose()?;

    let date_to = q
        .date_to
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| AppError::BadRequest(format!("invalid date_to: {s}")))
        })
        .transpose()?;

    let params = FeedbackListParams {
        rating: q.rating,
        date_from,
        date_to,
        user_id: q.user_id,
        page: q.page,
        per_page: q.per_page,
    };

    let page = fb_repo.list_all_feedback(params).await?;

    let items = page
        .items
        .into_iter()
        .map(|fb| {
            let rating_str = match fb.rating {
                FeedbackRating::Positive => "positive".to_string(),
                FeedbackRating::Negative => "negative".to_string(),
            };
            FeedbackItemResponse {
                id: fb.id,
                message_id: fb.message_id,
                session_id: fb.session_id,
                user_id: fb.user_id,
                rating: rating_str,
                comment: fb.comment,
                created_at: fb.created_at.to_rfc3339(),
                updated_at: fb.updated_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(FeedbackPageResponse {
        items,
        total: page.total,
        page: page.page,
        per_page: page.per_page,
    }))
}

/// Build a [`UserContext`] from the authenticated user for access-level filtering.
async fn build_user_context(
    state: &AppState,
    user: &crate::auth::models::AuthenticatedUser,
) -> Result<UserContext, AppError> {
    let permissions = state.user_repo.get_permissions(&user.user_id).await?;
    Ok(UserContext {
        user: user.clone(),
        permissions,
    })
}

async fn filter_source_references(
    state: &AppState,
    user_ctx: &UserContext,
    sources: Vec<SourceReference>,
) -> Result<Vec<SourceReference>, AppError> {
    use crate::db::repository::DocumentRepository;

    let (allowed_levels, include_draft) = user_ctx.document_visibility();
    let mut filtered = Vec::with_capacity(sources.len());

    for source in sources {
        let Some(document) = state
            .document_repo
            .find_by_slug(&source.document_slug)
            .await?
        else {
            continue;
        };

        if document.is_archived {
            continue;
        }

        if crate::app::doc_is_accessible(
            &document.access_level,
            document.is_draft,
            allowed_levels.as_deref(),
            include_draft,
        ) {
            filtered.push(source);
        }
    }

    Ok(filtered)
}
