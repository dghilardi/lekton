use leptos::prelude::*;

use crate::app::search_docs;
use crate::auth::refresh_client::with_auth_retry;
use crate::search::client::SearchHit;

const SEARCH_DEBOUNCE_MS: u32 = 250;
const SEARCH_RESULTS_LIST_ID: &str = "global-search-results";

fn search_hit_href(hit: &SearchHit) -> String {
    match hit.attachment_key.as_deref() {
        Some(key) => match hit.page {
            Some(page) => format!("/api/v1/assets/{key}#page={page}"),
            None => format!("/api/v1/assets/{key}"),
        },
        None => format!("/docs/{}", hit.slug),
    }
}

fn search_hit_target(hit: &SearchHit) -> &'static str {
    if hit.attachment_key.is_some() {
        "_blank"
    } else {
        "_self"
    }
}

fn search_result_id(index: usize) -> String {
    format!("global-search-result-{index}")
}

fn schedule_debounced_query(
    value: String,
    debounce_version: RwSignal<u64>,
    set_debounced_query: WriteSignal<String>,
) {
    let version = debounce_version.get_untracked().wrapping_add(1);
    debounce_version.set(version);

    leptos::task::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(SEARCH_DEBOUNCE_MS).await;
        if debounce_version.get_untracked() == version {
            set_debounced_query.set(value);
        }
    });
}

/// Global search modal triggered by Ctrl+K (or Cmd+K on Mac).
#[component]
pub fn SearchModal(is_open: ReadSignal<bool>, set_is_open: WriteSignal<bool>) -> impl IntoView {
    let (query, set_query) = signal(String::new());
    let (debounced_query, set_debounced_query) = signal(String::new());
    let (active_index, set_active_index) = signal(None::<usize>);
    let debounce_version = RwSignal::new(0_u64);
    let input_ref = NodeRef::<leptos::html::Input>::new();

    let search_resource = LocalResource::new(move || {
        let q = debounced_query.get();
        async move {
            if q.len() < 2 {
                return Ok(vec![]);
            }
            with_auth_retry(|| search_docs(q.clone())).await
        }
    });

    let search_hits = Signal::derive(move || match search_resource.get() {
        Some(Ok(hits)) => hits,
        _ => vec![],
    });

    let on_keydown = move |ev: leptos::web_sys::KeyboardEvent| match ev.key().as_str() {
        "Escape" => set_is_open.set(false),
        "ArrowDown" => {
            let hits = search_hits.get_untracked();
            if !hits.is_empty() {
                ev.prevent_default();
                set_active_index.update(|current| {
                    *current = Some(match *current {
                        Some(index) => (index + 1).min(hits.len().saturating_sub(1)),
                        None => 0,
                    });
                });
            }
        }
        "ArrowUp" => {
            let hits = search_hits.get_untracked();
            if !hits.is_empty() {
                ev.prevent_default();
                set_active_index.update(|current| {
                    *current = Some(match *current {
                        Some(index) if index > 0 => index - 1,
                        _ => 0,
                    });
                });
            }
        }
        "Enter" => {
            if let Some(index) = active_index.get_untracked() {
                let hits = search_hits.get_untracked();
                if hits.get(index).is_some() {
                    ev.prevent_default();
                    set_is_open.set(false);
                    #[cfg(feature = "hydrate")]
                    if let (Some(window), Some(hit)) = (web_sys::window(), hits.get(index)) {
                        let href = search_hit_href(hit);
                        if search_hit_target(hit) == "_blank" {
                            let _ = window.open_with_url_and_target(&href, "_blank");
                        } else {
                            let _ = window.location().set_href(&href);
                        }
                    }
                }
            }
        }
        _ => {}
    };

    Effect::new(move || {
        if is_open.get() {
            set_active_index.set(None);
            #[cfg(feature = "hydrate")]
            if let Some(input) = input_ref.get() {
                let _ = input.focus();
                let _ = input.select();
            }
        }
    });

    Effect::new(move || {
        let hits = search_hits.get();
        set_active_index.update(|current| {
            *current = match (*current, hits.is_empty()) {
                (_, true) => None,
                (Some(index), false) if index < hits.len() => Some(index),
                _ => Some(0),
            };
        });
    });

    view! {
        <Show when=move || is_open.get()>
            <div
                class="fixed inset-0 z-[200] flex items-start justify-center pt-20 bg-black/50 backdrop-blur-sm"
                on:click=move |_| set_is_open.set(false)
            >
                <div
                    class="bg-base-100 rounded-lg shadow-2xl w-full max-w-2xl mx-4"
                    on:click=move |ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                    on:keydown=on_keydown
                    role="dialog"
                    aria-modal="true"
                    aria-labelledby="global-search-title"
                    tabindex="-1"
                >
                    <h2 id="global-search-title" class="sr-only">"Search documentation"</h2>
                    // Search input
                    <div class="p-4 border-b border-base-200 bg-base-100/50 rounded-t-lg">
                        <div class="flex items-center gap-3">
                            <svg class="w-6 h-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path>
                            </svg>
                            <input
                                type="text"
                                placeholder="Search documentation..."
                                class="w-full bg-transparent focus:outline-none text-xl placeholder:text-base-content/30"
                                node_ref=input_ref
                                prop:value=query
                                aria-label="Search documentation"
                                aria-controls=SEARCH_RESULTS_LIST_ID
                                aria-activedescendant=move || {
                                    active_index
                                        .get()
                                        .map(search_result_id)
                                        .unwrap_or_default()
                                }
                                on:input=move |ev| {
                                    let value = event_target_value(&ev);
                                    set_query.set(value.clone());
                                    set_active_index.set(None);
                                    if value.len() < 2 {
                                        set_debounced_query.set(value);
                                    } else {
                                        schedule_debounced_query(
                                            value,
                                            debounce_version,
                                            set_debounced_query,
                                        );
                                    }
                                }
                                on:keydown=on_keydown
                                autofocus
                            />
                            <kbd class="kbd kbd-sm bg-base-200 border-none shadow-sm text-xs font-semibold">"ESC"</kbd>
                        </div>
                    </div>

                    // Results area
                    <div class="max-h-96 overflow-y-auto">
                        <Suspense fallback=move || view! {
                            <div class="flex justify-center p-8">
                                <span class="loading loading-spinner loading-lg"></span>
                            </div>
                        }>
                            {move || {
                                let q = query.get();
                                if q.len() < 2 {
                                    return Some(view! {
                                        <div class="p-8 text-center text-base-content/50">
                                            "Type at least 2 characters to search..."
                                        </div>
                                    }.into_any());
                                }

                                search_resource.get().map(|result| match result {
                                    Ok(hits) if hits.is_empty() => {
                                        view! {
                                            <div class="p-8 text-center text-base-content/50">
                                                "No results found for \"" {q.clone()} "\""
                                            </div>
                                        }.into_any()
                                    }
                                    Ok(hits) => {
                                        view! {
                                            <div id=SEARCH_RESULTS_LIST_ID role="listbox" class="divide-y divide-base-300">
                                                {hits.into_iter().enumerate().map(|(idx, hit)| {
                                                    let href = search_hit_href(&hit);
                                                    let target = search_hit_target(&hit);
                                                    let title = hit.title.clone();
                                                    let preview = hit.content_preview.clone();
                                                    let tags = hit.tags.clone();
                                                    let has_tags = !tags.is_empty();
                                                    let has_page = hit.page.is_some();
                                                    let page_badge = hit.page.map(|p| format!("PDF · page {p}")).unwrap_or_default();
                                                    let result_id = search_result_id(idx);

                                                    view! {
                                                        <a
                                                            id=result_id
                                                            href=href
                                                            target=target
                                                            role="option"
                                                            aria-selected=move || active_index.get() == Some(idx)
                                                            class=move || format!(
                                                                "block p-4 transition-colors {}",
                                                                if active_index.get() == Some(idx) {
                                                                    "bg-base-200"
                                                                } else {
                                                                    "hover:bg-base-200"
                                                                }
                                                            )
                                                            on:click=move |_| set_is_open.set(false)
                                                            on:mouseenter=move |_| set_active_index.set(Some(idx))
                                                        >
                                                            <div class="flex items-center gap-2 mb-1">
                                                                <div class="font-semibold text-lg">{title}</div>
                                                                <Show when=move || has_page>
                                                                    <span class="badge badge-sm badge-ghost text-base-content/60">
                                                                        {page_badge.clone()}
                                                                    </span>
                                                                </Show>
                                                            </div>
                                                            <div class="text-sm text-base-content/70 mb-2">{preview}</div>
                                                            <Show when=move || has_tags>
                                                                <div class="flex gap-2 flex-wrap">
                                                                    {tags.iter().map(|tag| {
                                                                        let tag_text = tag.clone();
                                                                        view! {
                                                                            <span class="badge badge-sm badge-outline border-primary/30 text-primary/80">{tag_text}</span>
                                                                        }
                                                                    }).collect::<Vec<_>>()}
                                                                </div>
                                                            </Show>
                                                        </a>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        }.into_any()
                                    }
                                    Err(e) => {
                                        view! {
                                            <div class="p-8 text-center text-error">
                                                "Search error: " {e.to_string()}
                                            </div>
                                        }.into_any()
                                    }
                                })
                            }}
                        </Suspense>
                    </div>

                    // Footer with keyboard hints
                    <div class="p-3 border-t border-base-300 bg-base-200/50 rounded-b-lg">
                        <div class="flex items-center justify-between text-xs text-base-content/50">
                            <div class="flex items-center gap-4">
                                <span>"Press ESC to close"</span>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </Show>
    }
}

/// Search bar component with live results dropdown.
#[component]
pub fn SearchBar() -> impl IntoView {
    let (query, set_query) = signal(String::new());
    let (debounced_query, set_debounced_query) = signal(String::new());
    let (show_results, set_show_results) = signal(false);
    let debounce_version = RwSignal::new(0_u64);
    let root_ref = NodeRef::<leptos::html::Div>::new();

    let search_resource = LocalResource::new(move || {
        let q = debounced_query.get();
        async move {
            if q.len() < 2 {
                return Ok(vec![]);
            }
            with_auth_retry(|| search_docs(q.clone())).await
        }
    });

    view! {
        <div
            class="dropdown dropdown-end"
            node_ref=root_ref
            on:focusout=move |_ev| {
                #[cfg(feature = "hydrate")]
                {
                    use wasm_bindgen::JsCast;

                    let next_target = _ev.related_target().and_then(|target| target.dyn_into::<web_sys::Node>().ok());
                    let contains_focus = root_ref
                        .get()
                        .map(|root| match next_target.as_ref() {
                            Some(next) => root.contains(Some(next)),
                            None => false,
                        })
                        .unwrap_or(false);

                    if !contains_focus {
                        set_show_results.set(false);
                    }
                }
            }
        >
            <div class="form-control">
                <input
                    type="text"
                    placeholder="Search docs..."
                    class="input input-bordered w-24 md:w-64"
                    prop:value=query
                    on:input=move |ev| {
                        let val = event_target_value(&ev);
                        set_query.set(val.clone());
                        set_show_results.set(val.len() >= 2);
                        if val.len() < 2 {
                            set_debounced_query.set(val);
                        } else {
                            schedule_debounced_query(
                                val,
                                debounce_version,
                                set_debounced_query,
                            );
                        }
                    }
                    on:focus=move |_| {
                        if query.get().len() >= 2 {
                            set_show_results.set(true);
                        }
                    }
                />
            </div>
            <Show when=move || show_results.get()>
                <ul class="dropdown-content menu bg-base-100 rounded-box z-[100] w-80 p-2 shadow-lg mt-2 max-h-80 overflow-y-auto">
                    <Suspense fallback=move || view! { <li><span class="loading loading-spinner loading-sm"></span></li> }>
                        {move || {
                            search_resource.get().map(|result| match result {
                                Ok(hits) if hits.is_empty() => {
                                    view! {
                                        <li class="text-base-content/50 p-2">"No results found"</li>
                                    }.into_any()
                                }
                                Ok(hits) => {
                                    view! {
                                        {hits.into_iter().map(|hit| {
                                            let href = search_hit_href(&hit);
                                            let target = search_hit_target(&hit);
                                            let page_badge = hit.page.map(|p| format!(" · PDF p.{p}"));
                                            view! {
                                                <li>
                                                    <a
                                                        href=href
                                                        target=target
                                                        class="flex flex-col items-start"
                                                        on:click=move |_| set_show_results.set(false)
                                                    >
                                                        <span class="font-semibold">
                                                            {hit.title}
                                                            {page_badge.map(|b| view! {
                                                                <span class="text-xs font-normal text-base-content/50">{b}</span>
                                                            })}
                                                        </span>
                                                        <span class="text-xs text-base-content/50 truncate w-full">
                                                            {hit.content_preview}
                                                        </span>
                                                    </a>
                                                </li>
                                            }
                                        }).collect::<Vec<_>>()}
                                    }.into_any()
                                }
                                Err(_) => {
                                    view! {
                                        <li class="text-error p-2">"Search error"</li>
                                    }.into_any()
                                }
                            })
                        }}
                    </Suspense>
                </ul>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_hit_href_points_pdf_hits_to_asset_page() {
        let hit = SearchHit {
            slug: "manual".into(),
            title: "Manual".into(),
            tags: vec![],
            content_preview: "preview".into(),
            attachment_key: Some("files/manual.pdf".into()),
            page: Some(7),
        };

        assert_eq!(
            search_hit_href(&hit),
            "/api/v1/assets/files/manual.pdf#page=7"
        );
        assert_eq!(search_hit_target(&hit), "_blank");
    }

    #[test]
    fn search_hit_href_points_document_hits_to_docs_page() {
        let hit = SearchHit {
            slug: "manual".into(),
            title: "Manual".into(),
            tags: vec![],
            content_preview: "preview".into(),
            attachment_key: None,
            page: None,
        };

        assert_eq!(search_hit_href(&hit), "/docs/manual");
        assert_eq!(search_hit_target(&hit), "_self");
    }
}
