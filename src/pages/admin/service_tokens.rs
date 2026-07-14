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

/// Component managing service tokens.
#[component]
pub fn ServiceTokenManager(
    set_created_token: WriteSignal<Option<CreateTokenResult>>,
) -> impl IntoView {
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
                <p class="text-sm text-base-content/65 max-w-xs mt-1">"Create your first token below to start automating document updates."</p>
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
#[allow(unused_variables)]
#[component]
fn TokenRow(
    token: ServiceTokenInfo,
    trigger_refresh: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let id = token.id.clone();
    let name = token.name.clone();
    let scopes: Vec<String> = token.allowed_scopes.clone();
    let created_at = token.created_at.clone();
    let last_used = token
        .last_used_at
        .clone()
        .unwrap_or_else(|| "Never".to_string());
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
                        view! { <span class="text-[10px] text-base-content/65 ml-1">" + "{token.allowed_scopes.len() - 3}" more"</span> }.into_any()
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
                    view! { <span class="badge badge-success badge-sm font-medium">"active"</span> }.into_any()
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
                        class="btn btn-outline btn-error btn-xs normal-case font-medium"
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
            let result = with_auth_retry(|| {
                create_service_token(name_val.clone(), scopes_val.clone(), can_write_val)
            })
            .await;
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
                <p class="text-sm text-base-content/65">"Configure a new scoped access token."</p>
            </div>

            <Show when=move || error.get().is_some()>
                <div class="alert alert-error shadow-sm border-none bg-error/10 text-error animate-in fade-in slide-in-from-top-2">
                    <svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-6 w-6" fill="none" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                    <span class="text-xs font-semibold">{move || error.get().unwrap_or_default()}</span>
                </div>
            </Show>

            <div class="grid grid-cols-1 md:grid-cols-2 gap-x-8 gap-y-6">
                <div>
                    <label class="block font-bold text-xs uppercase tracking-wider text-base-content/60 mb-1.5">"Token Name"</label>
                    <input
                        type="text"
                        placeholder="e.g. github-actions-ci"
                        class="input input-bordered w-full focus:input-primary transition-all shadow-sm"
                        prop:value=move || name.get()
                        on:input=move |ev| set_name.set(event_target_value(&ev))
                    />
                    <p class="mt-1.5 text-xs text-base-content/65 italic">"A descriptive name for identification."</p>
                </div>

                <div>
                    <label class="block font-bold text-xs uppercase tracking-wider text-base-content/60 mb-1.5">"Permissions"</label>
                    <div class="bg-base-100 rounded-lg border border-base-300 px-3 shadow-sm flex items-center min-h-12">
                      <label class="cursor-pointer flex items-center gap-4 w-full">
                          <input
                              type="checkbox"
                              class="checkbox checkbox-primary"
                              prop:checked=move || can_write.get()
                              on:change=move |ev| set_can_write.set(event_target_checked(&ev))
                          />
                          <div>
                            <span class="font-bold block mb-0.5">"Allow Write Access"</span>
                            <span class="text-xs text-base-content/65">"Permit updates and deletions via API."</span>
                          </div>
                      </label>
                    </div>
                    <p class="mt-1.5 text-xs text-base-content/65 italic">"Controls write permissions for this token."</p>
                </div>
            </div>

            <div>
                <label class="block font-bold text-xs uppercase tracking-wider text-base-content/60 mb-1.5">"Allowed Scopes"</label>
                <textarea
                    class="textarea textarea-bordered w-full h-32 font-mono text-sm leading-relaxed focus:textarea-primary transition-all shadow-sm"
                    placeholder={"docs/getting-started\nprojects/*\napi/v2/reference"}
                    prop:value=move || scopes.get()
                    on:input=move |ev| set_scopes.set(event_target_value(&ev))
                ></textarea>
                <p class="mt-1.5 text-xs text-base-content/65 inline-flex items-center gap-1.5">
                    <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path></svg>
                    "Enter one path prefix per line."
                </p>
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
