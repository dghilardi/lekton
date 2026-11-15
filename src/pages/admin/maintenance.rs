use leptos::prelude::*;

use crate::app::{
    format_bytes, format_count, list_embedding_cache_namespaces, purge_embedding_cache_namespaces,
    EmbeddingCacheNamespace,
};
use crate::auth::refresh_client::with_auth_retry;

/// The data-maintenance admin section.
#[component]
pub fn DataMaintenancePanel() -> impl IntoView {
    view! {
        <div class="space-y-6">
            <EmbeddingCacheSection />
            <ConsistencySection />
            <RevisionRetentionSection />
        </div>
    }
}

/// Embedding-cache namespaces with their size, and a purge of the ones the
/// running configuration does not use.
#[component]
fn EmbeddingCacheSection() -> impl IntoView {
    let (refresh, set_refresh) = signal(0u32);
    let namespaces = LocalResource::new(move || {
        let _ = refresh.get();
        with_auth_retry(list_embedding_cache_namespaces)
    });
    let selected = RwSignal::new(Vec::<String>::new());
    let confirm_open = RwSignal::new(false);
    let (outcome, set_outcome) = signal(Option::<Result<String, String>>::None);

    let purge_action = crate::components::browser_action(move |_: &()| {
        let chosen = selected.get_untracked();
        async move {
            let result = with_auth_retry(|| purge_embedding_cache_namespaces(chosen.clone())).await;
            set_outcome.set(Some(match &result {
                Ok(deleted) => Ok(format!(
                    "Deleted {} cached embeddings.",
                    format_count(*deleted)
                )),
                Err(e) => Err(e.to_string()),
            }));
            if result.is_ok() {
                selected.set(Vec::new());
                set_refresh.update(|n| *n += 1);
            }
        }
    });
    let purging = purge_action.pending();

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200 overflow-hidden">
            <div class="card-body p-8 space-y-4">
                <div>
                    <h2 class="card-title text-xl">"Embedding cache"</h2>
                    <p class="text-sm text-base-content/70 mt-1 max-w-3xl">
                        "Vectors are cached per model, dimensions and endpoint. Changing any of them starts a new \
                         namespace and leaves the old one unused. Namespaces the current configuration uses \
                         cannot be deleted; deleting another one only means it would be embedded again if that \
                         configuration came back."
                    </p>
                </div>

                <Suspense fallback=move || view! {
                    <div class="flex justify-center py-8">
                        <span class="loading loading-spinner loading-md text-primary"></span>
                    </div>
                }>
                    {move || {
                        let rows = match namespaces.get()? {
                            Ok(rows) => rows,
                            Err(error) => return Some(view! {
                                <crate::components::ReaderErrorNotice error=error context="The embedding cache could not be read" />
                            }.into_any()),
                        };
                        if rows.is_empty() {
                            return Some(view! {
                                <p class="text-sm text-base-content/70">"The embedding cache is empty."</p>
                            }.into_any());
                        }
                        Some(view! { <NamespaceTable rows=rows selected=selected /> }.into_any())
                    }}
                </Suspense>

                {move || outcome.get().map(|result| match result {
                    Ok(message) => view! { <div class="alert alert-success" role="status">{message}</div> }.into_any(),
                    Err(message) => view! { <div class="alert alert-error" role="alert">{message}</div> }.into_any(),
                })}

                <div class="flex justify-end">
                    <button
                        type="button"
                        class="btn btn-error btn-outline btn-sm"
                        disabled=move || selected.with(Vec::is_empty) || purging.get()
                        on:click=move |_| confirm_open.set(true)
                    >
                        {move || if purging.get() {
                            "Deleting…".to_string()
                        } else {
                            match selected.with(Vec::len) {
                                0 => "Delete selected".to_string(),
                                1 => "Delete 1 namespace".to_string(),
                                n => format!("Delete {n} namespaces"),
                            }
                        }}
                    </button>
                </div>

                {move || {
                    let chosen = selected.get();
                    let message = format!(
                        "Every cached vector of {} will be deleted: {}. This cannot be undone.",
                        if chosen.len() == 1 { "this namespace".to_string() } else { format!("these {} namespaces", chosen.len()) },
                        chosen.join(", "),
                    );
                    view! {
                        <crate::components::ConfirmDialog
                            open=confirm_open
                            id="purge-embedding-cache"
                            title="Delete cached embeddings?"
                            message=message
                            confirm_label="Delete"
                            on_cancel=Callback::new(move |_| confirm_open.set(false))
                            on_confirm=Callback::new(move |_| {
                                confirm_open.set(false);
                                purge_action.dispatch(());
                            })
                        />
                    }
                }}
            </div>
        </div>
    }
}

#[component]
fn NamespaceTable(
    rows: Vec<EmbeddingCacheNamespace>,
    selected: RwSignal<Vec<String>>,
) -> impl IntoView {
    let total_entries: u64 = rows.iter().map(|r| r.entries).sum();
    let total_bytes: u64 = rows.iter().map(|r| r.approx_bytes).sum();
    view! {
        <div class="overflow-x-auto" tabindex="0" role="region" aria-label="Embedding cache namespaces">
            <table class="table table-sm">
                <thead>
                    <tr>
                        <th class="w-8"><span class="sr-only">"Select"</span></th>
                        <th>"Model"</th>
                        <th class="text-right">"Dimensions"</th>
                        <th class="text-right">"Entries"</th>
                        <th class="text-right">"Approx. size"</th>
                        <th>"Status"</th>
                    </tr>
                </thead>
                <tbody>
                    {rows.into_iter().map(|row| {
                        let (model, dimensions) = row.model_and_dimensions();
                        let (model, dimensions) = (model.to_string(), dimensions.to_string());
                        let namespace = row.namespace.clone();
                        let toggle_namespace = namespace.clone();
                        let checked_namespace = namespace.clone();
                        let label = format!("Select {namespace}");
                        view! {
                            <tr>
                                <td>
                                    <input
                                        type="checkbox"
                                        class="checkbox checkbox-sm"
                                        aria-label=label
                                        disabled=row.in_use
                                        prop:checked=move || selected.with(|s| s.contains(&checked_namespace))
                                        on:change=move |_| selected.update(|s| {
                                            if let Some(i) = s.iter().position(|n| n == &toggle_namespace) {
                                                s.remove(i);
                                            } else {
                                                s.push(toggle_namespace.clone());
                                            }
                                        })
                                    />
                                </td>
                                <td>
                                    <div class="font-medium">{model}</div>
                                    <div class="font-mono text-xs text-base-content/70 break-all">{namespace}</div>
                                </td>
                                <td class="text-right tabular-nums">{dimensions}</td>
                                <td class="text-right tabular-nums">{format_count(row.entries)}</td>
                                <td class="text-right tabular-nums">{format_bytes(row.approx_bytes)}</td>
                                <td>
                                    {if row.in_use {
                                        view! { <span class="badge badge-success badge-sm">"In use"</span> }.into_any()
                                    } else {
                                        view! { <span class="badge badge-ghost badge-sm">"Unused"</span> }.into_any()
                                    }}
                                </td>
                            </tr>
                        }
                    }).collect::<Vec<_>>()}
                </tbody>
                <tfoot>
                    <tr class="font-semibold">
                        <td></td>
                        <td colspan="2">"Total"</td>
                        <td class="text-right tabular-nums">{format_count(total_entries)}</td>
                        <td class="text-right tabular-nums">{format_bytes(total_bytes)}</td>
                        <td></td>
                    </tr>
                </tfoot>
            </table>
        </div>
    }
}

#[component]
fn ConsistencySection() -> impl IntoView {
    let result =
        RwSignal::new(None::<Result<crate::server::maintenance::ConsistencyReport, String>>);
    let action = crate::components::browser_action(move |_: &()| async move {
        result.set(Some(
            with_auth_retry(crate::server::maintenance::run_consistency_report)
                .await
                .map_err(|e| e.to_string()),
        ));
    });
    view! {
        <section class="space-y-4" aria-labelledby="consistency-title">
            <h2 id="consistency-title" class="text-xl font-semibold">"Storage and index consistency"</h2>
            <p class="text-sm max-w-3xl">"Compare MongoDB with storage and indexes. This report only reads data. Concurrent imports can cause temporary findings; rerun after they finish. Samples are capped at 20 per category."</p>
            <button class="btn btn-outline btn-sm" disabled=move || action.pending().get()
                on:click=move |_| { action.dispatch(()); }>
                {move || if action.pending().get() { "Comparing…" } else { "Run consistency report" }}
            </button>
            <div aria-live="polite">
                {move || result.get().map(|r| match r {
                    Err(e) => view! { <p class="text-error" role="alert">{e}</p> }.into_any(),
                    Ok(report) => view! {
                        <dl>{report.totals.into_iter().map(|(name, count)| view! {
                            <div class="flex gap-4"><dt>{name}</dt><dd class="tabular-nums">{count}</dd></div>
                        }).collect_view()}</dl>
                        {report.skipped.into_iter().map(|s| view! { <p class="text-warning">{s}</p> }).collect_view()}
                        <div class="space-y-4 mt-4">{report.findings.into_iter().map(|f| view! {
                            <details class="border border-base-300 rounded-box p-4">
                                <summary class="cursor-pointer font-medium">{f.title} ": " {f.count}</summary>
                                <p class="text-sm mt-2">{f.description}</p>
                                <ul class="list-disc pl-5 break-all text-sm">{f.samples.into_iter().map(|s| view! { <li>{s}</li> }).collect_view()}</ul>
                            </details>
                        }).collect_view()}</div>
                    }.into_any(),
                })}
            </div>
        </section>
    }
}

#[component]
fn RevisionRetentionSection() -> impl IntoView {
    use crate::server::maintenance::{
        preview_revision_retention, prune_revision_history, RetentionPreview,
    };
    let preview = RwSignal::new(None::<RetentionPreview>);
    let message = RwSignal::new(String::new());
    let confirm = RwSignal::new(false);
    let inspect = crate::components::browser_action(move |_: &()| async move {
        preview.set(None);
        match with_auth_retry(preview_revision_retention).await {
            Ok(p) => {
                message.set(format!(
                    "{} revisions would be removed. Keep last {} per slug (0 means unlimited).",
                    p.candidates, p.keep_last
                ));
                preview.set(Some(p));
            }
            Err(e) => message.set(e.to_string()),
        }
    });
    let prune = crate::components::browser_action(move |_: &()| async move {
        preview.set(None);
        message.set(match with_auth_retry(prune_revision_history).await {
            Ok((deleted, failures)) => format!("Removed {deleted} revision rows; {failures} failures. Run the consistency report to check storage leftovers."),
            Err(e) => e.to_string(),
        });
    });
    view! {
        <section class="space-y-4" aria-labelledby="retention-title">
            <h2 id="retention-title" class="text-xl font-semibold">"Revision history retention"</h2>
            <p class="text-sm max-w-3xl">"New revisions apply the configured limit automatically. Preview and prune older history for all slugs here. Current document bodies are preserved. The count may change if documents are edited during the sweep."</p>
            <div class="flex flex-wrap gap-3">
                <button class="btn btn-outline btn-sm" disabled=move || inspect.pending().get() || prune.pending().get()
                    on:click=move |_| { inspect.dispatch(()); }>"Preview retention"</button>
                <button class="btn btn-error btn-outline btn-sm"
                    disabled=move || preview.get().is_none_or(|p| p.candidates == 0) || prune.pending().get()
                    on:click=move |_| confirm.set(true)>"Prune history…"</button>
            </div>
            <p role="status">{move || message.get()}</p>
            <crate::components::ConfirmDialog open=confirm id="prune-history"
                title="Remove older revisions?"
                message="Older historical bodies will be permanently deleted according to the configured limit."
                confirm_label="Prune history"
                on_cancel=Callback::new(move |_| confirm.set(false))
                on_confirm=Callback::new(move |_| { confirm.set(false); prune.dispatch(()); }) />
        </section>
    }
}
