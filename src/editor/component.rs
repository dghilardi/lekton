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
    use crate::db::repository::DocumentRepository;
    use crate::rendering::markdown::render_markdown;
    use crate::storage::client::StorageClient;

    let state = expect_context::<crate::app::AppState>();

    let doc = state
        .document_repo
        .find_by_slug(&slug)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(doc) = doc else {
        return Ok(None);
    };

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
    use crate::db::repository::DocumentRepository;
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
        content_hash: None,  // Editor saves don't compute content hash
        metadata_hash: None, // Populated on next lekton-sync run
        is_archived: false,
    };

    let search_doc = state
        .search_service
        .as_ref()
        .map(|_| crate::search::client::build_search_document(&doc, &html_content));

    state
        .document_repo
        .create_or_update(doc)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    state
        .document_repo
        .update_backlinks(&slug, &old_links, &links_out)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if let (Some(svc), Some(sdoc)) = (state.search_service.as_ref(), search_doc) {
        let _ = svc.index_document(&sdoc).await;
    }

    Ok(format!("Document '{}' saved successfully", slug))
}

/// The editor page component.
#[component]
pub fn EditorPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let slug = move || params.read().get("slug").unwrap_or_default();

    let doc_resource = Resource::new(move || slug(), |slug| get_doc_content(slug));

    let (msg, set_msg) = signal(TiptapInstanceMsg::Noop);
    let (value, set_value) = signal(String::new());
    let (title, set_title) = signal(String::new());
    let (disabled, _set_disabled) = signal(false);
    let (_selection, set_selection) = signal(TiptapSelectionState::default());
    let (save_status, set_save_status) = signal(String::new());
    let (saving, set_saving) = signal(false);

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
                    Ok(Some((doc_title, html))) => {
                        set_title.set(doc_title);
                        set_value.set(html);

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
