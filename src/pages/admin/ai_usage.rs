use leptos::prelude::*;

use crate::app::{list_top_consumers, ConsumerUsage};
use crate::auth::refresh_client::with_auth_retry;

/// Windows offered for the report, in days.
const WINDOWS: &[(u32, &str)] = &[(1, "24 hours"), (7, "7 days"), (30, "30 days")];
/// Kept short: the point is to find the outliers, not to browse everyone.
const LIMIT: usize = 25;

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
        with_auth_retry(move || list_top_consumers(window, LIMIT))
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

                        let top = rows.first().map(|r| r.credits).unwrap_or(1.0).max(f64::EPSILON);
                        Some(view! {
                            <div class="overflow-x-auto">
                                <table class="table table-sm">
                                    <thead>
                                        <tr>
                                            <th>"Caller"</th>
                                            <th class="text-right">"Calls"</th>
                                            <th class="text-right">"Tokens"</th>
                                            <th class="text-right">"Credits"</th>
                                            <th class="w-1/4">"Share"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {rows.iter().map(|row| {
                                            let ConsumerUsage { actor_kind, actor_id, calls, prompt_tokens, completion_tokens, credits } = row.clone();
                                            let tokens = prompt_tokens + completion_tokens;
                                            // Relative to the top spender: the
                                            // question this page answers is who
                                            // stands out, not the absolute total.
                                            let share = (credits / top * 100.0).clamp(0.0, 100.0);
                                            view! {
                                                <tr>
                                                    <td>
                                                        <div class="flex items-center gap-2">
                                                            <span class="badge badge-ghost badge-sm font-mono">{actor_kind}</span>
                                                            <span class="font-mono text-xs">
                                                                {actor_id.unwrap_or_else(|| "—".to_string())}
                                                            </span>
                                                        </div>
                                                    </td>
                                                    <td class="text-right tabular-nums">{calls}</td>
                                                    <td class="text-right tabular-nums">{tokens}</td>
                                                    <td class="text-right tabular-nums font-medium">
                                                        {format!("{credits:.1}")}
                                                    </td>
                                                    <td>
                                                        <progress
                                                            class="progress progress-primary w-full"
                                                            value=share
                                                            max="100"
                                                        ></progress>
                                                    </td>
                                                </tr>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </tbody>
                                </table>
                            </div>
                        }.into_any())
                    }}
                </Suspense>
            </div>
        </div>
    }
}
