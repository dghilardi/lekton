use leptos::prelude::*;

#[server]
pub async fn get_document_content(slug: String) -> Result<String, ServerFnError> {
    use crate::state::AppState;
    let state = leptos::prelude::use_context::<AppState>()
        .ok_or_else(|| ServerFnError::new("AppState not found in context"))?;
    
    // 1. Find document metadata in MongoDB
    let filter = mongodb::bson::doc! { "slug": slug };
    let doc = state.documents_collection()
        .find_one(filter)
        .await
        .map_err(|e| ServerFnError::new(format!("MongoDB error: {}", e)))?
        .ok_or_else(|| ServerFnError::new("Document not found"))?;

    // 2. Fetch content from S3
    let output = state.s3.get_object()
        .bucket(&state.config.s3_bucket)
        .key(&doc.s3_key)
        .send()
        .await
        .map_err(|e| ServerFnError::new(format!("S3 error: {}", e)))?;

    let body = output.body.collect().await
        .map_err(|e| ServerFnError::new(format!("S3 body error: {}", e)))?;
        
    Ok(String::from_utf8(body.to_vec())
        .map_err(|e| ServerFnError::new(format!("UTF-8 error: {}", e)))?)
}

#[component]
pub fn DocumentView(slug: String) -> impl IntoView {
    let doc_content = Resource::new(
        move || slug.clone(),
        |s| async move { get_document_content(s).await }
    );

    view! {
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || doc_content.get().map(|res| {
                match res {
                    Ok(content) => {
                        let parser = pulldown_cmark::Parser::new(&content);
                        let mut html_output = String::new();
                        pulldown_cmark::html::push_html(&mut html_output, parser);
                        view! { <div inner_html=html_output></div> }.into_any()
                    },
                    Err(e) => view! { <p class="error">"Error: " {e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}
