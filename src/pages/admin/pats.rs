use leptos::prelude::*;

#[allow(unused_imports)]
use crate::app::{
    admin_list_pats, admin_toggle_pat, create_admin_access_level, create_machine_pat,
    create_service_token, deactivate_service_token, delete_admin_access_level, get_custom_css,
    get_navigation, get_navigation_order, get_rag_reindex_status,
    get_schema_endpoint_reindex_status, get_search_reindex_status, list_admin_access_levels,
    list_admin_users, list_documentation_feedback, list_service_tokens,
    mark_documentation_feedback_duplicate, resolve_documentation_feedback, save_custom_css,
    save_navigation_order, set_admin_user_access_levels, trigger_rag_reindex,
    trigger_schema_endpoint_reindex, trigger_search_reindex, update_admin_access_level,
    AccessLevelInfo, CreatePatResult, CreateTokenResult, DocumentationFeedbackAdminItem,
    DocumentationFeedbackAdminListResult, NavItem, NavigationOrderEntry, ServiceTokenInfo,
};
#[allow(unused_imports)]
use crate::auth::refresh_client::with_auth_retry;

// ── Admin PAT Manager ─────────────────────────────────────────────────────────

const ADMIN_PAT_PER_PAGE: u64 = 20;

/// Admin section: paginated list of all PATs with user resolution and toggle.
#[component]
pub fn AdminPatManager() -> impl IntoView {
    let page = RwSignal::new(1u64);
    let total_pages = RwSignal::new(1u64);
    // A Memo (not a plain signal) so a runaway `page` write, of any origin,
    // cannot keep retriggering the resource forever: Memo only notifies
    // subscribers when the clamped value actually changes, so once `page`
    // reaches `total_pages` the resource stops refetching even if `page`
    // itself keeps moving underneath it.
    let requested_page = Memo::new(move |_| page.get().min(total_pages.get()));

    let pats_resource = LocalResource::new(move || {
        let requested_page = requested_page.get();
        with_auth_retry(move || admin_list_pats(requested_page, ADMIN_PAT_PER_PAGE))
    });

    Effect::new(move |_| {
        if let Some(Ok((_, total))) = pats_resource.get() {
            let computed = total.div_ceil(ADMIN_PAT_PER_PAGE).max(1);
            if total_pages.get_untracked() != computed {
                total_pages.set(computed);
            }
        }
    });

    let toggle_action = Action::new_local(move |(id, active): &(String, bool)| {
        let id = id.clone();
        let active = *active;
        async move {
            if with_auth_retry(|| admin_toggle_pat(id.clone(), active))
                .await
                .is_ok()
            {
                pats_resource.refetch();
            }
        }
    });

    // ── Machine-token creation (user-less PAT for the documentation agent) ──
    let machine_name = RwSignal::new(String::new());
    let created = RwSignal::new(Option::<CreatePatResult>::None);
    let create_error = RwSignal::new(Option::<String>::None);
    let create_action = Action::new_local(move |_: &()| async move {
        let name = machine_name.get_untracked();
        match with_auth_retry(|| create_machine_pat(name.clone())).await {
            Ok(result) => {
                created.set(Some(result));
                create_error.set(None);
                machine_name.set(String::new());
                pats_resource.refetch();
            }
            Err(e) => create_error.set(Some(e.to_string())),
        }
    });
    let creating = create_action.pending();

    view! {
        <div class="space-y-4">
            <div class="rounded-2xl border border-base-200 bg-base-100 p-5">
                <h3 class="text-sm font-bold text-base-content/80">"Machine token (MCP)"</h3>
                <p class="mt-1 text-xs leading-relaxed text-base-content/60">
                    "A token not tied to any user account, for machine-to-machine access to the MCP endpoint (e.g. the documentation agent). It survives the creator being removed. "
                    <span class="font-semibold text-warning">"It is full-admin"</span>
                    " — provision sparingly and store it in a secret manager."
                </p>
                {move || create_error.get().map(|e| view! {
                    <div role="alert" class="alert alert-error mt-3 text-sm"><span>{e}</span></div>
                })}
                {move || match created.get() {
                    Some(result) => {
                        #[cfg(feature = "hydrate")]
                        let raw_for_copy = result.raw_token.clone();
                        view! {
                            <div class="mt-3 rounded-xl border border-warning/30 bg-warning/10 p-4">
                                <p class="text-xs font-semibold text-base-content/80">{format!("Token \"{}\" created — copy it now, it is shown only once.", result.name)}</p>
                                <div class="mt-2 flex items-center gap-2">
                                    <input type="text" readonly class="input input-bordered input-sm w-full font-mono text-xs" prop:value=result.raw_token.clone() />
                                    <button class="btn btn-sm btn-ghost"
                                        on:click=move |_| {
                                            #[cfg(feature = "hydrate")]
                                            {
                                                let _ = js_sys::eval(&format!("navigator.clipboard.writeText('{}')", raw_for_copy.replace('\'', "\\'")));
                                            }
                                        }>"Copy"</button>
                                    <button class="btn btn-sm btn-ghost" on:click=move |_| created.set(None)>"Done"</button>
                                </div>
                            </div>
                        }.into_any()
                    }
                    None => view! {
                        <div class="mt-3 flex items-center gap-2">
                            <input type="text" class="input input-bordered input-sm w-full max-w-xs" placeholder="Token name, e.g. docs-agent"
                                prop:value=move || machine_name.get()
                                on:input=move |e| machine_name.set(event_target_value(&e)) />
                            <button class="btn btn-primary btn-sm gap-2"
                                prop:disabled=move || creating.get() || machine_name.get().trim().is_empty()
                                on:click=move |_| { create_action.dispatch(()); }>
                                {move || creating.get().then(|| view! { <span class="loading loading-spinner loading-xs"></span> })}
                                "Create"
                            </button>
                        </div>
                    }.into_any(),
                }}
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
                                                    <td colspan="6" class="text-center py-8 text-base-content/65">"No PATs found."</td>
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
                                                disabled=move || total_pages <= page.get()
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
