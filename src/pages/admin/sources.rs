use leptos::prelude::*;

#[allow(unused_imports)]
use crate::app::{
    delete_source, list_admin_users, list_sources, save_source, AdminUserInfo, MaintainerDto,
    SourceInfo,
};
#[allow(unused_imports)]
use crate::auth::refresh_client::with_auth_retry;

/// One editable maintainer row. Each field is its own signal so typing in a row
/// never re-renders (and unfocuses) its siblings; the row list only changes on
/// add/remove.
#[derive(Clone, Copy)]
struct MaintainerRow {
    key: usize,
    name: RwSignal<String>,
    email: RwSignal<String>,
    user_id: RwSignal<String>,
}

/// Resolve a Lekton user id to a "Name <email>" label for display.
fn user_label(users: &[AdminUserInfo], id: &str) -> String {
    users
        .iter()
        .find(|u| u.id == id)
        .map(|u| match &u.name {
            Some(n) if !n.is_empty() => format!("{n} <{}>", u.email),
            _ => u.email.clone(),
        })
        .unwrap_or_else(|| format!("(unknown user {id})"))
}

/// Human label for a maintainer plus whether it is linked to a Lekton user.
fn maintainer_display(m: &MaintainerDto, users: &[AdminUserInfo]) -> (String, bool) {
    if let Some(uid) = m.lekton_user_id.as_deref().filter(|s| !s.is_empty()) {
        (user_label(users, uid), true)
    } else {
        let label = match (&m.name, &m.email) {
            (Some(n), Some(e)) if !n.is_empty() && !e.is_empty() => format!("{n} <{e}>"),
            (Some(n), _) if !n.is_empty() => n.clone(),
            (_, Some(e)) if !e.is_empty() => e.clone(),
            _ => "?".to_string(),
        };
        (label, false)
    }
}

/// One or two initials for an avatar, derived from a display label.
fn initials(label: &str) -> String {
    let words: Vec<&str> = label
        .split(|c: char| c.is_whitespace() || c == '<' || c == '@')
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .collect();
    let first = |w: &str| {
        w.chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
    };
    match words.as_slice() {
        [] => "?".to_string(),
        [one] => one
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_uppercase(),
        [a, b, ..] => [first(a), first(b)].into_iter().flatten().collect(),
    }
}

#[component]
pub fn SourcesAdminPanel() -> impl IntoView {
    let (refresh, set_refresh) = signal(0u32);

    let sources_resource = LocalResource::new(move || {
        let _ = refresh.get();
        with_auth_retry(list_sources)
    });
    let users_resource = LocalResource::new(move || with_auth_retry(list_admin_users));

    let editing = RwSignal::new(Option::<String>::None);
    let edit_display_name = RwSignal::new(String::new());
    let edit_repo_url = RwSignal::new(String::new());
    let edit_branch = RwSignal::new(String::new());
    let edit_description = RwSignal::new(String::new());
    let edit_review_enabled = RwSignal::new(false);
    let edit_maintainers = RwSignal::new(Vec::<MaintainerRow>::new());
    let row_counter = RwSignal::new(0usize);
    let error_msg = RwSignal::new(Option::<String>::None);

    let add_row = move |m: Option<&MaintainerDto>| {
        let key = row_counter.get_untracked();
        row_counter.set(key + 1);
        let row = MaintainerRow {
            key,
            name: RwSignal::new(m.and_then(|m| m.name.clone()).unwrap_or_default()),
            email: RwSignal::new(m.and_then(|m| m.email.clone()).unwrap_or_default()),
            user_id: RwSignal::new(m.and_then(|m| m.lekton_user_id.clone()).unwrap_or_default()),
        };
        edit_maintainers.update(|v| v.push(row));
    };

    let start_edit = move |s: &SourceInfo| {
        editing.set(Some(s.id.clone()));
        edit_display_name.set(s.display_name.clone().unwrap_or_default());
        edit_repo_url.set(s.repo_url.clone().unwrap_or_default());
        edit_branch.set(s.mainline_branch.clone().unwrap_or_default());
        edit_description.set(s.description.clone().unwrap_or_default());
        edit_review_enabled.set(s.review_enabled);
        edit_maintainers.set(Vec::new());
        for m in &s.maintainers {
            add_row(Some(m));
        }
        error_msg.set(None);
    };

    let save_action = Action::new_local(move |_: &()| async move {
        let Some(id) = editing.get_untracked() else {
            return;
        };
        let maintainers = edit_maintainers
            .get_untracked()
            .iter()
            .map(|r| MaintainerDto {
                name: Some(r.name.get_untracked()),
                email: Some(r.email.get_untracked()),
                lekton_user_id: Some(r.user_id.get_untracked()),
            })
            .collect::<Vec<_>>();
        let result = with_auth_retry(|| {
            save_source(
                id.clone(),
                Some(edit_display_name.get_untracked()),
                Some(edit_repo_url.get_untracked()),
                Some(edit_branch.get_untracked()),
                Some(edit_description.get_untracked()),
                maintainers.clone(),
                edit_review_enabled.get_untracked(),
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

    let delete_action = Action::new_local(move |_: &()| async move {
        let Some(id) = editing.get_untracked() else {
            return;
        };
        let result = with_auth_retry(|| delete_source(id.clone())).await;
        match result {
            Ok(_) => {
                editing.set(None);
                error_msg.set(None);
                set_refresh.update(|c| *c += 1);
            }
            Err(e) => error_msg.set(Some(e.to_string())),
        }
    });

    let saving = save_action.pending();
    let deleting = delete_action.pending();

    view! {
        <div class="space-y-4">
            {move || error_msg.get().map(|e| view! {
                <div role="alert" class="alert alert-error text-sm shadow-sm">
                    <svg class="w-5 h-5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01M12 3a9 9 0 100 18 9 9 0 000-18z"/></svg>
                    <span class="flex-1">{e}</span>
                    <button class="btn btn-ghost btn-xs" on:click=move |_| error_msg.set(None)>"Dismiss"</button>
                </div>
            })}

            <Suspense fallback=move || view! {
                <div class="space-y-3" aria-busy="true">
                    {(0..3).map(|_| view! { <div class="h-[4.75rem] w-full rounded-2xl skeleton"></div> }).collect::<Vec<_>>()}
                </div>
            }>
                {move || {
                    let sources = sources_resource.get()?;
                    let users = users_resource.get()?.unwrap_or_default();
                    // StoredValue is Copy, so the users list can be shared across the
                    // nested source/maintainer closures without move conflicts.
                    let users = StoredValue::new(users);
                    let sources = sources.ok()?;
                    if sources.is_empty() {
                        return Some(view! {
                            <div class="flex flex-col items-center justify-center rounded-2xl border-2 border-dashed border-base-300 bg-base-200/20 px-6 py-16 text-center">
                                <div class="mb-4 grid h-16 w-16 place-items-center rounded-2xl bg-primary/10 text-primary">
                                    <svg class="h-8 w-8" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>
                                </div>
                                <h3 class="text-lg font-bold text-base-content/80">"No documentation sources yet"</h3>
                                <p class="mt-1.5 max-w-md text-sm leading-relaxed text-base-content/60">
                                    "Sources appear here once documents are ingested. Each one is identified by the "<code class="rounded bg-base-200 px-1 py-0.5 font-mono text-xs">"id"</code>" in its "<code class="rounded bg-base-200 px-1 py-0.5 font-mono text-xs">".lekton.yml"</code>". You can then attach a repository, branch, and maintainers."
                                </p>
                            </div>
                        }.into_any());
                    }
                    Some(view! {
                        <div class="space-y-3">
                            <For
                                each=move || sources.clone()
                                key=|s| s.id.clone()
                                children=move |source| {
                                    let sid = source.id.clone();
                                    let sid_for_memo = sid.clone();
                                    let is_editing = Memo::new(move |_| {
                                        editing.get().as_deref() == Some(sid_for_memo.as_str())
                                    });
                                    let source_edit = source.clone();
                                    let users_vec = users.get_value();

                                    // ── Metadata summary line (read view) ──
                                    let repo = source.repo_url.clone();
                                    let branch = source.mainline_branch.clone();
                                    let maints = source.maintainers.clone();
                                    let has_meta = source.has_metadata;
                                    let doc_count = source.document_count;

                                    let meta_line = if has_meta {
                                        let repo_view = repo.clone().map(|r| {
                                            let href = r.clone();
                                            let shown = r.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
                                            view! {
                                                <a class="inline-flex items-center gap-1.5 font-medium text-base-content/70 transition-colors hover:text-primary" href=href target="_blank" rel="noopener" on:click=|e| e.stop_propagation()>
                                                    <svg class="h-3.5 w-3.5 shrink-0" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/></svg>
                                                    {shown}
                                                </a>
                                            }
                                        });
                                        let branch_view = branch.clone().map(|b| view! {
                                            <span class="inline-flex items-center gap-1 font-mono text-[0.7rem] text-base-content/55">
                                                <svg class="h-3.5 w-3.5 shrink-0" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6 3v12m0 0a3 3 0 103 3M6 15a3 3 0 013-3h3a3 3 0 003-3V6m0 0a3 3 0 10-3-3 3 3 0 003 3z"/></svg>
                                                {b}
                                            </span>
                                        });
                                        let maint_view = (!maints.is_empty()).then(|| {
                                            let chips = maints.iter().map(|m| {
                                                let (label, linked) = maintainer_display(m, &users_vec);
                                                let display = label.split('<').next().unwrap_or(&label).trim().to_string();
                                                let display = if display.is_empty() { label.clone() } else { display };
                                                let ini = initials(&display);
                                                let ring = if linked { "ring-1 ring-primary/40 bg-primary/10 text-primary" } else { "bg-base-200 text-base-content/60" };
                                                view! {
                                                    <span class="inline-flex items-center gap-1.5 rounded-full bg-base-200/60 py-0.5 pl-0.5 pr-2" title=label>
                                                        <span class=format!("grid h-5 w-5 place-items-center rounded-full text-[0.6rem] font-bold {ring}")>{ini}</span>
                                                        <span class="max-w-[10rem] truncate text-[0.7rem] text-base-content/70">{display}</span>
                                                    </span>
                                                }
                                            }).collect::<Vec<_>>();
                                            view! { <div class="flex flex-wrap items-center gap-1.5">{chips}</div> }
                                        });

                                        let is_empty_meta = repo_view.is_none() && branch_view.is_none() && maints.is_empty();
                                        if is_empty_meta {
                                            view! { <span class="text-xs italic text-base-content/45">"Metadata saved, no details yet"</span> }.into_any()
                                        } else {
                                            view! {
                                                <div class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1.5">
                                                    {repo_view}
                                                    {branch_view}
                                                    {maint_view}
                                                </div>
                                            }.into_any()
                                        }
                                    } else {
                                        view! { <span class="mt-1.5 inline-block text-xs italic text-base-content/45">"No repository metadata yet — add a repo, branch, and maintainers."</span> }.into_any()
                                    };

                                    view! {
                                        <div class="overflow-hidden rounded-2xl border border-base-200 bg-base-100 transition-colors hover:border-base-300">
                                            <div class="flex items-center gap-4 p-4">
                                                <div class="grid h-11 w-11 shrink-0 place-items-center rounded-xl bg-primary/10 text-primary">
                                                    <svg class="h-5 w-5" fill="none" stroke="currentColor" stroke-width="1.75" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>
                                                </div>
                                                <div class="min-w-0 flex-1">
                                                    <div class="flex flex-wrap items-center gap-2">
                                                        <span class="truncate font-mono text-sm font-semibold text-base-content">{source.id.clone()}</span>
                                                        <span class="badge badge-sm badge-ghost gap-1 font-medium">
                                                            <svg class="h-3 w-3" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>
                                                            {format!("{doc_count}")}
                                                        </span>
                                                        {(!has_meta).then(|| view! { <span class="badge badge-sm badge-warning badge-outline">"needs setup"</span> })}
                                                        {source.review_enabled.then(|| view! { <span class="badge badge-sm badge-primary badge-outline gap-1" title="Automated documentation review is enabled">
                                                            <svg class="h-3 w-3" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/></svg>
                                                            "auto-review"
                                                        </span> })}
                                                    </div>
                                                    {meta_line}
                                                </div>
                                                <button
                                                    class="btn btn-ghost btn-sm shrink-0 gap-1.5"
                                                    on:click=move |_| {
                                                        if is_editing.get() { editing.set(None); } else { start_edit(&source_edit); }
                                                    }
                                                >
                                                    <span class="hidden sm:inline">{move || if is_editing.get() { "Close" } else if has_meta { "Edit" } else { "Set up" }}</span>
                                                    <svg class=move || format!("h-4 w-4 transition-transform duration-200 motion-reduce:transition-none {}", if is_editing.get() { "rotate-180" } else { "" }) fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M19 9l-7 7-7-7"/></svg>
                                                </button>
                                            </div>

                                            <Show when=move || is_editing.get()>
                                                <div class="animate-in fade-in slide-in-from-top-1 border-t border-base-200 bg-base-200/30 p-5 duration-200 motion-reduce:animate-none">
                                                    <div class="grid grid-cols-1 gap-x-6 gap-y-4 sm:grid-cols-2">
                                                        <div>
                                                            <label class="mb-1 block text-xs font-semibold text-base-content/70">"Display name"</label>
                                                            <input type="text" class="input input-bordered input-sm w-full bg-base-100" placeholder=source.id.clone()
                                                                prop:value=move || edit_display_name.get()
                                                                on:input=move |e| edit_display_name.set(event_target_value(&e)) />
                                                        </div>
                                                        <div>
                                                            <label class="mb-1 block text-xs font-semibold text-base-content/70">"Mainline branch"</label>
                                                            <input type="text" class="input input-bordered input-sm w-full bg-base-100 font-mono" placeholder="main"
                                                                prop:value=move || edit_branch.get()
                                                                on:input=move |e| edit_branch.set(event_target_value(&e)) />
                                                        </div>
                                                        <div class="sm:col-span-2">
                                                            <label class="mb-1 block text-xs font-semibold text-base-content/70">"Repository URL"</label>
                                                            <input type="url" class="input input-bordered input-sm w-full bg-base-100" placeholder="https://github.com/org/repo"
                                                                prop:value=move || edit_repo_url.get()
                                                                on:input=move |e| edit_repo_url.set(event_target_value(&e)) />
                                                        </div>
                                                        <div class="sm:col-span-2">
                                                            <label class="mb-1 block text-xs font-semibold text-base-content/70">"Description"</label>
                                                            <textarea class="textarea textarea-bordered textarea-sm min-h-16 w-full bg-base-100" placeholder="What lives in this source?"
                                                                prop:value=move || edit_description.get()
                                                                on:input=move |e| edit_description.set(event_target_value(&e)) />
                                                        </div>
                                                    </div>

                                                    <label class="mt-5 flex cursor-pointer items-start gap-3 rounded-xl border border-base-200 bg-base-100 p-3">
                                                        <input type="checkbox" class="toggle toggle-primary toggle-sm mt-0.5"
                                                            prop:checked=move || edit_review_enabled.get()
                                                            on:change=move |e| edit_review_enabled.set(event_target_checked(&e)) />
                                                        <span class="min-w-0">
                                                            <span class="block text-xs font-semibold text-base-content/80">"Automated documentation review"</span>
                                                            <span class="block text-[0.7rem] text-base-content/55">"Let the documentation agent open change proposals for this source. Off by default."</span>
                                                        </span>
                                                    </label>

                                                    <div class="mt-5">
                                                        <div class="flex items-center justify-between">
                                                            <span class="text-xs font-semibold text-base-content/70">"Maintainers"</span>
                                                            <button class="btn btn-ghost btn-xs gap-1 text-primary" on:click=move |_| add_row(None)>
                                                                <svg class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="2.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4"/></svg>
                                                                "Add"
                                                            </button>
                                                        </div>
                                                        <p class="mt-0.5 text-[0.7rem] text-base-content/50">"Add an email for external contacts, or link a Lekton user."</p>
                                                        <div class="mt-2 space-y-2">
                                                            <For
                                                                each=move || edit_maintainers.get()
                                                                key=|r| r.key
                                                                children=move |row| {
                                                                    let users_opt = users.get_value();
                                                                    view! {
                                                                        <div class="grid grid-cols-1 gap-2 rounded-xl border border-base-200 bg-base-100 p-2.5 sm:grid-cols-[1fr_1fr_1fr_auto] sm:items-center">
                                                                            <input type="text" class="input input-bordered input-xs w-full" placeholder="Name"
                                                                                prop:value=move || row.name.get()
                                                                                on:input=move |e| row.name.set(event_target_value(&e)) />
                                                                            <input type="email" class="input input-bordered input-xs w-full" placeholder="email@company.com"
                                                                                prop:value=move || row.email.get()
                                                                                on:input=move |e| row.email.set(event_target_value(&e)) />
                                                                            <select class="select select-bordered select-xs w-full"
                                                                                prop:value=move || row.user_id.get()
                                                                                on:change=move |e| row.user_id.set(event_target_value(&e))>
                                                                                <option value="">"— link Lekton user —"</option>
                                                                                {users_opt.iter().map(|u| {
                                                                                    let label = match &u.name {
                                                                                        Some(n) if !n.is_empty() => format!("{n} <{}>", u.email),
                                                                                        _ => u.email.clone(),
                                                                                    };
                                                                                    view! { <option value=u.id.clone()>{label}</option> }
                                                                                }).collect::<Vec<_>>()}
                                                                            </select>
                                                                            <button class="btn btn-ghost btn-xs justify-self-end text-base-content/50 hover:text-error" aria-label="Remove maintainer"
                                                                                on:click=move |_| edit_maintainers.update(|v| v.retain(|r2| r2.key != row.key))>
                                                                                <svg class="h-4 w-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>
                                                                            </button>
                                                                        </div>
                                                                    }
                                                                }
                                                            />
                                                            {move || edit_maintainers.get().is_empty().then(|| view! {
                                                                <p class="rounded-xl border border-dashed border-base-300 py-3 text-center text-xs text-base-content/45">"No maintainers yet."</p>
                                                            })}
                                                        </div>
                                                    </div>

                                                    <div class="mt-5 flex items-center gap-2 border-t border-base-200 pt-4">
                                                        <button class="btn btn-primary btn-sm gap-2"
                                                            prop:disabled=move || saving.get()
                                                            on:click=move |_| { save_action.dispatch(()); }>
                                                            {move || saving.get().then(|| view! { <span class="loading loading-spinner loading-xs"></span> })}
                                                            "Save"
                                                        </button>
                                                        <button class="btn btn-ghost btn-sm"
                                                            on:click=move |_| editing.set(None)>"Cancel"</button>
                                                        {has_meta.then(|| view! {
                                                            <button class="btn btn-ghost btn-sm ml-auto text-error hover:bg-error/10"
                                                                prop:disabled=move || deleting.get()
                                                                on:click=move |_| { delete_action.dispatch(()); }>
                                                                {move || deleting.get().then(|| view! { <span class="loading loading-spinner loading-xs"></span> })}
                                                                "Delete metadata"
                                                            </button>
                                                        })}
                                                    </div>
                                                </div>
                                            </Show>
                                        </div>
                                    }
                                }
                            />
                        </div>
                    }.into_any())
                }}
            </Suspense>
        </div>
    }
}
