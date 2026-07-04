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
    let mut results = search_service
        .search(&query, allowed_levels.as_deref(), include_draft)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Keyword hits inside PDF attachments, appended after document hits so
    // ordinary documents (matched on title/summary/tags) stay primary.
    if let Some(attachment_search) = &state.attachment_search_service {
        match attachment_search
            .search(&query, allowed_levels.as_deref())
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

    Ok(results)
}
