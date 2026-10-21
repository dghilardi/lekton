use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    db::{
        ingest_job_repository::{IngestJob, IngestJobState},
        models::{IngestRequest, IngestResponse},
    },
    error::AppError,
    jobs::ingest::context,
};

pub const LONG_POLLING_HEADER: &str = "lekton-long-polling";

#[derive(Debug, Serialize, Deserialize)]
pub struct IngestJobStatus {
    pub job_id: String,
    pub state: IngestJobState,
    pub status_url: String,
    pub result: Option<IngestResponse>,
    pub code: Option<String>,
    pub error: Option<String>,
}

pub async fn submit(
    state: AppState,
    headers: HeaderMap,
    request: IngestRequest,
) -> Result<Response, AppError> {
    if request.release.is_some() && !state.features.doc_versioning {
        return Err(AppError::BadRequest(
            "documentation versioning is disabled on this instance".into(),
        ));
    }
    super::ingest::validate_token(&context(&state), &request.service_token, &request.slug).await?;
    // Reject invalid identities and access levels before accepting durable work.
    if request.slug.is_empty() || request.slug.starts_with('/') || request.slug.contains("..") {
        return Err(AppError::BadRequest("Invalid document slug".into()));
    }
    if request.access_level.trim().is_empty()
        || !state
            .access_level_repo
            .exists(&request.access_level.to_lowercase())
            .await?
    {
        return Err(AppError::BadRequest("Unknown or empty access level".into()));
    }
    let actor = super::token_validation::resolve_service_token(
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        &request.service_token,
    )
    .await?
    .name;
    let queue = state
        .ingest_queue
        .as_ref()
        .ok_or_else(|| AppError::Internal("Import queue unavailable".into()))?;
    let job = queue
        .enqueue(state.storage_client.as_ref(), request, actor)
        .await?;
    let job = queue.wait(job).await?;
    let supported = headers
        .get(LONG_POLLING_HEADER)
        .and_then(|h| h.to_str().ok())
        == Some("true");
    Ok(job_response(job, supported, true))
}

#[derive(Deserialize)]
pub struct PollIngestRequest {
    pub service_token: String,
}

/// POST avoids placing service tokens in URLs or access logs. Scope and write
/// permissions are rechecked on every poll, including after token revocation.
pub async fn poll_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<PollIngestRequest>,
) -> Result<Response, AppError> {
    super::token_validation::validate_service_token(
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        &request.service_token,
    )
    .await?;
    let queue = state
        .ingest_queue
        .as_ref()
        .ok_or_else(|| AppError::Internal("Import queue unavailable".into()))?;
    let job = queue
        .repository
        .get(&id)
        .await?
        .ok_or_else(|| AppError::NotFound("Import job not found".into()))?;
    super::ingest::validate_token(&context(&state), &request.service_token, &job.request.slug)
        .await?;
    let job = queue.wait(job).await?;
    Ok(job_response(job, true, false))
}

pub fn job_response(job: IngestJob, supported: bool, initial: bool) -> Response {
    if initial && job.state == IngestJobState::Succeeded {
        return match job.result.clone() {
            Some(result) => Json(result).into_response(),
            None => AppError::Internal("Completed import has no result".into()).into_response(),
        };
    }
    let status_url = format!("/api/v1/ingest/jobs/{}/poll", job.id);
    let (status, code, error) = match job.state {
        IngestJobState::Succeeded => (StatusCode::OK, None, None),
        IngestJobState::Failed => (StatusCode::UNPROCESSABLE_ENTITY, Some("ingest_failed".into()), job.error),
        _ if supported => (StatusCode::ACCEPTED, None, None),
        _ => (StatusCode::CONFLICT, Some("ingest_pending".into()), Some("Import is still queued or processing and will continue in the background; upgrade lekton-sync to wait for its outcome".into())),
    };
    let body = IngestJobStatus {
        job_id: job.id,
        state: job.state,
        status_url: status_url.clone(),
        result: job.result,
        code,
        error,
    };
    (
        status,
        [(axum::http::header::LOCATION, status_url)],
        Json(body),
    )
        .into_response()
}

/// A concurrent legacy run must not archive or finalize ahead of queued work.
pub async fn ensure_source_idle(
    state: &AppState,
    source_id: &str,
    token: &str,
) -> Result<(), AppError> {
    super::token_validation::validate_service_token(
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        token,
    )
    .await?;
    if let Some(queue) = &state.ingest_queue {
        if queue.repository.has_active_source(source_id).await? {
            return Err(AppError::BadRequest("This source has pending imports; wait for their completion before archiving, finalizing or promoting".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(state: IngestJobState) -> IngestJob {
        IngestJob {
            id: "job-1".into(),
            sequence: 1, dedup_key: "hash".into(), active: matches!(state, IngestJobState::Queued | IngestJobState::Processing),
            state, request: serde_json::from_value(serde_json::json!({
                "service_token": "", "content": "", "slug": "doc", "title": "Doc",
                "access_level": "developer", "service_owner": "team", "source_id": "source", "source_path": "doc.md"
            })).unwrap(),
            actor: "ci".into(), staging_key: "staging/job-1".into(), worker: None,
            attempts: 0, created_at: chrono::Utc::now(), result: None, error: None, expires_at: None,
        }
    }

    #[tokio::test]
    async fn pending_is_never_a_success_for_legacy_clients() {
        for state in [IngestJobState::Queued, IngestJobState::Processing] {
            let old = job_response(job(state), false, true);
            assert_eq!(old.status(), StatusCode::CONFLICT);
            let bytes = axum::body::to_bytes(old.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: IngestJobStatus = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body.code.as_deref(), Some("ingest_pending"));
            let modern = job_response(job(state), true, true);
            assert_eq!(modern.status(), StatusCode::ACCEPTED);
            assert_eq!(
                modern.headers()[axum::http::header::LOCATION],
                "/api/v1/ingest/jobs/job-1/poll"
            );
        }
    }

    #[tokio::test]
    async fn terminal_failure_is_explicit_for_both_clients() {
        for supported in [false, true] {
            let mut job = job(IngestJobState::Failed);
            job.error = Some("Embedding failed".into());
            let response = job_response(job, supported, true);
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: IngestJobStatus = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body.code.as_deref(), Some("ingest_failed"));
            assert_eq!(body.error.as_deref(), Some("Embedding failed"));
        }
    }
}
