use async_openai::{
    config::OpenAIConfig,
    types::embeddings::{CreateEmbeddingRequest, EmbeddingInput},
    Client,
};
use async_trait::async_trait;

use crate::config::RagConfig;
use crate::error::AppError;

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

        let mut oai_config = OpenAIConfig::new().with_api_base(&config.embedding_url);
        if !config.embedding_api_key.is_empty() {
            oai_config = oai_config.with_api_key(&config.embedding_api_key);
        }

        Ok(Self {
            client: Client::with_config(oai_config),
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
            .map_err(|e| AppError::Internal(format!("embedding request failed: {e}")))?;

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

    #[test]
    fn from_rag_config_fails_with_empty_url() {
        let config = RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: "test".into(),
            embedding_url: String::new(),
            embedding_model: "nomic-embed-text".into(),
            embedding_dimensions: 768,
            embedding_api_key: String::new(),
            chat_url: String::new(),
            chat_model: String::new(),
            chat_api_key: String::new(),
            system_prompt_template: String::new(),
        };
        assert!(OpenAICompatibleEmbedding::from_rag_config(&config).is_err());
    }

    #[test]
    fn from_rag_config_succeeds_with_url() {
        let config = RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: "test".into(),
            embedding_url: "http://localhost:11434/v1".into(),
            embedding_model: "nomic-embed-text".into(),
            embedding_dimensions: 768,
            embedding_api_key: String::new(),
            chat_url: String::new(),
            chat_model: String::new(),
            chat_api_key: String::new(),
            system_prompt_template: String::new(),
        };
        assert!(OpenAICompatibleEmbedding::from_rag_config(&config).is_ok());
    }
}
