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

#[component]
pub fn DocumentationFeedbackAdminPanel() -> impl IntoView {
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
                                placeholder="Search title, summary, lekton://docs/ URI, or proposal"
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

#[allow(unused_variables)]
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
                <p class="text-sm text-base-content/65 max-w-xl mt-1">
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
    let (resolution_note, set_resolution_note) =
        signal(item.resolution_note.clone().unwrap_or_default());
    let (duplicate_of, set_duplicate_of) = signal(item.duplicate_of.clone().unwrap_or_default());
    let (error, set_error) = signal(Option::<String>::None);

    let resolve_action = Action::new_local(move |_: &()| {
        let id = item_id_for_resolve.clone();
        let note = resolution_note.get_untracked();
        async move {
            set_error.set(None);
            match with_auth_retry(|| {
                resolve_documentation_feedback(
                    id.clone(),
                    (!note.trim().is_empty()).then_some(note.clone()),
                )
            })
            .await
            {
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
            match with_auth_retry(|| {
                mark_documentation_feedback_duplicate(
                    id.clone(),
                    duplicate_of_value.clone(),
                    (!note.trim().is_empty()).then_some(note.clone()),
                )
            })
            .await
            {
                Ok(()) => trigger_refresh(),
                Err(err) => set_error.set(Some(err.to_string())),
            }
        }
    });

    let detail_sections = {
        let mut sections = Vec::new();

        if let Some(view) =
            documentation_feedback_detail_view("Related resources", item.related_resources.clone())
        {
            sections.push(view);
        }
        if let Some(view) =
            documentation_feedback_detail_view("Search queries", item.search_queries.clone())
        {
            sections.push(view);
        }
        if let Some(view) =
            documentation_feedback_optional_view("User goal", item.user_goal.clone())
        {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view(
            "Missing information",
            item.missing_information.clone(),
        ) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Impact", item.impact.clone()) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view(
            "Suggested target resource",
            item.suggested_target_resource.clone(),
        ) {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view(
            "Target resource",
            item.target_resource_uri.clone(),
        ) {
            sections.push(view);
        }
        if let Some(view) =
            documentation_feedback_optional_view("Problem summary", item.problem_summary.clone())
        {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_optional_view("Proposal", item.proposal.clone())
        {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_detail_view(
            "Supporting resources",
            item.supporting_resources.clone(),
        ) {
            sections.push(view);
        }
        if let Some(view) =
            documentation_feedback_optional_view("Expected benefit", item.expected_benefit.clone())
        {
            sections.push(view);
        }
        if let Some(view) = documentation_feedback_detail_view(
            "Related feedback ids",
            item.related_feedback_ids.clone(),
        ) {
            sections.push(view);
        }
        if let Some(view) =
            documentation_feedback_optional_view("Duplicate of", item.duplicate_of.clone())
        {
            sections.push(view);
        }
        if let Some(view) =
            documentation_feedback_optional_view("Resolution note", item.resolution_note.clone())
        {
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
                            <span class="text-xs text-base-content/65 font-mono break-all">{item.id.clone()}</span>
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
            <div class="text-xs font-bold uppercase tracking-wider text-base-content/65">{title}</div>
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

fn documentation_feedback_optional_view(
    title: &'static str,
    value: Option<String>,
) -> Option<AnyView> {
    let value = value.filter(|value| !value.trim().is_empty())?;

    Some(view! {
        <div class="space-y-2">
            <div class="text-xs font-bold uppercase tracking-wider text-base-content/65">{title}</div>
            <div class="rounded-xl bg-base-200/40 px-4 py-3 whitespace-pre-wrap break-words">{value}</div>
        </div>
    }.into_any())
}
