use leptos::prelude::*;
use leptos_tiptap::*;

/// Server function to fetch document content for editing.
#[server(GetDocContent, "/api")]
pub async fn get_doc_content(
    slug: String,
) -> Result<Option<(String, String)>, ServerFnError> {
    use crate::db::repository::DocumentRepository;
    use crate::rendering::markdown::render_markdown;
    use crate::storage::client::StorageClient;

    let state = expect_context::<crate::app::AppState>();

    let doc = state.document_repo.find_by_slug(&slug).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(doc) = doc else {
        return Ok(None);
    };

    let content_bytes = state.storage_client.get_object(&doc.s3_key).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(content_bytes) = content_bytes else {
        return Ok(None);
    };

    let raw_markdown = String::from_utf8(content_bytes)
        .map_err(|e| ServerFnError::new(e.to_string()))?;

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

    let content = html_content.clone();
    let links_out = extract_links_from_html(&content);

    let old_doc = state.document_repo.find_by_slug(&slug).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let old_links = old_doc
        .as_ref()
        .map(|d| d.links_out.clone())
        .unwrap_or_default();

    let s3_key = format!("docs/{}.md", slug.replace('/', "_"));

    state.storage_client
        .put_object(&s3_key, content.clone().into_bytes())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let doc = crate::db::models::Document {
        slug: slug.clone(),
        title: title.clone(),
        s3_key: s3_key.clone(),
        access_level: old_doc
            .as_ref()
            .map(|d| d.access_level)
            .unwrap_or(crate::auth::models::AccessLevel::Developer),
        service_owner: old_doc
            .as_ref()
            .map(|d| d.service_owner.clone())
            .unwrap_or_else(|| "web-editor".to_string()),
        last_updated: Utc::now(),
        tags: old_doc
            .as_ref()
            .map(|d| d.tags.clone())
            .unwrap_or_default(),
        links_out: links_out.clone(),
        backlinks: old_doc.map(|d| d.backlinks).unwrap_or_default(),
    };

    let search_doc = state.search_service.as_ref().map(|_| {
        crate::search::client::build_search_document(&doc, &content)
    });

    state.document_repo.create_or_update(doc).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    state.document_repo
        .update_backlinks(&slug, &old_links, &links_out)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if let (Some(svc), Some(sdoc)) = (state.search_service.as_ref(), search_doc) {
        let _ = svc.index_document(&sdoc).await;
    }

    Ok(format!("Document '{}' saved successfully", slug))
}

/// Extract internal link slugs from HTML content.
#[cfg(feature = "ssr")]
fn extract_links_from_html(html: &str) -> Vec<String> {
    let mut links = Vec::new();
    for segment in html.split("href=\"") {
        if let Some(end) = segment.find('"') {
            let url = &segment[..end];
            if url.starts_with("/docs/") {
                let slug = url
                    .trim_start_matches("/docs/")
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('/');
                if !slug.is_empty() && !links.contains(&slug.to_string()) {
                    links.push(slug.to_string());
                }
            }
        }
    }
    links
}

/// The editor page component.
#[component]
pub fn EditorPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let slug = move || params.read().get("slug").unwrap_or_default();

    let doc_resource = Resource::new(
        move || slug(),
        |slug| get_doc_content(slug),
    );

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
