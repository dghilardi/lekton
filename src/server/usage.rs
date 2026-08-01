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
