use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::app::{
    CreatePatResult, PatInfo,
    create_user_pat, delete_user_pat, list_user_pats, toggle_user_pat,
    get_current_user,
};

/// User profile page — shows account info and PAT management.
#[component]
pub fn ProfilePage() -> impl IntoView {
    let user_resource = LocalResource::new(get_current_user);
    let navigate = use_navigate();

    // Redirect to login if not authenticated
    Effect::new(move |_| {
        if let Some(Ok(None)) = user_resource.get() {
            navigate("/login", Default::default());
        }
    });

    view! {
        <div class="container mx-auto max-w-3xl px-4 py-8">
            <h1 class="text-2xl font-bold mb-6">"Profile"</h1>

            // User info card
            <Suspense fallback=|| view! { <div class="skeleton h-20 w-full mb-6" /> }>
                {move || user_resource.get().map(|res| match res.ok().flatten() {
                    Some(user) => view! {
                        <div class="card bg-base-200 mb-8">
                            <div class="card-body py-4">
                                <div class="flex items-center gap-4">
                                    <div class="avatar placeholder">
                                        <div class="bg-primary text-primary-content rounded-full w-12">
                                            <span class="text-lg">
                                                {user.name.as_ref()
                                                    .and_then(|n| n.chars().next())
                                                    .or_else(|| user.email.chars().next())
                                                    .map(|c| c.to_uppercase().to_string())
                                                    .unwrap_or_default()}
                                            </span>
                                        </div>
                                    </div>
                                    <div>
                                        <p class="font-semibold">
                                            {user.name.clone().unwrap_or_else(|| user.email.clone())}
                                        </p>
                                        <p class="text-sm text-base-content/60">{user.email.clone()}</p>
                                    </div>
                                    {if user.is_admin {
                                        view! { <span class="badge badge-error ml-auto">"Admin"</span> }.into_any()
                                    } else {
                                        view! { <span class="badge badge-ghost ml-auto">"User"</span> }.into_any()
                                    }}
                                </div>
                            </div>
                        </div>
                    }.into_any(),
                    None => view! { <div /> }.into_any(),
                })}
            </Suspense>

            // PAT section
            <PatSection />
        </div>
    }
}

#[component]
fn PatSection() -> impl IntoView {
    let pats = RwSignal::new(Vec::<PatInfo>::new());
    let new_token = RwSignal::new(None::<CreatePatResult>);

    // Load PATs on mount
    let load_pats = Action::new(move |_: &()| async move {
        match list_user_pats().await {
            Ok(tokens) => pats.set(tokens),
            Err(e) => tracing::error!("Failed to load PATs: {e}"),
        }
    });

    Effect::new(move |_| { load_pats.dispatch(()); });

    let on_created = move |result: CreatePatResult| {
        new_token.set(Some(result));
        load_pats.dispatch(());
    };

    view! {
        <div>
            <h2 class="text-xl font-semibold mb-4">"Personal Access Tokens"</h2>
            <p class="text-base-content/60 text-sm mb-6">
                "PATs allow IDE agents (Claude Code, Cursor, RooCode) to access documentation
                 on your behalf via the MCP server. Each token inherits your permissions."
            </p>

            // Show newly created token (one-time display)
            {move || new_token.get().map(|result| view! {
                <div class="alert alert-success mb-6">
                    <svg xmlns="http://www.w3.org/2000/svg" class="h-6 w-6 shrink-0 stroke-current" fill="none" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
                    </svg>
                    <div class="flex-1">
                        <p class="font-semibold">"Token created — copy it now, it won't be shown again."</p>
                        <code class="block mt-2 p-2 bg-success/20 rounded text-sm break-all select-all">
                            {result.raw_token.clone()}
                        </code>
                        <p class="text-xs mt-2 opacity-70">
                            "Add it to Claude Code: "
                            <code class="bg-success/20 px-1 rounded">
                                {format!("claude mcp add-json docs '{{\"type\":\"http\",\"url\":\"http://localhost:3000/mcp\",\"headers\":{{\"Authorization\":\"Bearer {}\"}}}}' ", result.raw_token)}
                            </code>
                        </p>
                    </div>
                    <button
                        class="btn btn-sm btn-ghost"
                        on:click=move |_| new_token.set(None)
                    >
                        "Dismiss"
                    </button>
                </div>
            })}

            // Create form
            <CreatePatForm on_created />

            // PAT table
            <div class="mt-6">
                {move || {
                    let tokens = pats.get();
                    if tokens.is_empty() {
                        view! {
                            <div class="text-center py-8 text-base-content/50">
                                <p>"No personal access tokens yet."</p>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="overflow-x-auto">
                                <table class="table table-sm">
                                    <thead>
                                        <tr>
                                            <th>"Name"</th>
                                            <th>"Created"</th>
                                            <th>"Last used"</th>
                                            <th>"Status"</th>
                                            <th></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        <For
                                            each=move || pats.get()
                                            key=|t| t.id.clone()
                                            children=move |token| {
                                                view! { <PatRow token pats load_pats /> }
                                            }
                                        />
                                    </tbody>
                                </table>
                            </div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

#[component]
fn CreatePatForm(on_created: impl Fn(CreatePatResult) + 'static + Copy + Send + Sync) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);

    let submit = Action::new(move |n: &String| {
        let n = n.clone();
        async move {
            loading.set(true);
            error.set(None);
            match create_user_pat(n).await {
                Ok(result) => {
                    name.set(String::new());
                    on_created(result);
                }
                Err(e) => error.set(Some(e.to_string())),
            }
            loading.set(false);
        }
    });

    view! {
        <div class="card bg-base-200">
            <div class="card-body py-4">
                <h3 class="card-title text-base">"Create new token"</h3>
                {move || error.get().map(|e| view! {
                    <div class="alert alert-error py-2 text-sm">{e}</div>
                })}
                <div class="flex gap-2">
                    <input
                        type="text"
                        placeholder="Token name (e.g. \"claude-code-work\")"
                        class="input input-bordered flex-1"
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                        on:keydown=move |ev| {
                            if ev.key() == "Enter" && !name.get().is_empty() {
                                submit.dispatch(name.get());
                            }
                        }
                    />
                    <button
                        class="btn btn-primary"
                        disabled=move || name.get().is_empty() || loading.get()
                        on:click=move |_| { submit.dispatch(name.get()); }
                    >
                        {move || if loading.get() {
                            view! { <span class="loading loading-spinner loading-sm" /> }.into_any()
                        } else {
                            view! { "Generate" }.into_any()
                        }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn PatRow(
    token: PatInfo,
    pats: RwSignal<Vec<PatInfo>>,
    load_pats: Action<(), ()>,
) -> impl IntoView {
    let id = token.id.clone();
    let is_active = token.is_active;

    let toggle = Action::new(move |active: &bool| {
        let active = *active;
        let id = id.clone();
        async move {
            if toggle_user_pat(id, active).await.is_ok() {
                load_pats.dispatch(());
            }
        }
    });

    let id_del = token.id.clone();
    let delete = Action::new(move |_: &()| {
        let id = id_del.clone();
        async move {
            if delete_user_pat(id).await.is_ok() {
                load_pats.dispatch(());
            }
        }
    });

    view! {
        <tr class=("opacity-40", !is_active)>
            <td class="font-mono text-sm">{token.name}</td>
            <td class="text-sm text-base-content/60">{token.created_at}</td>
            <td class="text-sm text-base-content/60">
                {token.last_used_at.unwrap_or_else(|| "Never".to_string())}
            </td>
            <td>
                {if is_active {
                    view! { <span class="badge badge-success badge-sm">"Active"</span> }.into_any()
                } else {
                    view! { <span class="badge badge-ghost badge-sm">"Inactive"</span> }.into_any()
                }}
            </td>
            <td>
                <div class="flex gap-1 justify-end">
                    <button
                        class=move || if is_active { "btn btn-xs btn-ghost" } else { "btn btn-xs btn-ghost text-success" }
                        on:click=move |_| { toggle.dispatch(!is_active); }
                        title=if is_active { "Deactivate" } else { "Activate" }
                    >
                        {if is_active { "Deactivate" } else { "Activate" }}
                    </button>
                    <button
                        class="btn btn-xs btn-ghost text-error"
                        on:click=move |_| {
                            #[cfg(feature = "hydrate")]
                            {
                                use leptos::web_sys::window;
                                if window().and_then(|w| w.confirm_with_message("Delete this token permanently?").ok()).unwrap_or(false) {
                                    delete.dispatch(());
                                }
                            }
                        }
                        title="Delete permanently"
                    >
                        "Delete"
                    </button>
                </div>
            </td>
        </tr>
    }
}
