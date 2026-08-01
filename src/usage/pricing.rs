//! Converting tokens into a comparable unit of spend.
//!
//! Tokens from different models differ in price by orders of magnitude — a
//! chat completion and an embedding of the same length are not remotely the
//! same cost. Anything that reasons about *spend* rather than volume therefore
//! has to weigh tokens by model first.
//!
//! The unit is a **credit**, defined by the operator: the natural choice is
//! thousandths of a euro, which makes the table read as a price list and the
//! ceilings read as budgets. Nothing here depends on that reading.
//!
//! A model with no entry in the table is charged at [`UNPRICED_PER_1K`] and
//! counted in `lekton_llm_unpriced_calls_total`, so an unlisted model shows up
//! as a gap in the price list instead of quietly costing nothing.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::config::ModelPrice;

static PRICES: OnceLock<PriceList> = OnceLock::new();

/// Register the process-wide price list. Call once at startup.
pub fn install(prices: PriceList) {
    if PRICES.set(prices).is_err() {
        tracing::warn!("price list already installed — ignoring");
    }
}

/// Cost in credits of one call, against the process-wide price list.
///
/// With no list installed every model falls back to [`UNPRICED_PER_1K`], which
/// is what the tests and the eval binaries get.
pub fn credits(model: &str, prompt_tokens: u64, completion_tokens: u64) -> f64 {
    static EMPTY: OnceLock<PriceList> = OnceLock::new();
    PRICES
        .get()
        .unwrap_or_else(|| EMPTY.get_or_init(PriceList::default))
        .credits(model, prompt_tokens, completion_tokens)
}

/// Credits per 1k tokens charged for a model that is not in the table.
///
/// Deliberately not zero: an unpriced model must not look free. One credit per
/// 1k tokens is the same scale as a cheap model, so a missing entry understates
/// rather than distorts.
pub const UNPRICED_PER_1K: f64 = 1.0;

/// Per-model token prices, in credits per 1k tokens.
#[derive(Debug, Default, Clone)]
pub struct PriceList {
    prices: HashMap<String, ModelPrice>,
}

impl PriceList {
    pub fn new(prices: HashMap<String, ModelPrice>) -> Self {
        Self { prices }
    }

    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }

    /// Cost in credits of one call.
    pub fn credits(&self, model: &str, prompt_tokens: u64, completion_tokens: u64) -> f64 {
        match self.prices.get(model) {
            Some(price) => {
                (prompt_tokens as f64 / 1_000.0) * price.prompt_per_1k
                    + (completion_tokens as f64 / 1_000.0) * price.completion_per_1k
            }
            None => {
                metrics::counter!("lekton_llm_unpriced_calls_total", "model" => model.to_string())
                    .increment(1);
                ((prompt_tokens + completion_tokens) as f64 / 1_000.0) * UNPRICED_PER_1K
            }
        }
    }

    /// Log the models that will be charged at the fallback rate.
    ///
    /// Called once at startup with the models actually configured, so the
    /// operator learns about a gap before it shows up in a bill rather than
    /// after.
    pub fn warn_about_unpriced(&self, configured_models: &[&str]) {
        if self.is_empty() {
            tracing::info!(
                "no [usage.pricing] entries — LLM spend is counted at the flat fallback rate of \
                 {UNPRICED_PER_1K} credits per 1k tokens"
            );
            return;
        }
        for model in configured_models {
            if !model.is_empty() && !self.prices.contains_key(*model) {
                tracing::warn!(
                    model = %model,
                    fallback = UNPRICED_PER_1K,
                    "model has no [usage.pricing] entry — charged at the fallback rate"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price_list() -> PriceList {
        PriceList::new(HashMap::from([
            (
                "chat-model".to_string(),
                ModelPrice {
                    prompt_per_1k: 0.15,
                    completion_per_1k: 0.60,
                },
            ),
            (
                "embed-model".to_string(),
                ModelPrice {
                    prompt_per_1k: 0.01,
                    completion_per_1k: 0.0,
                },
            ),
        ]))
    }

    #[test]
    fn charges_prompt_and_completion_at_their_own_rates() {
        // 2k prompt at 0.15 + 1k completion at 0.60
        let credits = price_list().credits("chat-model", 2_000, 1_000);

        assert!((credits - 0.90).abs() < 1e-9, "got {credits}");
    }

    #[test]
    fn a_cheap_model_costs_far_less_than_a_dear_one() {
        // The whole point of the table: same tokens, different spend.
        let prices = price_list();

        let chat = prices.credits("chat-model", 10_000, 0);
        let embedding = prices.credits("embed-model", 10_000, 0);

        assert!(
            chat > embedding * 10.0,
            "chat {chat} vs embedding {embedding}"
        );
    }

    #[test]
    fn an_unpriced_model_is_charged_not_ignored() {
        let credits = price_list().credits("who-is-this", 1_000, 1_000);

        assert!(
            (credits - 2.0 * UNPRICED_PER_1K).abs() < 1e-9,
            "got {credits}"
        );
    }

    #[test]
    fn an_empty_table_still_charges_every_call() {
        let credits = PriceList::default().credits("anything", 1_000, 0);

        assert!(credits > 0.0);
    }
}
