use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

/// Number of leading PDF pages read to generate a summary. Enough to convey the
/// document's topic without feeding the whole file to the LLM.
#[cfg(feature = "ssr")]
const SUMMARY_PREVIEW_PAGES: usize = 3;

/// Generate an AI summary from the first pages of an already-uploaded PDF asset.
///
/// `asset_key` is the logical asset key returned by the upload endpoint. The
/// PDF's leading pages are extracted (native text only — fast, no VLM) and sent
/// to the chat LLM for a short description. Requires admin, the
/// `document_upload` feature, and RAG (for the chat LLM).
#[server(GenerateDocumentSummary, "/api")]
pub async fn generate_document_summary(asset_key: String) -> Result<String, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    if !state.features.document_upload {
        return Err(ServerFnError::new("Document upload is disabled"));
    }
    let chat = state
        .chat_service
        .as_ref()
        .ok_or_else(|| ServerFnError::new("AI summary requires RAG to be enabled"))?;

    let asset = state
        .asset_repo
        .find_by_key(&asset_key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Asset not found"))?;

    if !asset.content_type.starts_with("application/pdf") {
        return Err(ServerFnError::new(
            "Summary generation supports PDF files only",
        ));
    }

    let bytes = state
        .storage_client
        .get_object(&asset.s3_key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Asset content not found in storage"))?;

    let preview = crate::rag::extraction::extract_preview(&bytes, SUMMARY_PREVIEW_PAGES)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    if preview.trim().is_empty() {
        return Err(ServerFnError::new(
            "Couldn't extract any text from the first pages of this PDF",
        ));
    }

    chat.summarize(&preview)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
