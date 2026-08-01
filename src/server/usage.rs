//! Server functions for AI spending budgets.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

/// What a user has left to spend, for the chat indicator.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BudgetStatus {
    /// Credits remaining.
    pub balance: f64,
    /// Bucket size.
    pub capacity: f64,
    /// Fraction remaining, `0.0..=1.0`.
    pub headroom: f64,
    /// Whether answers are currently being served without the optional
    /// retrieval steps to save credits.
    pub thrifty: bool,
}

/// The signed-in user's budget, or `None` when budgets are not enforced.
///
/// `None` is the normal case for an instance that has not opted in, and the UI
/// shows nothing at all rather than an empty gauge.
#[server(GetMyBudget, "/api")]
pub async fn get_my_budget() -> Result<Option<BudgetStatus>, ServerFnError> {
    let state = expect_context::<AppState>();
    let user_ctx = crate::server::require_user_context(&state).await?;

    let snapshot =
        crate::usage::budget::snapshot(&user_ctx.usage_key(), user_ctx.budget_plan.as_deref())
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(snapshot.map(|s| BudgetStatus {
        balance: s.balance,
        capacity: s.capacity,
        headroom: s.headroom,
        thrifty: s.thrifty,
    }))
}

/// Plan names available to assign, from `[usage.budget.plans]`.
///
/// Empty when budgets are disabled, which is how the admin UI knows to hide
/// the control rather than offer a choice that would have no effect.
#[server(ListBudgetPlans, "/api")]
pub async fn list_budget_plans() -> Result<Vec<String>, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    Ok(crate::usage::budget::plan_names())
}

/// One caller's AI spend over the reporting window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerUsage {
    /// `user`, `service_token`, `anonymous` or `system`.
    pub actor_kind: String,
    /// User or token id; absent for anonymous and background work.
    pub actor_id: Option<String>,
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Cost in credits, priced per model.
    pub credits: f64,
}

/// Top AI consumers over the last `days`, most expensive first.
///
/// Returns an empty list when the event log is off — there is nothing recorded
/// to report on, and the page says so rather than implying nobody spent
/// anything.
#[server(ListTopConsumers, "/api")]
pub async fn list_top_consumers(
    days: u32,
    limit: usize,
) -> Result<Vec<ConsumerUsage>, ServerFnError> {
    use std::collections::HashMap;

    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let Some(repo) = state.usage_event_repo.clone() else {
        return Ok(Vec::new());
    };

    let since = chrono::Utc::now() - chrono::Duration::days(i64::from(days.clamp(1, 365)));
    let rows = repo
        .usage_by_model(since)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Fold per-model rows into one entry per caller, pricing each model's
    // tokens on the way: that is the only point where the different rates can
    // still be applied.
    let mut by_actor: HashMap<(String, Option<String>), ConsumerUsage> = HashMap::new();
    for row in rows {
        let credits =
            crate::usage::pricing::credits(&row.model, row.prompt_tokens, row.completion_tokens);
        let entry = by_actor
            .entry((row.actor_kind.clone(), row.actor_id.clone()))
            .or_insert_with(|| ConsumerUsage {
                actor_kind: row.actor_kind.clone(),
                actor_id: row.actor_id.clone(),
                calls: 0,
                prompt_tokens: 0,
                completion_tokens: 0,
                credits: 0.0,
            });
        entry.calls += row.calls;
        entry.prompt_tokens += row.prompt_tokens;
        entry.completion_tokens += row.completion_tokens;
        entry.credits += credits;
    }

    let mut consumers: Vec<ConsumerUsage> = by_actor.into_values().collect();
    consumers.sort_by(|a, b| b.credits.total_cmp(&a.credits));
    consumers.truncate(limit.clamp(1, 200));
    Ok(consumers)
}
