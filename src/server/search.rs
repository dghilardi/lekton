use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::app::AppState;
use crate::search::client::SearchHit;
#[cfg(feature = "ssr")]
use crate::server::request_document_visibility;

#[server(SearchDocs, "/api")]
pub async fn search_docs(query: String) -> Result<Vec<SearchHit>, ServerFnError> {
    let state = expect_context::<AppState>();

    let search_service = state
        .search_service
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Search not available"))?;

    let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
    let results = search_service
        .search(&query, allowed_levels.as_deref(), include_draft)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(results)
}
