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

// ── Access Level Manager ──────────────────────────────────────────────────────

#[component]
pub fn AccessLevelManager() -> impl IntoView {
    let (refresh, set_refresh) = signal(0u32);

    let levels_resource = LocalResource::new(move || {
        let _ = refresh.get();
        with_auth_retry(list_admin_access_levels)
    });

    // Which level is currently being edited (by name)
    let editing = RwSignal::new(Option::<String>::None);
    let edit_label = RwSignal::new(String::new());
    let edit_description = RwSignal::new(String::new());
    let edit_inherits = RwSignal::new(Vec::<String>::new());

    let show_create = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_label = RwSignal::new(String::new());
    let new_description = RwSignal::new(String::new());
    let new_inherits = RwSignal::new(Vec::<String>::new());

    let error_msg = RwSignal::new(Option::<String>::None);

    let save_edit_action = Action::new_local(move |_: &()| async move {
        let Some(name) = editing.get_untracked() else {
            return;
        };
        let result = with_auth_retry(|| {
            update_admin_access_level(
                name.clone(),
                edit_label.get_untracked(),
                edit_description.get_untracked(),
                edit_inherits.get_untracked(),
            )
        })
        .await;
        match result {
            Ok(_) => {
                editing.set(None);
                error_msg.set(None);
                set_refresh.update(|c| *c += 1);
            }
            Err(e) => error_msg.set(Some(e.to_string())),
        }
    });

    let create_action = Action::new_local(move |_: &()| async move {
        let result = with_auth_retry(|| {
            create_admin_access_level(
                new_name.get_untracked(),
                new_label.get_untracked(),
                new_description.get_untracked(),
                new_inherits.get_untracked(),
            )
        })
        .await;
        match result {
            Ok(_) => {
                show_create.set(false);
                new_name.set(String::new());
                new_label.set(String::new());
                new_description.set(String::new());
                new_inherits.set(vec![]);
                error_msg.set(None);
                set_refresh.update(|c| *c += 1);
            }
            Err(e) => error_msg.set(Some(e.to_string())),
        }
    });

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200 overflow-hidden">
            <div class="card-body p-0">
                <div class="p-8 pb-4">
                    <h2 class="card-title text-2xl mb-1">"Access Levels"</h2>
                    <p class="text-base-content/60 text-sm">
                        "Define content access levels and their inheritance hierarchy. "
                        "System levels (public, loggeduser) are injected automatically and cannot be deleted."
                    </p>
                </div>

                {move || error_msg.get().map(|e| view! {
                    <div class="mx-8 alert alert-error text-sm">
                        <span>{e}</span>
                        <button class="btn btn-ghost btn-xs" on:click=move |_| error_msg.set(None)>"✕"</button>
                    </div>
                })}

                <div class="px-8 py-4 space-y-3">
                    <Suspense fallback=move || view! {
                        <div class="flex justify-center py-8">
                            <span class="loading loading-spinner loading-md text-primary"></span>
                        </div>
                    }>
                        {move || levels_resource.get().map(|result| match result {
                            Err(e) => view! {
                                <div class="alert alert-error text-sm"><span>{e.to_string()}</span></div>
                            }.into_any(),
                            Ok(levels) => {
                                let levels_for_inherit = levels.clone();
                                view! {
                                    <div class="space-y-2">
                                        <For
                                            each=move || levels.clone()
                                            key=|l| l.name.clone()
                                            children=move |level| {
                                                let name = level.name.clone();
                                                let name_for_edit = name.clone();
                                                let name_for_del = name.clone();
                                                let name_for_memo = name.clone();
                                                let is_editing = Memo::new(move |_| {
                                                    editing.get().as_deref() == Some(name_for_memo.as_str())
                                                });
                                                let all_levels = levels_for_inherit.clone();

                                                view! {
                                                    <div class="border border-base-200 rounded-xl overflow-hidden">
                                                        <div class="flex items-center gap-3 p-4 bg-base-100">
                                                            <div class="flex-1 min-w-0">
                                                                <div class="flex items-center gap-2 flex-wrap">
                                                                    <span class="font-mono text-sm font-semibold">{level.name.clone()}</span>
                                                                    <span class="text-base-content/60 text-sm">{level.label.clone()}</span>
                                                                    {if level.is_system {
                                                                        view! { <span class="badge badge-warning badge-sm">"system"</span> }.into_any()
                                                                    } else {
                                                                        view! { <span /> }.into_any()
                                                                    }}
                                                                </div>
                                                                {if !level.inherits_from.is_empty() {
                                                                    let chips = level.inherits_from.clone();
                                                                    view! {
                                                                        <div class="flex gap-1 mt-1 flex-wrap">
                                                                            <span class="text-xs text-base-content/65">"inherits:"</span>
                                                                            {chips.into_iter().map(|p| view! {
                                                                                <span class="badge badge-ghost badge-xs">{p}</span>
                                                                            }).collect::<Vec<_>>()}
                                                                        </div>
                                                                    }.into_any()
                                                                } else {
                                                                    view! { <span /> }.into_any()
                                                                }}
                                                            </div>
                                                            <div class="flex gap-2 shrink-0">
                                                                <button
                                                                    class="btn btn-ghost btn-xs"
                                                                    on:click=move |_| {
                                                                        if is_editing.get() {
                                                                            editing.set(None);
                                                                        } else {
                                                                            editing.set(Some(name_for_edit.clone()));
                                                                            edit_label.set(level.label.clone());
                                                                            edit_description.set(level.description.clone());
                                                                            edit_inherits.set(level.inherits_from.clone());
                                                                        }
                                                                    }
                                                                >
                                                                    {move || if is_editing.get() { "✕" } else { "Edit" }}
                                                                </button>
                                                                {if !level.is_system {
                                                                    let del_name = name_for_del.clone();
                                                                    view! {
                                                                        <button
                                                                            class="btn btn-ghost btn-xs text-error"
                                                                            on:click=move |_| {
                                                                                let n = del_name.clone();
                                                                                leptos::task::spawn_local(async move {
                                                                                    match with_auth_retry(|| delete_admin_access_level(n.clone())).await {
                                                                                        Ok(_) => error_msg.set(None),
                                                                                        Err(e) => error_msg.set(Some(e.to_string())),
                                                                                    }
                                                                                    set_refresh.update(|c| *c += 1);
                                                                                });
                                                                            }
                                                                        >"Delete"</button>
                                                                    }.into_any()
                                                                } else {
                                                                    view! { <span /> }.into_any()
                                                                }}
                                                            </div>
                                                        </div>

                                                        <Show when=move || is_editing.get()>
                                                            <div class="border-t border-base-200 p-4 bg-base-50 space-y-3">
                                                                <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
                                                                    <label class="form-control">
                                                                        <span class="label-text text-xs font-medium">"Label"</span>
                                                                        <input
                                                                            type="text"
                                                                            class="input input-sm input-bordered mt-1"
                                                                            prop:value=move || edit_label.get()
                                                                            on:input=move |e| edit_label.set(event_target_value(&e))
                                                                        />
                                                                    </label>
                                                                    <label class="form-control">
                                                                        <span class="label-text text-xs font-medium">"Description"</span>
                                                                        <input
                                                                            type="text"
                                                                            class="input input-sm input-bordered mt-1"
                                                                            prop:value=move || edit_description.get()
                                                                            on:input=move |e| edit_description.set(event_target_value(&e))
                                                                        />
                                                                    </label>
                                                                </div>
                                                                <div>
                                                                    <span class="label-text text-xs font-medium">"Inherits from"</span>
                                                                    <div class="flex gap-3 flex-wrap mt-1">
                                                                        {all_levels.iter().filter(|l| l.name != name).map(|l| {
                                                                            let lname = l.name.clone();
                                                                            let lname2 = lname.clone();
                                                                            view! {
                                                                                <label class="flex items-center gap-1 cursor-pointer">
                                                                                    <input
                                                                                        type="checkbox"
                                                                                        class="checkbox checkbox-xs"
                                                                                        prop:checked=move || edit_inherits.get().contains(&lname)
                                                                                        on:change=move |e| {
                                                                                            let checked = event_target_checked(&e);
                                                                                            edit_inherits.update(|v| {
                                                                                                if checked {
                                                                                                    if !v.contains(&lname2) { v.push(lname2.clone()); }
                                                                                                } else {
                                                                                                    v.retain(|x| x != &lname2);
                                                                                                }
                                                                                            });
                                                                                        }
                                                                                    />
                                                                                    <span class="text-xs font-mono">{l.name.clone()}</span>
                                                                                </label>
                                                                            }
                                                                        }).collect::<Vec<_>>()}
                                                                    </div>
                                                                </div>
                                                                <button
                                                                    class="btn btn-primary btn-sm"
                                                                    on:click=move |_| { save_edit_action.dispatch(()); }
                                                                >"Save"</button>
                                                            </div>
                                                        </Show>
                                                    </div>
                                                }
                                            }
                                        />
                                    </div>
                                }.into_any()
                            }
                        })}
                    </Suspense>
                </div>

                <div class="border-t border-base-200 p-8">
                    <Show
                        when=move || !show_create.get()
                        fallback=|| view! { <span /> }
                    >
                        <button
                            class="btn btn-primary btn-sm"
                            on:click=move |_| show_create.set(true)
                        >"+ New Access Level"</button>
                    </Show>
                    <Show when=move || show_create.get()>
                        <div class="space-y-3">
                            <h3 class="font-semibold text-sm">"Create Access Level"</h3>
                            <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
                                <label class="form-control">
                                    <span class="label-text text-xs font-medium">"Name (slug)"</span>
                                    <input
                                        type="text"
                                        class="input input-sm input-bordered mt-1"
                                        placeholder="e.g. cloud-developer"
                                        prop:value=move || new_name.get()
                                        on:input=move |e| new_name.set(event_target_value(&e))
                                    />
                                </label>
                                <label class="form-control">
                                    <span class="label-text text-xs font-medium">"Label"</span>
                                    <input
                                        type="text"
                                        class="input input-sm input-bordered mt-1"
                                        prop:value=move || new_label.get()
                                        on:input=move |e| new_label.set(event_target_value(&e))
                                    />
                                </label>
                                <label class="form-control">
                                    <span class="label-text text-xs font-medium">"Description"</span>
                                    <input
                                        type="text"
                                        class="input input-sm input-bordered mt-1"
                                        prop:value=move || new_description.get()
                                        on:input=move |e| new_description.set(event_target_value(&e))
                                    />
                                </label>
                            </div>
                            <div>
                                <span class="label-text text-xs font-medium">"Inherits from (comma-separated names)"</span>
                                <input
                                    type="text"
                                    class="input input-sm input-bordered mt-1 w-full"
                                    placeholder="e.g. internal,developer"
                                    on:input=move |e| {
                                        let val = event_target_value(&e);
                                        new_inherits.set(
                                            val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
                                        );
                                    }
                                />
                            </div>
                            <div class="flex gap-2">
                                <button class="btn btn-primary btn-sm" on:click=move |_| { create_action.dispatch(()); }>"Create"</button>
                                <button class="btn btn-ghost btn-sm" on:click=move |_| show_create.set(false)>"Cancel"</button>
                            </div>
                        </div>
                    </Show>
                </div>
            </div>
        </div>
    }
}
