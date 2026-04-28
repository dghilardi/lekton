use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

#[server(GetRagReindexStatus, "/api")]
pub async fn get_rag_reindex_status() -> Result<(bool, u32, bool), ServerFnError> {
    use std::sync::atomic::Ordering;
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;
    let rag_enabled = state.rag_service.is_some();
    match &state.reindex_state {
        Some(reindex) => Ok((
            reindex.is_running.load(Ordering::Acquire),
            reindex.progress.load(Ordering::Relaxed),
            rag_enabled,
        )),
        None => Ok((false, 0, rag_enabled)),
    }
}

#[server(TriggerRagReindex, "/api")]
pub async fn trigger_rag_reindex() -> Result<String, ServerFnError> {
    use std::sync::atomic::Ordering;
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let rag = state
        .rag_service
        .as_ref()
        .ok_or_else(|| ServerFnError::new("RAG is not enabled"))?;

    let reindex = state
        .reindex_state
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Reindex state not available"))?;

    if reindex
        .is_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ServerFnError::new("Re-index is already in progress"));
    }

    let reindex_clone = reindex.clone();
    let document_repo = state.document_repo.clone();
    let storage = state.storage_client.clone();
    let rag_clone = rag.clone();

    tokio::spawn(async move {
        crate::rag::reindex::run_reindex(reindex_clone, document_repo, storage, rag_clone).await;
    });

    Ok("Re-index started".to_string())
}

#[server(GetSearchReindexStatus, "/api")]
pub async fn get_search_reindex_status() -> Result<(bool, u32, bool), ServerFnError> {
    use std::sync::atomic::Ordering;
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;
    let search_enabled = state.search_service.is_some();
    match &state.search_reindex_state {
        Some(reindex) => Ok((
            reindex.is_running.load(Ordering::Acquire),
            reindex.progress.load(Ordering::Relaxed),
            search_enabled,
        )),
        None => Ok((false, 0, search_enabled)),
    }
}

#[server(TriggerSearchReindex, "/api")]
pub async fn trigger_search_reindex() -> Result<String, ServerFnError> {
    use std::sync::atomic::Ordering;
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let search = state
        .search_service
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Search is not enabled"))?;

    let reindex = state
        .search_reindex_state
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Search reindex state not available"))?;

    if reindex
        .is_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ServerFnError::new("Search re-index is already in progress"));
    }

    let reindex_clone = reindex.clone();
    let document_repo = state.document_repo.clone();
    let storage = state.storage_client.clone();
    let search_clone = search.clone();

    tokio::spawn(async move {
        crate::search::reindex::run_reindex(reindex_clone, document_repo, storage, search_clone)
            .await;
    });

    Ok("Search re-index started".to_string())
}

#[server(GetSchemaEndpointReindexStatus, "/api")]
pub async fn get_schema_endpoint_reindex_status() -> Result<(bool, u32), ServerFnError> {
    use std::sync::atomic::Ordering;
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;
    Ok((
        state
            .schema_endpoint_reindex_state
            .is_running
            .load(Ordering::Acquire),
        state
            .schema_endpoint_reindex_state
            .progress
            .load(Ordering::Relaxed),
    ))
}

#[server(TriggerSchemaEndpointReindex, "/api")]
pub async fn trigger_schema_endpoint_reindex() -> Result<String, ServerFnError> {
    use std::sync::atomic::Ordering;
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    if state
        .schema_endpoint_reindex_state
        .is_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ServerFnError::new(
            "Schema endpoint re-index is already in progress",
        ));
    }

    let reindex = state.schema_endpoint_reindex_state.clone();
    let schema_repo = state.schema_repo.clone();
    let storage = state.storage_client.clone();

    tokio::spawn(async move {
        crate::schema::reindex::run_schema_endpoint_reindex(reindex, schema_repo, storage).await;
    });

    Ok("Schema endpoint re-index started".to_string())
}
