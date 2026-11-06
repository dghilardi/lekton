use leptos::prelude::*;

use crate::app::search_docs;
use crate::auth::refresh_client::with_auth_retry;
use crate::search::client::SearchHit;

#[cfg(feature = "hydrate")]
const SEARCH_DEBOUNCE_MS: u32 = 250;
const SEARCH_RESULTS_LIST_ID: &str = "global-search-results";

fn search_hit_href(hit: &SearchHit, pins: &[String]) -> String {
    match hit.attachment_key.as_deref() {
        Some(key) => match hit.page {
            Some(page) => format!("/api/v1/assets/{key}#page={page}"),
            None => format!("/api/v1/assets/{key}"),
        },
        None => crate::versioning::with_release_pins(&format!("/docs/{}", hit.slug), pins),
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
    #[cfg(feature = "hydrate")]
    {
        let version = debounce_version.get_untracked().wrapping_add(1);
        debounce_version.set(version);

        leptos::task::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(SEARCH_DEBOUNCE_MS).await;
            if debounce_version.get_untracked() == version {
                set_debounced_query.set(value);
            }
        });
    }

    #[cfg(not(feature = "hydrate"))]
    {
        let _ = debounce_version;
        set_debounced_query.set(value);
    }
}

/// Global search modal triggered by Ctrl+K (or Cmd+K on Mac).
#[component]
pub fn SearchModal(is_open: ReadSignal<bool>, set_is_open: WriteSignal<bool>) -> impl IntoView {
    let (query, set_query) = signal(String::new());
    let (debounced_query, set_debounced_query) = signal(String::new());
    let (active_index, set_active_index) = signal(None::<usize>);
    let debounce_version = RwSignal::new(0_u64);
    let input_ref = NodeRef::<leptos::html::Input>::new();
    let dialog_ref = NodeRef::<leptos::html::Dialog>::new();

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
                        let href =
                            search_hit_href(hit, &crate::components::active_release_pin_values());
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
        let open = is_open.get();
        #[cfg(feature = "hydrate")]
        if let Some(dialog) = dialog_ref.get() {
            if open {
                // Native modality makes the background inert and restores the
                // invoking control on close, including Escape and backdrop clicks.
                let _ = dialog.show_modal();
                set_active_index.set(None);
                if let Some(input) = input_ref.get() {
                    let _ = input.focus();
                    input.select();
                }
            } else {
                dialog.close();
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = open;
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
        <dialog
            id="global-search-dialog"
            node_ref=dialog_ref
            on:keydown=move |ev: leptos::web_sys::KeyboardEvent| {
                if ev.key() == "Tab" {
                    #[cfg(feature = "hydrate")]
                    {
                        let handled = js_sys::eval(&format!(r#"(() => {{
                            const dialog = document.getElementById('global-search-dialog');
                            const controls = [...dialog.querySelectorAll('input, button, a[href]')].filter(el => el.tabIndex >= 0 && !el.disabled);
                            const first = controls[0], last = controls[controls.length - 1];
                            if ({} && document.activeElement === first) {{ last.focus(); return true; }}
                            if (!{} && document.activeElement === last) {{ first.focus(); return true; }}
                            return false;
                        }})()"#, ev.shift_key(), ev.shift_key())).ok().and_then(|v| v.as_bool()).unwrap_or(false);
                        if handled { ev.prevent_default(); }
                    }
                }
            }
            class="lekton-search-dialog bg-base-100 text-base-content rounded-lg shadow-2xl"
            aria-labelledby="global-search-title"
            on:cancel=move |_: leptos::web_sys::Event| set_is_open.set(false)
            on:close=move |_| set_is_open.set(false)
            on:click=move |ev: leptos::web_sys::MouseEvent| {
                #[cfg(feature = "hydrate")]
                if let Some(dialog) = dialog_ref.get() {
                    let rect = dialog.get_bounding_client_rect();
                    let x = f64::from(ev.client_x());
                    let y = f64::from(ev.client_y());
                    if x < rect.left() || x > rect.right() || y < rect.top() || y > rect.bottom() {
                        set_is_open.set(false);
                    }
                }
                #[cfg(not(feature = "hydrate"))]
                let _ = ev;
            }
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
                                class="min-w-0 w-full bg-transparent text-lg placeholder:text-base-content/65"
                                node_ref=input_ref
                                prop:value=query
                                aria-label="Search documentation"
                                role="combobox"
                                aria-expanded="true"
                                aria-autocomplete="list"
                                aria-controls=SEARCH_RESULTS_LIST_ID
                                aria-activedescendant=move || {
                                    active_index
                                        .get()
                                        .map(search_result_id)
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
                            <button type="button" class="btn btn-ghost btn-square shrink-0" aria-label="Close search" on:click=move |_| set_is_open.set(false)>"✕"</button>
                        </div>
                    </div>

                    // Results area — carries the id referenced by the input's
                    // `aria-controls` so the reference always resolves (the inner
                    // listbox only exists once there are results).
                    <div id=SEARCH_RESULTS_LIST_ID role=move || (!search_hits.get().is_empty()).then_some("listbox") aria-label="Search results" class="min-h-0 overflow-y-auto">
                        <Suspense fallback=move || view! {
                            <div class="flex justify-center p-8">
                                <span class="loading loading-spinner loading-lg"></span>
                            </div>
                        }>
                            {move || {
                                let q = query.get();
                                if q.len() < 2 {
                                    return Some(view! {
                                        <div role="status" class="p-8 text-center text-base-content/65">
                                            "Type at least 2 characters to search..."
                                        </div>
                                    }.into_any());
                                }

                                search_resource.get().map(|result| match result {
                                    Ok(hits) if hits.is_empty() => {
                                        view! {
                                            <div role="status" class="p-8 text-center text-base-content/65">
                                                "No results found for \"" {q.clone()} "\""
                                            </div>
                                        }.into_any()
                                    }
                                    Ok(hits) => {
                                        view! {
                                            <div class="divide-y divide-base-300">
                                                {hits.into_iter().enumerate().map(|(idx, hit)| {
                                                    let href = search_hit_href(
                                                        &hit,
                                                        &crate::components::active_release_pin_values(),
                                                    );
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
                                                            tabindex="-1"
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
                                        // Surface a friendly message; keep the raw
                                        // error in the browser console for debugging
                                        // rather than leaking internals to the user.
                                        leptos::logging::error!("search failed: {e}");
                                        view! {
                                            <div role="status" class="p-8 text-center">
                                                <p class="font-medium text-base-content/80">"Search is temporarily unavailable"</p>
                                                <p class="mt-1 text-sm text-base-content/60">"Please try again in a moment."</p>
                                            </div>
                                        }.into_any()
                                    }
                                })
                            }}
                        </Suspense>
                    </div>

                    <p class="sr-only" role="status">{move || {
                        let count = search_hits.get().len();
                        if count > 0 { format!("{count} results available") } else { String::new() }
                    }}</p>
                    // Footer with keyboard hints
                    <div class="p-3 border-t border-base-300 bg-base-200/50 rounded-b-lg">
                        <div class="lekton-search-hints flex flex-wrap items-center gap-4 text-xs text-base-content/65">
                            <span class="flex items-center gap-1.5">
                                <kbd class="kbd kbd-xs">"↑"</kbd>
                                <kbd class="kbd kbd-xs">"↓"</kbd>
                                "to navigate"
                            </span>
                            <span class="flex items-center gap-1.5">
                                <kbd class="kbd kbd-xs">"↵"</kbd>
                                "to select"
                            </span>
                            <span class="flex items-center gap-1.5">
                                <kbd class="kbd kbd-xs">"esc"</kbd>
                                "to close"
                            </span>
                        </div>
                    </div>
        </dialog>
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
                    aria-label="Search documentation"
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
                                        <li class="text-base-content/65 p-2">"No results found"</li>
                                    }.into_any()
                                }
                                Ok(hits) => {
                                    view! {
                                        {hits.into_iter().map(|hit| {
                                            let href = search_hit_href(
                                                &hit,
                                                &crate::components::active_release_pin_values(),
                                            );
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
                                                                <span class="text-xs font-normal text-base-content/65">{b}</span>
                                                            })}
                                                        </span>
                                                        <span class="text-xs text-base-content/65 truncate w-full">
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
                                        <li class="text-base-content/60 p-2">"Search unavailable"</li>
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
            search_hit_href(&hit, &[]),
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

        assert_eq!(search_hit_href(&hit, &[]), "/docs/manual");
        assert_eq!(
            search_hit_href(&hit, &["svc:1.0.0".to_string()]),
            "/docs/manual?v=svc:1.0.0"
        );
        assert_eq!(search_hit_target(&hit), "_self");
    }
}
