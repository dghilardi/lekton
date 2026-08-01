//! LLM token accounting.
//!
//! Every LLM call routes the provider's reported `usage` through [`record`],
//! which emits Prometheus counters. Like the product counters in
//! [`crate::metrics`], these are plain `metrics::counter!` calls — cheap no-ops
//! when no recorder is installed — so they need no feature gate; only the
//! exporter does.
//!
//! Counters are deliberately **not** labelled by user: that would be unbounded
//! cardinality. Per-user attribution goes to the event log in [`sink`], which
//! is off unless `usage.event_log` is set.

pub mod guard;
pub mod pricing;
pub mod sink;

pub use crate::db::usage_models::UsageKey;

use crate::db::usage_models::LlmUsageEvent;

use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestSystemMessageContentPart,
    ChatCompletionRequestUserMessageContent, ChatCompletionRequestUserMessageContentPart,
    CompletionUsage,
};

/// Which LLM call produced the usage. Used as a low-cardinality metric label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmFeature {
    /// RAG chat answer generation.
    Chat,
    /// Document summary for the guided upload.
    Summary,
    /// Query complexity classifier.
    Analyzer,
    /// Hypothetical document embeddings.
    Hyde,
    /// Conversational query rewriter.
    QueryRewriter,
    /// Learn-mode lesson generation.
    Learn,
    /// Vision transcription of image-heavy attachment pages.
    Vlm,
    /// Text embedding.
    Embedding,
}

impl LlmFeature {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Summary => "summary",
            Self::Analyzer => "analyzer",
            Self::Hyde => "hyde",
            Self::QueryRewriter => "query_rewriter",
            Self::Learn => "learn",
            Self::Vlm => "vlm",
            Self::Embedding => "embedding",
        }
    }
}

/// Token counts for a single LLM call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt: u64,
    pub completion: u64,
    /// `true` when the provider reported nothing and the counts come from
    /// [`estimate`]. Tracked as its own label so a dashboard can tell how much
    /// of the accounting is measured rather than guessed.
    pub estimated: bool,
}

impl From<&CompletionUsage> for TokenUsage {
    fn from(usage: &CompletionUsage) -> Self {
        Self {
            prompt: u64::from(usage.prompt_tokens),
            completion: u64::from(usage.completion_tokens),
            estimated: false,
        }
    }
}

/// Fallback for providers that do not report usage.
///
/// Four characters per token is the usual ballpark for the languages this
/// portal serves. It is coarse on purpose: the point is to keep unreported
/// calls visible in the totals rather than silently free, and the `estimated`
/// label says not to trust the number precisely.
pub fn estimate(prompt_chars: usize, completion_chars: usize) -> TokenUsage {
    TokenUsage {
        prompt: (prompt_chars as u64).div_ceil(4),
        completion: (completion_chars as u64).div_ceil(4),
        estimated: true,
    }
}

/// Total text length of a chat prompt, for the [`estimate`] fallback.
///
/// Non-text parts (images fed to the VLM) count as zero: there is no character
/// count to derive them from, so an estimate over a multimodal prompt
/// understates it. That is acceptable for a path that only runs when the
/// provider tells us nothing at all.
pub fn prompt_chars(messages: &[ChatCompletionRequestMessage]) -> usize {
    messages
        .iter()
        .map(|message| match message {
            ChatCompletionRequestMessage::System(m) => match &m.content {
                ChatCompletionRequestSystemMessageContent::Text(text) => text.len(),
                ChatCompletionRequestSystemMessageContent::Array(parts) => parts
                    .iter()
                    .map(
                        |ChatCompletionRequestSystemMessageContentPart::Text(text)| text.text.len(),
                    )
                    .sum(),
            },
            ChatCompletionRequestMessage::User(m) => match &m.content {
                ChatCompletionRequestUserMessageContent::Text(text) => text.len(),
                ChatCompletionRequestUserMessageContent::Array(parts) => parts
                    .iter()
                    .map(|part| match part {
                        ChatCompletionRequestUserMessageContentPart::Text(text) => text.text.len(),
                        _ => 0,
                    })
                    .sum(),
            },
            ChatCompletionRequestMessage::Assistant(m) => match &m.content {
                Some(ChatCompletionRequestAssistantMessageContent::Text(text)) => text.len(),
                _ => 0,
            },
            _ => 0,
        })
        .sum()
}

/// Emit the counters for one LLM call and, when the event log is enabled,
/// queue the per-caller record.
pub fn record(key: &UsageKey, feature: LlmFeature, model: &str, usage: TokenUsage) {
    sink::emit(LlmUsageEvent {
        actor_kind: key.kind().to_string(),
        actor_id: key.id().map(ToOwned::to_owned),
        feature: feature.as_str().to_string(),
        model: model.to_string(),
        prompt_tokens: usage.prompt,
        completion_tokens: usage.completion,
        estimated: usage.estimated,
        created_at: chrono::Utc::now(),
    });

    let credits = pricing::credits(model, usage.prompt, usage.completion);
    guard::spend(credits);
    metrics::counter!("lekton_llm_credits_millis_total", "model" => model.to_string())
        .increment((credits * 1_000.0) as u64);

    let feature = feature.as_str();
    let model = model.to_string();
    let estimated = if usage.estimated { "true" } else { "false" };

    metrics::counter!(
        "lekton_llm_requests_total",
        "feature" => feature,
        "model" => model.clone(),
        "estimated" => estimated,
    )
    .increment(1);
    metrics::counter!(
        "lekton_llm_tokens_total",
        "feature" => feature,
        "model" => model.clone(),
        "kind" => "prompt",
    )
    .increment(usage.prompt);
    metrics::counter!(
        "lekton_llm_tokens_total",
        "feature" => feature,
        "model" => model,
        "kind" => "completion",
    )
    .increment(usage.completion);
}

/// Record a chat completion, falling back to [`estimate`] when the provider
/// left `usage` empty.
///
/// `prompt_chars` and `completion_chars` are only read on the fallback path.
pub fn record_chat(
    key: &UsageKey,
    feature: LlmFeature,
    model: &str,
    reported: Option<&CompletionUsage>,
    prompt_chars: usize,
    completion_chars: usize,
) {
    let usage = match reported {
        Some(usage) => TokenUsage::from(usage),
        None => {
            tracing::debug!(
                feature = feature.as_str(),
                model = %model,
                "LLM provider reported no usage — falling back to an estimate"
            );
            estimate(prompt_chars, completion_chars)
        }
    };
    record(key, feature, model, usage);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion_usage(prompt: u32, completion: u32) -> CompletionUsage {
        CompletionUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            ..Default::default()
        }
    }

    #[test]
    fn reported_usage_is_not_marked_estimated() {
        let usage = TokenUsage::from(&completion_usage(10, 20));

        assert_eq!(
            usage,
            TokenUsage {
                prompt: 10,
                completion: 20,
                estimated: false
            }
        );
    }

    #[test]
    fn estimate_rounds_up_to_whole_tokens() {
        // 5 chars is more than one token's worth, so it must not round to 1.
        assert_eq!(
            estimate(5, 0),
            TokenUsage {
                prompt: 2,
                completion: 0,
                estimated: true
            }
        );
        assert_eq!(estimate(0, 0), TokenUsage::default().with_estimated());
    }

    #[test]
    fn record_chat_prefers_the_reported_usage() {
        // No recorder is installed in tests, so this asserts on the branch
        // taken rather than on emitted metrics: an estimate would have to
        // differ from the reported counts.
        let reported = completion_usage(3, 7);
        let usage = TokenUsage::from(&reported);

        assert!(!usage.estimated);
        assert_ne!(usage, estimate(4_000, 4_000));
    }

    impl TokenUsage {
        fn with_estimated(mut self) -> Self {
            self.estimated = true;
            self
        }
    }
}
