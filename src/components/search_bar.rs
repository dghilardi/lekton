use crate::models::search::SearchDocument;
use leptos::prelude::*;

#[component]
pub fn SearchBar() -> impl IntoView {
    let (query, set_query) = signal(String::new());

    let search_results = LocalResource::new(move || {
        let q = query.get();
        async move {
            if q.is_empty() {
                return Ok(Vec::<SearchDocument>::new());
            }
            let res = reqwest::get(format!("/api/v1/search?q={}", q)).await;
            match res {
                Ok(resp) => resp
                    .json::<Vec<SearchDocument>>()
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            }
        }
    });

    view! {
        <div class="search-container">
            <input
                type="text"
                placeholder="Search documentation..."
                on:input=move |ev| set_query.set(event_target_value(&ev))
                prop:value=query
            />
            <div class="search-results">
                <Suspense fallback=|| view! { <p>"Searching..."</p> }>
                    {move || search_results.get().and_then(|res| {
                        match &*res {
                            Ok(docs) => {
                                if docs.is_empty() && !query.get().is_empty() {
                                    Some(view! { <p>"No results found."</p> }.into_any())
                                } else if docs.is_empty() {
                                    None
                                } else {
                                    let docs = docs.clone();
                                    Some(view! {
                                        <ul>
                                            {docs.into_iter().map(|doc| view! {
                                                <li>
                                                    <a href=format!("/doc/{}", doc.slug)>{doc.title}</a>
                                                </li>
                                            }).collect_view()}
                                        </ul>
                                    }.into_any())
                                }
                            }
                            Err(e) => {
                                let e = e.clone();
                                Some(view! { <p class="error">"Search Error: " {e}</p> }.into_any())
                            }
                        }
                    })}
                </Suspense>
            </div>
        </div>
    }
}
