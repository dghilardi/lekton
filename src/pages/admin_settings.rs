use leptos::prelude::*;

use crate::app::{
    create_service_token, deactivate_service_token, list_service_tokens, CreateTokenResult,
    ServiceTokenInfo,
};

/// Admin settings page with service token management.
#[component]
pub fn AdminSettingsPage() -> impl IntoView {
    let current_user =
        use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();

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
                    <div class="alert alert-error max-w-md">
                        <svg xmlns="http://www.w3.org/2000/svg" class="h-6 w-6 shrink-0 stroke-current" fill="none" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
                        </svg>
                        <span>"Access denied. Admin privileges required."</span>
                    </div>
                </div>
            }
        >
            <AdminSettingsContent />
        </Show>
    }
}

/// Inner content, rendered only for admins.
#[component]
fn AdminSettingsContent() -> impl IntoView {
    // Signal to trigger token list reload
    let (refresh_counter, set_refresh_counter) = signal(0u32);

    // Load tokens
    let tokens_resource = Resource::new(
        move || refresh_counter.get(),
        |_| list_service_tokens(),
    );

    // Created token (shown once in modal)
    let (created_token, set_created_token) = signal(Option::<CreateTokenResult>::None);

    let trigger_refresh = move || set_refresh_counter.update(|c| *c += 1);

    view! {
        <div class="max-w-4xl mx-auto">
            <h1 class="text-3xl font-bold mb-8">"Settings"</h1>

            // Service Tokens section
            <div class="card bg-base-100 shadow-lg mb-8">
                <div class="card-body">
                    <div class="flex justify-between items-center mb-4">
                        <h2 class="card-title">"Service Tokens"</h2>
                    </div>

                    <p class="text-sm text-base-content/60 mb-6">
                        "Scoped tokens for CI/CD pipelines. Each token can read/write documents within its allowed scopes."
                    </p>

                    // Token list
                    <Suspense fallback=move || view! {
                        <div class="flex justify-center py-8">
                            <span class="loading loading-spinner loading-lg"></span>
                        </div>
                    }>
                        {move || tokens_resource.get().map(|result| match result {
                            Ok(tokens) => view! { <TokenTable tokens=tokens trigger_refresh=trigger_refresh /> }.into_any(),
                            Err(e) => view! {
                                <div class="alert alert-error">
                                    <span>{format!("Failed to load tokens: {e}")}</span>
                                </div>
                            }.into_any(),
                        })}
                    </Suspense>

                    <div class="divider"></div>

                    // Create form
                    <CreateTokenForm
                        on_created=move |result| {
                            set_created_token.set(Some(result));
                            trigger_refresh();
                        }
                    />
                </div>
            </div>
        </div>

        // Created token modal
        <CreatedTokenModal token=created_token set_token=set_created_token />
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
            <div class="text-center py-6 text-base-content/50">
                "No service tokens created yet."
            </div>
        }
        .into_any();
    }

    view! {
        <div class="overflow-x-auto">
            <table class="table table-sm">
                <thead>
                    <tr>
                        <th>"Name"</th>
                        <th>"Scopes"</th>
                        <th>"Write"</th>
                        <th>"Status"</th>
                        <th>"Created"</th>
                        <th>"Last Used"</th>
                        <th></th>
                    </tr>
                </thead>
                <tbody>
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
    let last_used = token.last_used_at.clone().unwrap_or_else(|| "-".to_string());
    let is_active = token.is_active;
    let can_write = token.can_write;

    let (deactivating, set_deactivating) = signal(false);

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
        <tr class=move || if !is_active { "opacity-50" } else { "" }>
            <td class="font-mono text-sm">{name}</td>
            <td>
                <div class="flex flex-wrap gap-1">
                    {scopes.into_iter().map(|scope| {
                        view! { <span class="badge badge-outline badge-xs">{scope}</span> }
                    }).collect::<Vec<_>>()}
                </div>
            </td>
            <td>
                {if can_write {
                    view! { <span class="badge badge-success badge-xs">"write"</span> }.into_any()
                } else {
                    view! { <span class="badge badge-ghost badge-xs">"read"</span> }.into_any()
                }}
            </td>
            <td>
                {if is_active {
                    view! { <span class="badge badge-primary badge-xs">"active"</span> }.into_any()
                } else {
                    view! { <span class="badge badge-ghost badge-xs">"inactive"</span> }.into_any()
                }}
            </td>
            <td class="text-xs text-base-content/60">{created_at}</td>
            <td class="text-xs text-base-content/60">{last_used}</td>
            <td>
                <Show when=move || is_active>
                    <button
                        class="btn btn-ghost btn-xs text-error"
                        disabled=move || deactivating.get()
                        on:click=move |_| { deactivate_action.dispatch(()); }
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
        <h3 class="font-semibold text-lg mb-4">"Create New Token"</h3>

        <Show when=move || error.get().is_some()>
            <div class="alert alert-error mb-4">
                <span>{move || error.get().unwrap_or_default()}</span>
            </div>
        </Show>

        <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div class="form-control">
                <label class="label">
                    <span class="label-text">"Token Name"</span>
                </label>
                <input
                    type="text"
                    placeholder="e.g. iot-protocols-ci"
                    class="input input-bordered"
                    prop:value=move || name.get()
                    on:input=move |ev| set_name.set(event_target_value(&ev))
                />
            </div>

            <div class="form-control">
                <label class="label">
                    <span class="label-text">"Permissions"</span>
                </label>
                <label class="label cursor-pointer justify-start gap-3">
                    <input
                        type="checkbox"
                        class="checkbox checkbox-primary checkbox-sm"
                        prop:checked=move || can_write.get()
                        on:change=move |ev| set_can_write.set(event_target_checked(&ev))
                    />
                    <span class="label-text">"Write access"</span>
                </label>
            </div>
        </div>

        <div class="form-control mt-4">
            <label class="label">
                <span class="label-text">"Allowed Scopes (one per line)"</span>
            </label>
            <textarea
                class="textarea textarea-bordered h-24 font-mono text-sm"
                placeholder={"protocols/*\nguides/intro"}
                prop:value=move || scopes.get()
                on:input=move |ev| set_scopes.set(event_target_value(&ev))
            ></textarea>
            <label class="label">
                <span class="label-text-alt text-base-content/50">
                    "Use exact slugs (e.g. guides/intro) or prefix patterns (e.g. protocols/*)"
                </span>
            </label>
        </div>

        <div class="mt-4">
            <button
                class="btn btn-primary"
                disabled=move || submitting.get() || name.get().trim().is_empty() || scopes.get().trim().is_empty()
                on:click=move |_| { submit_action.dispatch(()); }
            >
                {move || if submitting.get() {
                    view! { <span class="loading loading-spinner loading-sm"></span> }.into_any()
                } else {
                    view! { <span>"Create Token"</span> }.into_any()
                }}
            </button>
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
            <div class="fixed inset-0 z-[200] flex items-center justify-center bg-black/50 backdrop-blur-sm">
                <div class="bg-base-100 rounded-lg shadow-2xl w-full max-w-lg mx-4 p-6">
                    <h3 class="font-bold text-lg mb-2">"Token Created"</h3>

                    <div class="alert alert-warning mb-4">
                        <svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5 shrink-0 stroke-current" fill="none" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
                        </svg>
                        <span class="text-sm">"Copy this token now. It will not be shown again."</span>
                    </div>

                    {move || token.get().map(|t| {
                        let raw = t.raw_token.clone();
                        let raw_for_copy = t.raw_token.clone();
                        let name = t.name.clone();
                        let scopes_str = t.allowed_scopes.join(", ");
                        view! {
                            <div class="mb-4">
                                <label class="label"><span class="label-text font-medium">"Token"</span></label>
                                <div class="flex gap-2">
                                    <input
                                        type="text"
                                        readonly
                                        class="input input-bordered font-mono text-sm flex-1"
                                        value=raw
                                    />
                                    <button
                                        class="btn btn-outline btn-sm"
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
                                        {move || if copied.get() { "Copied!" } else { "Copy" }}
                                    </button>
                                </div>
                            </div>

                            <div class="text-sm text-base-content/60 mb-4">
                                <p><strong>"Name: "</strong>{name}</p>
                                <p><strong>"Scopes: "</strong>{scopes_str}</p>
                            </div>
                        }
                    })}

                    <div class="flex justify-end">
                        <button
                            class="btn btn-primary"
                            on:click=move |_| {
                                set_token.set(None);
                                set_copied.set(false);
                            }
                        >
                            "Done"
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    }
}
