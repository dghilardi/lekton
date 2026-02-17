use crate::auth::models::AccessLevel;
use crate::search::client::SearchHit;

/// Query parameters for the search endpoint.
#[cfg(feature = "ssr")]
#[derive(Debug, serde::Deserialize)]
pub struct SearchQuery {
    /// The search query string.
    pub q: String,
    /// Optional access level filter (defaults to "public").
    #[serde(default = "default_access_level")]
    pub access_level: String,
}

#[cfg(feature = "ssr")]
fn default_access_level() -> String {
    "public".to_string()
}

/// Axum handler for `GET /api/v1/search?q=<query>&access_level=<level>`.
#[cfg(feature = "ssr")]
pub async fn search_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> Result<axum::Json<Vec<SearchHit>>, crate::error::AppError> {
    let search_service = state
        .search_service
        .as_ref()
        .ok_or_else(|| crate::error::AppError::Internal("Search service not available".into()))?;

    let access_level = AccessLevel::from_str_ci(&params.access_level)
        .unwrap_or(AccessLevel::Public);

    let results = search_service.search(&params.q, access_level).await?;

    Ok(axum::Json(results))
}
