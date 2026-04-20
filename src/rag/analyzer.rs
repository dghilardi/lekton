use std::{collections::HashMap, sync::Arc};

use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
};
use serde::Deserialize;

use crate::config::RagConfig;
use crate::error::AppError;
use crate::rag::client::format_llm_error;
use crate::rag::provider::LlmProvider;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Simple,
    MultiEntity,
    MultiHop,
}

/// Output of the query analyzer stage.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub complexity: Complexity,
    /// Atomic sub-queries for parallel retrieval.
    /// Empty when `complexity == Simple` (original query is used directly).
    pub sub_queries: Vec<String>,
}

impl QueryPlan {
    pub fn simple() -> Self {
        Self {
            complexity: Complexity::Simple,
            sub_queries: vec![],
        }
    }
}

// ── Analyzer ──────────────────────────────────────────────────────────────────

const ANALYZER_SYSTEM: &str = "\
You are a query classifier for a document retrieval system. \
Respond ONLY with a JSON object — no preamble, no explanation, no markdown fences.

Schema:
{
  \"complexity\": \"simple\" | \"multi_entity\" | \"multi_hop\",
  \"sub_queries\": [\"string\"]
}

Rules:
- simple: single entity or concept; one retrieval pass is sufficient. sub_queries must be [].
- multi_entity: multiple independent entities that can be retrieved in parallel. \
  Break the query into 2-4 atomic sub-queries, each targeting one entity.
- multi_hop: answer to one sub-question depends on the previous result. \
  List sub-questions in dependency order.";

pub struct QueryAnalyzer {
    llm_provider: Arc<LlmProvider>,
    model: String,
    max_tokens: u32,
    headers: HashMap<String, String>,
}

impl QueryAnalyzer {
    /// Returns `None` when `analyzer_model` is empty (feature disabled).
    pub fn from_rag_config(config: &RagConfig, llm_provider: Arc<LlmProvider>) -> Option<Self> {
        if config.analyzer_model.is_empty() {
            return None;
        }
        Some(Self {
            llm_provider,
            model: config.analyzer_model.clone(),
            max_tokens: config.analyzer_max_tokens,
            headers: config.chat_headers.clone(),
        })
    }

    /// Classify `query` and return a retrieval plan.
    ///
    /// Falls back to `QueryPlan::simple()` on any LLM or parse error so that
    /// the chat pipeline is never blocked by the analyzer.
    pub async fn classify(&self, query: &str) -> Result<QueryPlan, AppError> {
        let messages = vec![
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(
                    ANALYZER_SYSTEM.to_string(),
                ),
                name: None,
            }),
            ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(query.to_string()),
                name: None,
            }),
        ];

        let request = CreateChatCompletionRequest {
            messages,
            model: self.model.clone(),
            max_completion_tokens: Some(self.max_tokens),
            stream: Some(false),
            temperature: Some(0.0),
            ..Default::default()
        };

        let client = self
            .llm_provider
            .get_client_with_headers(&self.headers)
            .await?;

        let response = client.chat().create(request).await.map_err(|e| {
            AppError::Internal(format!(
                "query analyzer LLM call failed: {}",
                format_llm_error(&e)
            ))
        })?;

        let raw = response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        tracing::debug!(
            query = %query,
            analyzer_raw = %raw,
            "RAG: query analyzer response"
        );

        Ok(parse_plan(&raw, query))
    }
}

// ── JSON parsing with graceful fallback ───────────────────────────────────────

#[derive(Deserialize)]
struct RawPlan {
    complexity: Complexity,
    #[serde(default)]
    sub_queries: Vec<String>,
}

fn parse_plan(raw: &str, original_query: &str) -> QueryPlan {
    // Strip optional markdown code fences that some LLMs add despite instructions.
    let json_str = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    match serde_json::from_str::<RawPlan>(json_str) {
        Ok(plan) => {
            let sub_queries: Vec<String> = plan
                .sub_queries
                .into_iter()
                .filter(|q| !q.trim().is_empty())
                .collect();

            // Validate: simple must have no sub-queries; others must have some.
            match plan.complexity {
                Complexity::Simple => QueryPlan::simple(),
                Complexity::MultiEntity | Complexity::MultiHop if sub_queries.is_empty() => {
                    tracing::warn!(
                        "analyzer returned {:?} with no sub_queries — falling back to simple",
                        plan.complexity
                    );
                    QueryPlan::simple()
                }
                complexity => QueryPlan {
                    complexity,
                    sub_queries,
                },
            }
        }
        Err(e) => {
            tracing::warn!(
                original_query = %original_query,
                error = %e,
                raw_response = %raw,
                "query analyzer parse error — falling back to simple"
            );
            // Preserve original query in simple plan
            QueryPlan::simple()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_plan() {
        let raw = r#"{"complexity":"simple","sub_queries":[]}"#;
        let plan = parse_plan(raw, "what is X?");
        assert_eq!(plan.complexity, Complexity::Simple);
        assert!(plan.sub_queries.is_empty());
    }

    #[test]
    fn parse_multi_entity_plan() {
        let raw = r#"{"complexity":"multi_entity","sub_queries":["config of A","config of B"]}"#;
        let plan = parse_plan(raw, "compare A and B");
        assert_eq!(plan.complexity, Complexity::MultiEntity);
        assert_eq!(plan.sub_queries, vec!["config of A", "config of B"]);
    }

    #[test]
    fn parse_multi_hop_plan() {
        let raw =
            r#"{"complexity":"multi_hop","sub_queries":["what is X?","how does X affect Y?"]}"#;
        let plan = parse_plan(raw, "how does X affect Y?");
        assert_eq!(plan.complexity, Complexity::MultiHop);
        assert_eq!(plan.sub_queries.len(), 2);
    }

    #[test]
    fn strips_markdown_fences() {
        let raw = "```json\n{\"complexity\":\"simple\",\"sub_queries\":[]}\n```";
        let plan = parse_plan(raw, "q");
        assert_eq!(plan.complexity, Complexity::Simple);
    }

    #[test]
    fn falls_back_on_invalid_json() {
        let plan = parse_plan("not json at all", "q");
        assert_eq!(plan.complexity, Complexity::Simple);
    }

    #[test]
    fn falls_back_when_multi_entity_has_no_sub_queries() {
        let raw = r#"{"complexity":"multi_entity","sub_queries":[]}"#;
        let plan = parse_plan(raw, "q");
        assert_eq!(plan.complexity, Complexity::Simple);
    }
}
