//! Per-caller credit budgets, as a token bucket in MongoDB.
//!
//! # Why a token bucket
//!
//! A sliding window needs either a log of every call (an insert and an
//! aggregate per LLM request) or a pair of counters plus a TTL sweep. A token
//! bucket is one document per caller, updated in place: it never grows, needs
//! no cleanup, allows the bursts a chat conversation naturally produces, and
//! yields the retry delay for free — `Retry-After` is just how long until the
//! shortfall is refilled.
//!
//! # Why reserve, then settle
//!
//! The cost of a call is only known once it finishes. Checking the balance
//! before the call and charging after it lets N concurrent calls all pass the
//! check and collectively overspend. So a call first *reserves* an estimate,
//! then *settles* the difference against the real cost.
//!
//! # The atomic refill
//!
//! Refill and reservation happen in a single update pipeline, so no read is
//! ever separated from its write: the pipeline recomputes the balance from
//! elapsed time, clamps it to capacity, and subtracts the reservation *only if
//! it fits*. A refused reservation therefore leaves the balance untouched and
//! carries no obligation to hand anything back.
//!
//! An earlier version subtracted unconditionally and expected the caller to
//! release a refusal. An integration test showed why that is a trap: forty
//! concurrent attempts on a hundred-credit bucket correctly granted ten, but
//! drove the stored balance to -300 in the meantime. Any caller that crashed
//! before releasing would have left a user locked out of a budget they had
//! never actually spent.

use async_trait::async_trait;

use crate::error::AppError;

/// Outcome of reserving credits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reservation {
    /// Whether the credits were actually held.
    pub granted: bool,
    /// Credits held. Zero when refused.
    pub reserved: f64,
    /// Balance after the attempt. Never negative.
    pub balance: f64,
}

#[async_trait]
pub trait BudgetRepository: Send + Sync {
    /// Refill from elapsed time and hold `credits`, in one atomic step.
    ///
    /// A refusal is a no-op on the balance, so nothing needs releasing.
    async fn reserve(
        &self,
        key: &str,
        credits: f64,
        capacity: f64,
        refill_per_hour: f64,
    ) -> Result<Reservation, AppError>;

    /// Return credits to a caller's balance, never exceeding `capacity`.
    ///
    /// Used both to settle a reservation against the real cost and to refund a
    /// call that never happened.
    async fn release(&self, key: &str, credits: f64, capacity: f64) -> Result<(), AppError>;

    /// Current balance, or `None` for a caller that has never spent.
    async fn balance(&self, key: &str) -> Result<Option<f64>, AppError>;
}

/// Read a stored credit amount, whatever numeric width it happens to have.
///
/// The app always writes doubles, but MongoDB does not enforce that and an
/// integral value arrives as an Int32 — which is what a shell edit produces.
/// A strict `get_f64` then fails, and the caller cannot tell "no budget
/// recorded" from "budget in an unexpected width": both look like a full
/// bucket. For a spending control that is the wrong way to fail.
#[cfg(feature = "ssr")]
fn credits_field(doc: &mongodb::bson::Document, key: &str) -> Option<f64> {
    use mongodb::bson::Bson;

    match doc.get(key)? {
        Bson::Double(n) => Some(*n),
        Bson::Int32(n) => Some(f64::from(*n)),
        Bson::Int64(n) => Some(*n as f64),
        _ => None,
    }
}

// ── MongoDB implementation ───────────────────────────────────────────────────

#[cfg(feature = "ssr")]
pub struct MongoBudgetRepository {
    budgets: mongodb::Collection<mongodb::bson::Document>,
}

#[cfg(feature = "ssr")]
impl MongoBudgetRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            budgets: db.collection("usage_budgets"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl BudgetRepository for MongoBudgetRepository {
    async fn reserve(
        &self,
        key: &str,
        credits: f64,
        capacity: f64,
        refill_per_hour: f64,
    ) -> Result<Reservation, AppError> {
        use mongodb::bson::doc;
        use mongodb::options::ReturnDocument;

        // A caller seen for the first time starts full: `$ifNull` supplies the
        // capacity and "now", so the first call neither fails nor gets a
        // windfall of accumulated refill.
        let refilled = doc! { "$min": [
            capacity,
            { "$add": [
                { "$ifNull": ["$balance", capacity] },
                { "$multiply": [
                    refill_per_hour,
                    { "$divide": [
                        { "$dateDiff": {
                            "startDate": { "$ifNull": ["$refilled_at", "$$NOW"] },
                            "endDate": "$$NOW",
                            "unit": "millisecond",
                        }},
                        3_600_000.0,
                    ]},
                ]},
            ]},
        ]};

        // Two stages: the first materialises the refilled balance so the second
        // can both test and spend it without recomputing the arithmetic.
        let pipeline = vec![
            doc! { "$set": { "refilled": refilled, "refilled_at": "$$NOW" } },
            doc! { "$set": {
                "granted": { "$gte": ["$refilled", credits] },
                "balance": { "$cond": [
                    { "$gte": ["$refilled", credits] },
                    { "$subtract": ["$refilled", credits] },
                    "$refilled",
                ]},
            }},
            doc! { "$unset": "refilled" },
        ];

        let updated = self
            .budgets
            .find_one_and_update(doc! { "_id": key }, pipeline)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await
            .map_err(|e| AppError::Internal(format!("mongo reserve usage_budget: {e}")))?
            .ok_or_else(|| {
                AppError::Internal("usage_budget reserve returned no document".to_string())
            })?;

        let balance = credits_field(&updated, "balance").ok_or_else(|| {
            AppError::Internal("usage_budget reserve returned no usable balance".to_string())
        })?;
        let granted = updated.get_bool("granted").unwrap_or(false);

        Ok(Reservation {
            granted,
            reserved: if granted { credits } else { 0.0 },
            balance,
        })
    }

    async fn release(&self, key: &str, credits: f64, capacity: f64) -> Result<(), AppError> {
        use mongodb::bson::doc;

        // Clamped to capacity so repeated releases cannot inflate a balance
        // beyond the bucket's size.
        let pipeline = vec![doc! { "$set": {
            "balance": { "$min": [capacity, { "$add": [{ "$ifNull": ["$balance", capacity] }, credits] }] },
        }}];

        self.budgets
            .update_one(doc! { "_id": key }, pipeline)
            .upsert(true)
            .await
            .map_err(|e| AppError::Internal(format!("mongo release usage_budget: {e}")))?;
        Ok(())
    }

    async fn balance(&self, key: &str) -> Result<Option<f64>, AppError> {
        use mongodb::bson::doc;

        let found = self
            .budgets
            .find_one(doc! { "_id": key })
            .await
            .map_err(|e| AppError::Internal(format!("mongo read usage_budget: {e}")))?;

        Ok(found.as_ref().and_then(|doc| credits_field(doc, "balance")))
    }
}
