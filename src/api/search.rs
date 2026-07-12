use crate::search::client::SearchHit;

#[cfg(feature = "ssr")]
#[derive(serde::Serialize)]
pub struct SearchReindexStatusResponse {
    pub is_running: bool,
    pub progress: u32,
    pub failed: u32,
    pub skipped: u32,
    pub last_error: Option<String>,
    pub search_enabled: bool,
}

/// Query parameters for the search endpoint.
///
#[cfg(feature = "ssr")]
#[derive(Debug, serde::Deserialize)]
pub struct SearchQuery {
    /// The search query string.
    pub q: String,
}

/// Axum handler for `GET /api/v1/search?q=<query>`.
#[cfg(feature = "ssr")]
pub async fn search_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::OptionalAuthUser(user): crate::auth::extractor::OptionalAuthUser,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> Result<axum::Json<Vec<SearchHit>>, crate::error::AppError> {
    let search_service = state
        .search_service
        .as_ref()
        .ok_or_else(|| crate::error::AppError::Internal("Search service not available".into()))?;

    let (allowed_levels, include_draft) =
        crate::app::resolve_user_visibility(&state, user.as_ref()).await?;

    let mut results = search_service
        .search(&params.q, allowed_levels.as_deref(), include_draft)
        .await?;

    if let Some(attachment_search) = &state.attachment_search_service {
        match attachment_search
            .search(&params.q, allowed_levels.as_deref())
            .await
        {
            Ok(hits) => results.extend(hits.into_iter().map(|h| SearchHit {
                slug: h.document_slug,
                title: h.document_title,
                tags: vec![],
                content_preview: h.content_preview,
                attachment_key: Some(h.attachment_key),
                page: h.page,
            })),
            Err(e) => tracing::warn!("Attachment search failed: {e}"),
        }
    }

    Ok(axum::Json(results))
}

/// `POST /api/v1/admin/search/reindex` — trigger full Meilisearch re-index.
#[cfg(feature = "ssr")]
pub async fn trigger_reindex_handler(
    crate::auth::extractor::RequiredAuthUser(user): crate::auth::extractor::RequiredAuthUser,
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
) -> Result<(axum::http::StatusCode, axum::Json<serde_json::Value>), crate::error::AppError> {
    use std::sync::atomic::Ordering;

    if !user.is_admin {
        return Err(crate::error::AppError::Forbidden(
            "Admin privileges required".into(),
        ));
    }

    let search = state
        .search_service
        .as_ref()
        .ok_or_else(|| crate::error::AppError::BadRequest("Search is not enabled".into()))?;

    let reindex = state.search_reindex_state.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("search reindex state not available".into())
    })?;

    if reindex
        .is_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok((
            axum::http::StatusCode::CONFLICT,
            axum::Json(serde_json::json!({
                "message": "Search re-index is already in progress",
                "progress": reindex.progress.load(Ordering::Relaxed),
            })),
        ));
    }

    let reindex_clone = reindex.clone();
    let document_repo = state.document_repo.clone();
    let storage = state.storage_client.clone();
    let search_clone = search.clone();

    tokio::spawn(async move {
        crate::search::reindex::run_reindex(reindex_clone, document_repo, storage, search_clone)
            .await;
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({
            "message": "Search re-index started",
        })),
    ))
}

/// `GET /api/v1/admin/search/reindex/status` — poll Meilisearch re-index progress.
#[cfg(feature = "ssr")]
pub async fn reindex_status_handler(
    crate::auth::extractor::RequiredAuthUser(user): crate::auth::extractor::RequiredAuthUser,
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
) -> Result<axum::Json<SearchReindexStatusResponse>, crate::error::AppError> {
    use std::sync::atomic::Ordering;

    if !user.is_admin {
        return Err(crate::error::AppError::Forbidden(
            "Admin privileges required".into(),
        ));
    }

    let search_enabled = state.search_service.is_some();
    let (is_running, progress, failed, skipped, last_error) = match &state.search_reindex_state {
        Some(reindex) => {
            let (failed, skipped, last_error) = reindex.outcome.snapshot();
            (
                reindex.is_running.load(Ordering::Acquire),
                reindex.progress.load(Ordering::Relaxed),
                failed,
                skipped,
                last_error,
            )
        }
        None => (false, 0, 0, 0, None),
    };

    Ok(axum::Json(SearchReindexStatusResponse {
        is_running,
        progress,
        failed,
        skipped,
        last_error,
        search_enabled,
    }))
}
