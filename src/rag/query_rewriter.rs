//! Conditional query rewriting for multi-turn RAG conversations.
//!
//! On the first turn (empty history) the original question is returned unchanged.
//! On subsequent turns an LLM call rewrites the follow-up question into a
//! self-contained standalone question, improving vector-search relevance without
//! polluting the embedding with raw conversation history.

use std::{collections::HashMap, sync::Arc};

use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
};

use crate::config::RagConfig;
use crate::db::chat_models::ChatMessage;
use crate::error::AppError;

use super::provider::LlmProvider;

/// Number of recent history messages used as context for query rewriting.
/// Keeping this smaller than `MAX_HISTORY_MESSAGES` limits the rewriting cost.
const REWRITE_CONTEXT_MESSAGES: usize = 6;

const REWRITE_SYSTEM: &str = "\
You are a query-rewriting assistant. \
Given a conversation history and a follow-up question, rewrite the follow-up question \
as a fully self-contained standalone question that preserves all intent and context. \
Output ONLY the rewritten question — no explanations, no prefixes, no punctuation changes \
beyond what is necessary.";

pub struct QueryRewriter {
    llm_provider: Arc<LlmProvider>,
    model: String,
    max_tokens: u32,
    headers: HashMap<String, String>,
}

impl QueryRewriter {
    /// Build a rewriter from `RagConfig`.
    ///
    /// Returns `None` when `rewrite_model` is empty (feature disabled).
    pub fn from_rag_config(config: &RagConfig, llm_provider: Arc<LlmProvider>) -> Option<Self> {
        if config.rewrite_model.is_empty() {
            return None;
        }

        Some(Self {
            llm_provider,
            model: config.rewrite_model.clone(),
            max_tokens: config.rewrite_max_tokens,
            headers: config.chat_headers.clone(),
        })
    }

    /// Rewrite `user_message` into a standalone query using `history` as context.
    ///
    /// Returns the original message unchanged when:
    /// - `history` is empty (first turn — nothing to resolve)
    /// - the LLM returns an empty response (graceful degradation)
    pub async fn rewrite(
        &self,
        user_message: &str,
        history: &[ChatMessage],
    ) -> Result<String, AppError> {
        if history.is_empty() {
            return Ok(user_message.to_string());
        }

        let history_text = Self::format_history(history);
        let user_prompt = format!(
            "Conversation history:\n{history_text}\n\nFollow-up question: {user_message}\n\nStandalone question:"
        );

        let messages = vec![
            ChatCompletionRequestMessage::System(
                async_openai::types::chat::ChatCompletionRequestSystemMessage {
                    content:
                        async_openai::types::chat::ChatCompletionRequestSystemMessageContent::Text(
                            REWRITE_SYSTEM.to_string(),
                        ),
                    name: None,
                },
            ),
            ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(user_prompt),
                name: None,
            }),
        ];

        let request = CreateChatCompletionRequest {
            messages,
            model: self.model.clone(),
            max_completion_tokens: Some(self.max_tokens),
            stream: Some(false),
            ..Default::default()
        };

        let client = self
            .llm_provider
            .get_client_with_headers(&self.headers)
            .await?;

        let response = client
            .chat()
            .create(request)
            .await
            .map_err(|e| AppError::Internal(format!("query rewrite LLM call failed: {e}")))?;

        let rewritten = response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| user_message.to_string());

        tracing::debug!(original = user_message, rewritten = %rewritten, "query rewritten");

        Ok(rewritten)
    }

    fn format_history(history: &[ChatMessage]) -> String {
        let window = if history.len() > REWRITE_CONTEXT_MESSAGES {
            &history[history.len() - REWRITE_CONTEXT_MESSAGES..]
        } else {
            history
        };
        window
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: "id".into(),
            session_id: "sess".into(),
            role: role.into(),
            content: content.into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn format_history_limits_window() {
        let history: Vec<ChatMessage> = (0..10)
            .map(|i| make_msg("user", &format!("msg {i}")))
            .collect();
        let formatted = QueryRewriter::format_history(&history);
        // Only last REWRITE_CONTEXT_MESSAGES entries
        assert!(!formatted.contains("msg 0"));
        assert!(formatted.contains("msg 9"));
    }

    #[test]
    fn format_history_empty() {
        assert_eq!(QueryRewriter::format_history(&[]), "");
    }
}
