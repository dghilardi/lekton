use leptos::prelude::*;
use leptos_tiptap::*;

#[server]
pub async fn save_document_content(slug: String, content: String) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use chrono::Utc;

    let state = leptos::prelude::use_context::<AppState>()
        .ok_or_else(|| ServerFnError::new("AppState not found"))?;

    // 1. Link Validation (simplified for now, reusing logic eventually)
    let links = crate::models::link_validator::LinkValidator::extract_links(&content);

    // 2. Upload to S3
    let s3_key = format!("docs/{}.md", slug);
    state.s3.put_object()
        .bucket(&state.config.s3_bucket)
        .key(&s3_key)
        .body(content.into_bytes().into())
        .send()
        .await
        .map_err(|e| ServerFnError::new(format!("S3 error: {}", e)))?;

    // 3. Update MongoDB
    let filter = mongodb::bson::doc! { "slug": &slug };
    let update = mongodb::bson::doc! {
        "$set": {
            "last_updated": Utc::now(),
            "links_out": links,
        }
    };

    state.documents_collection()
        .update_one(filter, update)
        .await
        .map_err(|e| ServerFnError::new(format!("MongoDB error: {}", e)))?;

    Ok(())
}

#[component]
pub fn Editor(slug: String, initial_content: String) -> impl IntoView {
    let content = RwSignal::new(TiptapContent::Html(initial_content));
    let msg = RwSignal::new(TiptapInstanceMsg::Noop);
    let disabled = RwSignal::new(false);
    
    let save_action = Action::new(|(s, c): &(String, String)| {
        let slug = s.clone();
        let content = c.clone();
        async move { save_document_content(slug, content).await }
    });

    let current_content_html = Memo::new(move |_| {
        match content.get() {
            TiptapContent::Html(html) => html,
            TiptapContent::Json(json) => json,
        }
    });

    view! {
        <div class="editor-container">
            <TiptapInstance
                id="lekton-editor"
                value=Signal::derive(move || current_content_html.get())
                set_value=move |v| content.set(v)
                msg=msg.read_only()
                disabled=disabled.read_only()
                on_selection_change=move |_| {}
            />
            <button
                on:click=move |_| { save_action.dispatch((slug.clone(), current_content_html.get())); }
                disabled=move || save_action.pending().get()
            >
                {move || if save_action.pending().get() { "Saving..." } else { "Save Changes" }}
            </button>
            {move || save_action.value().get().and_then(|res| match res {
                Ok(_) => Some(view! { <p class="success">"Saved successfully!"</p> }.into_any()),
                Err(e) => Some(view! { <p class="error">"Error: " {e.to_string()}</p> }.into_any()),
            })}
        </div>
    }
}
