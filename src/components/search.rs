use leptos::prelude::*;

use crate::app::search_docs;

/// Global search modal triggered by Ctrl+K (or Cmd+K on Mac).
#[component]
pub fn SearchModal(is_open: ReadSignal<bool>, set_is_open: WriteSignal<bool>) -> impl IntoView {
    let (query, set_query) = signal(String::new());

    let search_resource = Resource::new(
        move || query.get(),
        |q| async move {
            if q.len() < 2 {
                return Ok(vec![]);
            }
            search_docs(q).await
        },
    );

    let on_keydown = move |ev: leptos::web_sys::KeyboardEvent| {
        if ev.key() == "Escape" {
            set_is_open.set(false);
        }
    };

    view! {
        <Show when=move || is_open.get()>
            <div
                class="fixed inset-0 z-[200] flex items-start justify-center pt-20 bg-black/50 backdrop-blur-sm"
                on:click=move |_| set_is_open.set(false)
            >
                <div
                    class="bg-base-100 rounded-lg shadow-2xl w-full max-w-2xl mx-4"
                    on:click=move |ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                >
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
                                prop:value=query
                                on:input=move |ev| {
                                    set_query.set(event_target_value(&ev));
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
                                            <div class="divide-y divide-base-300">
                                                {hits.into_iter().map(|hit| {
                                                    let slug = hit.slug.clone();
                                                    let title = hit.title.clone();
                                                    let preview = hit.content_preview.clone();
                                                    let tags = hit.tags.clone();
                                                    let has_tags = !tags.is_empty();

                                                    view! {
                                                        <a
                                                            href=format!("/docs/{}", slug)
                                                            class="block p-4 hover:bg-base-200 transition-colors"
                                                            on:click=move |_| set_is_open.set(false)
                                                        >
                                                            <div class="font-semibold text-lg mb-1">{title}</div>
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
    let (show_results, set_show_results) = signal(false);

    let search_resource = Resource::new(
        move || query.get(),
        |q| async move {
            if q.len() < 2 {
                return Ok(vec![]);
            }
            search_docs(q).await
        },
    );

    view! {
        <div class="dropdown dropdown-end">
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
                                            let slug = hit.slug.clone();
                                            view! {
                                                <li>
                                                    <a href=format!("/docs/{}", slug) class="flex flex-col items-start">
                                                        <span class="font-semibold">{hit.title}</span>
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
