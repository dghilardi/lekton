use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::app::{
    get_current_user, get_prompt_detail, get_prompt_library_state, prompt_filter_options,
    save_prompt_preference, PromptFilter, PromptLibraryItem, PromptLibraryState,
};
use crate::auth::refresh_client::{with_auth_bootstrap, with_auth_retry};

#[component]
pub fn PromptsPage() -> impl IntoView {
    let user_resource = LocalResource::new(|| with_auth_bootstrap(get_current_user));
    let navigate = use_navigate();
    // Taken now: by the time the effect re-runs, the URL may be the login page.
    let login_href = crate::components::use_login_href().get_untracked();

    Effect::new(move |_| {
        if let Some(Ok(None)) = user_resource.get() {
            navigate(&login_href, Default::default());
        }
    });

    view! {
        <div class="container mx-auto max-w-6xl px-4 py-8">
            <leptos_meta::Title text=crate::components::page_title(&["Prompt Library"]) />
            <h1 class="text-3xl font-bold">"Prompt Library"</h1>
            <p class="text-base-content/70 mt-2 mb-6 max-w-3xl">
                "Published prompts can be added to your working context. Default primary prompts come from the shared library; favorites and hidden flags are personal preferences."
            </p>
            <PromptLibraryPanel />
        </div>
    }
}

/// Tags shown on a library row before the "+n" overflow.
const ROW_TAG_LIMIT: usize = 4;

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

#[component]
fn PromptLibraryPanel() -> impl IntoView {
    let state = RwSignal::new(None::<PromptLibraryState>);
    let query = RwSignal::new(String::new());
    let owner = RwSignal::new(String::new());
    let tag = RwSignal::new(String::new());
    let cost = RwSignal::new(String::new());
    let favorites_only = RwSignal::new(false);
    let saving_slug = RwSignal::new(None::<String>);
    let open_slug = RwSignal::new(None::<String>);

    let load = crate::components::browser_action(move |_: &()| async move {
        match with_auth_retry(get_prompt_library_state).await {
            Ok(value) => state.set(Some(value)),
            Err(err) => tracing::error!("Failed to load prompt library: {err}"),
        }
    });

    Effect::new(move |_| {
        load.dispatch(());
    });

    // One action for every favorite/hidden toggle; the server answers with
    // the refreshed library.
    let save = crate::components::browser_action(
        move |(slug, is_favorite, is_hidden): &(String, bool, bool)| {
            let (slug, is_favorite, is_hidden) = (slug.clone(), *is_favorite, *is_hidden);
            async move {
                saving_slug.set(Some(slug.clone()));
                let result = with_auth_retry(|| {
                    save_prompt_preference(slug.clone(), is_favorite, is_hidden)
                })
                .await;
                saving_slug.set(None);
                match result {
                    Ok(updated) => state.set(Some(updated)),
                    Err(err) => tracing::error!("Failed to save prompt preference: {err}"),
                }
            }
        },
    );
    let on_toggle = Callback::new(move |args: (String, bool, bool)| {
        save.dispatch(args);
    });
    let on_open = Callback::new(move |slug: String| open_slug.set(Some(slug)));

    let filter = Memo::new(move |_| PromptFilter {
        query: query.get(),
        owner: non_empty(owner.get()),
        tag: non_empty(tag.get()),
        cost: non_empty(cost.get()),
        favorites_only: favorites_only.get(),
    });
    let options = Memo::new(move |_| {
        state.with(|s| {
            s.as_ref()
                .map(|s| prompt_filter_options(&s.items))
                .unwrap_or_default()
        })
    });
    let visible = Memo::new(move |_| {
        state.with(|s| {
            s.as_ref()
                .map(|s| {
                    filter.with(|f| {
                        s.items
                            .iter()
                            .filter(|i| f.matches(i))
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                })
                .unwrap_or_default()
        })
    });
    let total = Memo::new(move |_| state.with(|s| s.as_ref().map(|s| s.items.len()).unwrap_or(0)));
    let clear_filters = move |_| {
        query.set(String::new());
        owner.set(String::new());
        tag.set(String::new());
        cost.set(String::new());
        favorites_only.set(false);
    };

    view! {
        <div class="space-y-4">
            <div class="flex flex-wrap items-center gap-2" role="search">
                <input
                    type="search"
                    class="input input-bordered input-sm w-full md:w-72"
                    placeholder="Search name, description, owner or tag"
                    aria-label="Search prompts"
                    prop:value=move || query.get()
                    on:input=move |ev| query.set(event_target_value(&ev))
                />
                <select
                    class="select select-bordered select-sm w-full sm:w-auto"
                    aria-label="Filter by owner"
                    prop:value=move || owner.get()
                    on:change=move |ev| owner.set(event_target_value(&ev))
                >
                    <option value="">"All owners"</option>
                    {move || options.get().0.into_iter().map(|o| view! { <option value=o.clone()>{o.clone()}</option> }).collect::<Vec<_>>()}
                </select>
                <select
                    class="select select-bordered select-sm w-full sm:w-auto"
                    aria-label="Filter by tag"
                    prop:value=move || tag.get()
                    on:change=move |ev| tag.set(event_target_value(&ev))
                >
                    <option value="">"All tags"</option>
                    {move || options.get().1.into_iter().map(|(t, n)| view! { <option value=t.clone()>{format!("{t} ({n})")}</option> }).collect::<Vec<_>>()}
                </select>
                <select
                    class="select select-bordered select-sm w-full sm:w-auto"
                    aria-label="Filter by context cost"
                    prop:value=move || cost.get()
                    on:change=move |ev| cost.set(event_target_value(&ev))
                >
                    <option value="">"Any cost"</option>
                    <option value="low">"Low cost"</option>
                    <option value="medium">"Medium cost"</option>
                    <option value="high">"High cost"</option>
                </select>
                <label class="label cursor-pointer gap-2 px-1">
                    <input
                        type="checkbox"
                        class="toggle toggle-sm toggle-primary"
                        prop:checked=move || favorites_only.get()
                        on:change=move |ev| favorites_only.set(event_target_checked(&ev))
                    />
                    <span class="label-text">"Favorites only"</span>
                </label>
                <Show when=move || filter.get() != PromptFilter::default() fallback=|| ()>
                    <button class="btn btn-ghost btn-sm" on:click=clear_filters>"Clear filters"</button>
                </Show>
            </div>

            <Show
                when=move || state.get().is_some()
                fallback=|| view! { <div class="skeleton h-64 w-full rounded-2xl" /> }
            >
                {move || {
                    let library = state.get()?;
                    let warning_text = library.warnings.join(" ");
                    Some(view! {
                        <div class="flex flex-wrap items-center gap-3 text-sm text-base-content/70">
                            <span aria-live="polite">
                                {move || format!("{} of {} prompts", visible.with(Vec::len), total.get())}
                            </span>
                            <span class="badge badge-outline">
                                {format!("Context cost: {}", library.estimated_context_cost)}
                            </span>
                            {(!warning_text.is_empty()).then(|| view! {
                                <span class="text-warning-content bg-warning/20 rounded-md px-2 py-0.5">{warning_text.clone()}</span>
                            })}
                        </div>
                    })
                }}

                <Show
                    when=move || !visible.with(Vec::is_empty)
                    fallback=|| view! {
                        <p class="rounded-box border border-base-300 px-4 py-8 text-center text-base-content/70">
                            "No prompts match these filters."
                        </p>
                    }
                >
                    <ul class="divide-y divide-base-300 rounded-box border border-base-300 bg-base-100">
                        <For
                            each=move || visible.get()
                            key=|item| (item.slug.clone(), item.is_favorite, item.is_hidden)
                            children=move |item| view! {
                                <PromptRow item=item saving_slug=saving_slug on_toggle=on_toggle on_open=on_open />
                            }
                        />
                    </ul>
                </Show>
            </Show>

            <PromptDetailModal open_slug=open_slug state=state saving_slug=saving_slug on_toggle=on_toggle />
        </div>
    }
}

/// One library entry: name (opens the prompt), description, cost, tags and
/// a favorite star.
#[component]
fn PromptRow(
    item: PromptLibraryItem,
    saving_slug: RwSignal<Option<String>>,
    on_toggle: Callback<(String, bool, bool)>,
    on_open: Callback<String>,
) -> impl IntoView {
    let slug = item.slug.clone();
    let slug_open = item.slug.clone();
    let slug_saving = item.slug.clone();
    let is_favorite = item.is_favorite;
    let is_hidden = item.is_hidden;
    let can_favorite = item.publish_to_mcp;
    let extra_tags = item.tags.len().saturating_sub(ROW_TAG_LIMIT);
    let star_label = if !can_favorite {
        "Not publishable to MCP"
    } else if is_favorite {
        "Remove from favorites"
    } else {
        "Add to favorites"
    };

    view! {
        <li class="flex items-start gap-2 px-3 py-3 sm:px-4">
            <button
                class=move || format!(
                    "btn btn-ghost btn-sm btn-square shrink-0 {}",
                    if is_favorite { "text-warning" } else { "text-base-content/70" }
                )
                aria-pressed=if is_favorite { "true" } else { "false" }
                aria-label=star_label
                title=star_label
                prop:disabled=move || !can_favorite || saving_slug.get().as_deref() == Some(slug_saving.as_str())
                on:click=move |_| on_toggle.run((slug.clone(), !is_favorite, is_hidden))
            >
                <svg class="w-4 h-4" viewBox="0 0 24 24" fill=if is_favorite { "currentColor" } else { "none" } stroke="currentColor" stroke-width="2" stroke-linejoin="round" aria-hidden="true"><path d="m12 2 3.09 6.26L22 9.27l-5 4.87L18.18 21 12 17.77 5.82 21 7 14.14l-5-4.87 6.91-1.01z"/></svg>
            </button>
            <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
                    <button
                        class="link link-hover min-w-0 max-w-full break-words text-left font-semibold"
                        on:click=move |_| on_open.run(slug_open.clone())
                    >
                        {item.name.clone()}
                    </button>
                    <span class="badge badge-ghost badge-sm">{format!("{} cost", item.context_cost)}</span>
                    {(item.status != "active").then(|| view! { <span class="badge badge-outline badge-sm">{item.status.clone()}</span> })}
                    {item.default_primary.then(|| view! {
                        <span class="badge badge-outline badge-sm">{if is_hidden { "primary · hidden" } else { "primary" }}</span>
                    })}
                </div>
                <p class="mt-0.5 text-sm text-base-content/70 line-clamp-2">{item.description.clone()}</p>
                <div class="mt-1 flex flex-wrap items-center gap-1 text-xs text-base-content/70">
                    <span>{item.owner.clone()}</span>
                    {item.tags.iter().take(ROW_TAG_LIMIT).map(|t| view! {
                        <span class="rounded bg-base-200 px-1.5 py-0.5">{t.clone()}</span>
                    }).collect::<Vec<_>>()}
                    {(extra_tags > 0).then(|| view! { <span>{format!("+{extra_tags}")}</span> })}
                </div>
            </div>
        </li>
    }
}

/// Copy text to the clipboard. The text is passed as an argument, never
/// spliced into script source.
#[cfg(feature = "hydrate")]
fn copy_to_clipboard(text: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    let write = || -> Option<()> {
        let navigator = js_sys::Reflect::get(&js_sys::global(), &"navigator".into()).ok()?;
        let clipboard = js_sys::Reflect::get(&navigator, &"clipboard".into()).ok()?;
        let write_text = js_sys::Reflect::get(&clipboard, &"writeText".into())
            .ok()?
            .dyn_into::<js_sys::Function>()
            .ok()?;
        write_text
            .call1(&clipboard, &JsValue::from_str(text))
            .ok()?;
        Some(())
    };
    let _ = write();
}

/// Reads a prompt: metadata, variables and its text, with the personal
/// favorite/hidden switches.
#[component]
fn PromptDetailModal(
    open_slug: RwSignal<Option<String>>,
    state: RwSignal<Option<PromptLibraryState>>,
    saving_slug: RwSignal<Option<String>>,
    on_toggle: Callback<(String, bool, bool)>,
) -> impl IntoView {
    let detail = LocalResource::new(move || {
        let slug = open_slug.get();
        async move {
            match slug {
                Some(slug) => Some(with_auth_retry(move || get_prompt_detail(slug.clone())).await),
                None => None,
            }
        }
    });
    let item = Memo::new(move |_| {
        let slug = open_slug.get()?;
        state.with(|s| s.as_ref()?.items.iter().find(|i| i.slug == slug).cloned())
    });
    let copied = RwSignal::new(false);
    let close = move || {
        open_slug.set(None);
        copied.set(false);
    };

    // A native modal dialog renders in the top layer, so the transformed page
    // content cannot become its containing block.
    view! {
        <crate::components::ModalDialog
            open=Signal::derive(move || open_slug.get().is_some())
            on_close=Callback::new(move |_| close())
            labelled_by="prompt-detail-title"
            class="max-w-3xl"
        >
                    <button type="button" class="btn btn-sm btn-circle btn-ghost absolute right-3 top-3" aria-label="Close" on:click=move |_| close()>
                        "✕"
                    </button>
                    {move || item.get().map(|item| {
                        let slug_fav = item.slug.clone();
                        let slug_hide = item.slug.clone();
                        let (is_favorite, is_hidden) = (item.is_favorite, item.is_hidden);
                        view! {
                            <h2 id="prompt-detail-title" class="pr-10 text-xl font-bold">{item.name.clone()}</h2>
                            <p class="mt-1 text-sm text-base-content/70 break-all">{item.slug.clone()}</p>
                            <p class="mt-3">{item.description.clone()}</p>
                            <div class="mt-3 flex flex-wrap gap-1.5 text-xs">
                                <span class="badge badge-outline badge-sm">{format!("owner: {}", item.owner)}</span>
                                <span class="badge badge-outline badge-sm">{format!("{} cost", item.context_cost)}</span>
                                <span class="badge badge-outline badge-sm">{item.status.clone()}</span>
                                <span class="badge badge-outline badge-sm">{item.access_level.clone()}</span>
                                {item.tags.iter().map(|t| view! { <span class="badge badge-ghost badge-sm">{t.clone()}</span> }).collect::<Vec<_>>()}
                            </div>
                            <div class="mt-4 flex flex-wrap gap-x-6 gap-y-2">
                                <label class="label cursor-pointer gap-2 p-0">
                                    <input
                                        type="checkbox"
                                        class="toggle toggle-sm toggle-primary"
                                        prop:checked=is_favorite
                                        prop:disabled=move || !item.publish_to_mcp || saving_slug.get().is_some()
                                        on:change=move |ev| on_toggle.run((slug_fav.clone(), event_target_checked(&ev), is_hidden))
                                    />
                                    <span class="label-text">
                                        {if item.publish_to_mcp { "Favorite in personal context" } else { "Not publishable to MCP" }}
                                    </span>
                                </label>
                                <label class="label cursor-pointer gap-2 p-0">
                                    <input
                                        type="checkbox"
                                        class="toggle toggle-sm toggle-warning"
                                        prop:checked=is_hidden
                                        prop:disabled=move || !item.default_primary || saving_slug.get().is_some()
                                        on:change=move |ev| on_toggle.run((slug_hide.clone(), is_favorite, event_target_checked(&ev)))
                                    />
                                    <span class="label-text">
                                        {if item.default_primary { "Hide shared primary prompt" } else { "Not a shared primary prompt" }}
                                    </span>
                                </label>
                            </div>
                        }
                    })}

                    <Suspense fallback=|| view! { <div class="skeleton mt-4 h-40 w-full" /> }>
                        {move || detail.get().flatten().map(|result| match result {
                            Ok(detail) => {
                                #[cfg_attr(not(feature = "hydrate"), allow(unused_variables))]
                                let body = detail.body.clone();
                                view! {
                                    {(!detail.variables.is_empty()).then(|| view! {
                                        <div class="mt-5">
                                            <h3 class="text-sm font-semibold">"Variables"</h3>
                                            <ul class="mt-1 space-y-1 text-sm">
                                                {detail.variables.iter().map(|v| view! {
                                                    <li>
                                                        <code>{format!("{{{{{}}}}}", v.name)}</code>
                                                        {if v.required { " (required) — " } else { " — " }}
                                                        <span class="text-base-content/70">{v.description.clone()}</span>
                                                    </li>
                                                }).collect::<Vec<_>>()}
                                            </ul>
                                        </div>
                                    })}
                                    <div class="mt-5 flex items-center justify-between gap-2">
                                        <h3 class="text-sm font-semibold">"Prompt"</h3>
                                        <button
                                            class="btn btn-xs btn-outline"
                                            on:click=move |_| {
                                                #[cfg(feature = "hydrate")]
                                                copy_to_clipboard(&body);
                                                copied.set(true);
                                            }
                                        >
                                            {move || if copied.get() { "Copied" } else { "Copy" }}
                                        </button>
                                    </div>
                                    <pre class="mt-2 max-h-[50vh] overflow-auto whitespace-pre-wrap break-words rounded-box border border-base-300 bg-base-200 p-4 font-mono text-sm leading-6">{detail.body.clone()}</pre>
                                }.into_any()
                            }
                            Err(err) => view! {
                                <div class="alert alert-error mt-4"><span>{format!("Could not load the prompt: {err}")}</span></div>
                            }.into_any(),
                        })}
                    </Suspense>
        </crate::components::ModalDialog>
    }
}
