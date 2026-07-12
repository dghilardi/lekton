use leptos::prelude::*;
use leptos_tiptap::*;

use super::asset_panel::AssetPanel;

#[cfg(feature = "hydrate")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "hydrate")]
#[wasm_bindgen(module = "/public/js/editor-assets.js")]
extern "C" {
    #[wasm_bindgen(js_name = "uploadAndInsertImage")]
    fn upload_and_insert_image(editor_id: &str) -> js_sys::Promise;

    #[wasm_bindgen(js_name = "uploadAsset")]
    pub fn upload_asset_js() -> js_sys::Promise;
}

/// Server function to fetch document content for editing.
#[server(GetDocContent, "/api")]
pub async fn get_doc_content(slug: String) -> Result<Option<(String, String)>, ServerFnError> {
    use crate::rendering::markdown::render_markdown;

    let state = expect_context::<crate::app::AppState>();

    let doc = state
        .document_repo
        .find_by_slug(&slug)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(doc) = doc else {
        return Ok(None);
    };

    // Externally-managed (ingest API / lekton-sync) and upload-form documents
    // are read-only in the markdown editor: editing them here would be lost on
    // the next sync, or diverge from the upload form.
    if doc.source_id.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err(ServerFnError::new(
            "This page is managed outside the editor and can't be edited here.",
        ));
    }

    let content_bytes = state
        .storage_client
        .get_object(&doc.s3_key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(content_bytes) = content_bytes else {
        return Ok(None);
    };

    let raw_markdown =
        String::from_utf8(content_bytes).map_err(|e| ServerFnError::new(e.to_string()))?;

    let html = render_markdown(&raw_markdown);

    Ok(Some((doc.title, html)))
}

/// Server function to save edited document content.
#[server(SaveDocContent, "/api")]
pub async fn save_doc_content(
    slug: String,
    title: String,
    html_content: String,
) -> Result<String, ServerFnError> {
    use chrono::Utc;

    let state = expect_context::<crate::app::AppState>();

    if slug.contains("..") || slug.starts_with('/') {
        return Err(ServerFnError::new("Invalid slug"));
    }

    let links_out = crate::rendering::links::extract_internal_links_from_html(&html_content);

    let old_doc = state
        .document_repo
        .find_by_slug(&slug)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Refuse to overwrite externally-managed (ingest / lekton-sync) or
    // upload-form documents from the markdown editor.
    if let Some(ref d) = old_doc {
        if d.source_id.as_deref().is_some_and(|s| !s.is_empty()) {
            return Err(ServerFnError::new(
                "This page is managed outside the editor and can't be edited here.",
            ));
        }
    }

    let (
        old_links,
        access_level,
        is_draft,
        service_owner,
        tags,
        backlinks,
        parent_slug,
        order,
        is_hidden,
    ) = match old_doc {
        Some(d) => (
            d.links_out,
            d.access_level,
            d.is_draft,
            d.service_owner,
            d.tags,
            d.backlinks,
            d.parent_slug,
            d.order,
            d.is_hidden,
        ),
        None => (
            vec![],
            "public".to_string(),
            false,
            "web-editor".to_string(),
            vec![],
            vec![],
            None,
            0,
            false,
        ),
    };

    let s3_key = format!("docs/{}.md", slug.replace('/', "_"));

    state
        .storage_client
        .put_object(&s3_key, html_content.clone().into_bytes())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let doc = crate::db::models::Document {
        slug: slug.clone(),
        title,
        summary: None,
        s3_key,
        access_level,
        is_draft,
        service_owner,
        last_updated: Utc::now(),
        tags,
        links_out: links_out.clone(),
        backlinks,
        parent_slug,
        order,
        is_hidden,
        content_hash: None,
        metadata_hash: None,
        is_archived: false,
        source_path: None,
        source_id: None,
        needs_reindex: false,
        skip_rag: false,
    };

    finalize_document_save(
        state.document_repo.as_ref(),
        state.asset_repo.as_ref(),
        state.search_service.as_deref(),
        state.rag_service.as_deref(),
        state.attachment_search_service.as_deref(),
        state.storage_client.as_ref(),
        doc,
        &html_content,
        &old_links,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Index the document into search, persist it, reconcile backlinks and asset
/// references, and report the outcome.
///
/// Indexing runs *before* the metadata upsert so the stored document records
/// whether it is in sync: a search-indexing failure leaves a durable
/// `needs_reindex` flag (mirroring the ingest API) instead of silently
/// reporting success. Any sink failure (search or asset reconcile) is surfaced
/// in the returned message rather than swallowed.
#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)]
async fn finalize_document_save(
    document_repo: &dyn crate::db::repository::DocumentRepository,
    asset_repo: &dyn crate::db::asset_repository::AssetRepository,
    search_service: Option<&dyn crate::search::client::SearchService>,
    rag_service: Option<&dyn crate::rag::service::RagService>,
    attachment_search: Option<&dyn crate::search::attachment_search::AttachmentSearchService>,
    storage: &dyn crate::storage::client::StorageClient,
    mut doc: crate::db::models::Document,
    html_content: &str,
    old_links: &[String],
) -> Result<String, crate::error::AppError> {
    let slug = doc.slug.clone();
    let links_out = doc.links_out.clone();

    let search_doc =
        search_service.map(|_| crate::search::client::build_search_document(&doc, html_content));

    // Index into search before persisting so `needs_reindex` records whether
    // the stored document is in sync with the index.
    let mut warnings = Vec::new();
    if let (Some(svc), Some(sdoc)) = (search_service, search_doc) {
        if let Err(e) = svc.index_document(&sdoc).await {
            tracing::warn!(slug = %slug, "Failed to index document in search: {e}");
            warnings.push(format!("search indexing failed: {e}"));
        }
    }
    doc.needs_reindex = !warnings.is_empty();

    document_repo.create_or_update(doc).await?;
    document_repo
        .update_backlinks(&slug, old_links, &links_out)
        .await?;

    // Reconcile asset references so referenced assets record this document.
    let asset_keys = crate::rendering::links::extract_asset_keys_from_html(html_content);
    match asset_repo.set_references(&slug, &asset_keys).await {
        Ok(affected) => {
            // Recompute over the full current key set (plus dropped ones) so a
            // change to this document's access_level/draft state propagates to
            // its attachments even when its referenced assets are unchanged.
            if let Some(rag) = rag_service {
                let mut to_recompute = affected;
                for key in &asset_keys {
                    if !to_recompute.contains(key) {
                        to_recompute.push(key.clone());
                    }
                }
                if !to_recompute.is_empty() {
                    crate::rag::attachment_extraction::recompute_access_levels(
                        rag,
                        asset_repo,
                        document_repo,
                        storage,
                        attachment_search,
                        &to_recompute,
                    )
                    .await;
                }
            }
        }
        Err(e) => {
            tracing::warn!(slug = %slug, "Failed to update asset references: {e}");
            warnings.push(format!("asset reference update failed: {e}"));
        }
    }

    if warnings.is_empty() {
        Ok(format!("Document '{slug}' saved successfully"))
    } else {
        Ok(format!(
            "Document '{slug}' saved, but some indexing did not complete: {}. \
             It will be reconciled on the next reindex.",
            warnings.join("; ")
        ))
    }
}

/// The editor page component.
#[component]
pub fn EditorPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let slug = move || params.read().get("slug").unwrap_or_default();

    #[allow(clippy::redundant_closure)]
    let doc_resource = Resource::new(move || slug(), |slug| get_doc_content(slug));

    let (msg, set_msg) = signal(TiptapInstanceMsg::Noop);
    let (value, set_value) = signal(String::new());
    let (title, set_title) = signal(String::new());
    let (disabled, _set_disabled) = signal(false);
    let (_selection, set_selection) = signal(TiptapSelectionState::default());
    let (save_status, set_save_status) = signal(String::new());
    let (saving, set_saving) = signal(false);
    // Sentinel: the slug whose content is currently loaded in the editor signals.
    // The Effect below only overwrites title/value when the slug changes, so
    // in-progress edits are never clobbered by a resource refetch for the same doc.
    let (loaded_slug, set_loaded_slug) = signal(String::new());

    Effect::new(move || {
        let current_slug = slug();
        if let Some(Ok(Some((doc_title, html)))) = doc_resource.get() {
            if loaded_slug.get_untracked() != current_slug {
                set_loaded_slug.set(current_slug);
                set_title.set(doc_title);
                set_value.set(html);
            }
        }
    });

    let save_action = Action::new(move |_: &()| {
        let current_slug = slug();
        let current_title = title.get();
        let current_content = value.get();
        async move {
            set_saving.set(true);
            set_save_status.set(String::new());
            match save_doc_content(current_slug, current_title, current_content).await {
                Ok(msg) => set_save_status.set(msg),
                Err(e) => set_save_status.set(format!("Error: {e}")),
            }
            set_saving.set(false);
        }
    });

    view! {
        <Suspense fallback=move || view! { <div class="loading loading-spinner loading-lg"></div> }>
            {move || {
                doc_resource.get().map(|result| match result {
                    Ok(Some(_)) => {
                        view! {
                            <div class="space-y-4">
                                // Title input
                                <div class="form-control">
                                    <label class="label">
                                        <span class="label-text font-semibold">"Document Title"</span>
                                    </label>
                                    <input
                                        type="text"
                                        class="input input-bordered w-full"
                                        prop:value=title
                                        on:input=move |ev| {
                                            set_title.set(event_target_value(&ev));
                                        }
                                    />
                                </div>

                                // Toolbar
                                <div class="flex flex-wrap gap-1 p-2 bg-base-200 rounded-lg">
                                    <button class="btn btn-sm btn-ghost" title="Bold"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::Bold)>
                                        <strong>"B"</strong>
                                    </button>
                                    <button class="btn btn-sm btn-ghost" title="Italic"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::Italic)>
                                        <em>"I"</em>
                                    </button>
                                    <button class="btn btn-sm btn-ghost" title="Strikethrough"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::Strike)>
                                        <s>"S"</s>
                                    </button>
                                    <div class="divider divider-horizontal mx-0"></div>
                                    <button class="btn btn-sm btn-ghost" title="Heading 1"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::H1)>
                                        "H1"
                                    </button>
                                    <button class="btn btn-sm btn-ghost" title="Heading 2"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::H2)>
                                        "H2"
                                    </button>
                                    <button class="btn btn-sm btn-ghost" title="Heading 3"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::H3)>
                                        "H3"
                                    </button>
                                    <div class="divider divider-horizontal mx-0"></div>
                                    <button class="btn btn-sm btn-ghost" title="Bullet List"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::BulletList)>
                                        "List"
                                    </button>
                                    <button class="btn btn-sm btn-ghost" title="Ordered List"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::OrderedList)>
                                        "1. List"
                                    </button>
                                    <button class="btn btn-sm btn-ghost" title="Blockquote"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::Blockquote)>
                                        "Quote"
                                    </button>
                                    <button class="btn btn-sm btn-ghost" title="Highlight"
                                        on:click=move |_| set_msg.set(TiptapInstanceMsg::Highlight)>
                                        "HL"
                                    </button>
                                    <div class="divider divider-horizontal mx-0"></div>
                                    <button class="btn btn-sm btn-ghost" title="Insert Image"
                                        on:click=move |_| {
                                            #[cfg(feature = "hydrate")]
                                            leptos::task::spawn_local(async {
                                                let _ = wasm_bindgen_futures::JsFuture::from(
                                                    upload_and_insert_image("lekton-editor")
                                                ).await;
                                            });
                                        }>
                                        "Img"
                                    </button>
                                </div>

                                // Editor
                                <div class="border border-base-300 rounded-lg min-h-[400px] p-4 bg-base-100 prose prose-lg max-w-none">
                                    <TiptapInstance
                                        id=Signal::derive(|| "lekton-editor".to_string())
                                        msg=msg
                                        disabled=disabled
                                        value=value
                                        set_value=Callback::new(move |(v,): (TiptapContent,)| {
                                            set_value.set(match v {
                                                TiptapContent::Html(content) => content,
                                                TiptapContent::Json(content) => content,
                                            });
                                        })
                                        on_selection_change=Callback::new(move |(state,): (TiptapSelectionState,)| {
                                            set_selection.set(state);
                                        })
                                    />
                                </div>

                                // Save controls
                                <div class="flex items-center gap-4">
                                    <button
                                        class="btn btn-primary"
                                        prop:disabled=saving
                                        on:click=move |_| { save_action.dispatch(()); }
                                    >
                                        {move || if saving.get() { "Saving..." } else { "Save Document" }}
                                    </button>
                                    <a
                                        href=move || format!("/docs/{}", slug())
                                        class="btn btn-ghost"
                                    >
                                        "Cancel"
                                    </a>
                                    {move || {
                                        let status = save_status.get();
                                        if status.is_empty() {
                                            view! { <span></span> }.into_any()
                                        } else if status.starts_with("Error") {
                                            view! { <span class="text-error">{status}</span> }.into_any()
                                        } else {
                                            view! { <span class="text-success">{status}</span> }.into_any()
                                        }
                                    }}
                                </div>

                                // Asset panel
                                <AssetPanel set_msg=set_msg />
                            </div>
                        }.into_any()
                    }
                    Ok(None) => {
                        view! {
                            <div class="alert alert-warning">
                                <span>"Document not found. You can create a new document from this editor."</span>
                            </div>
                        }.into_any()
                    }
                    Err(e) => {
                        view! {
                            <div class="alert alert-error">
                                <span>{format!("Error loading document: {e}")}</span>
                            </div>
                        }.into_any()
                    }
                })
            }}
        </Suspense>
    }
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;
    use crate::db::asset_repository::{AssetRepository, ExtractionUpdate};
    use crate::db::models::{Asset, Document};
    use crate::db::repository::DocumentRepository;
    use crate::error::AppError;
    use crate::search::client::{SearchDocument, SearchHit, SearchService};
    use crate::test_utils::MockStorage;
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::Mutex;

    /// Document repo that records the last persisted document so tests can
    /// assert what was written (in particular `needs_reindex`).
    #[derive(Default)]
    struct CapturingDocumentRepo {
        saved: Mutex<Option<Document>>,
    }

    #[async_trait]
    impl DocumentRepository for CapturingDocumentRepo {
        async fn create_or_update(&self, doc: Document) -> Result<(), AppError> {
            *self.saved.lock().unwrap() = Some(doc);
            Ok(())
        }
        async fn find_by_slug(&self, _: &str) -> Result<Option<Document>, AppError> {
            Ok(None)
        }
        async fn find_by_slugs(&self, _: &[String]) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
        async fn list_all(&self) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
        async fn list_by_access_levels(
            &self,
            _: Option<&[String]>,
            _: bool,
        ) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
        async fn update_backlinks(
            &self,
            _: &str,
            _: &[String],
            _: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_slug_prefix(&self, _: &str) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
        async fn set_archived(&self, _: &str, _: bool) -> Result<(), AppError> {
            Ok(())
        }
        async fn rename_slug(&self, _: &str, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_source_path(&self, _: &str) -> Result<Option<Document>, AppError> {
            Ok(None)
        }
        async fn find_all_by_source_id(&self, _: &str) -> Result<Vec<Document>, AppError> {
            Ok(vec![])
        }
    }

    /// Asset repo whose `set_references` result is configurable.
    #[derive(Default)]
    struct StubAssetRepo {
        fail_set_references: bool,
    }

    #[async_trait]
    impl AssetRepository for StubAssetRepo {
        async fn create_or_update(&self, _: Asset) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_key(&self, _: &str) -> Result<Option<Asset>, AppError> {
            Ok(None)
        }
        async fn find_by_keys(&self, _: &[String]) -> Result<Vec<Asset>, AppError> {
            Ok(vec![])
        }
        async fn list_all(&self) -> Result<Vec<Asset>, AppError> {
            Ok(vec![])
        }
        async fn list_by_prefix(&self, _: &str) -> Result<Vec<Asset>, AppError> {
            Ok(vec![])
        }
        async fn delete(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn update_extraction(&self, _: &str, _: ExtractionUpdate) -> Result<(), AppError> {
            Ok(())
        }
        async fn set_references(&self, _: &str, _: &[String]) -> Result<Vec<String>, AppError> {
            if self.fail_set_references {
                Err(AppError::Database(
                    "simulated set_references failure".to_string(),
                ))
            } else {
                Ok(vec![])
            }
        }
        async fn list_unfinished_extractions(&self) -> Result<Vec<Asset>, AppError> {
            Ok(vec![])
        }
    }

    /// Search service whose `index_document` result is configurable.
    #[derive(Default)]
    struct StubSearchService {
        fail_index: bool,
    }

    #[async_trait]
    impl SearchService for StubSearchService {
        async fn index_document(&self, _: &SearchDocument) -> Result<(), AppError> {
            if self.fail_index {
                Err(AppError::Internal(
                    "simulated search index failure".to_string(),
                ))
            } else {
                Ok(())
            }
        }
        async fn delete_document(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn search(
            &self,
            _: &str,
            _: Option<&[String]>,
            _: bool,
        ) -> Result<Vec<SearchHit>, AppError> {
            Ok(vec![])
        }
        async fn configure_index(&self) -> Result<(), AppError> {
            Ok(())
        }
    }

    fn make_doc() -> Document {
        Document {
            slug: "guide".to_string(),
            title: "Guide".to_string(),
            summary: None,
            s3_key: "docs/guide.md".to_string(),
            access_level: "public".to_string(),
            is_draft: false,
            service_owner: "web-editor".to_string(),
            last_updated: Utc::now(),
            tags: vec![],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            content_hash: None,
            metadata_hash: None,
            is_archived: false,
            source_path: None,
            source_id: None,
            needs_reindex: false,
            skip_rag: false,
        }
    }

    #[tokio::test]
    async fn save_flags_needs_reindex_and_warns_when_search_index_fails() {
        let doc_repo = CapturingDocumentRepo::default();
        let asset_repo = StubAssetRepo::default();
        let search = StubSearchService { fail_index: true };
        let storage = MockStorage::new();

        let msg = finalize_document_save(
            &doc_repo,
            &asset_repo,
            Some(&search),
            None,
            None,
            &storage,
            make_doc(),
            "<p>body</p>",
            &[],
        )
        .await
        .unwrap();

        let saved = doc_repo.saved.lock().unwrap().clone().unwrap();
        assert!(
            saved.needs_reindex,
            "a failed search index must leave needs_reindex set"
        );
        assert!(
            msg.contains("search indexing failed"),
            "message must surface the failure, got: {msg}"
        );
        assert!(
            msg.contains("reindex"),
            "message must be actionable, got: {msg}"
        );
    }

    #[tokio::test]
    async fn save_warns_when_asset_reconcile_fails() {
        let doc_repo = CapturingDocumentRepo::default();
        let asset_repo = StubAssetRepo {
            fail_set_references: true,
        };
        let search = StubSearchService::default();
        let storage = MockStorage::new();

        let msg = finalize_document_save(
            &doc_repo,
            &asset_repo,
            Some(&search),
            None,
            None,
            &storage,
            make_doc(),
            "<p>body</p>",
            &[],
        )
        .await
        .unwrap();

        assert!(
            msg.contains("asset reference update failed"),
            "message must surface the asset failure, got: {msg}"
        );
    }

    #[tokio::test]
    async fn save_reports_success_when_all_sinks_ok() {
        let doc_repo = CapturingDocumentRepo::default();
        let asset_repo = StubAssetRepo::default();
        let search = StubSearchService::default();
        let storage = MockStorage::new();

        let msg = finalize_document_save(
            &doc_repo,
            &asset_repo,
            Some(&search),
            None,
            None,
            &storage,
            make_doc(),
            "<p>body</p>",
            &[],
        )
        .await
        .unwrap();

        let saved = doc_repo.saved.lock().unwrap().clone().unwrap();
        assert!(!saved.needs_reindex);
        assert!(msg.contains("saved successfully"), "got: {msg}");
    }
}
