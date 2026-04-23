use std::{collections::HashMap, sync::Arc};

use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
};

use crate::error::AppError;
use crate::rag::client::format_llm_error;
use crate::rag::provider::LlmProvider;

const HYDE_SYSTEM: &str = "\
You are a technical documentation writer. Given a question, write a short passage \
(2-4 sentences) that directly answers it as if extracted from internal developer documentation. \
Write in a factual, reference style. Output ONLY the passage — no title, no preamble, \
no explanation.";

pub struct HydeService {
    llm_provider: Arc<LlmProvider>,
    model: String,
    max_tokens: u32,
    headers: HashMap<String, String>,
}

impl HydeService {
    pub fn new(
        llm_provider: Arc<LlmProvider>,
        model: String,
        max_tokens: u32,
        headers: HashMap<String, String>,
    ) -> Self {
        Self {
            llm_provider,
            model,
            max_tokens,
            headers,
        }
    }

    /// Replace each query with a hypothetical document for embedding.
    ///
    /// Runs all generations in parallel. If any individual generation fails,
    /// the original query is kept for that slot (graceful degradation).
    pub async fn expand_queries(&self, queries: Vec<String>) -> Vec<String> {
        let futures: Vec<_> = queries
            .iter()
            .map(|q| self.generate_hypothetical(q))
            .collect();

        let results = futures::future::join_all(futures).await;

        results
            .into_iter()
            .zip(queries)
            .map(|(result, original)| match result {
                Ok(doc) if !doc.is_empty() => doc,
                Ok(_) => original,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "HyDE generation failed — falling back to original query"
                    );
                    original
                }
            })
            .collect()
    }

    async fn generate_hypothetical(&self, query: &str) -> Result<String, AppError> {
        let messages = vec![
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(HYDE_SYSTEM.to_string()),
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
            temperature: Some(0.1),
            ..Default::default()
        };

        let client = self
            .llm_provider
            .get_client_with_headers(&self.headers)
            .await?;

        let response = client.chat().create(request).await.map_err(|e| {
            AppError::Internal(format!("HyDE LLM call failed: {}", format_llm_error(&e)))
        })?;

        let doc = response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        tracing::debug!(
            query = %query,
            hyde_doc = %doc,
            "RAG: HyDE hypothetical document generated"
        );

        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ResolvedLlmConfig;

    fn make_provider() -> Arc<LlmProvider> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let resolved = ResolvedLlmConfig {
            url: "http://localhost:11434/v1".into(),
            api_key: String::new(),
            model: String::new(),
            headers: HashMap::new(),
            vertex_project_id: None,
            vertex_location: None,
        };
        Arc::new(rt.block_on(LlmProvider::initialize(&resolved)).unwrap())
    }

    #[test]
    fn constructs_with_model() {
        let provider = make_provider();
        let svc = HydeService::new(provider, "phi3:mini".into(), 256, HashMap::new());
        assert_eq!(svc.model, "phi3:mini");
    }
}
