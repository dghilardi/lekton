use axum::{extract::State, response::IntoResponse, Json};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use crate::state::AppState;
// Note: meilisearch-sdk doesn't yet support tenant tokens directly in a simple way in older versions, 
// but we can generate them using JWT if needed. For now, we'll implement a simple search proxy 
// or a token generation placeholder if the SDK version allows.
// In 0.27, it might requires manual JWT generation for tenant tokens.

#[derive(Serialize)]
pub struct SearchTokenResponse {
    pub token: String,
    pub expiry: i64,
}

pub async fn get_search_token(
    State(_state): State<AppState>,
    // In a real app, we'd extract the user session here
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Placeholder: In a real implementation, we'd use the user's roles to generate a 
    // filtered JWT for Meilisearch.
    
    // For MVP, we'll return a placeholder or implement a simple search proxy if easier.
    // Let's implement a simple search handler instead for now to keep it moving.
    
    Ok(Json(SearchTokenResponse {
        token: "placeholder-token".to_string(),
        expiry: 3600,
    }))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

pub async fn search_handler(
    State(_state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let meili = _state.meili.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Search not configured".to_string()))?;
    let index = meili.index("documents");
    
    let results = index.search()
        .with_query(&params.q)
        .execute::<crate::api::ingest::SearchDocument>()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Search error: {}", e)))?;
        
    let hits: Vec<crate::api::ingest::SearchDocument> = results.hits.into_iter().map(|h| h.result).collect();
    Ok(Json(hits))
}
