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

/// Metadata + rendered body returned to the editor for an existing page.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EditorDoc {
    pub title: String,
    /// Rendered HTML body, ready for the WYSIWYG editor.
    pub html: String,
    pub access_level: String,
    pub parent_slug: Option<String>,
    pub order: u32,
}

/// Server function to fetch a page's content and editable metadata. Admin only.
/// Returns `Ok(None)` when no page exists at `slug` (the editor then opens in
/// creation mode).
#[server(GetDocContent, "/api")]
pub async fn get_doc_content(slug: String) -> Result<Option<EditorDoc>, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    crate::server::require_admin_user(&state).await?;

    load_editor_page(
        state.document_repo.as_ref(),
        state.storage_client.as_ref(),
        &slug,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Fields the editor form submits to create or update a hand-authored page.
#[cfg(feature = "ssr")]
pub struct EditorPageInput {
    pub slug: String,
    pub title: String,
    pub html_content: String,
    pub access_level: String,
    pub parent_slug: Option<String>,
    pub order: u32,
}

/// Server function to save a hand-authored page (create or update). Admin only.
#[server(SaveDocContent, "/api")]
pub async fn save_doc_content(
    slug: String,
    title: String,
    html_content: String,
    access_level: String,
    parent_slug: Option<String>,
    order: u32,
) -> Result<String, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    crate::server::require_admin_user(&state).await?;

    save_editor_page(
        state.document_repo.as_ref(),
        state.asset_repo.as_ref(),
        state.search_service.as_deref(),
        state.rag_service.as_deref(),
        state.attachment_search_service.as_deref(),
        state.storage_client.as_ref(),
        EditorPageInput {
            slug,
            title,
            html_content,
            access_level,
            parent_slug,
            order,
        },
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Load a page's rendered body and editable metadata, or `None` when it does
/// not exist. Errors when the page is managed outside the editor
/// (ingest / lekton-sync / upload form), which is read-only here.
#[cfg(feature = "ssr")]
pub async fn load_editor_page(
    document_repo: &dyn crate::db::repository::DocumentRepository,
    storage: &dyn crate::storage::client::StorageClient,
    slug: &str,
) -> Result<Option<EditorDoc>, crate::error::AppError> {
    use crate::error::AppError;
    use crate::rendering::markdown::render_markdown;

    let Some(doc) = document_repo.find_by_slug(slug).await? else {
        return Ok(None);
    };

    // Externally-managed (ingest API / lekton-sync) and upload-form documents
    // are read-only in the markdown editor: editing them here would be lost on
    // the next sync, or diverge from the upload form.
    if doc.source_id.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err(AppError::BadRequest(
            "This page is managed outside the editor and can't be edited here.".into(),
        ));
    }

    let Some(content_bytes) = storage.get_object(&doc.s3_key).await? else {
        return Ok(None);
    };
    let raw_markdown =
        String::from_utf8(content_bytes).map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Some(EditorDoc {
        title: doc.title,
        html: render_markdown(&raw_markdown),
        access_level: doc.access_level,
        parent_slug: doc.parent_slug,
        order: doc.order,
    }))
}

/// Create or update a hand-authored page from the editor. Validates the slug and
/// metadata, refuses to overwrite externally-managed pages, writes the body to
/// storage, then persists the document and reconciles search / backlinks / asset
/// references.
#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)]
pub async fn save_editor_page(
    document_repo: &dyn crate::db::repository::DocumentRepository,
    asset_repo: &dyn crate::db::asset_repository::AssetRepository,
    search_service: Option<&dyn crate::search::client::SearchService>,
    rag_service: Option<&dyn crate::rag::service::RagService>,
    attachment_search: Option<&dyn crate::search::attachment_search::AttachmentSearchService>,
    storage: &dyn crate::storage::client::StorageClient,
    input: EditorPageInput,
) -> Result<String, crate::error::AppError> {
    use crate::error::AppError;

    let slug = input.slug.trim();
    if slug.is_empty() {
        return Err(AppError::BadRequest("Slug is required".into()));
    }
    if slug.contains("..") || slug.starts_with('/') {
        return Err(AppError::BadRequest("Invalid slug".into()));
    }
    if input.access_level.trim().is_empty() {
        return Err(AppError::BadRequest("Access level is required".into()));
    }
    let parent_slug = crate::api::ingest::normalize_parent_slug(input.parent_slug.as_deref())?;

    let html_content = input.html_content;
    let links_out = crate::rendering::links::extract_internal_links_from_html(&html_content);

    let old_doc = document_repo.find_by_slug(slug).await?;
    // Refuse to overwrite externally-managed (ingest / lekton-sync) or
    // upload-form documents from the markdown editor.
    if let Some(ref d) = old_doc {
        if d.source_id.as_deref().is_some_and(|s| !s.is_empty()) {
            return Err(AppError::BadRequest(
                "This page is managed outside the editor and can't be edited here.".into(),
            ));
        }
    }
    let old_links = old_doc
        .as_ref()
        .map(|d| d.links_out.clone())
        .unwrap_or_default();

    let doc = build_editor_document(
        slug,
        input.title,
        input.access_level,
        parent_slug,
        input.order,
        links_out,
        old_doc,
    );

    // The body lives in storage; finalize_document_save only persists metadata
    // and reconciles the derived indexes, so write the content here first.
    storage
        .put_object(&doc.s3_key, html_content.clone().into_bytes())
        .await?;

    let result = finalize_document_save(
        document_repo,
        asset_repo,
        search_service,
        rag_service,
        attachment_search,
        storage,
        doc,
        &html_content,
        &old_links,
    )
    .await;

    if result.is_ok() {
        metrics::counter!("lekton_editor_saves_total").increment(1);
    }
    result
}

/// Build the `Document` to persist. Metadata the form owns (title, access level,
/// parent, order) comes from the input; fields it does not (draft state, tags,
/// backlinks, visibility, service owner) are preserved from the existing page on
/// edit and defaulted on creation. A hand-authored page never carries a
/// `source_id`, so it stays editable here.
#[cfg(feature = "ssr")]
fn build_editor_document(
    slug: &str,
    title: String,
    access_level: String,
    parent_slug: Option<String>,
    order: u32,
    links_out: Vec<String>,
    old_doc: Option<crate::db::models::Document>,
) -> crate::db::models::Document {
    use chrono::Utc;

    let (is_draft, service_owner, tags, backlinks, is_hidden) = match &old_doc {
        Some(d) => (
            d.is_draft,
            d.service_owner.clone(),
            d.tags.clone(),
            d.backlinks.clone(),
            d.is_hidden,
        ),
        None => (false, "web-editor".to_string(), vec![], vec![], false),
    };

    crate::db::models::Document {
        slug: slug.to_string(),
        title,
        summary: None,
        s3_key: format!("docs/{}.md", slug.replace('/', "_")),
        access_level,
        is_draft,
        service_owner,
        last_updated: Utc::now(),
        tags,
        links_out,
        backlinks,
        parent_slug,
        order,
        is_hidden,
        content_hash: None,
        metadata_hash: None,
        is_archived: false,
        source_path: None,
        source_id: None,
        release: None,
        is_latest: true,
        needs_reindex: false,
        skip_rag: false,
    }
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

/// The editor page component. Renders the WYSIWYG editor for an existing
/// hand-authored page (edit mode) or, when no page exists at the route slug, a
/// blank editor with an editable slug for creating one (creation mode).
#[component]
pub fn EditorPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let route_slug = move || params.read().get("slug").unwrap_or_default();

    #[allow(clippy::redundant_closure)]
    let doc_resource = Resource::new(move || route_slug(), |slug| get_doc_content(slug));
    let levels_resource = LocalResource::new(list_levels);
    let nav_resource = LocalResource::new(|| crate::server::nav::get_navigation(None));

    let (msg, set_msg) = signal(TiptapInstanceMsg::Noop);
    let (value, set_value) = signal(String::new());
    let (title, set_title) = signal(String::new());
    let (slug, set_slug) = signal(String::new());
    let (access_level, set_access_level) = signal(String::new());
    let (parent_slug, set_parent_slug) = signal(String::new());
    let (order, set_order) = signal(0u32);
    let (disabled, _set_disabled) = signal(false);
    let (_selection, set_selection) = signal(TiptapSelectionState::default());
    let (save_status, set_save_status) = signal(String::new());
    let (saving, set_saving) = signal(false);
    // Sentinel: the route slug whose content is currently loaded in the editor
    // signals. The Effect below only prefills when the route changes, so
    // in-progress edits are never clobbered by a resource refetch for the same doc.
    let (loaded_slug, set_loaded_slug) = signal(Option::<String>::None);

    Effect::new(move || {
        let current_slug = route_slug();
        if let Some(Ok(maybe)) = doc_resource.get() {
            if loaded_slug.get_untracked().as_deref() != Some(current_slug.as_str()) {
                set_loaded_slug.set(Some(current_slug.clone()));
                match maybe {
                    Some(doc) => {
                        set_slug.set(current_slug);
                        set_title.set(doc.title);
                        set_value.set(doc.html);
                        set_access_level.set(doc.access_level);
                        set_parent_slug.set(doc.parent_slug.unwrap_or_default());
                        set_order.set(doc.order);
                    }
                    // Creation mode: seed the (editable) slug from the route, if any.
                    None => set_slug.set(current_slug),
                }
            }
        }
    });

    let save_action = Action::new(move |_: &()| {
        let current_slug = slug.get();
        let current_title = title.get();
        let current_content = value.get();
        let current_access_level = access_level.get();
        let current_parent = {
            let p = parent_slug.get();
            if p.is_empty() {
                None
            } else {
                Some(p)
            }
        };
        let current_order = order.get();
        async move {
            set_saving.set(true);
            set_save_status.set(String::new());
            match save_doc_content(
                current_slug,
                current_title,
                current_content,
                current_access_level,
                current_parent,
                current_order,
            )
            .await
            {
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
                    Ok(existing) => {
                        let is_new = existing.is_none();
                        view! {
                            <div class="space-y-4">
                                <h2 class="text-2xl font-bold">
                                    {if is_new { "Create Page" } else { "Edit Page" }}
                                </h2>

                                // Slug — editable only when creating a new page.
                                <div class="form-control">
                                    <label class="label">
                                        <span class="label-text font-semibold">"Slug (page path)"</span>
                                    </label>
                                    {if is_new {
                                        view! {
                                            <input
                                                type="text"
                                                class="input input-bordered w-full"
                                                placeholder="e.g. guides/getting-started"
                                                prop:value=slug
                                                on:input=move |ev| set_slug.set(event_target_value(&ev))
                                            />
                                        }.into_any()
                                    } else {
                                        view! {
                                            <input type="text" class="input input-bordered w-full" prop:value=slug disabled=true />
                                        }.into_any()
                                    }}
                                </div>

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

                                // Access level
                                <div class="form-control">
                                    <label class="label">
                                        <span class="label-text font-semibold">"Access level"</span>
                                    </label>
                                    <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
                                        {move || {
                                            let levels = levels_resource.get().and_then(|r| r.ok()).unwrap_or_default();
                                            view! {
                                                <select
                                                    class="select select-bordered w-full"
                                                    prop:value=move || access_level.get()
                                                    on:change=move |e| set_access_level.set(event_target_value(&e))
                                                >
                                                    <option value="" disabled selected=move || access_level.get().is_empty()>
                                                        "Select an access level…"
                                                    </option>
                                                    {levels.into_iter().map(|l| {
                                                        let name = l.name.clone();
                                                        let name_sel = l.name.clone();
                                                        let label = if l.label.is_empty() { l.name.clone() } else { l.label.clone() };
                                                        view! {
                                                            <option value=name.clone() selected=move || access_level.get() == name_sel>
                                                                {format!("{label} ({name})")}
                                                            </option>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </select>
                                            }
                                        }}
                                    </Suspense>
                                </div>

                                // Parent + order
                                <div class="grid grid-cols-1 sm:grid-cols-3 gap-4">
                                    <div class="form-control sm:col-span-2">
                                        <label class="label"><span class="label-text font-semibold">"Parent (optional)"</span></label>
                                        <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
                                            {move || {
                                                let nav = nav_resource.get().and_then(|r| r.ok()).unwrap_or_default();
                                                let mut flat = Vec::new();
                                                flatten_nav(&nav, 0, &mut flat);
                                                view! {
                                                    <select
                                                        class="select select-bordered w-full"
                                                        prop:value=move || parent_slug.get()
                                                        on:change=move |e| set_parent_slug.set(event_target_value(&e))
                                                    >
                                                        <option value="">"— Top level —"</option>
                                                        {flat.into_iter().map(|(slug, label)| {
                                                            let slug_sel = slug.clone();
                                                            view! {
                                                                <option value=slug.clone() selected=move || parent_slug.get() == slug_sel>
                                                                    {label}
                                                                </option>
                                                            }
                                                        }).collect::<Vec<_>>()}
                                                    </select>
                                                }
                                            }}
                                        </Suspense>
                                    </div>
                                    <div class="form-control">
                                        <label class="label"><span class="label-text font-semibold">"Order"</span></label>
                                        <input
                                            type="number"
                                            min="0"
                                            class="input input-bordered w-full"
                                            prop:value=move || order.get().to_string()
                                            on:input=move |e| {
                                                if let Ok(n) = event_target_value(&e).parse::<u32>() {
                                                    set_order.set(n);
                                                }
                                            }
                                        />
                                    </div>
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
                                        {move || if saving.get() {
                                            "Saving...".to_string()
                                        } else if is_new {
                                            "Create Page".to_string()
                                        } else {
                                            "Save Document".to_string()
                                        }}
                                    </button>
                                    <a
                                        href=move || {
                                            let s = slug.get();
                                            if s.is_empty() { "/".to_string() } else { format!("/docs/{s}") }
                                        }
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

/// Wrapper so `LocalResource` gets a plain async fn for the access-level picker.
async fn list_levels() -> Result<Vec<crate::server::access_levels::AccessLevelInfo>, ServerFnError>
{
    crate::server::access_levels::list_admin_access_levels().await
}

/// Flatten the navigation tree into `(slug, indented_title)` pairs for the
/// parent-page picker.
fn flatten_nav(items: &[crate::app::NavItem], depth: usize, out: &mut Vec<(String, String)>) {
    for item in items {
        let prefix = "\u{00a0}\u{00a0}".repeat(depth);
        out.push((item.slug.clone(), format!("{prefix}{}", item.title)));
        flatten_nav(&item.children, depth + 1, out);
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
            _: &crate::versioning::ReleasePins,
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
        async fn set_archived(&self, _: &str, _: Option<&str>, _: bool) -> Result<(), AppError> {
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
        async fn health_check(&self) -> Result<(), AppError> {
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
            release: None,
            is_latest: true,
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

    #[test]
    fn build_editor_document_defaults_a_new_page() {
        let doc = build_editor_document(
            "guides/intro",
            "Intro".to_string(),
            "internal".to_string(),
            Some("guides".to_string()),
            3,
            vec!["docs/other".to_string()],
            None,
        );

        assert_eq!(doc.slug, "guides/intro");
        // Slashes are flattened in the storage key.
        assert_eq!(doc.s3_key, "docs/guides_intro.md");
        assert_eq!(doc.title, "Intro");
        assert_eq!(doc.access_level, "internal");
        assert_eq!(doc.parent_slug.as_deref(), Some("guides"));
        assert_eq!(doc.order, 3);
        assert_eq!(doc.links_out, vec!["docs/other".to_string()]);
        // A hand-authored page: web-editor owned, no external source, published.
        assert_eq!(doc.service_owner, "web-editor");
        assert_eq!(doc.source_id, None);
        assert!(!doc.is_draft);
        assert!(!doc.is_archived);
    }

    #[test]
    fn build_editor_document_applies_form_metadata_but_preserves_untouched_fields() {
        // An existing hand-authored page with fields the form does not manage.
        let mut existing = make_doc();
        existing.is_draft = true;
        existing.is_hidden = true;
        existing.tags = vec!["kept".to_string()];
        existing.backlinks = vec!["docs/ref".to_string()];
        existing.access_level = "public".to_string();
        existing.order = 0;
        existing.parent_slug = None;

        let doc = build_editor_document(
            "guide",
            "Guide v2".to_string(),
            "internal".to_string(), // changed via the form
            Some("handbook".to_string()),
            9,
            vec![],
            Some(existing),
        );

        // Form-owned metadata is taken from the input.
        assert_eq!(doc.title, "Guide v2");
        assert_eq!(doc.access_level, "internal");
        assert_eq!(doc.parent_slug.as_deref(), Some("handbook"));
        assert_eq!(doc.order, 9);
        // Fields the form does not manage are preserved from the existing page.
        assert!(doc.is_draft);
        assert!(doc.is_hidden);
        assert_eq!(doc.tags, vec!["kept".to_string()]);
        assert_eq!(doc.backlinks, vec!["docs/ref".to_string()]);
        assert_eq!(doc.service_owner, "web-editor");
    }
}
