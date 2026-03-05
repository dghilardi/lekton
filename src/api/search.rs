use crate::search::client::SearchHit;

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
