use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::auth::refresh_client::with_auth_retry;
use crate::app::{get_current_user, get_prompt_library_state, save_prompt_preference, PromptLibraryItem, PromptLibraryState};

#[component]
pub fn PromptsPage() -> impl IntoView {
    let user_resource = LocalResource::new(|| with_auth_retry(get_current_user));
    let navigate = use_navigate();

    Effect::new(move |_| {
        if let Some(Ok(None)) = user_resource.get() {
            navigate("/login", Default::default());
        }
    });

    view! {
        <div class="container mx-auto max-w-6xl px-4 py-8">
            <h1 class="text-3xl font-bold">"Prompt Library"</h1>
            <p class="text-base-content/60 mt-2 mb-6 max-w-3xl">
                "Published prompts can be added to your working context. Default primary prompts come from the shared library; favorites and hidden flags are personal preferences."
            </p>
            <PromptLibraryPanel />
        </div>
    }
}

#[component]
fn PromptLibraryPanel() -> impl IntoView {
    let state = RwSignal::new(None::<PromptLibraryState>);
    let filter = RwSignal::new(String::new());
    let saving_slug = RwSignal::new(None::<String>);

    let load = Action::new_local(move |_: &()| async move {
        match with_auth_retry(get_prompt_library_state).await {
            Ok(value) => state.set(Some(value)),
            Err(err) => tracing::error!("Failed to load prompt library: {err}"),
        }
    });

    Effect::new(move |_| {
        load.dispatch(());
    });

    view! {
        <div class="space-y-6">
            <input
                type="text"
                class="input input-bordered w-full md:max-w-md"
                placeholder="Search prompts by name, slug, owner, or tag"
                prop:value=move || filter.get()
                on:input=move |ev| filter.set(event_target_value(&ev))
            />

            <Show
                when=move || state.get().is_some()
                fallback=|| view! { <div class="skeleton h-64 w-full rounded-2xl" /> }
            >
                {move || {
                    let library = state.get().unwrap();
                    let query = filter.get().to_lowercase();
                    let filtered: Vec<PromptLibraryItem> = library
                        .items
                        .into_iter()
                        .filter(|item| {
                            query.is_empty()
                                || item.name.to_lowercase().contains(&query)
                                || item.slug.to_lowercase().contains(&query)
                                || item.owner.to_lowercase().contains(&query)
                                || item.tags.iter().any(|tag| tag.to_lowercase().contains(&query))
                        })
                        .collect();
                    let warning_text = library.warnings.join(" ");
                    let has_warning = !warning_text.is_empty();

                    view! {
                        <div class="flex flex-wrap items-start gap-3">
                            <div class="badge badge-outline badge-lg">
                                {format!("Context cost: {}", library.estimated_context_cost)}
                            </div>
                            <Show
                                when=move || has_warning
                                fallback=|| view! { <div /> }
                            >
                                <div class="alert alert-warning py-2">
                                    <span>{warning_text.clone()}</span>
                                </div>
                            </Show>
                        </div>

                        <div class="grid gap-4 mt-4">
                            <For
                                each=move || filtered.clone()
                                key=|item| item.slug.clone()
                                children=move |item| {
                                    let slug = item.slug.clone();
                                    let toggle = Action::new_local(move |(is_favorite, is_hidden): &(bool, bool)| {
                                        let slug = slug.clone();
                                        let is_favorite = *is_favorite;
                                        let is_hidden = *is_hidden;
                                        async move {
                                            saving_slug.set(Some(slug.clone()));
                                            let result = with_auth_retry(|| {
                                                save_prompt_preference(slug.clone(), is_favorite, is_hidden)
                                            })
                                            .await;
                                            saving_slug.set(None);
                                            result
                                        }
                                    });

                                    let item_for_favorite = item.clone();
                                    let item_for_hidden = item.clone();

                                    Effect::new(move |_| {
                                        if let Some(Ok(updated)) = toggle.value().get() {
                                            state.set(Some(updated));
                                        }
                                    });

                                    view! {
                                        <div class="card bg-base-200 shadow-sm border border-base-300/60">
                                            <div class="card-body gap-4">
                                                <div class="flex flex-wrap items-start justify-between gap-3">
                                                    <div class="space-y-2">
                                                        <div class="flex flex-wrap items-center gap-2">
                                                            <h2 class="card-title text-xl">{item.name.clone()}</h2>
                                                            <span class="badge badge-outline">{item.status.clone()}</span>
                                                            <span class="badge badge-ghost">{item.access_level.clone()}</span>
                                                            <span class="badge badge-ghost">{format!("cost: {}", item.context_cost)}</span>
                                                        </div>
                                                        <p class="text-sm text-base-content/60">{item.slug.clone()}</p>
                                                        <p>{item.description.clone()}</p>
                                                    </div>
                                                    <Show
                                                        when=move || saving_slug.get().as_deref() == Some(item.slug.as_str())
                                                        fallback=|| view! { <div /> }
                                                    >
                                                        <span class="loading loading-spinner loading-sm"></span>
                                                    </Show>
                                                </div>

                                                <div class="flex flex-wrap gap-2 text-sm">
                                                    <span class="badge badge-outline">{format!("owner: {}", item.owner.clone())}</span>
                                                    {item.tags.iter().map(|tag| view! {
                                                        <span class="badge badge-neutral badge-outline">{tag.clone()}</span>
                                                    }).collect::<Vec<_>>()}
                                                </div>

                                                <div class="grid gap-3 md:grid-cols-2">
                                                    <label class="label cursor-pointer justify-start gap-3 rounded-xl bg-base-100 px-4 py-3">
                                                        <input
                                                            type="checkbox"
                                                            class="toggle toggle-primary"
                                                            prop:checked=item.is_favorite
                                                            disabled=move || !item_for_favorite.publish_to_mcp
                                                            on:change=move |ev| {
                                                                let checked = event_target_checked(&ev);
                                                                toggle.dispatch((checked, item_for_favorite.is_hidden));
                                                            }
                                                        />
                                                        <span class="label-text">
                                                            {if item.publish_to_mcp {
                                                                "Favorite in personal context"
                                                            } else {
                                                                "Not publishable to MCP"
                                                            }}
                                                        </span>
                                                    </label>

                                                    <label class="label cursor-pointer justify-start gap-3 rounded-xl bg-base-100 px-4 py-3">
                                                        <input
                                                            type="checkbox"
                                                            class="toggle toggle-warning"
                                                            prop:checked=item.is_hidden
                                                            disabled=move || !item_for_hidden.default_primary
                                                            on:change=move |ev| {
                                                                let checked = event_target_checked(&ev);
                                                                toggle.dispatch((item_for_hidden.is_favorite, checked));
                                                            }
                                                        />
                                                        <span class="label-text">
                                                            {if item.default_primary {
                                                                "Hide shared primary prompt"
                                                            } else {
                                                                "Not a shared primary prompt"
                                                            }}
                                                        </span>
                                                    </label>
                                                </div>
                                            </div>
                                        </div>
                                    }
                                }
                            />
                        </div>
                    }.into_any()
                }}
            </Show>
        </div>
    }
}
