use leptos::prelude::*;
use leptos_router::hooks::use_params;
use leptos_router::params::Params;

use crate::app::{
    admin_list_pats, admin_toggle_pat,
    create_service_token, deactivate_service_token, get_custom_css, get_navigation,
    get_navigation_order, get_rag_reindex_status, list_documentation_feedback, list_service_tokens,
    mark_documentation_feedback_duplicate, resolve_documentation_feedback, save_custom_css,
    save_navigation_order, trigger_rag_reindex, AdminPatInfo, CreateTokenResult,
    DocumentationFeedbackAdminItem, DocumentationFeedbackAdminListResult, NavItem,
    NavigationOrderEntry, ServiceTokenInfo,
};
use crate::auth::refresh_client::with_auth_retry;


#[derive(Params, PartialEq, Clone, Debug)]
pub struct AdminParams {
    pub section: String,
}

/// Admin settings page with service token management and theming.
#[component]
pub fn AdminSettingsPage() -> impl IntoView {
    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();

    let is_admin = move || {
        current_user
            .and_then(|sig| sig.get())
            .map(|u| u.is_admin)
            .unwrap_or(false)
    };

    let params = use_params::<AdminParams>();
    let section = move || params.with(|p| p.as_ref().map(|p| p.section.clone()).unwrap_or_else(|_| "tokens".to_string()));

    view! {
        <Show
            when=is_admin
            fallback=|| view! {
                <div class="flex items-center justify-center min-h-[50vh]">
                    <div class="alert alert-error max-w-md shadow-lg border-none bg-error/10 text-error">
                        <svg xmlns="http://www.w3.org/2000/svg" class="h-6 w-6 shrink-0 stroke-current text-error" fill="none" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
                        </svg>
                        <span class="font-medium">"Access denied. Admin privileges required."</span>
                    </div>
                </div>
            }
        >
            <div class="animate-in fade-in slide-in-from-bottom-4 duration-500">
                <AdminSettingsContent section=section />
            </div>
        </Show>
    }
}

/// Inner content, rendered only for admins.
#[component]
fn AdminSettingsContent(section: impl Fn() -> String + Send + Sync + 'static) -> impl IntoView {
    // Created token (shown once in modal)
    let (created_token, set_created_token) = signal(Option::<CreateTokenResult>::None);

    let section = std::sync::Arc::new(section);
    let section2 = section.clone();

    view! {
        <div class="max-w-5xl mx-auto space-y-8 pb-20">
            <header class="flex flex-col items-start gap-4 border-b border-base-200 pb-8 sm:flex-row sm:items-center">
                <div class="p-3 bg-primary/10 rounded-2xl text-primary">
                    <svg class="w-8 h-8" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"></path>
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"></path>
                    </svg>
                </div>
                <div>
                   {move || {
                       let current_section = section();
                       let title = match current_section.as_str() {
                           "tokens" => "Service Tokens",
                           "pats" => "Personal Access Tokens",
                           "documentation-feedback" => "Documentation Feedback",
                           "navigation" => "Navigation Setup",
                           "css" => "Visual Customization",
                           "rag" => "RAG Index Management",
                           _ => "Administration",
                       };
                       let subtitle = match current_section.as_str() {
                           "documentation-feedback" => "Review MCP-reported documentation gaps, resolve them, and keep the registry tidy.",
                           _ => "Manage your instance configuration, service tokens, and theming.",
                       };
                       view! {
                           <>
                               <h1 class="text-4xl font-extrabold tracking-tight">{title}</h1>
                               <p class="text-base-content/60 mt-1">{subtitle}</p>
                           </>
                       }
                   }}
                </div>
            </header>

            <div class="grid grid-cols-1 gap-8">
                {move || match section2().as_str() {
                    "tokens" => view! { <ServiceTokenManager set_created_token=set_created_token /> }.into_any(),
                    "pats" => view! { <AdminPatManager /> }.into_any(),
                    "documentation-feedback" => view! { <DocumentationFeedbackAdminPanel /> }.into_any(),
                    "navigation" => view! { <NavigationOrderEditor /> }.into_any(),
                    "css" => view! { <CustomCssEditor /> }.into_any(),
                    "rag" => view! { <RagReindexSection /> }.into_any(),
                    _ => view! { <div class="alert alert-warning">"Page not found"</div> }.into_any(),
                }}
            </div>
        </div>

        // Created token modal
        <CreatedTokenModal token=created_token set_token=set_created_token />
    }
}

/// Component managing service tokens.
#[component]
fn ServiceTokenManager(set_created_token: WriteSignal<Option<CreateTokenResult>>) -> impl IntoView {
    // Signal to trigger token list reload
    let (refresh_counter, set_refresh_counter) = signal(0u32);

    // Load tokens
    let tokens_resource = LocalResource::new(move || {
        let _ = refresh_counter.get();
        with_auth_retry(list_service_tokens)
    });

    let trigger_refresh = move || set_refresh_counter.update(|c| *c += 1);

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200 overflow-hidden">
            <div class="card-body p-0">
                <div class="p-8 pb-4">
                    <div class="flex items-center gap-3 mb-2">
                        <svg class="w-6 h-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"></path>
                        </svg>
                        <h2 class="card-title text-2xl">"Service Tokens"</h2>
                    </div>
                    <p class="text-base-content/60">
                        "Long-lived tokens for CI/CD pipelines and external service integrations. Each token is restricted to specific documentation scopes."
                    </p>
                </div>

                <div class="px-8 py-4">
                    <Suspense fallback=move || view! {
                        <div class="flex flex-col items-center justify-center py-12 gap-4">
                            <span class="loading loading-spinner loading-lg text-primary"></span>
                            <p class="text-sm font-medium animate-pulse">"Loading tokens..."</p>
                        </div>
                    }>
                        {move || tokens_resource.get().map(|result| match result {
                            Ok(tokens) => view! { <TokenTable tokens=tokens trigger_refresh=trigger_refresh /> }.into_any(),
                            Err(e) => view! {
                                <div class="alert alert-error shadow-sm border-none bg-error/10 text-error">
                                    <svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-6 w-6" fill="none" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                                    <span>{format!("Failed to load tokens: {e}")}</span>
                                </div>
                            }.into_any(),
                        })}
                    </Suspense>
                </div>

                <div class="bg-base-200/30 p-8 pt-6 border-t border-base-200">
                    <CreateTokenForm
                        on_created=move |result| {
                            set_created_token.set(Some(result));
                            trigger_refresh();
                        }
                    />
                </div>
            </div>
        </div>
    }
}

/// Table displaying existing service tokens.
#[component]
fn TokenTable(
    tokens: Vec<ServiceTokenInfo>,
    trigger_refresh: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    if tokens.is_empty() {
        return view! {
            <div class="flex flex-col items-center justify-center py-10 px-4 text-center border-2 border-dashed border-base-300 rounded-xl bg-base-200/20">
                <div class="w-16 h-16 bg-base-300/30 rounded-full flex items-center justify-center mb-4">
                    <svg class="w-8 h-8 text-base-content/30" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"></path>
                    </svg>
                </div>
                <h3 class="font-bold text-lg text-base-content/70">"No service tokens yet"</h3>
                <p class="text-sm text-base-content/50 max-w-xs mt-1">"Create your first token below to start automating document updates."</p>
            </div>
        }
        .into_any();
    }

    view! {
        <div class="overflow-x-auto rounded-lg border border-base-200">
            <table class="table table-zebra w-full overflow-hidden">
                <thead>
                    <tr class="bg-base-200/50">
                        <th class="py-4">"Name"</th>
                        <th>"Scopes"</th>
                        <th class="text-center">"Write"</th>
                        <th class="text-center">"Status"</th>
                        <th>"Created"</th>
                        <th>"Usage"</th>
                        <th class="text-right">"Actions"</th>
                    </tr>
                </thead>
                <tbody class="divide-y divide-base-200">
                    {tokens.into_iter().map(|token| {
                        view! { <TokenRow token=token trigger_refresh=trigger_refresh /> }
                    }).collect::<Vec<_>>()}
                </tbody>
            </table>
        </div>
    }
    .into_any()
}

/// A single row in the token table.
#[component]
fn TokenRow(
    token: ServiceTokenInfo,
    trigger_refresh: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let id = token.id.clone();
    let name = token.name.clone();
    let scopes: Vec<String> = token.allowed_scopes.clone();
    let created_at = token.created_at.clone();
    let last_used = token.last_used_at.clone().unwrap_or_else(|| "Never".to_string());
    let is_active = token.is_active;
    let can_write = token.can_write;

    let (deactivating, set_deactivating) = signal(false);

    #[cfg(feature = "hydrate")]
    let deactivate_action = Action::new_local(move |_: &()| {
        let id = id.clone();
        async move {
            set_deactivating.set(true);
            let result = with_auth_retry(|| deactivate_service_token(id.clone())).await;
            set_deactivating.set(false);
            if result.is_ok() {
                trigger_refresh();
            }
            result
        }
    });

    view! {
        <tr class=move || if !is_active { "opacity-40 grayscale italic" } else { "hover:bg-base-200/30 transition-colors" }>
            <td class="font-bold text-sm min-w-[140px]">{name}</td>
            <td class="max-w-[200px]">
                <div class="flex flex-wrap gap-1">
                    {scopes.into_iter().take(3).map(|scope| {
                        view! { <span class="badge badge-outline badge-xs px-2 py-2 font-mono">{scope}</span> }
                    }).collect::<Vec<_>>()}
                    {if token.allowed_scopes.len() > 3 {
                        view! { <span class="text-[10px] text-base-content/40 ml-1">" + "{token.allowed_scopes.len() - 3}" more"</span> }.into_any()
                    } else {
                        view! { <span></span> }.into_any()
                    }}
                </div>
            </td>
            <td class="text-center">
                {if can_write {
                    view! { <div class="badge badge-success badge-outline badge-sm font-bold text-[10px]">"WRITE"</div> }.into_any()
                } else {
                    view! { <div class="badge badge-ghost badge-outline badge-sm font-bold text-[10px] opacity-40">"READ"</div> }.into_any()
                }}
            </td>
            <td class="text-center">
                {if is_active {
                    view! { <span class="badge badge-primary badge-sm font-medium">"active"</span> }.into_any()
                } else {
                    view! { <span class="badge badge-ghost badge-sm text-xs font-medium">"deactivated"</span> }.into_any()
                }}
            </td>
            <td class="text-xs text-base-content/60">{created_at}</td>
            <td class="text-xs text-base-content/60">
                <div class="flex items-center gap-1.5">
                  <svg class="w-3.5 h-3.5 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"></path></svg>
                  {last_used}
                </div>
            </td>
            <td class="text-right">
                <Show when=move || is_active>
                    <button
                        class="btn btn-ghost btn-xs text-error hover:bg-error/10 normal-case font-medium"
                        disabled=move || deactivating.get()
                        on:click=move |_| {
                            #[cfg(feature = "hydrate")]
                            {
                                if window().confirm_with_message("Are you sure you want to deactivate this token? This action cannot be undone.").unwrap_or(false) {
                                    deactivate_action.dispatch(());
                                }
                            }
                        }
                    >
                        {move || if deactivating.get() { "..." } else { "Deactivate" }}
                    </button>
                </Show>
            </td>
        </tr>
    }
}

/// Form for creating a new service token.
#[component]
fn CreateTokenForm(
    on_created: impl Fn(CreateTokenResult) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let (name, set_name) = signal(String::new());
    let (scopes, set_scopes) = signal(String::new());
    let (can_write, set_can_write) = signal(true);
    let (error, set_error) = signal(Option::<String>::None);
    let (submitting, set_submitting) = signal(false);

    let submit_action = Action::new_local(move |_: &()| {
        let name_val = name.get_untracked();
        let scopes_val = scopes.get_untracked();
        let can_write_val = can_write.get_untracked();
        async move {
            set_error.set(None);
            set_submitting.set(true);
            let result = with_auth_retry(|| create_service_token(name_val.clone(), scopes_val.clone(), can_write_val)).await;
            set_submitting.set(false);
            match result {
                Ok(token_result) => {
                    set_name.set(String::new());
                    set_scopes.set(String::new());
                    set_can_write.set(true);
                    on_created(token_result);
                }
                Err(e) => {
                    set_error.set(Some(e.to_string()));
                }
            }
        }
    });

    view! {
        <div class="flex flex-col gap-6">
            <div>
                <h3 class="font-bold text-lg">"Create New Token"</h3>
                <p class="text-sm text-base-content/50">"Configure a new scoped access token."</p>
            </div>

            <Show when=move || error.get().is_some()>
                <div class="alert alert-error shadow-sm border-none bg-error/10 text-error animate-in fade-in slide-in-from-top-2">
                    <svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-6 w-6" fill="none" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                    <span class="text-xs font-semibold">{move || error.get().unwrap_or_default()}</span>
                </div>
            </Show>

            <div class="grid grid-cols-1 md:grid-cols-2 gap-x-8 gap-y-6">
                <div class="form-control w-full">
                    <label class="label pt-0">
                        <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60">"Token Name"</span>
                    </label>
                    <input
                        type="text"
                        placeholder="e.g. github-actions-ci"
                        class="input input-bordered focus:input-primary transition-all shadow-sm"
                        prop:value=move || name.get()
                        on:input=move |ev| set_name.set(event_target_value(&ev))
                    />
                    <label class="label">
                      <span class="label-text-alt text-base-content/40 italic">"A descriptive name for identification."</span>
                    </label>
                </div>

                <div class="form-control w-full">
                    <label class="label pt-0">
                        <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60">"Permissions"</span>
                    </label>
                    <div class="bg-base-100 rounded-lg border border-base-300 px-3 shadow-sm flex items-center min-h-12">
                      <label class="cursor-pointer flex items-center gap-4 w-full">
                          <input
                              type="checkbox"
                              class="checkbox checkbox-primary"
                              prop:checked=move || can_write.get()
                              on:change=move |ev| set_can_write.set(event_target_checked(&ev))
                          />
                          <div>
                            <span class="label-text font-bold block mb-0.5">"Allow Write Access"</span>
                            <span class="label-text-alt text-base-content/40">"Permit updates and deletions via API."</span>
                          </div>
                      </label>
                    </div>
                    <label class="label">
                      <span class="label-text-alt text-base-content/40 italic">"Controls write permissions for this token."</span>
                    </label>
                </div>
            </div>

            <div class="form-control">
                <label class="label pt-0">
                    <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60">"Allowed Scopes"</span>
                </label>
                <textarea
                    class="textarea textarea-bordered h-32 font-mono text-sm leading-relaxed focus:textarea-primary transition-all shadow-sm"
                    placeholder={"docs/getting-started\nprojects/*\napi/v2/reference"}
                    prop:value=move || scopes.get()
                    on:input=move |ev| set_scopes.set(event_target_value(&ev))
                ></textarea>
                <label class="label">
                    <span class="label-text-alt text-base-content/50 bg-base-300/30 px-2 py-1 rounded inline-flex items-center gap-1.5">
                        <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path></svg>
                        "Enter one path prefix per line."
                    </span>
                </label>
            </div>

            <div class="flex justify-end pt-2">
                <button
                    class="btn btn-primary w-full sm:w-64 shadow-lg shadow-primary/20"
                    disabled=move || submitting.get() || name.get().trim().is_empty() || scopes.get().trim().is_empty()
                    on:click=move |_| { submit_action.dispatch(()); }
                >
                    {move || if submitting.get() {
                        view! {
                          <span class="loading loading-spinner loading-sm"></span>
                          "Creating..."
                        }.into_any()
                    } else {
                        view! {
                          <svg class="w-5 h-5 mr-1" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6v6m0 0v6m0-6h6m-6 0H6"></path></svg>
                          "Create Service Token"
                        }.into_any()
                    }}
                </button>
            </div>
        </div>
    }
}

#[component]
fn DocumentationFeedbackAdminPanel() -> impl IntoView {
    let (refresh_counter, set_refresh_counter) = signal(0u32);
    let (page, set_page) = signal(0u64);
    let (query, set_query) = signal(String::new());
    let (kind_filter, set_kind_filter) = signal(String::new());
    let (status_filter, set_status_filter) = signal("open".to_string());

    let list_resource = LocalResource::new(move || {
        let page = page.get();
        let query = query.get();
        let kind = kind_filter.get();
        let status = status_filter.get();
        let _ = refresh_counter.get();
        async move {
            with_auth_retry(|| {
                list_documentation_feedback(
                    page,
                    20,
                    (!query.trim().is_empty()).then_some(query.clone()),
                    (!kind.trim().is_empty()).then_some(kind.clone()),
                    (!status.trim().is_empty()).then_some(status.clone()),
                )
            })
            .await
        }
    });

    let trigger_refresh = move || set_refresh_counter.update(|value| *value += 1);

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200 overflow-hidden">
            <div class="card-body p-0">
                <div class="p-8 pb-4">
                    <div class="flex items-center gap-3 mb-2">
                        <svg class="w-6 h-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 10h8M8 14h6"></path>
                        </svg>
                        <h2 class="card-title text-2xl">"Documentation Feedback Registry"</h2>
                    </div>
                    <p class="text-base-content/60">
                        "Admin-only registry of MCP-reported documentation gaps and improvement proposals. Use this to resolve, deduplicate, and prioritize documentation maintenance without turning Lekton into a ticket tracker."
                    </p>
                </div>

                <div class="px-8 pb-6">
                    <div class="grid grid-cols-1 gap-4 md:grid-cols-4 md:items-end">
                        <label class="form-control md:col-span-2">
                            <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60 mb-2">"Search"</span>
                            <input
                                type="text"
                                class="input input-bordered h-12 focus:input-primary"
                                placeholder="Search title, summary, docs:// URI, or proposal"
                                prop:value=move || query.get()
                                on:input=move |ev| {
                                    set_page.set(0);
                                    set_query.set(event_target_value(&ev));
                                }
                            />
                        </label>

                        <label class="form-control">
                            <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60 mb-2">"Kind"</span>
                            <select
                                class="select select-bordered h-12 focus:select-primary"
                                prop:value=move || kind_filter.get()
                                on:change=move |ev| {
                                    set_page.set(0);
                                    set_kind_filter.set(event_target_value(&ev));
                                }
                            >
                                <option value="">"All kinds"</option>
                                <option value="missing_info">"Missing info"</option>
                                <option value="improvement">"Improvement"</option>
                            </select>
                        </label>

                        <label class="form-control">
                            <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60 mb-2">"Status"</span>
                            <select
                                class="select select-bordered h-12 focus:select-primary"
                                prop:value=move || status_filter.get()
                                on:change=move |ev| {
                                    set_page.set(0);
                                    set_status_filter.set(event_target_value(&ev));
                                }
                            >
                                <option value="open">"Open"</option>
                                <option value="resolved">"Resolved"</option>
                                <option value="">"All statuses"</option>
                            </select>
                        </label>
                    </div>
                </div>

                <div class="px-8 pb-8">
                    <Suspense fallback=move || view! {
                        <div class="flex flex-col items-center justify-center py-12 gap-4">
                            <span class="loading loading-spinner loading-lg text-primary"></span>
                            <p class="text-sm font-medium animate-pulse">"Loading documentation feedback..."</p>
                        </div>
                    }>
                        {move || list_resource.get().map(|result| match result {
                            Ok(result) => view! {
                                <DocumentationFeedbackList
                                    result=result
                                    page=page
                                    set_page=set_page
                                    trigger_refresh=trigger_refresh
                                />
                            }.into_any(),
                            Err(e) => view! {
                                <div class="alert alert-error shadow-sm border-none bg-error/10 text-error">
                                    <span>{format!("Failed to load documentation feedback: {e}")}</span>
                                </div>
                            }.into_any(),
                        })}
                    </Suspense>
                </div>
            </div>
        </div>
    }
}

#[component]
fn DocumentationFeedbackList(
    result: DocumentationFeedbackAdminListResult,
    page: ReadSignal<u64>,
    set_page: WriteSignal<u64>,
    trigger_refresh: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    if result.items.is_empty() {
        return view! {
            <div class="flex flex-col items-center justify-center py-10 px-4 text-center border-2 border-dashed border-base-300 rounded-xl bg-base-200/20">
                <h3 class="font-bold text-lg text-base-content/70">"No matching feedback items"</h3>
                <p class="text-sm text-base-content/50 max-w-xl mt-1">
                    "The registry is empty for the selected filters. MCP agents will populate it through the documentation feedback tools."
                </p>
            </div>
        }.into_any();
    }

    let total_pages = result.total.div_ceil(result.per_page).max(1);
    let has_prev = result.page > 0;
    let has_next = result.page + 1 < total_pages;

    view! {
        <div class="space-y-5">
            <div class="flex flex-wrap items-center justify-between gap-2 text-sm text-base-content/60">
                <span>{format!("{} total item(s)", result.total)}</span>
                <span>{format!("Page {} of {}", result.page + 1, total_pages)}</span>
            </div>

            <div class="space-y-4">
                {result
                    .items
                    .into_iter()
                    .map(|item| view! { <DocumentationFeedbackCard item=item trigger_refresh=trigger_refresh /> })
                    .collect::<Vec<_>>()}
            </div>

            <div class="flex items-center justify-end gap-3 pt-2">
                <button
                    class="btn btn-outline btn-sm"
                    disabled=!has_prev
                    on:click=move |_| set_page.update(|value| {
                        if *value > 0 {
                            *value -= 1;
                        }
                    })
                >
                    "Previous"
                </button>
                <button
                    class="btn btn-outline btn-sm"
                    disabled=!has_next
                    on:click=move |_| set_page.update(|value| *value += 1)
                >
                    "Next"
                </button>
            </div>
        </div>
    }.into_any()
}

#[component]
fn DocumentationFeedbackCard(
    item: DocumentationFeedbackAdminItem,
    trigger_refresh: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let status_is_open = item.status == "open";
    let item_id_for_resolve = item.id.clone();
    let item_id_for_duplicate = item.id.clone();
    let (resolution_note, set_resolution_note) = signal(item.resolution_note.clone().unwrap_or_default());
    let (duplicate_of, set_duplicate_of) = signal(item.duplicate_of.clone().unwrap_or_default());
    let (error, set_error) = signal(Option::<String>::None);

    let resolve_action = Action::new_local(move |_: &()| {
        let id = item_id_for_resolve.clone();
        let note = resolution_note.get_untracked();
        async move {
            set_error.set(None);
            match with_auth_retry(|| resolve_documentation_feedback(id.clone(), (!note.trim().is_empty()).then_some(note.clone()))).await {
                Ok(()) => trigger_refresh(),
                Err(err) => set_error.set(Some(err.to_string())),
            }
        }
    });

    let duplicate_action = Action::new_local(move |_: &()| {
        let id = item_id_for_duplicate.clone();
        let duplicate_of_value = duplicate_of.get_untracked();
        let note = resolution_note.get_untracked();
        async move {
            set_error.set(None);
            match with_auth_retry(|| mark_documentation_feedback_duplicate(
                id.clone(),
                duplicate_of_value.clone(),
                (!note.trim().is_empty()).then_some(note.clone()),
            ))
            .await
            {
                Ok(()) => trigger_refresh(),
                Err(err) => set_error.set(Some(err.to_string())),
            }
        }
    });

    let detail_sections = {
        let mut sections = Vec::new();

        if let Some(view) = documentation_feedback_detail_view("Related resources", item.related_resources.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_detail_view("Search queries", item.search_queries.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("User goal", item.user_goal.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Missing information", item.missing_information.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Impact", item.impact.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Suggested target resource", item.suggested_target_resource.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Target resource", item.target_resource_uri.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Problem summary", item.problem_summary.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Proposal", item.proposal.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_detail_view("Supporting resources", item.supporting_resources.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Expected benefit", item.expected_benefit.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_detail_view("Related feedback ids", item.related_feedback_ids.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Duplicate of", item.duplicate_of.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Resolution note", item.resolution_note.clone()) {
            sections.push(view);
        }

        sections
    };

    view! {
        <div class="rounded-2xl border border-base-200 bg-base-100 shadow-sm">
            <div class="p-6 space-y-5">
                <div class="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
                    <div class="space-y-2 min-w-0">
                        <div class="flex flex-wrap items-center gap-2">
                            <span class=move || format!(
                                "badge badge-sm font-semibold {}",
                                if item.kind == "missing_info" { "badge-warning" } else { "badge-info" }
                            )>
                                {item.kind.clone()}
                            </span>
                            <span class=move || format!(
                                "badge badge-sm badge-outline {}",
                                if item.status == "open" { "badge-primary" } else { "badge-ghost" }
                            )>
                                {item.status.clone()}
                            </span>
                            <span class="text-xs text-base-content/50 font-mono break-all">{item.id.clone()}</span>
                        </div>
                        <h3 class="text-lg font-bold">{item.title.clone()}</h3>
                        <p class="text-sm text-base-content/70 break-words">{item.summary.clone()}</p>
                    </div>
                    <div class="self-start rounded-xl bg-base-200/40 px-3 py-2 text-xs leading-relaxed text-base-content/60 md:max-w-xs md:text-right">
                        <div class="font-medium break-words text-base-content/75">{item.created_by.clone()}</div>
                        <div class="whitespace-nowrap">{item.created_at.clone()}</div>
                    </div>
                </div>

                <Show when=move || error.get().is_some()>
                    <div class="alert alert-error shadow-sm border-none bg-error/10 text-error text-sm">
                        <span>{move || error.get().unwrap_or_default()}</span>
                    </div>
                </Show>

                <div class="grid grid-cols-1 gap-5 text-sm lg:grid-cols-2">
                    {detail_sections}
                </div>

                <Show when=move || status_is_open>
                    <div class="border-t border-base-200 pt-5">
                        <div class="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]">
                            <label class="form-control">
                                <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60 mb-2">"Resolution note"</span>
                                <textarea
                                    class="textarea textarea-bordered min-h-28 focus:textarea-primary"
                                    placeholder="Optional note describing how the item was resolved or why it was marked duplicate."
                                    prop:value=move || resolution_note.get()
                                    on:input=move |ev| set_resolution_note.set(event_target_value(&ev))
                                ></textarea>
                            </label>

                            <div class="flex h-full flex-col gap-3">
                                <label class="form-control flex-1">
                                    <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60 mb-2">"Duplicate of"</span>
                                    <input
                                        type="text"
                                        class="input input-bordered h-12 focus:input-primary font-mono"
                                        placeholder="Existing feedback id"
                                        prop:value=move || duplicate_of.get()
                                        on:input=move |ev| set_duplicate_of.set(event_target_value(&ev))
                                    />
                                </label>

                                <div class="flex flex-col gap-3 sm:flex-row xl:justify-end">
                                    <button
                                        class="btn btn-outline sm:flex-1 xl:flex-none"
                                        on:click=move |_| { resolve_action.dispatch(()); }
                                    >
                                        "Resolve"
                                    </button>

                                    <button
                                        class="btn btn-primary sm:flex-1 xl:flex-none"
                                        disabled=move || duplicate_of.get().trim().is_empty()
                                        on:click=move |_| { duplicate_action.dispatch(()); }
                                    >
                                        "Mark Duplicate"
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>
                </Show>
            </div>
        </div>
    }
}

fn documentation_feedback_detail_view(title: &'static str, values: Vec<String>) -> Option<AnyView> {
    if values.is_empty() {
        return None;
    }

    let multiline = title == "Search queries";

    Some(view! {
        <div class="space-y-2">
            <div class="text-xs font-bold uppercase tracking-wider text-base-content/50">{title}</div>
            {if multiline {
                view! {
                    <div class="space-y-2">
                        {values.into_iter().map(|value| view! {
                            <div class="rounded-xl border border-base-300/70 bg-base-200/30 px-3 py-2 font-mono text-xs leading-relaxed whitespace-pre-wrap break-words overflow-hidden">
                                {value}
                            </div>
                        }).collect::<Vec<_>>()}
                    </div>
                }.into_any()
            } else {
                view! {
                    <div class="flex flex-wrap gap-2">
                        {values.into_iter().map(|value| view! {
                            <span class="badge badge-outline badge-sm max-w-full font-mono whitespace-normal break-all py-3">{value}</span>
                        }).collect::<Vec<_>>()}
                    </div>
                }.into_any()
            }}
        </div>
    }.into_any())
}

fn documentation_feedback_optional_view(title: &'static str, value: Option<String>) -> Option<AnyView> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return None;
    };

    Some(view! {
        <div class="space-y-2">
            <div class="text-xs font-bold uppercase tracking-wider text-base-content/50">{title}</div>
            <div class="rounded-xl bg-base-200/40 px-4 py-3 whitespace-pre-wrap break-words">{value}</div>
        </div>
    }.into_any())
}

/// Flattened item for the ordering UI.
#[derive(Debug, Clone)]
struct OrderableItem {
    slug: String,
    title: String,
    level: u32,
}

/// Extract sections/categories (nodes with children) from the nav tree, recursively.
fn collect_sections(items: &[NavItem], level: u32, out: &mut Vec<OrderableItem>) {
    for item in items {
        if !item.children.is_empty() {
            out.push(OrderableItem {
                slug: item.slug.clone(),
                title: item.title.clone(),
                level,
            });
            collect_sections(&item.children, level + 1, out);
        }
    }
}

/// Admin component for reordering navigation sections and categories.
#[component]
fn NavigationOrderEditor() -> impl IntoView {
    let (items, set_items) = signal(Vec::<OrderableItem>::new());
    let (original_slugs, set_original_slugs) = signal(Vec::<String>::new());
    let (saving, set_saving) = signal(false);
    let (message, set_message) = signal(Option::<(bool, String)>::None);
    let (dragging_idx, set_dragging_idx) = signal(Option::<usize>::None);

    // Load nav tree and existing weights
    let nav_resource = LocalResource::new(|| with_auth_retry(get_navigation));
    let order_resource = LocalResource::new(|| with_auth_retry(get_navigation_order));

    // Merge nav tree with existing weights to build the orderable list
    let _ = Effect::new(move |_| {
        let nav = nav_resource.get();
        let order = order_resource.get();

        if let (Some(Ok(nav_items)), Some(Ok(order_entries))) = (nav, order) {
            let mut sections = Vec::new();
            collect_sections(&nav_items, 0, &mut sections);

            // If there are existing weights, reorder sections by weight at each level
            if !order_entries.is_empty() {
                let weight_map: std::collections::HashMap<String, i32> = order_entries
                    .iter()
                    .map(|e| (e.slug.clone(), e.weight))
                    .collect();

                // Group by level and parent prefix, then sort by weight within each group
                // Simple approach: sort the whole list respecting hierarchy
                reorder_by_weights(&mut sections, &weight_map);
            }

            set_original_slugs.set(sections.iter().map(|s| s.slug.clone()).collect());
            set_items.set(sections);
        }
    });

    let has_changes = move || {
        let current: Vec<String> = items.get().iter().map(|s| s.slug.clone()).collect();
        current != original_slugs.get()
    };

    let save_action = Action::new_local(move |_: &()| {
        let current_items = items.get_untracked();
        async move {
            set_saving.set(true);
            set_message.set(None);

            // Build entries with weights based on position within each level
            let entries: Vec<NavigationOrderEntry> = current_items
                .iter()
                .enumerate()
                .map(|(i, item)| NavigationOrderEntry {
                    slug: item.slug.clone(),
                    weight: (i as i32) * 10,
                })
                .collect();

            let result = with_auth_retry(|| save_navigation_order(entries.clone())).await;
            set_saving.set(false);

            match result {
                Ok(msg) => {
                    set_original_slugs.set(current_items.iter().map(|s| s.slug.clone()).collect());
                    set_message.set(Some((true, msg)));
                }
                Err(e) => {
                    set_message.set(Some((false, e.to_string())));
                }
            }
        }
    });

    // Find the subtree range for an item: [idx .. end) where end is the first
    // item at the same or higher level, i.e. the item plus all its descendants.
    fn subtree_range(items: &[OrderableItem], idx: usize) -> std::ops::Range<usize> {
        let level = items[idx].level;
        let mut end = idx + 1;
        while end < items.len() && items[end].level > level {
            end += 1;
        }
        idx..end
    }

    let move_item = move |idx: usize, direction: i32| {
        set_items.update(|items| {
            let level = items[idx].level;
            let src_range = subtree_range(items, idx);

            if direction < 0 {
                // Move up: find previous sibling at the same level
                if src_range.start == 0 {
                    return;
                }
                // Walk backwards from src_range.start to find the previous sibling
                let mut prev_idx = src_range.start - 1;
                while prev_idx > 0 && items[prev_idx].level > level {
                    prev_idx -= 1;
                }
                if items[prev_idx].level != level {
                    return;
                }
                // Extract subtree and reinsert before previous sibling
                let subtree: Vec<_> = items.drain(src_range.clone()).collect();
                items.splice(prev_idx..prev_idx, subtree);
            } else {
                // Move down: find next sibling at the same level
                if src_range.end >= items.len() {
                    return;
                }
                let next_range = subtree_range(items, src_range.end);
                if items[next_range.start].level != level {
                    return;
                }
                // Extract the next sibling's subtree and insert before current
                let next_subtree: Vec<_> = items.drain(next_range.clone()).collect();
                items.splice(src_range.start..src_range.start, next_subtree);
            }
        });
    };

    let on_drag_start = move |idx: usize| {
        set_dragging_idx.set(Some(idx));
    };

    let on_drag_over = move |idx: usize| {
        if let Some(from) = dragging_idx.get_untracked() {
            if from == idx {
                return;
            }
            set_items.update(|items| {
                if from >= items.len() || idx >= items.len() {
                    return;
                }
                let from_level = items[from].level;
                let idx_level = items[idx].level;
                if from_level != idx_level {
                    return;
                }

                // Find the root of the sibling subtree that idx belongs to.
                // Walk backwards from idx to find the first item at the same level.
                let mut target_root = idx;
                while target_root > 0 && items[target_root].level > from_level {
                    target_root -= 1;
                }
                if items[target_root].level != from_level {
                    return;
                }

                // Don't move onto ourselves
                let src = subtree_range(items, from);
                if src.contains(&target_root) {
                    return;
                }

                let dst = subtree_range(items, target_root);

                // Swap the two subtrees by extracting both and reinserting
                if from < target_root {
                    // Moving down: extract dst first (higher indices), then src
                    let dst_subtree: Vec<_> = items.drain(dst.clone()).collect();
                    let src_subtree: Vec<_> = items.drain(src.clone()).collect();
                    let src_len = src_subtree.len();
                    // Reinsert: src was at src.start, dst was at dst.start
                    // After removing both, insert at src.start: dst then src
                    items.splice(src.start..src.start, dst_subtree);
                    let new_src_start = src.start + (dst.len());
                    items.splice(new_src_start..new_src_start, src_subtree);
                    set_dragging_idx.set(Some(new_src_start));
                } else {
                    // Moving up: extract src first (higher indices), then dst
                    let src_subtree: Vec<_> = items.drain(src.clone()).collect();
                    let dst_subtree: Vec<_> = items.drain(dst.clone()).collect();
                    let dst_len = dst_subtree.len();
                    // Reinsert at dst.start: src then dst
                    items.splice(dst.start..dst.start, src_subtree.clone());
                    let new_dst_start = dst.start + src_subtree.len();
                    items.splice(new_dst_start..new_dst_start, dst_subtree);
                    set_dragging_idx.set(Some(dst.start));
                }
            });
        }
    };

    let on_drag_end = move || {
        set_dragging_idx.set(None);
    };

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200">
            <div class="card-body p-0">
                <div class="p-8 pb-4">
                    <div class="flex items-center gap-3 mb-2">
                        <svg class="w-6 h-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 10h16M4 14h16M4 18h16"></path>
                        </svg>
                        <h2 class="card-title text-2xl">"Navigation Ordering"</h2>
                    </div>
                    <p class="text-base-content/60">
                        "Reorder sections and categories in the navigation. Drag items or use the arrow buttons to change their position. Documents within each section are always sorted by their own weight or alphabetically."
                    </p>
                </div>

                <div class="px-8 py-4">
                    <Suspense fallback=move || view! {
                        <div class="flex flex-col items-center justify-center py-12 gap-4">
                            <span class="loading loading-spinner loading-lg text-primary"></span>
                            <p class="text-sm font-medium animate-pulse">"Loading navigation tree..."</p>
                        </div>
                    }>
                        {move || {
                            let current_items = items.get();
                            if current_items.is_empty() {
                                return view! {
                                    <div class="flex flex-col items-center justify-center py-10 px-4 text-center border-2 border-dashed border-base-300 rounded-xl bg-base-200/20">
                                        <div class="w-16 h-16 bg-base-300/30 rounded-full flex items-center justify-center mb-4">
                                            <svg class="w-8 h-8 text-base-content/30" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 10h16M4 14h16M4 18h16"></path>
                                            </svg>
                                        </div>
                                        <h3 class="font-bold text-lg text-base-content/70">"No sections found"</h3>
                                        <p class="text-sm text-base-content/50 max-w-xs mt-1">"Sections will appear here once documents with hierarchical slugs are ingested."</p>
                                    </div>
                                }.into_any();
                            }

                            view! {
                                <div class="space-y-1">
                                    {current_items.into_iter().enumerate().map(|(idx, item)| {
                                        let indent = item.level;
                                        let slug = item.slug.clone();
                                        let title = item.title.clone();
                                        let level_label = if indent == 0 { "Section" } else { "Category" };

                                        view! {
                                            <div
                                                class="flex items-center gap-2 p-3 rounded-lg border border-base-200 bg-base-100 hover:bg-base-200/30 transition-colors cursor-grab active:cursor-grabbing"
                                                style=format!("margin-left: {}rem", indent as f32 * 1.5)
                                                draggable="true"
                                                on:dragstart=move |_| on_drag_start(idx)
                                                on:dragover=move |ev| {
                                                    ev.prevent_default();
                                                    on_drag_over(idx);
                                                }
                                                on:dragend=move |_| on_drag_end()
                                            >
                                                // Drag handle
                                                <svg class="w-5 h-5 text-base-content/30 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 8h16M4 16h16"></path>
                                                </svg>

                                                // Title and info
                                                <div class="flex-1 min-w-0">
                                                    <div class="flex items-center gap-2">
                                                        <span class="font-medium text-sm truncate">{title}</span>
                                                        <span class="badge badge-ghost badge-xs text-[10px] uppercase">{level_label}</span>
                                                    </div>
                                                    <span class="text-xs text-base-content/40 font-mono truncate block">{slug}</span>
                                                </div>

                                                // Move buttons
                                                <div class="flex gap-1 flex-shrink-0">
                                                    <button
                                                        class="btn btn-ghost btn-xs btn-square"
                                                        title="Move up"
                                                        on:click=move |_| move_item(idx, -1)
                                                    >
                                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 15l7-7 7 7"></path>
                                                        </svg>
                                                    </button>
                                                    <button
                                                        class="btn btn-ghost btn-xs btn-square"
                                                        title="Move down"
                                                        on:click=move |_| move_item(idx, 1)
                                                    >
                                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"></path>
                                                        </svg>
                                                    </button>
                                                </div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    </Suspense>
                </div>

                <div class="bg-base-200/30 p-8 pt-6 border-t border-base-200">
                    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
                        <div class="min-h-[2.5rem]">
                            <Show when=move || message.get().is_some()>
                                {move || {
                                    let (success, text) = message.get().unwrap();
                                    let alert_class = if success { "alert-success bg-success/10 text-success" } else { "alert-error bg-error/10 text-error" };
                                    view! {
                                        <div class=format!("alert {alert_class} py-2 px-4 shadow-sm border-none flex items-center gap-2 text-sm font-semibold")>
                                            {if success {
                                                view! { <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"></path></svg> }.into_any()
                                            } else {
                                                view! { <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"></path></svg> }.into_any()
                                            }}
                                            <span>{text}</span>
                                        </div>
                                    }
                                }}
                            </Show>
                        </div>

                        <div class="flex flex-col sm:flex-row gap-3 w-full sm:w-auto">
                            <button
                                class="btn btn-ghost w-full sm:w-auto"
                                disabled=move || !has_changes() || saving.get()
                                on:click=move |_| {
                                    // Reset to original order
                                    let _ = nav_resource.get();
                                    let _ = order_resource.get();
                                }
                            >
                                "Discard"
                            </button>
                            <button
                                class="btn btn-primary w-full sm:w-64 shadow-lg shadow-primary/20"
                                disabled=move || !has_changes() || saving.get()
                                on:click=move |_| { save_action.dispatch(()); }
                            >
                                {move || if saving.get() {
                                    view! { <span class="loading loading-spinner loading-sm"></span> }.into_any()
                                } else {
                                    view! { "Save Order" }.into_any()
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Reorder sections in-place based on weight map, preserving hierarchy.
fn reorder_by_weights(sections: &mut Vec<OrderableItem>, weights: &std::collections::HashMap<String, i32>) {
    // Group items by (level, parent_prefix) and sort within each group
    // We need to identify groups of siblings at the same level
    let mut i = 0;
    while i < sections.len() {
        let level = sections[i].level;
        // Find the range of consecutive items at this level (siblings)
        let start = i;
        let mut end = i + 1;
        while end < sections.len() {
            if sections[end].level == level {
                // Check this is actually a sibling, not a same-level item from another parent
                // by ensuring no lower-level item from a different subtree is between them
                end += 1;
            } else if sections[end].level > level {
                // Child of current group, skip
                end += 1;
            } else {
                // Higher level item, different group
                break;
            }
        }

        // Collect the indices of items at this level within [start..end]
        let sibling_indices: Vec<usize> = (start..end)
            .filter(|&j| sections[j].level == level)
            .collect();

        if sibling_indices.len() > 1 {
            // Extract siblings with their subtrees
            let mut groups: Vec<Vec<OrderableItem>> = Vec::new();
            for &si in sibling_indices.iter().rev() {
                // Find the subtree: from si to next sibling (or end)
                let subtree_end = sibling_indices.iter()
                    .find(|&&j| j > si && sections[j].level == level)
                    .copied()
                    .unwrap_or(end);
                let subtree: Vec<OrderableItem> = sections[si..subtree_end].to_vec();
                groups.push(subtree);
            }
            groups.reverse();

            // Sort groups by weight of the root item
            groups.sort_by(|a, b| {
                let aw = weights.get(&a[0].slug).copied().unwrap_or(i32::MAX);
                let bw = weights.get(&b[0].slug).copied().unwrap_or(i32::MAX);
                aw.cmp(&bw).then_with(|| a[0].title.to_lowercase().cmp(&b[0].title.to_lowercase()))
            });

            // Reconstruct the range
            let mut new_range: Vec<OrderableItem> = Vec::new();
            for group in groups {
                new_range.extend(group);
            }

            // Replace range in sections
            sections.splice(start..end, new_range);
        }

        // Move past this group
        i = end;
        if i <= start {
            i = start + 1; // Safety: always advance
        }
    }
}

/// Component for editing custom application CSS.
#[component]
fn CustomCssEditor() -> impl IntoView {
    let (css, set_css) = signal(String::new());
    let (original_css, set_original_css) = signal(String::new());
    let (saving, set_saving) = signal(false);
    let (message, set_message) = signal(Option::<(bool, String)>::None);

    let load_resource = LocalResource::new(|| with_auth_retry(get_custom_css));

    let _ = Effect::new(move |_| {
        if let Some(Ok(loaded_css)) = load_resource.get() {
            set_css.set(loaded_css.clone());
            set_original_css.set(loaded_css);
        }
    });

    let save_action = Action::new_local(move |new_css: &String| {
        let new_css = new_css.clone();
        async move {
            set_saving.set(true);
            set_message.set(None);
            let result = with_auth_retry(|| save_custom_css(new_css.clone())).await;
            set_saving.set(false);
            match result {
                Ok(msg) => {
                    set_original_css.set(new_css);
                    set_message.set(Some((true, msg)));
                }
                Err(e) => {
                    set_message.set(Some((false, e.to_string())));
                }
            }
        }
    });

    let has_changes = move || css.get() != original_css.get();

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200">
            <div class="card-body p-0">
                <div class="p-8 pb-4">
                    <div class="flex items-center gap-3 mb-2">
                        <svg class="w-6 h-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 21a4 4 0 01-4-4V5a2 2 0 012-2h4a2 2 0 012 2v12a4 4 0 01-4 4zm0 0h12a2 2 0 002-2v-4a2 2 0 00-2-2h-2.343M11 7.343l1.172-1.172a4 4 0 115.656 5.656L15 13"></path>
                        </svg>
                        <h2 class="card-title text-2xl">"Theming & Custom CSS"</h2>
                    </div>
                    <p class="text-base-content/60">
                        "Customize the visual appearance of your Lekton instance. The CSS below is injected into every page at runtime."
                    </p>
                </div>

                <div class="p-8 pt-0 flex flex-col gap-6">
                    <div class="form-control">
                        <textarea
                            class="textarea textarea-bordered h-64 font-mono text-sm leading-relaxed focus:textarea-primary transition-all shadow-inner bg-base-200/20"
                            placeholder={"/* Example:\n:root {\n  --p: 262 80% 50%;\n}\n*/"}
                            prop:value=move || css.get()
                            on:input=move |ev| set_css.set(event_target_value(&ev))
                        ></textarea>
                    </div>

                    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
                        <div class="min-h-[2.5rem]">
                            <Show when=move || message.get().is_some()>
                                {move || {
                                    let (success, text) = message.get().unwrap();
                                    let alert_class = if success { "alert-success bg-success/10 text-success" } else { "alert-error bg-error/10 text-error" };
                                    view! {
                                        <div class=format!("alert {alert_class} py-2 px-4 shadow-sm border-none flex items-center gap-2 text-sm font-semibold")>
                                            {if success {
                                                view! { <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"></path></svg> }.into_any()
                                            } else {
                                                view! { <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"></path></svg> }.into_any()
                                            }}
                                            <span>{text}</span>
                                        </div>
                                    }
                                }}
                            </Show>
                        </div>

                        <div class="flex flex-col sm:flex-row gap-3 w-full sm:w-auto">
                            <button
                                class="btn btn-ghost w-full sm:w-auto"
                                disabled=move || !has_changes() || saving.get()
                                on:click=move |_| set_css.set(original_css.get())
                            >
                                "Discard"
                            </button>
                            <button
                                class="btn btn-primary w-full sm:w-64 shadow-lg shadow-primary/20"
                                disabled=move || !has_changes() || saving.get()
                                on:click=move |_| { save_action.dispatch(css.get()); }
                            >
                                {move || if saving.get() {
                                    view! { <span class="loading loading-spinner loading-sm"></span> }.into_any()
                                } else {
                                    view! { "Save Changes" }.into_any()
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Modal shown once after creating a token, displaying the raw token value.
#[component]
fn CreatedTokenModal(
    token: ReadSignal<Option<CreateTokenResult>>,
    set_token: WriteSignal<Option<CreateTokenResult>>,
) -> impl IntoView {
    let (copied, set_copied) = signal(false);

    view! {
        <Show when=move || token.get().is_some()>
            <div class="fixed inset-0 z-[200] flex items-center justify-center bg-black/60 backdrop-blur-sm animate-in fade-in duration-300">
                <div class="bg-base-100 rounded-3xl shadow-2xl w-full max-w-xl mx-4 p-10 relative overflow-hidden animate-in zoom-in-95 duration-300">
                    <div class="absolute top-0 inset-x-0 h-2 bg-warning"></div>

                    <div class="mb-8 flex items-center gap-4">
                        <div class="w-12 h-12 bg-warning/10 rounded-2xl flex items-center justify-center text-warning">
                          <svg xmlns="http://www.w3.org/2000/svg" class="h-8 w-8" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
                          </svg>
                        </div>
                        <div>
                          <h3 class="font-black text-3xl tracking-tight">"Token Created"</h3>
                          <p class="text-base-content/60 font-medium">"This is your only chance to copy it."</p>
                        </div>
                    </div>

                    <div class="bg-warning/10 border border-warning/20 rounded-2xl p-6 mb-8 flex items-start gap-4">
                        <div class="text-warning mt-1">
                          <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path></svg>
                        </div>
                        <p class="text-base-content text-sm font-semibold leading-relaxed">
                          "For security reasons, we do not store the raw token. If you lose it, you will need to deactivate it and create a new one."
                        </p>
                    </div>

                    {move || token.get().map(|t| {
                        let raw = t.raw_token.clone();
                        #[cfg(feature = "hydrate")]
                        let raw_for_copy = t.raw_token.clone();
                        let name = t.name.clone();
                        let scopes_str = t.allowed_scopes.join(", ");
                        view! {
                            <div class="space-y-6">
                                <div class="form-control">
                                    <label class="label pt-0"><span class="label-text font-bold text-xs uppercase tracking-widest text-base-content/50">"Generated Token"</span></label>
                                    <div class="relative group">
                                        <input
                                            type="text"
                                            readonly
                                            class="input input-bordered w-full font-mono text-lg py-8 pr-16 bg-base-200/50 border-base-300 focus:outline-none focus:border-warning/50 selection:bg-warning/20"
                                            value=raw
                                        />
                                        <button
                                            class="btn btn-warning shadow-lg shadow-warning/20 absolute right-2 top-1/2 -translate-y-1/2 normal-case font-bold"
                                            on:click=move |_| {
                                                #[cfg(feature = "hydrate")]
                                                {
                                                    let raw = raw_for_copy.clone();
                                                    let _ = js_sys::eval(&format!(
                                                        "navigator.clipboard.writeText('{}')",
                                                        raw.replace('\'', "\\'")
                                                    ));
                                                    set_copied.set(true);
                                                }
                                            }
                                        >
                                            {move || if copied.get() {
                                              view! { <span class="flex items-center gap-1"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"></path></svg>"Copied"</span> }.into_any()
                                            } else {
                                              view! { <span class="flex items-center gap-1"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3"></path></svg>"Copy"</span> }.into_any()
                                            }}
                                        </button>
                                    </div>
                                </div>

                                <div class="grid grid-cols-2 gap-8 py-6 border-y border-base-200">
                                    <div>
                                      <p class="text-[10px] font-black uppercase text-base-content/40 tracking-widest mb-1">"Internal Name"</p>
                                      <p class="font-bold text-base">{name}</p>
                                    </div>
                                    <div>
                                      <p class="text-[10px] font-black uppercase text-base-content/40 tracking-widest mb-1">"Scopes"</p>
                                      <p class="font-mono text-xs truncate" title=scopes_str.clone()>{scopes_str.clone()}</p>
                                    </div>
                                </div>
                            </div>
                        }
                    })}

                    <div class="flex justify-end pt-8">
                        <button
                            class="btn btn-ghost hover:bg-base-200 w-full sm:w-64 font-bold"
                            on:click=move |_| {
                                set_token.set(None);
                                set_copied.set(false);
                            }
                        >
                            "I have saved the token"
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    }
}

// ── RAG Re-index ─────────────────────────────────────────────────────────────

/// RAG re-index section — visible only when RAG is enabled.
#[component]
fn RagReindexSection() -> impl IntoView {
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

// ── Admin PAT Manager ─────────────────────────────────────────────────────────

const ADMIN_PAT_PER_PAGE: u64 = 20;

/// Admin section: paginated list of all PATs with user resolution and toggle.
#[component]
fn AdminPatManager() -> impl IntoView {
    let page = RwSignal::new(1u64);

    let pats_resource = LocalResource::new(move || {
        let page = page.get();
        with_auth_retry(move || admin_list_pats(page, ADMIN_PAT_PER_PAGE))
    });

    let toggle_action = Action::new_local(move |(id, active): &(String, bool)| {
        let id = id.clone();
        let active = *active;
        async move {
            if with_auth_retry(|| admin_toggle_pat(id.clone(), active)).await.is_ok() {
                pats_resource.refetch();
            }
        }
    });

    view! {
        <div class="space-y-4">
            <div class="flex items-center justify-between">
                <div>
                    <h2 class="text-lg font-semibold">"Personal Access Tokens"</h2>
                    <p class="text-sm text-base-content/60 mt-1">
                        "PATs issued to users for IDE agent access (Claude Code, Cursor, etc.)."
                    </p>
                </div>
            </div>

            <Suspense fallback=|| view! { <div class="skeleton h-40 w-full" /> }>
                {move || pats_resource.get().map(|res| match res {
                    Err(e) => view! {
                        <div class="alert alert-error">{e.to_string()}</div>
                    }.into_any(),
                    Ok((pats, total)) => {
                        let total_pages = total.div_ceil(ADMIN_PAT_PER_PAGE).max(1);
                        view! {
                            <div class="overflow-x-auto rounded-lg border border-base-200">
                                <table class="table table-sm">
                                    <thead>
                                        <tr class="bg-base-200/50">
                                            <th>"Token name"</th>
                                            <th>"User"</th>
                                            <th>"Created"</th>
                                            <th>"Last used"</th>
                                            <th>"Status"</th>
                                            <th></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {if pats.is_empty() {
                                            view! {
                                                <tr>
                                                    <td colspan="6" class="text-center py-8 text-base-content/40">"No PATs found."</td>
                                                </tr>
                                            }.into_any()
                                        } else {
                                            pats.into_iter().map(|pat| {
                                                let id = pat.id.clone();
                                                let is_active = pat.is_active;
                                                view! {
                                                    <tr class=("opacity-40", !is_active)>
                                                        <td class="font-mono text-sm">{pat.name}</td>
                                                        <td class="text-sm">
                                                            {match pat.user_email {
                                                                Some(email) => view! { <span>{email}</span> }.into_any(),
                                                                None => view! { <span class="badge badge-ghost badge-xs">"admin-pat"</span> }.into_any(),
                                                            }}
                                                        </td>
                                                        <td class="text-sm text-base-content/60">{pat.created_at}</td>
                                                        <td class="text-sm text-base-content/60">
                                                            {pat.last_used_at.unwrap_or_else(|| "Never".to_string())}
                                                        </td>
                                                        <td>
                                                            {if is_active {
                                                                view! { <span class="badge badge-success badge-sm">"Active"</span> }.into_any()
                                                            } else {
                                                                view! { <span class="badge badge-ghost badge-sm">"Inactive"</span> }.into_any()
                                                            }}
                                                        </td>
                                                        <td>
                                                            <button
                                                                class="btn btn-xs btn-ghost"
                                                                on:click=move |_| { toggle_action.dispatch((id.clone(), !is_active)); }
                                                            >
                                                                {if is_active { "Deactivate" } else { "Activate" }}
                                                            </button>
                                                        </td>
                                                    </tr>
                                                }
                                            }).collect_view().into_any()
                                        }}
                                    </tbody>
                                </table>
                            </div>

                            // Pagination
                            {if total_pages > 1 {
                                view! {
                                    <div class="flex justify-between items-center pt-2">
                                        <span class="text-sm text-base-content/60">
                                            {format!("{total} tokens total")}
                                        </span>
                                        <div class="join">
                                            <button
                                                class="join-item btn btn-sm"
                                                disabled=move || page.get() <= 1
                                                on:click=move |_| { page.update(|p| *p = p.saturating_sub(1)); }
                                            >"«"</button>
                                            <button class="join-item btn btn-sm btn-disabled">
                                                {move || format!("{} / {total_pages}", page.get())}
                                            </button>
                                            <button
                                                class="join-item btn btn-sm"
                                                disabled=move || page.get() >= total_pages
                                                on:click=move |_| { page.update(|p| *p += 1); }
                                            >"»"</button>
                                        </div>
                                    </div>
                                }.into_any()
                            } else {
                                view! { <div /> }.into_any()
                            }}
                        }.into_any()
                    }
                })}
            </Suspense>
        </div>
    }
}
