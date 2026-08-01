//! Admission control for LLM calls.
//!
//! Two ceilings, both cheap and both about the same failure: something —
//! usually a script, occasionally a bug — asking for far more generation than
//! a person ever would.
//!
//! - **Per-caller concurrency.** A rate limit lets a caller start N requests a
//!   second and keep every one of them running; a concurrency cap is what
//!   actually stops a loop, because the loop has to wait for its own previous
//!   call to finish.
//! - **A daily instance-wide token ceiling.** The last line of defence, aimed
//!   at the case no per-caller limit catches: a runaway reindex, or a bug that
//!   fans out across many callers.
//!
//! Enforcement lives here rather than in an HTTP layer because the same
//! services are reached from REST handlers, Leptos server functions and MCP —
//! a middleware would cover them unevenly.
//!
//! The ceiling counts **tokens, not cost**. Tokens from different models are
//! not comparable in price, so this is a runaway guard, not a spend cap; a
//! cost-accurate ceiling needs the per-model pricing table.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::NaiveDate;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::db::usage_models::UsageKey;
use crate::error::AppError;

/// Sweep the per-caller slot map once it grows past this many entries.
const SLOT_MAP_SWEEP_THRESHOLD: usize = 1_024;

static GUARD: OnceLock<LlmGuard> = OnceLock::new();

/// Held for the duration of an LLM call; releases the caller's slot on drop.
pub struct Admission {
    _permit: Option<OwnedSemaphorePermit>,
}

/// Caps concurrent LLM calls per caller and total tokens per day.
pub struct LlmGuard {
    /// `0` disables the concurrency cap.
    max_concurrent_per_caller: usize,
    slots: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// `0` disables the daily ceiling.
    daily_token_cap: u64,
    /// `(UTC day, tokens counted that day)`.
    day: Mutex<(NaiveDate, u64)>,
}

impl LlmGuard {
    pub fn new(max_concurrent_per_caller: usize, daily_token_cap: u64, today: NaiveDate) -> Self {
        Self {
            max_concurrent_per_caller,
            slots: Mutex::new(HashMap::new()),
            daily_token_cap,
            day: Mutex::new((today, 0)),
        }
    }

    /// Admit an LLM call, or explain why not.
    pub fn admit(&self, key: &UsageKey, today: NaiveDate) -> Result<Admission, AppError> {
        if self.daily_cap_reached(today) {
            metrics::counter!("lekton_llm_daily_cap_rejections_total").increment(1);
            return Err(AppError::TooManyRequests(
                "The daily AI usage limit for this instance has been reached. \
                 Please try again tomorrow."
                    .into(),
            ));
        }

        Ok(Admission {
            _permit: self.acquire_slot(key)?,
        })
    }

    /// Count tokens against today's ceiling, rolling over at UTC midnight.
    pub fn count_tokens(&self, tokens: u64, today: NaiveDate) {
        if self.daily_token_cap == 0 {
            return;
        }
        let mut day = self.day.lock().expect("daily token counter poisoned");
        if day.0 != today {
            *day = (today, 0);
        }
        let before = day.1;
        day.1 = day.1.saturating_add(tokens);

        // Log exactly once, on the call that crosses the line, so the operator
        // sees when it happened rather than a flood of identical errors.
        if before < self.daily_token_cap && day.1 >= self.daily_token_cap {
            tracing::error!(
                cap = self.daily_token_cap,
                counted = day.1,
                "daily LLM token ceiling reached — AI features are refusing new calls until \
                 UTC midnight"
            );
        }
    }

    fn daily_cap_reached(&self, today: NaiveDate) -> bool {
        if self.daily_token_cap == 0 {
            return false;
        }
        let day = self.day.lock().expect("daily token counter poisoned");
        day.0 == today && day.1 >= self.daily_token_cap
    }

    fn acquire_slot(&self, key: &UsageKey) -> Result<Option<OwnedSemaphorePermit>, AppError> {
        if self.max_concurrent_per_caller == 0 {
            return Ok(None);
        }

        let semaphore = {
            let mut slots = self.slots.lock().expect("LLM slot map poisoned");
            if slots.len() > SLOT_MAP_SWEEP_THRESHOLD {
                // An entry is live exactly while a permit holds an Arc clone.
                slots.retain(|_, semaphore| Arc::strong_count(semaphore) > 1);
            }
            slots
                .entry(slot_id(key))
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_concurrent_per_caller)))
                .clone()
        };

        semaphore.try_acquire_owned().map(Some).map_err(|_| {
            metrics::counter!("lekton_llm_concurrency_rejections_total").increment(1);
            AppError::TooManyRequests(match self.max_concurrent_per_caller {
                1 => "You already have an AI request running. Wait for it to finish and retry."
                    .to_string(),
                n => format!(
                    "You already have {n} AI requests running. \
                     Wait for one to finish and retry."
                ),
            })
        })
    }
}

/// Slot identity. Anonymous callers deliberately share one bucket: without an
/// account there is nothing to tell them apart, and letting each unidentified
/// request have its own slot would defeat the cap entirely.
fn slot_id(key: &UsageKey) -> String {
    match key.id() {
        Some(id) => format!("{}:{id}", key.kind()),
        None => key.kind().to_string(),
    }
}

/// Register the process-wide guard. Call once at startup.
pub fn install(guard: LlmGuard) {
    if GUARD.set(guard).is_err() {
        tracing::warn!("LLM guard already installed — ignoring");
    }
}

/// Admit an LLM call against the process-wide guard.
///
/// Without an installed guard every call is admitted, which is what the tests
/// and the eval binaries want.
pub fn admit(key: &UsageKey) -> Result<Admission, AppError> {
    match GUARD.get() {
        Some(guard) => guard.admit(key, chrono::Utc::now().date_naive()),
        None => Ok(Admission { _permit: None }),
    }
}

/// Count tokens against the process-wide daily ceiling.
pub fn count_tokens(tokens: u64) {
    if let Some(guard) = GUARD.get() {
        guard.count_tokens(tokens, chrono::Utc::now().date_naive());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
    }

    fn user(id: &str) -> UsageKey {
        UsageKey::User(id.into())
    }

    #[test]
    fn admits_up_to_the_concurrency_cap_then_refuses() {
        let guard = LlmGuard::new(2, 0, today());

        let _first = guard.admit(&user("u1"), today()).expect("first admitted");
        let _second = guard.admit(&user("u1"), today()).expect("second admitted");

        let third = guard.admit(&user("u1"), today());
        assert!(matches!(third, Err(AppError::TooManyRequests(_))));
    }

    #[test]
    fn releases_the_slot_when_the_admission_is_dropped() {
        let guard = LlmGuard::new(1, 0, today());

        let first = guard.admit(&user("u1"), today()).expect("admitted");
        assert!(guard.admit(&user("u1"), today()).is_err());

        drop(first);
        assert!(guard.admit(&user("u1"), today()).is_ok());
    }

    #[test]
    fn callers_do_not_consume_each_others_slots() {
        let guard = LlmGuard::new(1, 0, today());

        let _mine = guard.admit(&user("u1"), today()).expect("admitted");

        assert!(guard.admit(&user("u2"), today()).is_ok());
    }

    #[test]
    fn zero_disables_the_concurrency_cap() {
        let guard = LlmGuard::new(0, 0, today());

        let _a = guard.admit(&user("u1"), today()).expect("admitted");
        let _b = guard.admit(&user("u1"), today()).expect("admitted");
        assert!(guard.admit(&user("u1"), today()).is_ok());
    }

    #[test]
    fn refuses_once_the_daily_ceiling_is_reached() {
        let guard = LlmGuard::new(0, 100, today());

        guard.count_tokens(99, today());
        assert!(guard.admit(&user("u1"), today()).is_ok());

        guard.count_tokens(1, today());
        assert!(matches!(
            guard.admit(&user("u1"), today()),
            Err(AppError::TooManyRequests(_))
        ));
    }

    #[test]
    fn the_daily_ceiling_rolls_over_at_utc_midnight() {
        let guard = LlmGuard::new(0, 100, today());
        guard.count_tokens(500, today());
        assert!(guard.admit(&user("u1"), today()).is_err());

        let tomorrow = NaiveDate::from_ymd_opt(2026, 8, 2).unwrap();
        assert!(
            guard.admit(&user("u1"), tomorrow).is_ok(),
            "yesterday's overspend must not block today"
        );
    }

    #[test]
    fn zero_disables_the_daily_ceiling() {
        let guard = LlmGuard::new(0, 0, today());

        guard.count_tokens(u64::MAX, today());

        assert!(guard.admit(&user("u1"), today()).is_ok());
    }

    #[test]
    fn anonymous_callers_share_one_slot() {
        // Otherwise every unidentified request would get its own cap, which is
        // no cap at all.
        let guard = LlmGuard::new(1, 0, today());

        let _first = guard
            .admit(&UsageKey::Anonymous, today())
            .expect("admitted");

        assert!(guard.admit(&UsageKey::Anonymous, today()).is_err());
    }
}
