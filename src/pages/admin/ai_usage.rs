use leptos::prelude::*;

use crate::app::{credit_shares, format_count, format_credits, list_top_consumers, ConsumerUsage};
use crate::auth::refresh_client::with_auth_retry;

/// Windows offered for the report, in days.
const WINDOWS: &[(u32, &str)] = &[(1, "24 hours"), (7, "7 days"), (30, "30 days")];
/// Callers fetched: all of them in practice (the server caps at 200), so the
/// totals and shares cover everyone, not just the rows shown.
const FETCH_LIMIT: usize = 200;
/// Rows shown. Kept short: the point is to find the outliers, not to browse
/// everyone.
const SHOWN: usize = 25;

/// Who is spending on AI features, and how much.
///
/// Reads the event log, so it is empty unless `usage.event_log` is enabled —
/// which the empty state says outright, because "nobody spent anything" and
/// "nothing is being recorded" look identical otherwise.
#[component]
pub fn AiUsageReport() -> impl IntoView {
    let days = RwSignal::new(7u32);

    let consumers = LocalResource::new(move || {
        let window = days.get();
        with_auth_retry(move || list_top_consumers(window, FETCH_LIMIT))
    });

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200 overflow-hidden">
            <div class="card-body p-8 space-y-4">
                <div class="flex items-center justify-between gap-4 flex-wrap">
                    <p class="text-sm text-base-content/65">
                        "Ranked by credits spent. Background indexing appears as "
                        <span class="font-mono text-xs">"system"</span>
                        " — it has no caller to bill."
                    </p>
                    <div class="join">
                        {WINDOWS.iter().map(|(value, label)| {
                            let value = *value;
                            view! {
                                <button
                                    class=move || if days.get() == value {
                                        "join-item btn btn-sm btn-primary"
                                    } else {
                                        "join-item btn btn-sm btn-ghost"
                                    }
                                    on:click=move |_| days.set(value)
                                >
                                    {*label}
                                </button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </div>

                <Suspense fallback=move || view! {
                    <div class="flex justify-center py-8">
                        <span class="loading loading-spinner loading-md text-primary"></span>
                    </div>
                }>
                    {move || {
                        let rows = consumers.get()?.ok()?;
                        if rows.is_empty() {
                            return Some(view! {
                                <div class="flex flex-col items-center justify-center py-12 px-4 text-center border-2 border-dashed border-base-300 rounded-xl bg-base-200/20">
                                    <h3 class="font-bold text-lg text-base-content/70">"No usage recorded"</h3>
                                    <p class="text-sm text-base-content/65 max-w-md mt-1">
                                        "Either nothing was spent in this window, or the per-caller event log is off. \
                                         Set "
                                        <span class="font-mono text-xs">"usage.event_log"</span>
                                        " to start recording who spends what; the Prometheus token counters are \
                                         collected either way."
                                    </p>
                                </div>
                            }.into_any());
                        }

                        let shares = credit_shares(&rows);
                        let hidden = rows.len().saturating_sub(SHOWN);
                        let total_calls: u64 = rows.iter().map(|r| r.calls).sum();
                        let total_embedding_calls: u64 = rows.iter().map(|r| r.embedding_calls).sum();
                        let total_tokens: u64 = rows.iter().map(|r| r.prompt_tokens + r.completion_tokens).sum();
                        let total_credits: f64 = rows.iter().map(|r| r.credits).sum();
                        Some(view! {
                            <div class="overflow-x-auto">
                                <table class="table table-sm">
                                    <thead>
                                        <tr>
                                            <th>"Caller"</th>
                                            <th class="text-right">"Calls"</th>
                                            <th class="text-right">"Tokens"</th>
                                            <th class="text-right">"Credits"</th>
                                            <th class="w-1/4">"Share of total"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {rows.iter().zip(shares).take(SHOWN).map(|(row, share)| {
                                            let caller = row.caller_label().to_string();
                                            let ConsumerUsage { actor_kind, actor_id, calls, embedding_calls, prompt_tokens, completion_tokens, credits, .. } = row.clone();
                                            let tokens = prompt_tokens + completion_tokens;
                                            let share_label = format!("{share:.1}%");
                                            view! {
                                                <tr>
                                                    <td>
                                                        <div class="flex items-center gap-2">
                                                            <span class="badge badge-ghost badge-sm font-mono">{actor_kind}</span>
                                                            <span class="font-mono text-xs break-all" title=actor_id>
                                                                {caller}
                                                            </span>
                                                        </div>
                                                    </td>
                                                    <td class="text-right tabular-nums">
                                                        {format_count(calls)}
                                                        // Embedding a query costs a few tokens: say how many
                                                        // calls were embeddings, so a low tokens-per-call
                                                        // figure does not read as missing data.
                                                        {(embedding_calls > 0).then(|| view! {
                                                            <div class="text-xs text-base-content/55 whitespace-nowrap">
                                                                {format!("{} embeddings", format_count(embedding_calls))}
                                                            </div>
                                                        })}
                                                    </td>
                                                    <td class="text-right tabular-nums">{format_count(tokens)}</td>
                                                    <td class="text-right tabular-nums font-medium">
                                                        {format_credits(credits)}
                                                    </td>
                                                    <td>
                                                        <div class="flex items-center gap-2">
                                                            <progress
                                                                class="progress progress-primary w-full"
                                                                value=format!("{share:.1}")
                                                                max="100"
                                                                aria-label=format!("{share_label} of total credits")
                                                            ></progress>
                                                            <span class="w-12 shrink-0 text-right text-xs tabular-nums text-base-content/65">
                                                                {share_label}
                                                            </span>
                                                        </div>
                                                    </td>
                                                </tr>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </tbody>
                                    <tfoot>
                                        <tr class="font-semibold">
                                            <td>
                                                {format!("Total, {} callers", format_count(rows.len() as u64))}
                                                {(hidden > 0).then(|| view! {
                                                    <div class="text-xs font-normal text-base-content/55">
                                                        {format!("{hidden} smaller callers not listed")}
                                                    </div>
                                                })}
                                            </td>
                                            <td class="text-right tabular-nums">
                                                {format_count(total_calls)}
                                                {(total_embedding_calls > 0).then(|| view! {
                                                    <div class="text-xs font-normal text-base-content/55 whitespace-nowrap">
                                                        {format!("{} embeddings", format_count(total_embedding_calls))}
                                                    </div>
                                                })}
                                            </td>
                                            <td class="text-right tabular-nums">{format_count(total_tokens)}</td>
                                            <td class="text-right tabular-nums">{format_credits(total_credits)}</td>
                                            <td></td>
                                        </tr>
                                    </tfoot>
                                </table>
                            </div>
                        }.into_any())
                    }}
                </Suspense>
            </div>
        </div>
    }
}
