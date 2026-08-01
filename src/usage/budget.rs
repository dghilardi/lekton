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

static BUDGETS: OnceLock<Budgets> = OnceLock::new();

/// Register the process-wide budget. Call once at startup, only when
/// `usage.budget.enabled` is set; with none installed nothing is enforced.
pub fn install(budgets: Budgets) {
    if BUDGETS.set(budgets).is_err() {
        tracing::warn!("budgets already installed — ignoring");
    }
}

/// Reserve against the process-wide budget, if one is enforced.
pub async fn reserve(
    key: &UsageKey,
    access_levels: &[String],
) -> Result<Option<BudgetHold>, AppError> {
    match BUDGETS.get() {
        Some(budgets) => budgets.reserve(key, access_levels).await.map(Some),
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

/// A held reservation. Refunds itself when dropped.
pub struct BudgetHold {
    repo: Arc<dyn BudgetRepository>,
    key: String,
    credits: f64,
    capacity: f64,
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
    pub fn profile(&self, key: &UsageKey, access_levels: &[String]) -> BudgetProfile {
        match key {
            UsageKey::ServiceToken(_) => self.config.service_token.unwrap_or(self.config.default),
            UsageKey::Anonymous => self.config.anonymous.unwrap_or(self.config.default),
            // Background work has no bucket; it is bounded by the daily ceiling.
            UsageKey::System => self.config.default,
            UsageKey::User(_) => access_levels
                .iter()
                .filter_map(|level| self.config.per_access_level.get(level))
                .copied()
                // Most generous wins, so granting a level never costs budget.
                .max_by(|a, b| a.capacity.total_cmp(&b.capacity))
                .unwrap_or(self.config.default),
        }
    }

    /// Reserve enough to admit one call, or explain the wait.
    pub async fn reserve(
        &self,
        key: &UsageKey,
        access_levels: &[String],
    ) -> Result<BudgetHold, AppError> {
        let profile = self.profile(key, access_levels);
        let bucket = bucket_id(key);

        let reservation = self
            .repo
            .reserve(
                &bucket,
                RESERVATION_ESTIMATE,
                profile.capacity,
                profile.refill_per_hour,
            )
            .await?;

        if !reservation.granted {
            metrics::counter!("lekton_llm_budget_rejections_total").increment(1);
            return Err(AppError::TooManyRequests(format!(
                "You have used your AI budget for now. It refills in about {}.",
                humanise_wait(
                    RESERVATION_ESTIMATE - reservation.balance,
                    profile.refill_per_hour
                )
            )));
        }

        Ok(BudgetHold {
            repo: self.repo.clone(),
            key: bucket,
            credits: reservation.reserved,
            capacity: profile.capacity,
        })
    }

    /// Charge a call's real cost.
    pub async fn charge(&self, key: &UsageKey, credits: f64) {
        if credits <= 0.0 {
            return;
        }
        let profile = self.profile(key, &[]);
        if let Err(e) = self
            .repo
            .release(&bucket_id(key), -credits, profile.capacity)
            .await
        {
            tracing::error!(error = %e, "failed to charge LLM spend to a budget");
        }
    }
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

    fn config_with_levels() -> BudgetConfig {
        BudgetConfig {
            enabled: true,
            default: BudgetProfile {
                capacity: 100.0,
                refill_per_hour: 50.0,
            },
            per_access_level: HashMap::from([
                (
                    "power-user".to_string(),
                    BudgetProfile {
                        capacity: 500.0,
                        refill_per_hour: 250.0,
                    },
                ),
                (
                    "guest".to_string(),
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
            config_with_levels(),
        )
    }

    #[test]
    fn a_user_without_a_matching_level_gets_the_default() {
        let profile = budgets(true).profile(&UsageKey::User("u".into()), &["other".into()]);

        assert_eq!(profile.capacity, 100.0);
    }

    #[test]
    fn several_levels_resolve_to_the_most_generous() {
        // Holding a restrictive level as well as a generous one must not
        // penalise the user: permissions accumulate, they do not subtract.
        let profile = budgets(true).profile(
            &UsageKey::User("u".into()),
            &["guest".into(), "power-user".into()],
        );

        assert_eq!(profile.capacity, 500.0);
    }

    #[test]
    fn machine_tokens_get_their_own_profile() {
        let profile = budgets(true).profile(&UsageKey::ServiceToken("t".into()), &[]);

        assert_eq!(profile.capacity, 5_000.0);
    }

    #[test]
    fn anonymous_falls_back_to_the_default_when_unset() {
        let profile = budgets(true).profile(&UsageKey::Anonymous, &[]);

        assert_eq!(profile.capacity, 100.0);
    }

    #[tokio::test]
    async fn a_refused_reservation_explains_the_wait() {
        let Err(error) = budgets(false)
            .reserve(&UsageKey::User("u".into()), &[])
            .await
        else {
            panic!("an exhausted budget must refuse");
        };

        let AppError::TooManyRequests(message) = error else {
            panic!("expected a throttling error, got {error:?}");
        };
        assert!(
            message.contains("refills in about"),
            "the caller needs to know when to come back: {message}"
        );
    }

    #[tokio::test]
    async fn dropping_a_hold_returns_the_reservation() {
        let repo = Arc::new(FakeRepo {
            granted: true,
            calls: Mutex::new(Vec::new()),
        });
        let budgets = Budgets::new(repo.clone(), config_with_levels());

        {
            let _hold = budgets
                .reserve(&UsageKey::User("u1".into()), &[])
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
        assert_eq!(calls[1], ("user:u1".to_string(), RESERVATION_ESTIMATE));
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
