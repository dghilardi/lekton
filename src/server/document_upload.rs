use leptos::prelude::*;
use serde::{Deserialize, Serialize};

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

/// Fields submitted by the admin document-upload form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentUploadForm {
    /// Existing document slug when editing; `None` to create a new document
    /// (the slug is then derived from the title and must not already exist).
    pub slug: Option<String>,
    pub title: String,
    /// Short description shown on the document page and used as its summary.
    pub summary: String,
    pub access_level: String,
    /// Key of the already-uploaded PDF asset the document links to.
    pub asset_key: String,
    /// Optional parent slug for tree placement.
    pub parent_slug: Option<String>,
    /// Sort order among siblings.
    pub order: u32,
}

/// Create or update a document from an uploaded PDF: build a markdown page
/// (description + download link) and run it through the ingest pipeline, which
/// indexes it for search and RAG and links the PDF so its chunks inherit the
/// document's access level.
///
/// Requires admin and the `document_upload` feature. When `form.slug` is `None`
/// the slug is derived from the title and must be free; when set, that document
/// is updated.
#[server(SaveDocumentWithAttachment, "/api")]
pub async fn save_document_with_attachment(
    form: DocumentUploadForm,
) -> Result<String, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    if !state.features.document_upload {
        return Err(ServerFnError::new("Document upload is disabled"));
    }

    let title = form.title.trim();
    if title.is_empty() {
        return Err(ServerFnError::new("Title is required"));
    }
    if form.access_level.trim().is_empty() {
        return Err(ServerFnError::new("Access level is required"));
    }

    let asset = state
        .asset_repo
        .find_by_key(&form.asset_key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Asset not found"))?;
    if !asset.content_type.starts_with("application/pdf") {
        return Err(ServerFnError::new("Only PDF attachments are supported"));
    }

    let is_edit = form.slug.is_some();
    let slug = match form.slug {
        Some(s) => {
            let existing = state
                .document_repo
                .find_by_slug(&s)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            if existing.is_none_or(|d| d.is_archived) {
                return Err(ServerFnError::new(format!("No document at '{s}' to edit")));
            }
            s
        }
        None => {
            let slug = slugify(title);
            if slug.is_empty() {
                return Err(ServerFnError::new(
                    "Title must contain at least one alphanumeric character",
                ));
            }
            let existing = state
                .document_repo
                .find_by_slug(&slug)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            if existing.is_some_and(|d| !d.is_archived) {
                return Err(ServerFnError::new(format!(
                    "A document already exists at '{slug}'"
                )));
            }
            slug
        }
    };

    let content = build_stub_markdown(&form.summary, &form.asset_key);

    let request = crate::db::models::IngestRequest {
        service_token: state.service_token.clone(),
        slug: slug.clone(),
        title: title.to_string(),
        summary: Some(form.summary.trim().to_string()).filter(|s| !s.is_empty()),
        content,
        access_level: form.access_level,
        is_draft: false,
        service_owner: "document-upload".to_string(),
        tags: vec![],
        parent_slug: form.parent_slug.filter(|s| !s.trim().is_empty()),
        order: form.order,
        is_hidden: false,
        source_path: format!("document-upload/{slug}.md"),
        source_id: "document-upload".to_string(),
    };

    let ctx = crate::api::ingest::IngestContext {
        repo: state.document_repo.as_ref(),
        asset_repo: state.asset_repo.as_ref(),
        storage: state.storage_client.as_ref(),
        search: state.search_service.as_deref(),
        access_level_repo: state.access_level_repo.as_ref(),
        service_token_repo: state.service_token_repo.as_ref(),
        version_repo: state.document_version_repo.as_ref(),
        rag: state.rag_service.as_deref(),
        legacy_token: Some(&state.service_token),
    };

    crate::api::ingest::process_ingest(&ctx, request)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // On edit, the access level may have changed while the linked asset stayed
    // the same. process_ingest only recomputes attachment ACLs when the
    // *reference set* changes, so explicitly recompute here to propagate an
    // access-level change to the (already-indexed) PDF's chunks. Cheap: a
    // payload update, no re-embedding. No-op when the asset isn't indexed yet.
    if is_edit {
        if let Some(rag) = state.rag_service.as_deref() {
            crate::rag::attachment_extraction::recompute_access_levels(
                rag,
                state.asset_repo.as_ref(),
                state.document_repo.as_ref(),
                std::slice::from_ref(&form.asset_key),
            )
            .await;
        }
    }

    Ok(slug)
}

/// Prefill data for editing a document created through the upload form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentEditData {
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub access_level: String,
    pub asset_key: String,
    pub parent_slug: Option<String>,
    pub order: u32,
}

/// Load a document's fields to prefill the upload form for editing. The linked
/// PDF asset key is recovered from the document's markdown body.
///
/// Requires admin and the `document_upload` feature.
#[server(GetDocumentForEdit, "/api")]
pub async fn get_document_for_edit(slug: String) -> Result<DocumentEditData, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    if !state.features.document_upload {
        return Err(ServerFnError::new("Document upload is disabled"));
    }

    let doc = state
        .document_repo
        .find_by_slug(&slug)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .filter(|d| !d.is_archived)
        .ok_or_else(|| ServerFnError::new("Document not found"))?;

    let content = state
        .storage_client
        .get_object(&doc.s3_key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default();
    let asset_key = crate::rendering::links::extract_asset_keys(&content)
        .into_iter()
        .next()
        .unwrap_or_default();

    Ok(DocumentEditData {
        slug: doc.slug,
        title: doc.title,
        summary: doc.summary.unwrap_or_default(),
        access_level: doc.access_level,
        asset_key,
        parent_slug: doc.parent_slug,
        order: doc.order,
    })
}

/// Build the markdown body for an uploaded document: the description followed by
/// a download link to the asset. The link target is `/api/v1/assets/{key}` so
/// the ingest pipeline records the document as referencing the asset.
#[cfg(feature = "ssr")]
fn build_stub_markdown(summary: &str, asset_key: &str) -> String {
    let summary = summary.trim();
    let link = format!("[Download (PDF)](/api/v1/assets/{asset_key})");
    if summary.is_empty() {
        link
    } else {
        format!("{summary}\n\n{link}\n")
    }
}

/// Derive a URL-safe slug from a title: lowercase, alphanumerics kept, runs of
/// other characters collapsed to single hyphens, trimmed of leading/trailing
/// hyphens.
#[cfg(feature = "ssr")]
fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut prev_hyphen = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                slug.push(lower);
            }
            prev_hyphen = false;
        } else if !prev_hyphen && !slug.is_empty() {
            slug.push('-');
            prev_hyphen = true;
        }
    }
    slug.trim_end_matches('-').to_string()
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;

    #[test]
    fn slugify_normalizes_titles() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("  Ferie & Permessi 2026!  "), "ferie-permessi-2026");
        assert_eq!(slugify("Già pronto"), "già-pronto");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn stub_markdown_embeds_asset_link() {
        let md = build_stub_markdown("A summary.", "editor/123_doc.pdf");
        assert!(md.contains("A summary."));
        assert!(md.contains("(/api/v1/assets/editor/123_doc.pdf)"));
        // The link must be discoverable as an asset reference by the ingest pipeline.
        let keys = crate::rendering::links::extract_asset_keys(&md);
        assert_eq!(keys, vec!["editor/123_doc.pdf".to_string()]);
    }

    #[test]
    fn stub_markdown_without_summary_is_just_the_link() {
        let md = build_stub_markdown("   ", "editor/1_a.pdf");
        assert_eq!(md, "[Download (PDF)](/api/v1/assets/editor/1_a.pdf)");
    }
}
