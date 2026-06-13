use leptos::prelude::*;

#[allow(unused_imports)]
use crate::app::{
    admin_list_pats, admin_toggle_pat, create_admin_access_level, create_service_token,
    deactivate_service_token, delete_admin_access_level, get_custom_css, get_navigation,
    get_navigation_order, get_rag_reindex_status, get_schema_endpoint_reindex_status,
    get_search_reindex_status, list_admin_access_levels, list_admin_users,
    list_documentation_feedback, list_service_tokens, mark_documentation_feedback_duplicate,
    resolve_documentation_feedback, save_custom_css, save_navigation_order,
    set_admin_user_access_levels, trigger_rag_reindex, trigger_schema_endpoint_reindex,
    trigger_search_reindex, update_admin_access_level, AccessLevelInfo, CreateTokenResult,
    DocumentationFeedbackAdminItem, DocumentationFeedbackAdminListResult, NavItem,
    NavigationOrderEntry, ServiceTokenInfo,
};
#[allow(unused_imports)]
use crate::auth::refresh_client::with_auth_retry;

// ── Index Rebuilds ───────────────────────────────────────────────────────────

#[component]
pub fn SearchReindexSection() -> impl IntoView {
    let (poll_counter, set_poll_counter) = signal(0u32);
    let (is_polling, set_is_polling) = signal(false);

    let status_resource = LocalResource::new(move || {
        let _ = poll_counter.get();
        with_auth_retry(get_search_reindex_status)
    });

    let trigger_action = Action::new_local(move |_: &()| async move {
        let result = with_auth_retry(trigger_search_reindex).await;
        set_is_polling.set(true);
        set_poll_counter.update(|c| *c += 1);
        result
    });

    #[cfg(feature = "hydrate")]
    Effect::new(move || {
        if is_polling.get() {
            use leptos::task::spawn_local;
            spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(2000).await;
                set_poll_counter.update(|c| *c += 1);
            });
        }
    });

    Effect::new(move || {
        if let Some(Ok((is_running, _progress, _search_enabled))) = status_resource.get() {
            if !is_running && is_polling.get() {
                set_is_polling.set(false);
            }
        }
    });

    view! {
        <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
            {move || {
                status_resource.get().map(|result| {
                    match result {
                        Ok((_is_running, _progress, search_enabled)) => {
                            if !search_enabled {
                                return view! { <span></span> }.into_any();
                            }
                            view! {
                                <div class="card bg-base-100 shadow-xl border border-base-200">
                                    <div class="card-body p-0">
                                        <div class="p-8">
                                            <div class="flex items-center gap-3 mb-2">
                                                <svg class="w-6 h-6 text-primary" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
                                                <h2 class="text-2xl font-bold">"Meilisearch Re-index"</h2>
                                            </div>
                                            <p class="text-base-content/60">"Rebuild the full-text search index from MongoDB metadata and stored Markdown content."</p>
                                        </div>
                                        <div class="px-8 pb-8">
                                            <SearchReindexControls
                                                status_resource=status_resource
                                                trigger_action=trigger_action
                                                is_polling=is_polling
                                            />
                                        </div>
                                    </div>
                                </div>
                            }.into_any()
                        }
                        Err(_) => view! { <span></span> }.into_any(),
                    }
                })
            }}
        </Suspense>
    }
}

#[component]
fn SearchReindexControls(
    status_resource: LocalResource<Result<(bool, u32, bool), ServerFnError>>,
    trigger_action: Action<(), Result<String, ServerFnError>>,
    is_polling: ReadSignal<bool>,
) -> impl IntoView {
    let _ = is_polling;
    let is_running = Signal::derive(move || {
        status_resource
            .get()
            .and_then(|r| r.ok())
            .map(|(running, _, _)| running)
            .unwrap_or(false)
    });

    let progress = Signal::derive(move || {
        status_resource
            .get()
            .and_then(|r| r.ok())
            .map(|(_, p, _)| p)
            .unwrap_or(0)
    });

    view! {
        <div class="space-y-4">
            <Show when=move || is_running.get() fallback=move || view! {
                <button
                    class="btn btn-primary"
                    on:click=move |_| { trigger_action.dispatch(()); }
                    prop:disabled=move || trigger_action.pending().get()
                >
                    <Show when=move || trigger_action.pending().get() fallback=|| view! {
                        <svg class="w-4 h-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2"/></svg>
                    }>
                        <span class="loading loading-spinner loading-sm"></span>
                    </Show>
                    "Start Re-index"
                </button>
            }>
                <div class="space-y-2">
                    <div class="flex items-center justify-between">
                        <span class="text-sm font-medium">"Re-indexing in progress..."</span>
                        <span class="text-sm text-base-content/60">{move || format!("{}%", progress.get())}</span>
                    </div>
                    <progress
                        class="progress progress-primary w-full"
                        value=move || progress.get().to_string()
                        max="100"
                    ></progress>
                </div>
            </Show>

            {move || {
                trigger_action.value().get().and_then(|result| {
                    result.err().map(|e| {
                        view! {
                            <div class="alert alert-error text-sm mt-2">
                                <span>{e.to_string()}</span>
                            </div>
                        }
                    })
                })
            }}
        </div>
    }
}

/// RAG re-index section — visible only when RAG is enabled.
#[component]
pub fn RagReindexSection() -> impl IntoView {
    let (poll_counter, set_poll_counter) = signal(0u32);
    let (is_polling, set_is_polling) = signal(false);

    let status_resource = LocalResource::new(move || {
        let _ = poll_counter.get();
        with_auth_retry(get_rag_reindex_status)
    });

    let trigger_action = Action::new_local(move |_: &()| async move {
        let result = with_auth_retry(trigger_rag_reindex).await;
        // Start polling after triggering
        set_is_polling.set(true);
        set_poll_counter.update(|c| *c += 1);
        result
    });

    // Polling effect: refetch status every 2s while running
    #[cfg(feature = "hydrate")]
    Effect::new(move || {
        let polling = is_polling.get();
        if polling {
            use leptos::task::spawn_local;
            spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(2000).await;
                set_poll_counter.update(|c| *c += 1);
            });
        }
    });

    // Check if polling should stop
    Effect::new(move || {
        if let Some(Ok((is_running, _progress, _rag_enabled))) = status_resource.get() {
            if !is_running && is_polling.get() {
                set_is_polling.set(false);
            }
        }
    });

    view! {
        <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
            {move || {
                status_resource.get().map(|result| {
                    match result {
                        Ok((_is_running, _progress, rag_enabled)) => {
                            if !rag_enabled {
                                return view! { <span></span> }.into_any();
                            }
                            view! {
                                <div class="card bg-base-100 shadow-xl border border-base-200">
                                    <div class="card-body p-0">
                                        <div class="p-8">
                                            <div class="flex items-center gap-3 mb-2">
                                                <svg class="w-6 h-6 text-secondary" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/></svg>
                                                <h2 class="text-2xl font-bold">"RAG Re-index"</h2>
                                            </div>
                                            <p class="text-base-content/60">"Re-embed all documents in the vector store. Use this after changing the embedding model."</p>
                                        </div>
                                        <div class="px-8 pb-8">
                                            <RagReindexControls
                                                status_resource=status_resource
                                                trigger_action=trigger_action
                                                is_polling=is_polling
                                            />
                                        </div>
                                    </div>
                                </div>
                            }.into_any()
                        }
                        Err(_) => view! { <span></span> }.into_any(),
                    }
                })
            }}
        </Suspense>
    }
}

#[component]
fn RagReindexControls(
    status_resource: LocalResource<Result<(bool, u32, bool), ServerFnError>>,
    trigger_action: Action<(), Result<String, ServerFnError>>,
    is_polling: ReadSignal<bool>,
) -> impl IntoView {
    let _ = is_polling; // used by caller for reactive polling, not needed in view
    let is_running = Signal::derive(move || {
        status_resource
            .get()
            .and_then(|r| r.ok())
            .map(|(running, _, _)| running)
            .unwrap_or(false)
    });

    let progress = Signal::derive(move || {
        status_resource
            .get()
            .and_then(|r| r.ok())
            .map(|(_, p, _)| p)
            .unwrap_or(0)
    });

    view! {
        <div class="space-y-4">
            <Show when=move || is_running.get() fallback=move || view! {
                <button
                    class="btn btn-secondary"
                    on:click=move |_| { trigger_action.dispatch(()); }
                    prop:disabled=move || trigger_action.pending().get()
                >
                    <Show when=move || trigger_action.pending().get() fallback=|| view! {
                        <svg class="w-4 h-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2"/></svg>
                    }>
                        <span class="loading loading-spinner loading-sm"></span>
                    </Show>
                    "Start Re-index"
                </button>
            }>
                <div class="space-y-2">
                    <div class="flex items-center justify-between">
                        <span class="text-sm font-medium">"Re-indexing in progress..."</span>
                        <span class="text-sm text-base-content/60">{move || format!("{}%", progress.get())}</span>
                    </div>
                    <progress
                        class="progress progress-primary w-full"
                        value=move || progress.get().to_string()
                        max="100"
                    ></progress>
                </div>
            </Show>

            // Show error from trigger action
            {move || {
                trigger_action.value().get().and_then(|result| {
                    result.err().map(|e| {
                        view! {
                            <div class="alert alert-error text-sm mt-2">
                                <span>{e.to_string()}</span>
                            </div>
                        }
                    })
                })
            }}
        </div>
    }
}

/// Schema endpoint re-index section — always visible to admins.
#[component]
pub fn SchemaEndpointReindexSection() -> impl IntoView {
    let (poll_counter, set_poll_counter) = signal(0u32);
    let (is_polling, set_is_polling) = signal(false);

    let status_resource = LocalResource::new(move || {
        let _ = poll_counter.get();
        with_auth_retry(get_schema_endpoint_reindex_status)
    });

    let trigger_action = Action::new_local(move |_: &()| async move {
        let result = with_auth_retry(trigger_schema_endpoint_reindex).await;
        set_is_polling.set(true);
        set_poll_counter.update(|c| *c += 1);
        result
    });

    #[cfg(feature = "hydrate")]
    Effect::new(move || {
        if is_polling.get() {
            use leptos::task::spawn_local;
            spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(2000).await;
                set_poll_counter.update(|c| *c += 1);
            });
        }
    });

    Effect::new(move || {
        if let Some(Ok((is_running, _progress))) = status_resource.get() {
            if !is_running && is_polling.get() {
                set_is_polling.set(false);
            }
        }
    });

    view! {
        <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
            {move || {
                status_resource.get().map(|result| {
                    match result {
                        Ok(_) => view! {
                            <div class="card bg-base-100 shadow-xl border border-base-200">
                                <div class="card-body p-0">
                                    <div class="p-8">
                                        <div class="flex items-center gap-3 mb-2">
                                            <svg class="w-6 h-6 text-accent" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><line x1="10" y1="9" x2="8" y2="9"/></svg>
                                            <h2 class="text-2xl font-bold">"Schema Endpoint Re-index"</h2>
                                        </div>
                                        <p class="text-base-content/60">"Re-extract API operations (path, method, summary) from all schema versions stored in S3 and update the index in MongoDB. Run this to backfill schemas ingested before endpoint indexing was introduced."</p>
                                    </div>
                                    <div class="px-8 pb-8">
                                        <SchemaEndpointReindexControls
                                            status_resource=status_resource
                                            trigger_action=trigger_action
                                            is_polling=is_polling
                                        />
                                    </div>
                                </div>
                            </div>
                        }.into_any(),
                        Err(_) => view! { <span></span> }.into_any(),
                    }
                })
            }}
        </Suspense>
    }
}

#[component]
fn SchemaEndpointReindexControls(
    status_resource: LocalResource<Result<(bool, u32), ServerFnError>>,
    trigger_action: Action<(), Result<String, ServerFnError>>,
    is_polling: ReadSignal<bool>,
) -> impl IntoView {
    let _ = is_polling;
    let is_running = Signal::derive(move || {
        status_resource
            .get()
            .and_then(|r| r.ok())
            .map(|(running, _)| running)
            .unwrap_or(false)
    });

    let progress = Signal::derive(move || {
        status_resource
            .get()
            .and_then(|r| r.ok())
            .map(|(_, p)| p)
            .unwrap_or(0)
    });

    view! {
        <div class="space-y-4">
            <Show when=move || is_running.get() fallback=move || view! {
                <button
                    class="btn btn-accent"
                    on:click=move |_| { trigger_action.dispatch(()); }
                    prop:disabled=move || trigger_action.pending().get()
                >
                    <Show when=move || trigger_action.pending().get() fallback=|| view! {
                        <svg class="w-4 h-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2"/></svg>
                    }>
                        <span class="loading loading-spinner loading-sm"></span>
                    </Show>
                    "Start Re-index"
                </button>
            }>
                <div class="space-y-2">
                    <div class="flex items-center justify-between">
                        <span class="text-sm font-medium">"Re-indexing in progress..."</span>
                        <span class="text-sm text-base-content/60">{move || format!("{}%", progress.get())}</span>
                    </div>
                    <progress
                        class="progress progress-accent w-full"
                        value=move || progress.get().to_string()
                        max="100"
                    ></progress>
                </div>
            </Show>

            {move || {
                trigger_action.value().get().and_then(|result| {
                    result.err().map(|e| {
                        view! {
                            <div class="alert alert-error text-sm mt-2">
                                <span>{e.to_string()}</span>
                            </div>
                        }
                    })
                })
            }}
        </div>
    }
}
