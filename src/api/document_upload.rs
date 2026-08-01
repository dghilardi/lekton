//! Axum handlers for the admin document-upload flow.
//!
//! Currently provides a streaming SSE endpoint for AI summary generation so
//! that long-running LLM calls do not hit the GCP Load Balancer 30-second
//! timeout that would occur with a non-streaming Leptos server function.

use std::convert::Infallible;

use axum::extract::{Query, State};
use axum::response::sse::{Event, Sse};
use futures::StreamExt;
use serde::Deserialize;

use crate::app::AppState;
use crate::auth::extractor::RequiredAuthUser;
use crate::db::usage_models::UsageKey;
use crate::error::AppError;

/// Number of leading PDF pages read to generate a summary.
const SUMMARY_PREVIEW_PAGES: usize = 3;

#[derive(Deserialize)]
pub struct SummaryStreamQuery {
    pub asset_key: String,
}

/// `GET /api/v1/document-upload/summary` — stream an AI summary of an uploaded
/// PDF asset as Server-Sent Events.
///
/// Each SSE message carries one token string. A final event with type `"done"`
/// and empty data signals completion. Requires an admin user, and both the
/// `document_upload` and `rag` features must be enabled.
pub async fn summary_stream_handler(
    RequiredAuthUser(user): RequiredAuthUser,
    State(state): State<AppState>,
    Query(params): Query<SummaryStreamQuery>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin privileges required".into()));
    }
    if !state.features.document_upload {
        return Err(AppError::BadRequest("Document upload is disabled".into()));
    }

    let chat = state
        .chat_service
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("AI summary requires RAG to be enabled".into()))?;

    let asset = state
        .asset_repo
        .find_by_key(&params.asset_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Asset not found".into()))?;

    if !asset.content_type.starts_with("application/pdf") {
        return Err(AppError::BadRequest(
            "Summary generation supports PDF files only".into(),
        ));
    }

    let bytes = state
        .storage_client
        .get_object(&asset.s3_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Asset content not found in storage".into()))?;

    let preview = crate::rag::extraction::extract_preview(&bytes, SUMMARY_PREVIEW_PAGES)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if preview.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Couldn't extract any text from the first pages of this PDF".into(),
        ));
    }

    let token_stream = chat
        .summarize_stream(&UsageKey::User(user.user_id.clone()), &preview)
        .await?;

    let sse_stream = async_stream::stream! {
        futures::pin_mut!(token_stream);
        while let Some(result) = token_stream.next().await {
            match result {
                Ok(token) => yield Ok(Event::default().data(token)),
                Err(e) => {
                    tracing::warn!("Summary stream error: {e}");
                    // Named "summary_error" (not "error") so it does not collide
                    // with EventSource's built-in connection-error event.
                    yield Ok(Event::default().event("summary_error").data(e.to_string()));
                    return;
                }
            }
        }
        // Non-empty data: SSE drops events whose data buffer is empty, so a
        // completion marker MUST carry a payload to be dispatched to the client.
        yield Ok(Event::default().event("done").data("ok"));
    };

    Ok(Sse::new(sse_stream))
}
