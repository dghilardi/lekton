use std::collections::HashMap;
use std::sync::Arc;

use async_openai::{
    config::OpenAIConfig,
    types::embeddings::{CreateEmbeddingRequest, EmbeddingInput},
    Client,
};
use async_trait::async_trait;
use gcp_auth::TokenProvider;

use crate::config::RagConfig;
use crate::error::AppError;
use crate::rag::{build_oai_client, client::format_llm_error};

const DEFAULT_VERTEX_LOCATION: &str = "us-central1";
const GCP_SCOPE_CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";

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

// ── Vertex AI implementation ──────────────────────────────────────────────────

pub struct VertexAIEmbedding {
    auth_manager: Arc<dyn TokenProvider>,
    project_id: String,
    location: String,
    model: String,
}

impl VertexAIEmbedding {
    pub async fn from_rag_config(config: &RagConfig) -> Result<Self, AppError> {
        let location = if config.embedding_vertex_location.is_empty() {
            DEFAULT_VERTEX_LOCATION.to_string()
        } else {
            config.embedding_vertex_location.clone()
        };
        let auth_manager = gcp_auth::provider().await.map_err(|e| {
            AppError::Internal(format!(
                "failed to initialize Vertex AI auth for embedding: {e}"
            ))
        })?;
        Ok(Self {
            auth_manager,
            project_id: config.embedding_vertex_project_id.clone(),
            location,
            model: config.embedding_model.clone(),
        })
    }
}

#[async_trait]
impl EmbeddingService for VertexAIEmbedding {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let token = self
            .auth_manager
            .token(&[GCP_SCOPE_CLOUD_PLATFORM])
            .await
            .map_err(|e| {
                AppError::Internal(format!("failed to acquire Vertex AI embedding token: {e}"))
            })?;

        let api_base = format!(
            "https://{location}-aiplatform.googleapis.com/v1beta1/projects/{project_id}/locations/{location}/endpoints/openapi",
            location = self.location,
            project_id = self.project_id,
        );
        let client = build_oai_client(&api_base, token.as_str(), &HashMap::new())?;

        let request = CreateEmbeddingRequest {
            model: self.model.clone(),
            input: EmbeddingInput::StringArray(texts.to_vec()),
            encoding_format: None,
            user: None,
            dimensions: None,
        };

        let response = client.embeddings().create(request).await.map_err(|e| {
            AppError::Internal(format!(
                "Vertex AI embedding request failed: {}",
                format_llm_error(&e)
            ))
        })?;

        let mut embeddings = response.data;
        embeddings.sort_by_key(|e| e.index);
        Ok(embeddings.into_iter().map(|e| e.embedding).collect())
    }
}

// ── Factory ───────────────────────────────────────────────────────────────────

/// Build the appropriate [`EmbeddingService`] from config.
/// Uses Vertex AI when `embedding_vertex_project_id` is set; otherwise OpenAI-compatible.
pub async fn build_embedding_service(
    config: &RagConfig,
) -> Result<Arc<dyn EmbeddingService>, AppError> {
    if !config.embedding_vertex_project_id.is_empty() {
        Ok(Arc::new(VertexAIEmbedding::from_rag_config(config).await?))
    } else {
        Ok(Arc::new(OpenAICompatibleEmbedding::from_rag_config(
            config,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChatStepConfig, LlmConfig, LlmStepConfig, RagConfig};

    fn make_config(embedding_url: &str) -> RagConfig {
        RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: "test".into(),
            embedding_url: embedding_url.into(),
            embedding_model: "nomic-embed-text".into(),
            embedding_dimensions: 768,
            embedding_api_key: String::new(),
            embedding_headers: std::collections::HashMap::new(),
            embedding_vertex_project_id: String::new(),
            embedding_vertex_location: String::new(),
            embedding_cache_store_text: false,
            embedding_cache_query: false,
            chunk_size_tokens: 256,
            chunk_overlap_tokens: 64,
            expand_to_parent: false,
            hybrid_search_enabled: false,
            reranker_url: String::new(),
            reranker_model: String::new(),
            reranker_api_key: String::new(),
            reranker_headers: std::collections::HashMap::new(),
            llm: LlmConfig {
                url: String::new(),
                api_key: String::new(),
                model: String::new(),
                headers: std::collections::HashMap::new(),
                vertex_project_id: String::new(),
                vertex_location: String::new(),
            },
            chat: ChatStepConfig {
                model: None,
                url: None,
                api_key: None,
                headers: None,
                vertex_project_id: None,
                vertex_location: None,
                system_prompt_template: String::new(),
            },
            analyzer: None,
            hyde: None,
            rewriter: None,
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
        assert!(OpenAICompatibleEmbedding::from_rag_config(&config).is_ok());
    }
}
