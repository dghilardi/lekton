use crate::search::client::SearchHit;

#[cfg(feature = "ssr")]
#[derive(serde::Serialize)]
pub struct SearchReindexStatusResponse {
    pub is_running: bool,
    pub progress: u32,
    pub search_enabled: bool,
}

/// Query parameters for the search endpoint.
///
/// NOTE: Once the auth extractor is wired up (task 11) the `access_levels`
/// parameter will be ignored in favour of the authenticated user's permissions.
/// For now it is kept for backward compatibility with the demo setup.
#[cfg(feature = "ssr")]
#[derive(Debug, serde::Deserialize)]
pub struct SearchQuery {
    /// The search query string.
    pub q: String,
    /// Comma-separated access level names to filter by (e.g. `"public,internal"`).
    /// Defaults to `"public"` (anonymous access).
    #[serde(default = "default_access_level")]
    pub access_levels: String,
}

#[cfg(feature = "ssr")]
fn default_access_level() -> String {
    "public".to_string()
}

/// Axum handler for `GET /api/v1/search?q=<query>&access_levels=<levels>`.
#[cfg(feature = "ssr")]
pub async fn search_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> Result<axum::Json<Vec<SearchHit>>, crate::error::AppError> {
    let search_service = state
        .search_service
        .as_ref()
        .ok_or_else(|| crate::error::AppError::Internal("Search service not available".into()))?;

    // Parse the comma-separated level list.
    let levels: Vec<String> = params
        .access_levels
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let results = search_service
        .search(&params.q, Some(levels.as_slice()), false)
        .await?;

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
    let (is_running, progress) = match &state.search_reindex_state {
        Some(reindex) => (
            reindex.is_running.load(Ordering::Acquire),
            reindex.progress.load(Ordering::Relaxed),
        ),
        None => (false, 0),
    };

    Ok(axum::Json(SearchReindexStatusResponse {
        is_running,
        progress,
        search_enabled,
    }))
}
