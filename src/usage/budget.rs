//! Per-caller spending budget.
//!
//! # Reserve to admit, charge to account
//!
//! A call's real cost is only known once it finishes, so the budget does two
//! separate things:
//!
//! - **admission** reserves an *estimate* before the call, which is what stops
//!   several concurrent calls from each seeing a full bucket and collectively
//!   overspending;
//! - **accounting** charges the *actual* cost from [`super::record`], once the
//!   provider has reported it.
//!
//! The reservation is then handed back unconditionally when the call ends. It
//! is a lock, not a payment: the only thing that permanently leaves the bucket
//! is the real cost. That separation is what makes the failure modes safe — an
//! abandoned stream, a provider error or a panic all release the reservation
//! and charge nothing, whereas an estimate that doubled as the charge would
//! have to be reconciled on every one of those paths.
//!
//! Because the refund runs from `Drop`, which cannot await, it is spawned. A
//! refund lost to a process kill costs the caller one estimate's worth of
//! budget, which the bucket refills on its own.

use std::sync::{Arc, OnceLock};

use crate::config::{BudgetConfig, BudgetProfile};
use crate::db::budget_repository::BudgetRepository;
use crate::db::usage_models::UsageKey;
use crate::error::AppError;

/// Credits reserved for a call whose cost is not yet known.
///
/// Deliberately coarse: it only has to be large enough that concurrent calls
/// cannot slip past the check together, and it is handed straight back.
const RESERVATION_ESTIMATE: f64 = 5.0;

/// Ceiling on the reservation, as a fraction of the caller's bucket.
///
/// A flat estimate is a floor under the balance: a caller is refused while
/// still holding it, because that much must be free to reserve. Against a
/// production-sized bucket that is a rounding error, but a small plan would
/// lose a third of its budget to it — observed live, where a 12-credit bucket
/// started refusing at 4.5 remaining.
const MAX_RESERVATION_FRACTION: f64 = 0.05;

/// What to hold for one call against a bucket of `capacity`.
fn reservation_for(capacity: f64) -> f64 {
    RESERVATION_ESTIMATE.min(capacity * MAX_RESERVATION_FRACTION)
}

static BUDGETS: OnceLock<Budgets> = OnceLock::new();

/// Register the process-wide budget. Call once at startup, only when
/// `usage.budget.enabled` is set; with none installed nothing is enforced.
pub fn install(budgets: Budgets) {
    if BUDGETS.set(budgets).is_err() {
        tracing::warn!("budgets already installed — ignoring");
    }
}

/// Reserve against the process-wide budget, if one is enforced.
pub async fn reserve(key: &UsageKey, plan: Option<&str>) -> Result<Option<BudgetHold>, AppError> {
    match BUDGETS.get() {
        Some(budgets) => budgets.reserve(key, plan).await.map(Some),
        None => Ok(None),
    }
}

/// Names of the configured plans, sorted, for the admin UI. Empty when no
/// budget is enforced.
pub fn plan_names() -> Vec<String> {
    BUDGETS.get().map(Budgets::plan_names).unwrap_or_default()
}

/// A caller's current standing, or `None` when no budget is enforced.
pub async fn snapshot(key: &UsageKey, plan: Option<&str>) -> Result<Option<Snapshot>, AppError> {
    match BUDGETS.get() {
        Some(budgets) => budgets.snapshot(key, plan).await.map(Some),
        None => Ok(None),
    }
}

/// Charge real spend against the process-wide budget, if one is enforced.
pub async fn charge(key: &UsageKey, credits: f64) {
    if let Some(budgets) = BUDGETS.get() {
        budgets.charge(key, credits).await;
    }
}

/// Live budget enforcement.
pub struct Budgets {
    repo: Arc<dyn BudgetRepository>,
    config: BudgetConfig,
}

/// Fraction of a bucket below which a call is served in thrifty mode.
///
/// Degrading before refusing keeps someone working — a plainer answer beats a
/// 429 — and buys back the headroom that would otherwise run out mid-task.
pub const THRIFTY_BELOW: f64 = 0.2;

/// A held reservation. Refunds itself when dropped.
pub struct BudgetHold {
    repo: Arc<dyn BudgetRepository>,
    key: String,
    credits: f64,
    capacity: f64,
    balance: f64,
}

impl BudgetHold {
    /// Fraction of the bucket still available, `0.0..=1.0`.
    pub fn headroom(&self) -> f64 {
        if self.capacity <= 0.0 {
            return 1.0;
        }
        (self.balance / self.capacity).clamp(0.0, 1.0)
    }

    /// Whether this call should economise rather than run the full pipeline.
    pub fn thrifty(&self) -> bool {
        self.headroom() < THRIFTY_BELOW
    }
}

impl Drop for BudgetHold {
    fn drop(&mut self) {
        if self.credits <= 0.0 {
            return;
        }
        let (repo, key, credits, capacity) = (
            self.repo.clone(),
            std::mem::take(&mut self.key),
            self.credits,
            self.capacity,
        );
        tokio::spawn(async move {
            if let Err(e) = repo.release(&key, credits, capacity).await {
                tracing::error!(key = %key, error = %e, "failed to release a budget reservation");
            }
        });
    }
}

impl Budgets {
    pub fn new(repo: Arc<dyn BudgetRepository>, config: BudgetConfig) -> Self {
        Self { repo, config }
    }

    /// The bucket that applies to a caller.
    ///
    /// `plan` is the name of an entry in `[usage.budget.plans]`, assigned to
    /// the user by an administrator. A name that no longer exists in the config
    /// falls back to the default rather than failing: a plan removed from the
    /// config should not lock its holders out.
    pub fn profile(&self, key: &UsageKey, plan: Option<&str>) -> BudgetProfile {
        match key {
            UsageKey::ServiceToken(_) => self.config.service_token.unwrap_or(self.config.default),
            UsageKey::Anonymous => self.config.anonymous.unwrap_or(self.config.default),
            // Background work has no bucket; it is bounded by the daily ceiling.
            UsageKey::System => self.config.default,
            UsageKey::User(_) => plan
                .and_then(|name| {
                    let found = self.config.plans.get(name);
                    if found.is_none() {
                        tracing::warn!(
                            plan = %name,
                            "user has an AI plan that is not in [usage.budget.plans] — \
                             falling back to the default budget"
                        );
                    }
                    found
                })
                .copied()
                .unwrap_or(self.config.default),
        }
    }

    /// Reserve enough to admit one call, or explain the wait.
    pub async fn reserve(
        &self,
        key: &UsageKey,
        plan: Option<&str>,
    ) -> Result<BudgetHold, AppError> {
        let profile = self.profile(key, plan);
        let bucket = bucket_id(key);

        let estimate = reservation_for(profile.capacity);
        let reservation = self
            .repo
            .reserve(&bucket, estimate, profile.capacity, profile.refill_per_hour)
            .await?;

        if !reservation.granted {
            metrics::counter!("lekton_llm_budget_rejections_total").increment(1);
            return Err(AppError::TooManyRequests(format!(
                "You have used your AI budget for now. It refills in about {}.",
                humanise_wait(estimate - reservation.balance, profile.refill_per_hour)
            )));
        }

        Ok(BudgetHold {
            repo: self.repo.clone(),
            key: bucket,
            credits: reservation.reserved,
            capacity: profile.capacity,
            balance: reservation.balance,
        })
    }

    /// Plan names, sorted so the admin UI is stable between reloads.
    pub fn plan_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.config.plans.keys().cloned().collect();
        names.sort();
        names
    }

    /// A caller's current standing.
    pub async fn snapshot(&self, key: &UsageKey, plan: Option<&str>) -> Result<Snapshot, AppError> {
        let profile = self.profile(key, plan);
        // A caller who has never spent has no document; they are full, which is
        // what `reserve` assumes too.
        let balance = self
            .repo
            .balance(&bucket_id(key))
            .await?
            .unwrap_or(profile.capacity);
        let headroom = if profile.capacity <= 0.0 {
            1.0
        } else {
            (balance / profile.capacity).clamp(0.0, 1.0)
        };
        Ok(Snapshot {
            balance,
            capacity: profile.capacity,
            headroom,
            thrifty: headroom < THRIFTY_BELOW,
        })
    }

    /// Charge a call's real cost.
    pub async fn charge(&self, key: &UsageKey, credits: f64) {
        if credits <= 0.0 {
            return;
        }
        // The charge lands on whichever bucket the caller owns; the profile is
        // only needed for the capacity clamp, and every plan clamps the same
        // bucket, so the default's capacity is the right ceiling to pass.
        let profile = self.profile(key, None);
        if let Err(e) = self
            .repo
            .release(&bucket_id(key), -credits, profile.capacity)
            .await
        {
            tracing::error!(error = %e, "failed to charge LLM spend to a budget");
        }
    }
}

/// What a caller has left to spend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    pub balance: f64,
    pub capacity: f64,
    /// Fraction of the bucket still available, `0.0..=1.0`.
    pub headroom: f64,
    /// Whether the next call will be served in thrifty mode.
    pub thrifty: bool,
}

/// Bucket identity, matching the concurrency slot's shape: anonymous callers
/// share one bucket because there is nothing to tell them apart.
fn bucket_id(key: &UsageKey) -> String {
    match key.id() {
        Some(id) => format!("{}:{id}", key.kind()),
        None => key.kind().to_string(),
    }
}

/// Turn a credit shortfall into the wait a person can act on.
fn humanise_wait(shortfall: f64, refill_per_hour: f64) -> String {
    if refill_per_hour <= 0.0 {
        return "a while".to_string();
    }
    let minutes = (shortfall / refill_per_hour * 60.0).ceil().max(1.0) as u64;
    match minutes {
        0..=1 => "a minute".to_string(),
        2..=59 => format!("{minutes} minutes"),
        60..=119 => "an hour".to_string(),
        _ => format!("{} hours", minutes / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::budget_repository::Reservation;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRepo {
        granted: bool,
        calls: Mutex<Vec<(String, f64)>>,
    }

    #[async_trait::async_trait]
    impl BudgetRepository for FakeRepo {
        async fn reserve(
            &self,
            key: &str,
            credits: f64,
            _capacity: f64,
            _refill_per_hour: f64,
        ) -> Result<Reservation, AppError> {
            self.calls.lock().unwrap().push((key.to_string(), credits));
            Ok(Reservation {
                granted: self.granted,
                reserved: if self.granted { credits } else { 0.0 },
                balance: if self.granted { 10.0 } else { 1.0 },
            })
        }
        async fn release(&self, key: &str, credits: f64, _capacity: f64) -> Result<(), AppError> {
            self.calls.lock().unwrap().push((key.to_string(), credits));
            Ok(())
        }
        async fn balance(&self, _key: &str) -> Result<Option<f64>, AppError> {
            Ok(None)
        }
    }

    fn config_with_plans() -> BudgetConfig {
        BudgetConfig {
            enabled: true,
            default: BudgetProfile {
                capacity: 100.0,
                refill_per_hour: 50.0,
            },
            plans: HashMap::from([
                (
                    "heavy".to_string(),
                    BudgetProfile {
                        capacity: 500.0,
                        refill_per_hour: 250.0,
                    },
                ),
                (
                    "light".to_string(),
                    BudgetProfile {
                        capacity: 20.0,
                        refill_per_hour: 10.0,
                    },
                ),
            ]),
            service_token: Some(BudgetProfile {
                capacity: 5_000.0,
                refill_per_hour: 1_000.0,
            }),
            anonymous: None,
        }
    }

    fn budgets(granted: bool) -> Budgets {
        Budgets::new(
            Arc::new(FakeRepo {
                granted,
                calls: Mutex::new(Vec::new()),
            }),
            config_with_plans(),
        )
    }

    #[test]
    fn a_user_with_no_plan_gets_the_default() {
        let profile = budgets(true).profile(&UsageKey::User("u".into()), None);

        assert_eq!(profile.capacity, 100.0);
    }

    #[test]
    fn a_named_plan_is_applied() {
        let profile = budgets(true).profile(&UsageKey::User("u".into()), Some("heavy"));

        assert_eq!(profile.capacity, 500.0);
    }

    #[test]
    fn a_plan_removed_from_the_config_falls_back_rather_than_locking_out() {
        // Deleting a plan from the config must not strand the users holding it.
        let profile = budgets(true).profile(&UsageKey::User("u".into()), Some("retired-plan"));

        assert_eq!(profile.capacity, 100.0);
    }

    #[test]
    fn a_plan_is_independent_of_what_the_user_may_read() {
        // The point of the split: a light plan can belong to someone who sees
        // everything, and a heavy plan to someone restricted to public docs.
        let budgets = budgets(true);

        assert!(
            budgets
                .profile(&UsageKey::User("exec".into()), Some("light"))
                .capacity
                < budgets
                    .profile(&UsageKey::User("support".into()), Some("heavy"))
                    .capacity
        );
    }

    #[test]
    fn machine_tokens_get_their_own_profile() {
        let profile = budgets(true).profile(&UsageKey::ServiceToken("t".into()), None);

        assert_eq!(profile.capacity, 5_000.0);
    }

    #[test]
    fn anonymous_falls_back_to_the_default_when_unset() {
        let profile = budgets(true).profile(&UsageKey::Anonymous, None);

        assert_eq!(profile.capacity, 100.0);
    }

    #[tokio::test]
    async fn a_refused_reservation_explains_the_wait() {
        let Err(error) = budgets(false)
            .reserve(&UsageKey::User("u".into()), None)
            .await
        else {
            panic!("an exhausted budget must refuse");
        };

        let AppError::TooManyRequests(message) = error else {
            panic!("expected a throttling error, got {error:?}");
        };
        // Pin the number, not just the shape. An earlier version of this test
        // only checked that the sentence was there, and missed the message
        // quoting the flat estimate instead of the scaled one — telling a
        // caller to wait five hours for a thirty-minute refill.
        //
        // Capacity 100 caps the reservation at 5; the fake reports 1 credit
        // left, so 4 are owed at 50/hour — just under five minutes.
        assert_eq!(
            message,
            "You have used your AI budget for now. It refills in about 5 minutes."
        );
    }

    #[tokio::test]
    async fn dropping_a_hold_returns_the_reservation() {
        let repo = Arc::new(FakeRepo {
            granted: true,
            calls: Mutex::new(Vec::new()),
        });
        let budgets = Budgets::new(repo.clone(), config_with_plans());

        {
            let _hold = budgets
                .reserve(&UsageKey::User("u1".into()), None)
                .await
                .expect("granted");
        }
        // The refund is spawned, so let the runtime run it.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let calls = repo.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            2,
            "expected a reserve and a release: {calls:?}"
        );
        assert_eq!(calls[1], ("user:u1".to_string(), reservation_for(100.0)));
    }

    #[tokio::test]
    async fn a_full_bucket_is_not_thrifty() {
        let hold = budgets(true)
            .reserve(&UsageKey::User("u".into()), None)
            .await
            .expect("granted");

        // The fake repo reports a balance of 10 against a capacity of 100.
        assert!((hold.headroom() - 0.1).abs() < 1e-9, "{}", hold.headroom());
    }

    #[test]
    fn thrifty_kicks_in_below_the_threshold_and_not_above() {
        let hold = |balance: f64| BudgetHold {
            repo: Arc::new(FakeRepo {
                granted: true,
                calls: Mutex::new(Vec::new()),
            }),
            key: "user:u".into(),
            credits: 0.0,
            capacity: 100.0,
            balance,
        };

        assert!(!hold(100.0).thrifty(), "a full bucket runs everything");
        assert!(
            !hold(20.0).thrifty(),
            "exactly at the threshold is not below it"
        );
        assert!(hold(19.0).thrifty(), "just under must degrade");
        assert!(hold(0.0).thrifty());
    }

    #[test]
    fn headroom_is_full_when_there_is_no_capacity_to_divide_by() {
        let hold = BudgetHold {
            repo: Arc::new(FakeRepo::default()),
            key: "user:u".into(),
            credits: 0.0,
            capacity: 0.0,
            balance: 0.0,
        };

        // A zero capacity means the budget is not really configured; degrading
        // every call would be worse than degrading none.
        assert_eq!(hold.headroom(), 1.0);
        assert!(!hold.thrifty());
    }

    #[test]
    fn the_reservation_never_swallows_a_small_budget() {
        // A flat estimate is a floor under the balance, so it has to scale with
        // the bucket: otherwise a small plan is refused while still a third full.
        assert!((reservation_for(12.0) - 0.6).abs() < 1e-9);
        // Large buckets keep the flat estimate — 5 credits of 1000 is noise.
        assert_eq!(reservation_for(1_000.0), RESERVATION_ESTIMATE);
    }

    #[test]
    fn the_wait_is_expressed_in_units_a_person_can_act_on() {
        assert_eq!(humanise_wait(25.0, 50.0), "30 minutes");
        assert_eq!(humanise_wait(50.0, 50.0), "an hour");
        assert_eq!(humanise_wait(500.0, 50.0), "10 hours");
        // A rate of zero means it never refills on its own.
        assert_eq!(humanise_wait(1.0, 0.0), "a while");
    }
}
