use async_openai::{
    config::OpenAIConfig,
    types::embeddings::{CreateEmbeddingRequest, EmbeddingInput},
    Client,
};
use async_trait::async_trait;

use crate::config::RagConfig;
use crate::error::AppError;
use crate::rag::{build_oai_client, client::format_llm_error};

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// Embed one or more texts and return the corresponding vectors.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError>;
}

// ── OpenAI-compatible implementation ─────────────────────────────────────────

pub struct OpenAICompatibleEmbedding {
    client: Client<OpenAIConfig>,
    model: String,
}

impl OpenAICompatibleEmbedding {
    pub fn from_rag_config(config: &RagConfig) -> Result<Self, AppError> {
        if config.embedding_url.is_empty() {
            return Err(AppError::Internal(
                "embedding_url is required for RAG".into(),
            ));
        }

        Ok(Self {
            client: build_oai_client(
                &config.embedding_url,
                &config.embedding_api_key,
                &config.embedding_headers,
            )?,
            model: config.embedding_model.clone(),
        })
    }
}

#[async_trait]
impl EmbeddingService for OpenAICompatibleEmbedding {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = CreateEmbeddingRequest {
            model: self.model.clone(),
            input: EmbeddingInput::StringArray(texts.to_vec()),
            encoding_format: None,
            user: None,
            dimensions: None,
        };

        let response = self
            .client
            .embeddings()
            .create(request)
            .await
            .map_err(|e| {
                AppError::Internal(format!(
                    "embedding request failed: {}",
                    format_llm_error(&e)
                ))
            })?;

        // Sort by index to guarantee ordering matches input
        let mut embeddings = response.data;
        embeddings.sort_by_key(|e| e.index);

        let result: Vec<Vec<f32>> = embeddings.into_iter().map(|e| e.embedding).collect();
        tracing::info!(
            sent = texts.len(),
            received = result.len(),
            dims = result.first().map(|v| v.len()).unwrap_or(0),
            "embed: Ollama response"
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(embedding_url: &str) -> RagConfig {
        RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: "test".into(),
            embedding_url: embedding_url.into(),
            embedding_model: "nomic-embed-text".into(),
            embedding_dimensions: 768,
            embedding_api_key: String::new(),
            chat_url: String::new(),
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
            reranker_url: String::new(),
            analyzer_model: String::new(),
            analyzer_max_tokens: 256,
            hyde_model: String::new(),
            hyde_max_tokens: 256,
            analyzer_url: String::new(),
            hyde_url: String::new(),
            reranker_model: String::new(),
            reranker_api_key: String::new(),
            chunk_size_tokens: 256,
            chunk_overlap_tokens: 64,
        }
    }

    #[test]
    fn from_rag_config_fails_with_empty_url() {
        assert!(OpenAICompatibleEmbedding::from_rag_config(&make_config("")).is_err());
    }

    #[test]
    fn from_rag_config_succeeds_with_url() {
        assert!(OpenAICompatibleEmbedding::from_rag_config(&make_config(
            "http://localhost:11434/v1"
        ))
        .is_ok());
    }

    #[test]
    fn from_rag_config_applies_embedding_headers() {
        let mut config = make_config("http://localhost:11434/v1");
        config
            .embedding_headers
            .insert("x_producer".to_string(), "LEKTON".to_string());
        // Header normalisation is exercised in client.rs; here we just verify
        // that the config path succeeds end-to-end.
        assert!(OpenAICompatibleEmbedding::from_rag_config(&config).is_ok());
    }
}
