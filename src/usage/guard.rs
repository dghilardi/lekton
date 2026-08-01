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
//! - **A daily instance-wide spend ceiling.** The last line of defence, aimed
//!   at the case no per-caller limit catches: a runaway reindex, or a bug that
//!   fans out across many callers. Background work is cut off at a fraction of
//!   the ceiling so that it starves before people do — see
//!   [`SYSTEM_SHARE_OF_CEILING`].
//!
//! Enforcement lives here rather than in an HTTP layer because the same
//! services are reached from REST handlers, Leptos server functions and MCP —
//! a middleware would cover them unevenly.
//!
//! The ceiling is denominated in credits, not tokens, so that it means the
//! same thing across models — see [`super::pricing`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::NaiveDate;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::db::usage_models::UsageKey;
use crate::error::AppError;

/// Sweep the per-caller slot map once it grows past this many entries.
const SLOT_MAP_SWEEP_THRESHOLD: usize = 1_024;

/// Share of the daily ceiling that background work may consume before it is
/// refused, leaving the rest for people.
///
/// A single shared ceiling gets the priority backwards: an overrunning reindex
/// spends it all and the resulting refusals land on users, while the reindex —
/// which is never itself admitted — carries on. Cutting background work off
/// early keeps a reserve for the interactive path, which is the one someone is
/// waiting on.
const SYSTEM_SHARE_OF_CEILING: f64 = 0.8;

static GUARD: OnceLock<LlmGuard> = OnceLock::new();

/// Held for the duration of an LLM call; releases the caller's slot — and any
/// budget reservation — on drop.
pub struct Admission {
    _permit: Option<OwnedSemaphorePermit>,
    budget: Option<super::budget::BudgetHold>,
}

impl Admission {
    /// Whether this call should economise: the caller is near the bottom of
    /// their budget, so a cheaper answer now beats a refusal shortly.
    ///
    /// False when no budget is enforced — nothing to economise against.
    pub fn thrifty(&self) -> bool {
        self.budget.as_ref().is_some_and(|hold| hold.thrifty())
    }
}

/// Caps concurrent LLM calls per caller and total tokens per day.
pub struct LlmGuard {
    /// `0` disables the concurrency cap.
    max_concurrent_per_caller: usize,
    slots: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// `0.0` disables the daily ceiling.
    daily_credit_cap: f64,
    /// `(UTC day, credits spent that day, of which by background work)`.
    ///
    /// The ceiling is checked against the total — it bounds what the instance
    /// spends, not what any one caller spends. The background figure is kept
    /// alongside so a long-running job can measure its own cost without
    /// counting everyone else's chat.
    day: Mutex<DailySpend>,
}

/// Credits spent on one UTC day.
#[derive(Debug, Clone, Copy)]
struct DailySpend {
    date: NaiveDate,
    total: f64,
    system: f64,
}

impl DailySpend {
    fn new(date: NaiveDate) -> Self {
        Self {
            date,
            total: 0.0,
            system: 0.0,
        }
    }
}

impl LlmGuard {
    pub fn new(max_concurrent_per_caller: usize, daily_credit_cap: f64, today: NaiveDate) -> Self {
        Self {
            max_concurrent_per_caller,
            slots: Mutex::new(HashMap::new()),
            daily_credit_cap,
            day: Mutex::new(DailySpend::new(today)),
        }
    }

    /// Admit an LLM call, or explain why not.
    pub fn admit(&self, key: &UsageKey, today: NaiveDate) -> Result<Admission, AppError> {
        if self.ceiling_reached_for(key, today) {
            metrics::counter!(
                "lekton_llm_daily_cap_rejections_total",
                "actor_kind" => key.kind(),
            )
            .increment(1);
            return Err(AppError::TooManyRequests(match key {
                UsageKey::System => "The daily AI spending reserve for background work is used \
                                     up. Indexing resumes after UTC midnight."
                    .to_string(),
                _ => "The daily AI usage limit for this instance has been reached. \
                      Please try again tomorrow."
                    .to_string(),
            }));
        }

        Ok(Admission {
            // Background work is bounded by its own queue, and has no peer to
            // be fair to, so it takes no per-caller slot.
            _permit: match key {
                UsageKey::System => None,
                _ => self.acquire_slot(key)?,
            },
            budget: None,
        })
    }

    /// Charge credits against today's ceiling, rolling over at UTC midnight.
    ///
    /// Every caller's spend counts toward the same total; `key` only decides
    /// which threshold the crossing message refers to.
    pub fn spend(&self, key: &UsageKey, credits: f64, today: NaiveDate) {
        if self.daily_credit_cap <= 0.0 {
            return;
        }
        let mut day = self.day.lock().expect("daily spend counter poisoned");
        if day.date != today {
            *day = DailySpend::new(today);
        }
        let before = day.total;
        day.total += credits;
        if matches!(key, UsageKey::System) {
            day.system += credits;
        }

        // Log exactly once, on the call that crosses the line, so the operator
        // sees when it happened rather than a flood of identical errors. Both
        // thresholds are worth a line: the first says indexing has stopped, the
        // second that everything has.
        let system_ceiling = self.daily_credit_cap * SYSTEM_SHARE_OF_CEILING;
        if before < system_ceiling && day.total >= system_ceiling {
            tracing::warn!(
                ceiling = system_ceiling,
                spent = day.total,
                actor_kind = key.kind(),
                "daily AI spend reserve for background work is used up — indexing pauses until \
                 UTC midnight, interactive use continues"
            );
        }
        if before < self.daily_credit_cap && day.total >= self.daily_credit_cap {
            tracing::error!(
                cap = self.daily_credit_cap,
                spent = day.total,
                "daily LLM spend ceiling reached — AI features are refusing new calls until \
                 UTC midnight"
            );
        }
    }

    /// The ceiling this caller is held to: background work stops early, so the
    /// remainder is still there for whoever is waiting on an answer.
    fn ceiling_for(&self, key: &UsageKey) -> f64 {
        match key {
            UsageKey::System => self.daily_credit_cap * SYSTEM_SHARE_OF_CEILING,
            _ => self.daily_credit_cap,
        }
    }

    fn ceiling_reached_for(&self, key: &UsageKey, today: NaiveDate) -> bool {
        if self.daily_credit_cap <= 0.0 {
            return false;
        }
        let day = self.day.lock().expect("daily spend counter poisoned");
        day.date == today && day.total >= self.ceiling_for(key)
    }

    /// Whether a call from this caller would be admitted by the ceiling.
    ///
    /// Lets a queue hold back rather than dequeue work it cannot pay for —
    /// asking first is the difference between pausing and losing the item.
    pub fn has_headroom(&self, key: &UsageKey, today: NaiveDate) -> bool {
        !self.ceiling_reached_for(key, today)
    }

    /// Credits background work has spent today, ignoring everyone else.
    pub fn system_spend(&self, today: NaiveDate) -> f64 {
        let day = self.day.lock().expect("daily spend counter poisoned");
        if day.date == today {
            day.system
        } else {
            0.0
        }
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

/// Without an installed guard every call is admitted, which is what the tests
/// and the eval binaries want.
/// Admit an LLM call: the instance ceiling, then the caller's concurrency slot,
/// then their budget — cheapest check first, so a refusal costs the least.
pub async fn admit(key: &UsageKey, plan: Option<&str>) -> Result<Admission, AppError> {
    let mut admission = match GUARD.get() {
        Some(guard) => guard.admit(key, chrono::Utc::now().date_naive())?,
        None => Admission {
            _permit: None,
            budget: None,
        },
    };
    admission.budget = super::budget::reserve(key, plan).await?;
    Ok(admission)
}

/// Whether the process-wide ceiling would currently admit this caller.
///
/// `true` with no guard installed: nothing is enforced, so nothing is held back.
pub fn has_headroom(key: &UsageKey) -> bool {
    GUARD
        .get()
        .is_none_or(|guard| guard.has_headroom(key, chrono::Utc::now().date_naive()))
}

/// Credits background work has spent today against the process-wide guard.
///
/// `0.0` with no guard installed, which is what the eval binaries and the tests
/// see.
pub fn system_spend_today() -> f64 {
    GUARD
        .get()
        .map(|guard| guard.system_spend(chrono::Utc::now().date_naive()))
        .unwrap_or_default()
}

/// Charge credits against the process-wide daily ceiling.
pub fn spend(key: &UsageKey, credits: f64) {
    if let Some(guard) = GUARD.get() {
        guard.spend(key, credits, chrono::Utc::now().date_naive());
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
        let guard = LlmGuard::new(2, 0.0, today());

        let _first = guard.admit(&user("u1"), today()).expect("first admitted");
        let _second = guard.admit(&user("u1"), today()).expect("second admitted");

        let third = guard.admit(&user("u1"), today());
        assert!(matches!(third, Err(AppError::TooManyRequests(_))));
    }

    #[test]
    fn releases_the_slot_when_the_admission_is_dropped() {
        let guard = LlmGuard::new(1, 0.0, today());

        let first = guard.admit(&user("u1"), today()).expect("admitted");
        assert!(guard.admit(&user("u1"), today()).is_err());

        drop(first);
        assert!(guard.admit(&user("u1"), today()).is_ok());
    }

    #[test]
    fn callers_do_not_consume_each_others_slots() {
        let guard = LlmGuard::new(1, 0.0, today());

        let _mine = guard.admit(&user("u1"), today()).expect("admitted");

        assert!(guard.admit(&user("u2"), today()).is_ok());
    }

    #[test]
    fn zero_disables_the_concurrency_cap() {
        let guard = LlmGuard::new(0, 0.0, today());

        let _a = guard.admit(&user("u1"), today()).expect("admitted");
        let _b = guard.admit(&user("u1"), today()).expect("admitted");
        assert!(guard.admit(&user("u1"), today()).is_ok());
    }

    #[test]
    fn background_work_is_starved_before_people_are() {
        // The defect this encodes: with one shared ceiling, an overrunning
        // reindex spent it all and the refusals landed on users, while the
        // reindex itself — never admitted — carried on.
        let guard = LlmGuard::new(0, 100.0, today());

        // 85 spent: past the background reserve (80), short of the ceiling.
        guard.spend(&UsageKey::System, 85.0, today());

        assert!(
            guard.admit(&UsageKey::System, today()).is_err(),
            "indexing must stop once it has eaten its share"
        );
        assert!(
            guard.admit(&user("u1"), today()).is_ok(),
            "a person must still be served from the reserve indexing left behind"
        );
    }

    #[test]
    fn headroom_answers_before_the_work_is_taken_off_the_queue() {
        let guard = LlmGuard::new(0, 100.0, today());
        assert!(guard.has_headroom(&UsageKey::System, today()));

        // Past the background reserve: indexing must hold back, while a person
        // is still served.
        guard.spend(&user("u1"), 85.0, today());

        assert!(!guard.has_headroom(&UsageKey::System, today()));
        assert!(guard.has_headroom(&user("u1"), today()));
    }

    #[test]
    fn background_spend_is_measured_apart_from_everyone_elses() {
        // A long-running index needs to know what *it* cost, not what the
        // instance cost while it happened to be running.
        let guard = LlmGuard::new(0, 100.0, today());

        guard.spend(&user("u1"), 30.0, today());
        guard.spend(&UsageKey::System, 7.0, today());
        guard.spend(&UsageKey::ServiceToken("t".into()), 5.0, today());

        assert_eq!(guard.system_spend(today()), 7.0);
    }

    #[test]
    fn background_spend_resets_with_the_day() {
        let guard = LlmGuard::new(0, 100.0, today());
        guard.spend(&UsageKey::System, 7.0, today());

        let tomorrow = NaiveDate::from_ymd_opt(2026, 8, 2).unwrap();

        assert_eq!(guard.system_spend(tomorrow), 0.0);
    }

    #[test]
    fn the_full_ceiling_still_stops_everyone() {
        let guard = LlmGuard::new(0, 100.0, today());

        guard.spend(&user("u1"), 100.0, today());

        assert!(guard.admit(&user("u2"), today()).is_err());
        assert!(guard.admit(&UsageKey::System, today()).is_err());
    }

    #[test]
    fn background_work_takes_no_per_caller_slot() {
        // It is bounded by its own queue and has no peer to be fair to; taking
        // a slot would only let one extraction block the next.
        let guard = LlmGuard::new(1, 0.0, today());

        let _first = guard.admit(&UsageKey::System, today()).expect("admitted");

        assert!(guard.admit(&UsageKey::System, today()).is_ok());
    }

    #[test]
    fn refuses_once_the_daily_ceiling_is_reached() {
        let guard = LlmGuard::new(0, 100.0, today());

        guard.spend(&user("u1"), 99.0, today());
        assert!(guard.admit(&user("u1"), today()).is_ok());

        guard.spend(&user("u1"), 1.0, today());
        assert!(matches!(
            guard.admit(&user("u1"), today()),
            Err(AppError::TooManyRequests(_))
        ));
    }

    #[test]
    fn the_daily_ceiling_rolls_over_at_utc_midnight() {
        let guard = LlmGuard::new(0, 100.0, today());
        guard.spend(&user("u1"), 500.0, today());
        assert!(guard.admit(&user("u1"), today()).is_err());

        let tomorrow = NaiveDate::from_ymd_opt(2026, 8, 2).unwrap();
        assert!(
            guard.admit(&user("u1"), tomorrow).is_ok(),
            "yesterday's overspend must not block today"
        );
    }

    #[test]
    fn zero_disables_the_daily_ceiling() {
        let guard = LlmGuard::new(0, 0.0, today());

        guard.spend(&user("u1"), f64::MAX, today());

        assert!(guard.admit(&user("u1"), today()).is_ok());
    }

    #[test]
    fn anonymous_callers_share_one_slot() {
        // Otherwise every unidentified request would get its own cap, which is
        // no cap at all.
        let guard = LlmGuard::new(1, 0.0, today());

        let _first = guard
            .admit(&UsageKey::Anonymous, today())
            .expect("admitted");

        assert!(guard.admit(&UsageKey::Anonymous, today()).is_err());
    }
}
