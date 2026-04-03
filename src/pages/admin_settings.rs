use leptos::prelude::*;

use crate::app::{
    create_service_token, deactivate_service_token, get_custom_css, list_service_tokens,
    save_custom_css, CreateTokenResult, ServiceTokenInfo,
};

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
                <AdminSettingsContent />
            </div>
        </Show>
    }
}

/// Inner content, rendered only for admins.
#[component]
fn AdminSettingsContent() -> impl IntoView {
    // Created token (shown once in modal)
    let (created_token, set_created_token) = signal(Option::<CreateTokenResult>::None);

    view! {
        <div class="max-w-5xl mx-auto space-y-12 pb-20">
            <header class="flex items-center gap-4 border-b border-base-200 pb-8">
                <div class="p-3 bg-primary/10 rounded-2xl text-primary">
                    <svg class="w-8 h-8" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"></path>
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"></path>
                    </svg>
                </div>
                <div>
                  <h1 class="text-4xl font-extrabold tracking-tight">"Admin Settings"</h1>
                  <p class="text-base-content/60 mt-1">"Manage your instance configuration, service tokens, and theming."</p>
                </div>
            </header>

            // Service Tokens section
            <div class="grid grid-cols-1 gap-8">
                <ServiceTokenManager set_created_token=set_created_token />
                <CustomCssEditor />
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
    let tokens_resource = Resource::new(move || refresh_counter.get(), |_| list_service_tokens());

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
    let deactivate_action = Action::new(move |_: &()| {
        let id = id.clone();
        async move {
            set_deactivating.set(true);
            let result = deactivate_service_token(id).await;
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

    let submit_action = Action::new(move |_: &()| {
        let name_val = name.get_untracked();
        let scopes_val = scopes.get_untracked();
        let can_write_val = can_write.get_untracked();
        async move {
            set_error.set(None);
            set_submitting.set(true);
            let result = create_service_token(name_val, scopes_val, can_write_val).await;
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
                    <div class="bg-base-100 rounded-lg border border-base-300 p-2 shadow-sm">
                      <label class="label cursor-pointer justify-start gap-4 h-full">
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
                    class="btn btn-primary btn-wide shadow-lg shadow-primary/20"
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

/// Component for editing custom application CSS.
#[component]
fn CustomCssEditor() -> impl IntoView {
    let (css, set_css) = signal(String::new());
    let (original_css, set_original_css) = signal(String::new());
    let (saving, set_saving) = signal(false);
    let (message, set_message) = signal(Option::<(bool, String)>::None);

    let load_resource = Resource::new(|| (), |_| get_custom_css());

    let _ = Effect::new(move |_| {
        if let Some(Ok(loaded_css)) = load_resource.get() {
            set_css.set(loaded_css.clone());
            set_original_css.set(loaded_css);
        }
    });

    let save_action = Action::new(move |new_css: &String| {
        let new_css = new_css.clone();
        async move {
            set_saving.set(true);
            set_message.set(None);
            let result = save_custom_css(new_css.clone()).await;
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

                        <div class="flex gap-3">
                            <button
                                class="btn btn-ghost"
                                disabled=move || !has_changes() || saving.get()
                                on:click=move |_| set_css.set(original_css.get())
                            >
                                "Discard"
                            </button>
                            <button
                                class="btn btn-primary btn-wide shadow-lg shadow-primary/20"
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

                    <div class="bg-orange-50 border border-orange-200 rounded-2xl p-6 mb-8 flex items-start gap-4">
                        <div class="text-orange-600 mt-1">
                          <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path></svg>
                        </div>
                        <p class="text-orange-800 text-sm font-semibold leading-relaxed">
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
                            class="btn btn-ghost hover:bg-base-200 btn-wide font-bold"
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
