use std::{collections::HashMap, sync::Arc};

use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
};

use crate::config::RagConfig;
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
    /// Returns `None` when `hyde_model` is empty (feature disabled).
    pub fn from_rag_config(config: &RagConfig, llm_provider: Arc<LlmProvider>) -> Option<Self> {
        if config.hyde_model.is_empty() {
            return None;
        }
        Some(Self {
            llm_provider,
            model: config.hyde_model.clone(),
            max_tokens: config.hyde_max_tokens,
            headers: config.chat_headers.clone(),
        })
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

    fn make_config(hyde_model: &str) -> RagConfig {
        RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: String::new(),
            embedding_url: String::new(),
            embedding_model: String::new(),
            embedding_dimensions: 768,
            embedding_api_key: String::new(),
            chat_url: "http://localhost:11434/v1".to_string(),
            chat_model: String::new(),
            chat_api_key: String::new(),
            vertex_project_id: String::new(),
            vertex_location: String::new(),
            system_prompt_template: String::new(),
            rewrite_model: String::new(),
            rewrite_max_tokens: 80,
            chat_headers: std::collections::HashMap::new(),
            embedding_headers: std::collections::HashMap::new(),
            embedding_cache_store_text: false,
            embedding_cache_query: false,
            hybrid_search_enabled: false,
            analyzer_model: String::new(),
            analyzer_max_tokens: 256,
            reranker_url: String::new(),
            reranker_model: String::new(),
            reranker_api_key: String::new(),
            hyde_model: hyde_model.to_string(),
            hyde_max_tokens: 256,
        }
    }

    fn make_provider(config: &RagConfig) -> Arc<LlmProvider> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        Arc::new(rt.block_on(LlmProvider::initialize(config)).unwrap())
    }

    #[test]
    fn returns_none_when_model_empty() {
        let config = make_config("");
        let provider = make_provider(&config);
        assert!(HydeService::from_rag_config(&config, provider).is_none());
    }

    #[test]
    fn returns_some_when_model_set() {
        let config = make_config("phi3:mini");
        let provider = make_provider(&config);
        assert!(HydeService::from_rag_config(&config, provider).is_some());
    }
}
