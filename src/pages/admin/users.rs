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

// ── User Manager ──────────────────────────────────────────────────────────────

#[component]
pub fn UserManager() -> impl IntoView {
    let (refresh, set_refresh) = signal(0u32);

    let users_resource = LocalResource::new(move || {
        let _ = refresh.get();
        with_auth_retry(list_admin_users)
    });

    let levels_resource = LocalResource::new(move || with_auth_retry(list_admin_access_levels));

    let editing = RwSignal::new(Option::<String>::None);
    let edit_assigned = RwSignal::new(Vec::<String>::new());
    let edit_can_write = RwSignal::new(false);
    let edit_can_read_draft = RwSignal::new(false);
    let edit_can_write_draft = RwSignal::new(false);
    let error_msg = RwSignal::new(Option::<String>::None);

    let save_action = Action::new_local(move |_: &()| async move {
        let Some(user_id) = editing.get_untracked() else {
            return;
        };
        let result = with_auth_retry(|| {
            set_admin_user_access_levels(
                user_id.clone(),
                edit_assigned.get_untracked(),
                edit_can_write.get_untracked(),
                edit_can_read_draft.get_untracked(),
                edit_can_write_draft.get_untracked(),
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

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200 overflow-hidden">
            <div class="card-body p-0">
                <div class="p-8 pb-4">
                    <h2 class="card-title text-2xl mb-1">"Users"</h2>
                    <p class="text-base-content/60 text-sm">
                        "Assign access levels and write permissions to registered users."
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
                        {move || {
                            let users = users_resource.get()?;
                            let levels = levels_resource.get()?.unwrap_or_default();
                            let users = users.ok()?;
                            Some(view! {
                                <div class="space-y-2">
                                    <For
                                        each=move || users.clone()
                                        key=|u| u.id.clone()
                                        children=move |user| {
                                            let uid = user.id.clone();
                                            let uid_for_memo = uid.clone();
                                            let is_editing = Memo::new(move |_| {
                                                editing.get().as_deref() == Some(uid_for_memo.as_str())
                                            });
                                            let all_levels = levels.clone();

                                            view! {
                                                <div class="border border-base-200 rounded-xl overflow-hidden">
                                                    <div class="flex items-start gap-3 p-4">
                                                        <div class="flex-1 min-w-0">
                                                            <div class="flex items-center gap-2 flex-wrap">
                                                                <span class="font-medium text-sm">{user.email.clone()}</span>
                                                                {if user.is_admin {
                                                                    view! { <span class="badge badge-primary badge-sm">"admin"</span> }.into_any()
                                                                } else { view! { <span /> }.into_any() }}
                                                            </div>
                                                            <div class="flex gap-1 mt-1 flex-wrap">
                                                                {if user.assigned_access_levels.is_empty() {
                                                                    view! { <span class="text-xs text-base-content/40 italic">"no levels assigned"</span> }.into_any()
                                                                } else {
                                                                    view! {
                                                                        <>{user.assigned_access_levels.iter().map(|l| view! {
                                                                            <span class="badge badge-outline badge-sm">{l.clone()}</span>
                                                                        }).collect::<Vec<_>>()}</>
                                                                    }.into_any()
                                                                }}
                                                            </div>
                                                            {if !user.effective_access_levels.is_empty() && user.effective_access_levels != user.assigned_access_levels {
                                                                let eff = user.effective_access_levels.clone();
                                                                view! {
                                                                    <div class="text-xs text-base-content/50 mt-1">
                                                                        "effective: "
                                                                        {eff.join(", ")}
                                                                    </div>
                                                                }.into_any()
                                                            } else { view! { <span /> }.into_any() }}
                                                        </div>
                                                        <div class="flex gap-2 shrink-0">
                                                            <button
                                                                class="btn btn-ghost btn-xs"
                                                                on:click=move |_| {
                                                                    if is_editing.get() {
                                                                        editing.set(None);
                                                                    } else {
                                                                        editing.set(Some(uid.clone()));
                                                                        edit_assigned.set(user.assigned_access_levels.clone());
                                                                        edit_can_write.set(user.can_write);
                                                                        edit_can_read_draft.set(user.can_read_draft);
                                                                        edit_can_write_draft.set(user.can_write_draft);
                                                                    }
                                                                }
                                                            >
                                                                {move || if is_editing.get() { "✕" } else { "Edit" }}
                                                            </button>
                                                        </div>
                                                    </div>

                                                    <Show when=move || is_editing.get()>
                                                        <div class="border-t border-base-200 p-4 bg-base-50 space-y-3">
                                                            <div>
                                                                <span class="label-text text-xs font-medium">"Assigned access levels"</span>
                                                                <div class="flex gap-3 flex-wrap mt-2">
                                                                    {all_levels.iter().filter(|l| !l.is_system).map(|l| {
                                                                        let lname = l.name.clone();
                                                                        let lname2 = lname.clone();
                                                                        view! {
                                                                            <label class="flex items-center gap-1 cursor-pointer">
                                                                                <input
                                                                                    type="checkbox"
                                                                                    class="checkbox checkbox-xs"
                                                                                    prop:checked=move || edit_assigned.get().contains(&lname)
                                                                                    on:change=move |e| {
                                                                                        let checked = event_target_checked(&e);
                                                                                        edit_assigned.update(|v| {
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
                                                            <div class="flex gap-4">
                                                                <label class="flex items-center gap-2 cursor-pointer">
                                                                    <input type="checkbox" class="checkbox checkbox-xs"
                                                                        prop:checked=move || edit_can_write.get()
                                                                        on:change=move |e| edit_can_write.set(event_target_checked(&e))
                                                                    />
                                                                    <span class="text-xs">"Can write"</span>
                                                                </label>
                                                                <label class="flex items-center gap-2 cursor-pointer">
                                                                    <input type="checkbox" class="checkbox checkbox-xs"
                                                                        prop:checked=move || edit_can_read_draft.get()
                                                                        on:change=move |e| edit_can_read_draft.set(event_target_checked(&e))
                                                                    />
                                                                    <span class="text-xs">"Read drafts"</span>
                                                                </label>
                                                                <label class="flex items-center gap-2 cursor-pointer">
                                                                    <input type="checkbox" class="checkbox checkbox-xs"
                                                                        prop:checked=move || edit_can_write_draft.get()
                                                                        on:change=move |e| edit_can_write_draft.set(event_target_checked(&e))
                                                                    />
                                                                    <span class="text-xs">"Write drafts"</span>
                                                                </label>
                                                            </div>
                                                            <button
                                                                class="btn btn-primary btn-sm"
                                                                on:click=move |_| { save_action.dispatch(()); }
                                                            >"Save"</button>
                                                        </div>
                                                    </Show>
                                                </div>
                                            }
                                        }
                                    />
                                </div>
                            })
                        }}
                    </Suspense>
                </div>
            </div>
        </div>
    }
}
