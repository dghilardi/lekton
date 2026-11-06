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
    /// Current user email, when the account still exists and has an email.
    #[serde(default)]
    pub actor_email: Option<String>,
    pub calls: u64,
    /// Of `calls`, how many were embeddings. Embedding a search query costs a
    /// handful of tokens, so a caller with many of them shows few tokens per
    /// call without anything being lost.
    #[serde(default)]
    pub embedding_calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Cost in credits, priced per model.
    pub credits: f64,
}

/// `1234567` → `"1,234,567"`.
pub fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Credits with enough precision to tell small amounts from zero: two
/// decimals below 10, one above, thousands separators throughout, and
/// "< 0.01" rather than a misleading "0.00" for tiny non-zero spend.
pub fn format_credits(credits: f64) -> String {
    if credits <= 0.0 {
        return "0".to_string();
    }
    if credits < 0.01 {
        return "< 0.01".to_string();
    }
    let decimals = if credits < 10.0 { 2 } else { 1 };
    let fixed = format!("{credits:.decimals$}");
    let (whole, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let whole = format_count(whole.parse().unwrap_or(0));
    format!("{whole}.{fraction}")
}

/// Each row's share of the total credits, in percent (all zero when nothing
/// was spent).
pub fn credit_shares(rows: &[ConsumerUsage]) -> Vec<f64> {
    let total: f64 = rows.iter().map(|row| row.credits).sum();
    rows.iter()
        .map(|row| {
            if total > 0.0 {
                (row.credits / total * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            }
        })
        .collect()
}

impl ConsumerUsage {
    /// Prefer a user's email, retaining the recorded identity as a fallback.
    pub fn caller_label(&self) -> &str {
        if self.actor_kind == "user" {
            if let Some(email) = self
                .actor_email
                .as_deref()
                .map(str::trim)
                .filter(|email| !email.is_empty())
            {
                return email;
            }
        }
        self.actor_id.as_deref().unwrap_or("—")
    }
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
                actor_email: None,
                calls: 0,
                embedding_calls: 0,
                prompt_tokens: 0,
                completion_tokens: 0,
                credits: 0.0,
            });
        entry.calls += row.calls;
        if row.feature == crate::usage::LlmFeature::Embedding.as_str() {
            entry.embedding_calls += row.calls;
        }
        entry.prompt_tokens += row.prompt_tokens;
        entry.completion_tokens += row.completion_tokens;
        entry.credits += credits;
    }

    let mut consumers: Vec<ConsumerUsage> = by_actor.into_values().collect();
    consumers.sort_by(|a, b| b.credits.total_cmp(&a.credits));
    consumers.truncate(limit.clamp(1, 200));

    // Resolve only the users in the bounded report, after ranking. Keep usage
    // attributed to the recorded ID even if the account is missing or lookup fails.
    for consumer in &mut consumers {
        if consumer.actor_kind != "user" {
            continue;
        }
        let Some(id) = consumer.actor_id.as_deref() else {
            continue;
        };
        match state.user_repo.find_user_by_id(id).await {
            Ok(Some(user)) => {
                let email = user.email.trim();
                if !email.is_empty() {
                    consumer.actor_email = Some(email.to_string());
                }
            }
            Ok(None) if state.demo_mode => {
                consumer.actor_email = id
                    .strip_prefix("demo-")
                    .and_then(crate::auth::demo_auth::resolve_demo_session_user)
                    .map(|user| user.email);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "Failed to resolve AI usage user email"),
        }
    }
    Ok(consumers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consumer(kind: &str, id: Option<&str>, email: Option<&str>) -> ConsumerUsage {
        ConsumerUsage {
            actor_kind: kind.into(),
            actor_id: id.map(str::to_string),
            actor_email: email.map(str::to_string),
            calls: 1,
            embedding_calls: 0,
            prompt_tokens: 10,
            completion_tokens: 5,
            credits: 1.0,
        }
    }

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_728), "1,728");
        assert_eq!(format_count(12_345_678), "12,345,678");
    }

    #[test]
    fn small_credits_do_not_round_to_zero() {
        assert_eq!(format_credits(0.0), "0");
        assert_eq!(format_credits(0.004), "< 0.01");
        assert_eq!(format_credits(0.0412), "0.04");
        assert_eq!(format_credits(1.728), "1.73");
        assert_eq!(format_credits(12_345.67), "12,345.7");
    }

    #[test]
    fn shares_are_relative_to_the_total() {
        let mut a = consumer("user", Some("a"), None);
        a.credits = 3.0;
        let mut b = consumer("user", Some("b"), None);
        b.credits = 1.0;
        assert_eq!(credit_shares(&[a.clone(), b.clone()]), vec![75.0, 25.0]);
        a.credits = 0.0;
        b.credits = 0.0;
        assert_eq!(credit_shares(&[a, b]), vec![0.0, 0.0]);
    }

    #[test]
    fn user_label_prefers_email_and_preserves_id() {
        let row = consumer("user", Some("user-1"), Some(" person@example.com "));
        assert_eq!(row.caller_label(), "person@example.com");
        assert_eq!(row.actor_id.as_deref(), Some("user-1"));
    }

    #[test]
    fn missing_or_blank_email_falls_back_to_user_id() {
        for email in [None, Some(""), Some(" \t ")] {
            assert_eq!(
                consumer("user", Some("user-1"), email).caller_label(),
                "user-1"
            );
        }
        assert_eq!(consumer("user", None, None).caller_label(), "—");
    }

    #[test]
    fn non_user_callers_keep_their_recorded_identity() {
        assert_eq!(
            consumer("service_token", Some("token-1"), Some("person@example.com")).caller_label(),
            "token-1"
        );
        for kind in ["system", "anonymous"] {
            assert_eq!(consumer(kind, None, None).caller_label(), "—");
        }
    }

    #[test]
    fn reports_without_email_remain_deserializable() {
        let row: ConsumerUsage = serde_json::from_value(serde_json::json!({
            "actor_kind": "user", "actor_id": "user-1", "calls": 1,
            "prompt_tokens": 10, "completion_tokens": 5, "credits": 1.0
        }))
        .unwrap();
        assert_eq!(row.caller_label(), "user-1");
    }
}
