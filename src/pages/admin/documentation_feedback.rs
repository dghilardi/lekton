use leptos::prelude::*;

#[allow(unused_imports)]
use crate::app::{
    admin_list_pats, admin_toggle_pat, create_admin_access_level, create_service_token,
    deactivate_service_token, delete_admin_access_level, get_custom_css, get_navigation,
    get_navigation_order, get_rag_reindex_status, get_schema_endpoint_reindex_status,
    get_search_reindex_status, list_admin_access_levels, list_admin_users,
    list_documentation_feedback, list_documentation_feedback_choices, list_service_tokens,
    mark_documentation_feedback_duplicate, resolve_documentation_feedback, save_custom_css,
    save_navigation_order, set_admin_user_access_levels, trigger_rag_reindex,
    trigger_schema_endpoint_reindex, trigger_search_reindex, update_admin_access_level,
    AccessLevelInfo, CreateTokenResult, DocumentationFeedbackAdminItem,
    DocumentationFeedbackAdminListResult, DocumentationFeedbackChoice, NavItem,
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

    // Candidates for "Duplicate of", and titles for the ids shown in details.
    let choices_resource = LocalResource::new(move || {
        let _ = refresh_counter.get();
        async move { with_auth_retry(list_documentation_feedback_choices).await }
    });
    let choices = Signal::derive(move || {
        choices_resource
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
    });
    // Only one item is expanded at a time, so the page stays a short list.
    let expanded = RwSignal::new(None::<String>);

    let trigger_refresh = move || set_refresh_counter.update(|value| *value += 1);

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200 overflow-hidden">
            <div class="card-body p-0">
                <div class="p-8 pb-6">
                    <div class="grid grid-cols-1 gap-4 md:grid-cols-4 md:items-end">
                        <label class="form-control md:col-span-2">
                            <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60 mb-2">"Search"</span>
                            <input
                                type="text"
                                class="input input-bordered h-12 w-full focus:input-primary"
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
                                class="select select-bordered h-12 w-full focus:select-primary"
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
                                class="select select-bordered h-12 w-full focus:select-primary"
                                prop:value=move || status_filter.get()
                                on:change=move |ev| {
                                    set_page.set(0);
                                    set_status_filter.set(event_target_value(&ev));
                                }
                            >
                                <option value="open">"Open"</option>
                                <option value="in_progress">"In progress"</option>
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
                                    expanded=expanded
                                    choices=choices
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

/// Readable label for a feedback kind (`missing_info` → "Missing information").
pub fn feedback_kind_label(kind: &str) -> String {
    match kind {
        "missing_info" => "Missing information".to_string(),
        "improvement" => "Improvement".to_string(),
        other => other.replace('_', " "),
    }
}

/// Readable label for a feedback status (`in_progress` → "In progress").
pub fn feedback_status_label(status: &str) -> String {
    match status {
        "open" => "Open".to_string(),
        "in_progress" => "In progress".to_string(),
        "resolved" => "Resolved".to_string(),
        other => other.replace('_', " "),
    }
}

/// In-app link for a `lekton://docs/<slug>` resource URI, if it is one.
pub fn docs_uri_href(uri: &str) -> Option<String> {
    let slug = uri.trim().strip_prefix("lekton://docs/")?;
    (!slug.is_empty()).then(|| format!("/docs/{slug}"))
}

#[allow(unused_variables)]
#[component]
fn DocumentationFeedbackList(
    result: DocumentationFeedbackAdminListResult,
    page: ReadSignal<u64>,
    set_page: WriteSignal<u64>,
    trigger_refresh: impl Fn() + Copy + Send + Sync + 'static,
    expanded: RwSignal<Option<String>>,
    choices: Signal<Vec<DocumentationFeedbackChoice>>,
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
        <div class="space-y-4">
            <div class="flex flex-wrap items-center justify-between gap-2 text-sm text-base-content/60">
                <span>{format!("{} total item(s)", result.total)}</span>
                <span>{format!("Page {} of {}", result.page + 1, total_pages)}</span>
            </div>

            <ul class="divide-y divide-base-200 overflow-hidden rounded-xl border border-base-200">
                {result
                    .items
                    .into_iter()
                    .map(|item| view! {
                        <DocumentationFeedbackRow
                            item=item
                            expanded=expanded
                            choices=choices
                            trigger_refresh=trigger_refresh
                        />
                    })
                    .collect::<Vec<_>>()}
            </ul>

            <div class="flex items-center justify-end gap-3 pt-2">
                <button
                    class="btn btn-outline btn-sm"
                    disabled=!has_prev
                    on:click=move |_| {
                        expanded.set(None);
                        set_page.update(|value| {
                            if *value > 0 {
                                *value -= 1;
                            }
                        })
                    }
                >
                    "Previous"
                </button>
                <button
                    class="btn btn-outline btn-sm"
                    disabled=!has_next
                    on:click=move |_| {
                        expanded.set(None);
                        set_page.update(|value| *value += 1)
                    }
                >
                    "Next"
                </button>
            </div>
        </div>
    }.into_any()
}

/// One feedback item: a one-line summary that expands into the details and
/// the resolve / duplicate form.
#[component]
fn DocumentationFeedbackRow(
    item: DocumentationFeedbackAdminItem,
    expanded: RwSignal<Option<String>>,
    choices: Signal<Vec<DocumentationFeedbackChoice>>,
    trigger_refresh: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let id = item.id.clone();
    let panel_id = format!("feedback-{}", item.id);
    let is_open = Memo::new({
        let id = id.clone();
        move |_| expanded.get().as_deref() == Some(id.as_str())
    });
    let status_badge_class = match item.status.as_str() {
        "open" => "badge badge-sm badge-outline badge-primary",
        "in_progress" => "badge badge-sm badge-outline badge-info",
        _ => "badge badge-sm badge-outline badge-ghost",
    };
    let kind_badge_class = if item.kind == "missing_info" {
        "badge badge-sm badge-soft badge-warning"
    } else {
        "badge badge-sm badge-soft badge-info"
    };
    let header = view! {
        <button
            class="flex w-full items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-base-200/50"
            aria-expanded=move || if is_open.get() { "true" } else { "false" }
            aria-controls=panel_id.clone()
            on:click=move |_| {
                expanded.update(|current| {
                    *current = if current.as_deref() == Some(id.as_str()) { None } else { Some(id.clone()) };
                })
            }
        >
            <svg
                class=move || format!("mt-1 h-4 w-4 shrink-0 opacity-60 transition-transform {}", if is_open.get() { "rotate-90" } else { "" })
                viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
            ><path d="m9 18 6-6-6-6"/></svg>
            <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-center gap-2">
                    <span class=kind_badge_class>{feedback_kind_label(&item.kind)}</span>
                    <span class=status_badge_class>{feedback_status_label(&item.status)}</span>
                    <span class="font-semibold">{item.title.clone()}</span>
                </div>
                <p class="mt-0.5 text-sm text-base-content/65 line-clamp-1 break-all sm:break-normal">{item.summary.clone()}</p>
            </div>
            <div class="hidden shrink-0 text-right text-xs leading-relaxed text-base-content/60 sm:block">
                <div class="max-w-[14rem] truncate">{item.created_by.clone()}</div>
                <div class="whitespace-nowrap">{item.created_at.clone()}</div>
            </div>
        </button>
    };
    let panel_id = format!("feedback-{}", item.id);
    let item = StoredValue::new(item);

    view! {
        <li>
            {header}
            <Show when=move || is_open.get() fallback=|| ()>
                <div id=panel_id.clone() class="border-t border-base-200 bg-base-200/20 px-4 py-5 sm:px-11">
                    <DocumentationFeedbackDetail item=item.get_value() choices=choices trigger_refresh=trigger_refresh />
                </div>
            </Show>
        </li>
    }
}

#[component]
fn DocumentationFeedbackDetail(
    item: DocumentationFeedbackAdminItem,
    choices: Signal<Vec<DocumentationFeedbackChoice>>,
    trigger_refresh: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    // Both open and in-progress items can be resolved/duplicated by an admin.
    let status_is_actionable = item.status != "resolved";
    // Delivery box shown when the item has been claimed (agent took it in charge).
    let delivery_box = item.delivery_source_id.clone().map(|source_id| {
        let claimed_at = item.claimed_at.clone();
        let delivery_ref = item.delivery_ref.clone();
        view! {
            <div class="rounded-xl border border-info/30 bg-info/10 px-4 py-3 text-sm">
                <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-base-content/70">
                    <span class="inline-flex items-center gap-1.5">
                        <span class="font-semibold text-base-content/80">"Delivering to"</span>
                        <span class="font-mono">{source_id}</span>
                    </span>
                    {delivery_ref.map(|r| {
                        let href = r.clone();
                        view! {
                            <a class="link link-primary inline-flex items-center gap-1" href=href target="_blank" rel="noopener">
                                <svg class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/></svg>
                                "delivery"
                            </a>
                        }
                    })}
                    {claimed_at.map(|c| view! { <span>"claimed "{c}</span> })}
                </div>
            </div>
        }
    });
    let item_id = StoredValue::new(item.id.clone());
    let item_id_for_resolve = item.id.clone();
    let item_id_for_duplicate = item.id.clone();
    let (resolution_note, set_resolution_note) =
        signal(item.resolution_note.clone().unwrap_or_default());
    let (duplicate_of, set_duplicate_of) = signal(item.duplicate_of.clone().unwrap_or_default());
    let (error, set_error) = signal(Option::<String>::None);

    let resolve_action = crate::components::browser_action(move |_: &()| {
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

    let duplicate_action = crate::components::browser_action(move |_: &()| {
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

    let sections: Vec<AnyView> = [
        resource_list_view("Related resources", item.related_resources.clone()),
        text_list_view("Search queries", item.search_queries.clone()),
        text_view("User goal", item.user_goal.clone()),
        text_view("Missing information", item.missing_information.clone()),
        text_view("Impact", item.impact.clone()),
        resource_view(
            "Suggested target resource",
            item.suggested_target_resource.clone(),
        ),
        resource_view("Target resource", item.target_resource_uri.clone()),
        text_view("Problem summary", item.problem_summary.clone()),
        text_view("Proposal", item.proposal.clone()),
        resource_list_view("Supporting resources", item.supporting_resources.clone()),
        text_view("Expected benefit", item.expected_benefit.clone()),
        feedback_refs_view(
            "Related feedback",
            item.related_feedback_ids.clone(),
            choices,
        ),
        feedback_refs_view(
            "Duplicate of",
            item.duplicate_of.clone().into_iter().collect(),
            choices,
        ),
        text_view("Resolution note", item.resolution_note.clone()),
    ]
    .into_iter()
    .flatten()
    .collect();

    view! {
        <div class="space-y-5">
            <div class="space-y-1">
                <p class="text-sm whitespace-pre-wrap break-words">{item.summary.clone()}</p>
                <p class="text-xs text-base-content/55">
                    <span class="sm:hidden">{format!("{} · {} · ", item.created_by, item.created_at)}</span>
                    <span class="font-mono break-all">{item.id.clone()}</span>
                </p>
            </div>

            <Show when=move || error.get().is_some()>
                <div class="alert alert-error shadow-sm border-none bg-error/10 text-error text-sm">
                    <span>{move || error.get().unwrap_or_default()}</span>
                </div>
            </Show>

            {delivery_box}

            <div class="grid grid-cols-1 gap-5 text-sm lg:grid-cols-2">
                {sections}
            </div>

            <Show when=move || status_is_actionable>
                <div class="border-t border-base-200 pt-5">
                    <div class="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]">
                        <label class="form-control">
                            <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60 mb-2">"Resolution note"</span>
                            <textarea
                                class="textarea textarea-bordered min-h-24 w-full focus:textarea-primary"
                                placeholder="Optional note describing how the item was resolved or why it was marked duplicate."
                                prop:value=move || resolution_note.get()
                                on:input=move |ev| set_resolution_note.set(event_target_value(&ev))
                            ></textarea>
                        </label>

                        <div class="flex h-full flex-col gap-3">
                            <label class="form-control flex-1">
                                <span class="label-text font-bold text-xs uppercase tracking-wider text-base-content/60 mb-2">"Duplicate of"</span>
                                <select
                                    class="select select-bordered w-full focus:select-primary"
                                    prop:value=move || duplicate_of.get()
                                    on:change=move |ev| set_duplicate_of.set(event_target_value(&ev))
                                >
                                    <option value="">"Choose the original item…"</option>
                                    {move || {
                                        let current = item_id.get_value();
                                        choices
                                            .get()
                                            .into_iter()
                                            .filter(|choice| choice.id != current)
                                            .map(|choice| {
                                                let label = format!(
                                                    "{} — {} · {}",
                                                    choice.title,
                                                    feedback_status_label(&choice.status),
                                                    choice.created_at
                                                );
                                                view! { <option value=choice.id>{label}</option> }
                                            })
                                            .collect::<Vec<_>>()
                                    }}
                                </select>
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
    }
}

fn section_title(title: &'static str) -> impl IntoView {
    view! { <div class="text-xs font-bold uppercase tracking-wider text-base-content/65">{title}</div> }
}

/// A resource reference: `lekton://docs/…` URIs become links into the portal.
fn resource_link(value: String) -> AnyView {
    match docs_uri_href(&value) {
        Some(href) => view! {
            <a class="link link-primary font-mono text-xs break-all" href=href>{value}</a>
        }
        .into_any(),
        None => view! { <span class="font-mono text-xs break-all">{value}</span> }.into_any(),
    }
}

fn resource_list_view(title: &'static str, values: Vec<String>) -> Option<AnyView> {
    if values.is_empty() {
        return None;
    }
    Some(view! {
        <div class="space-y-2">
            {section_title(title)}
            <ul class="space-y-1">
                {values.into_iter().map(|value| view! { <li>{resource_link(value)}</li> }).collect::<Vec<_>>()}
            </ul>
        </div>
    }.into_any())
}

fn resource_view(title: &'static str, value: Option<String>) -> Option<AnyView> {
    let value = value.filter(|value| !value.trim().is_empty())?;
    Some(
        view! {
            <div class="space-y-2">
                {section_title(title)}
                <div>{resource_link(value)}</div>
            </div>
        }
        .into_any(),
    )
}

fn text_list_view(title: &'static str, values: Vec<String>) -> Option<AnyView> {
    if values.is_empty() {
        return None;
    }
    Some(view! {
        <div class="space-y-2">
            {section_title(title)}
            <div class="space-y-2">
                {values.into_iter().map(|value| view! {
                    <div class="rounded-xl border border-base-300/70 bg-base-100 px-3 py-2 font-mono text-xs leading-relaxed whitespace-pre-wrap break-words overflow-hidden">
                        {value}
                    </div>
                }).collect::<Vec<_>>()}
            </div>
        </div>
    }.into_any())
}

fn text_view(title: &'static str, value: Option<String>) -> Option<AnyView> {
    let value = value.filter(|value| !value.trim().is_empty())?;
    Some(view! {
        <div class="space-y-2">
            {section_title(title)}
            <div class="rounded-xl bg-base-100 px-4 py-3 whitespace-pre-wrap break-words">{value}</div>
        </div>
    }.into_any())
}

/// Other feedback items referenced by id, shown by title when known.
fn feedback_refs_view(
    title: &'static str,
    ids: Vec<String>,
    choices: Signal<Vec<DocumentationFeedbackChoice>>,
) -> Option<AnyView> {
    if ids.is_empty() {
        return None;
    }
    Some(view! {
        <div class="space-y-2">
            {section_title(title)}
            <ul class="space-y-1">
                {ids.into_iter().map(|id| {
                    let label = {
                        let id = id.clone();
                        move || {
                            choices
                                .get()
                                .into_iter()
                                .find(|choice| choice.id == id)
                                .map(|choice| format!("{} ({})", choice.title, feedback_status_label(&choice.status)))
                        }
                    };
                    view! {
                        <li>
                            {move || label().map(|l| view! { <span>{l}</span> })}
                            <span class="ml-1 font-mono text-xs text-base-content/55 break-all">{id.clone()}</span>
                        </li>
                    }
                }).collect::<Vec<_>>()}
            </ul>
        </div>
    }.into_any())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_uris_link_into_the_portal() {
        assert_eq!(
            docs_uri_href("lekton://docs/docs/guides/getting-started").as_deref(),
            Some("/docs/docs/guides/getting-started")
        );
        assert_eq!(docs_uri_href("lekton://docs/").as_deref(), None);
        assert_eq!(docs_uri_href("https://example.test/page"), None);
    }

    #[test]
    fn kinds_and_statuses_have_readable_labels() {
        assert_eq!(feedback_kind_label("missing_info"), "Missing information");
        assert_eq!(feedback_kind_label("improvement"), "Improvement");
        assert_eq!(feedback_status_label("in_progress"), "In progress");
        assert_eq!(feedback_kind_label("new_kind"), "new kind");
    }
}
