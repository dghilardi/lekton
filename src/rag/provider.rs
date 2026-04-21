use std::{collections::HashMap, sync::Arc};

use async_openai::{config::OpenAIConfig, Client};
use gcp_auth::TokenProvider;

use crate::config::RagConfig;
use crate::error::AppError;

use super::client::build_oai_client;

const DEFAULT_VERTEX_LOCATION: &str = "us-central1";
const GCP_SCOPE_CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Clone)]
pub enum LlmProvider {
    OpenAiCompatible {
        api_base: String,
        api_key: String,
    },
    VertexAI {
        auth_manager: Arc<dyn TokenProvider>,
        project_id: String,
        location: String,
    },
}

impl LlmProvider {
    /// Build an OpenAI-compatible provider directly from a URL and API key.
    /// Useful when a pipeline step needs a different endpoint than the main chat URL.
    pub fn new_openai_compatible(url: String, api_key: String) -> Self {
        Self::OpenAiCompatible {
            api_base: url,
            api_key,
        }
    }

    pub async fn initialize(config: &RagConfig) -> Result<Self, AppError> {
        match non_empty_config(&config.vertex_project_id) {
            Some(project_id) => {
                let location = non_empty_config(&config.vertex_location)
                    .unwrap_or_else(|| DEFAULT_VERTEX_LOCATION.to_string());
                let auth_manager = gcp_auth::provider().await.map_err(|e| {
                    AppError::Internal(format!("failed to initialize Vertex AI auth manager: {e}"))
                })?;

                Ok(Self::VertexAI {
                    auth_manager,
                    project_id,
                    location,
                })
            }
            None => {
                let api_base = non_empty_config(&config.chat_url).ok_or_else(|| {
                    AppError::Internal(
                        "rag.chat_url is required when Vertex AI is not configured".into(),
                    )
                })?;
                let api_key = config.chat_api_key.trim().to_string();
                Ok(Self::OpenAiCompatible { api_base, api_key })
            }
        }
    }

    pub async fn get_client(&self) -> Result<Client<OpenAIConfig>, AppError> {
        self.get_client_with_headers(&HashMap::new()).await
    }

    pub async fn get_client_with_headers(
        &self,
        extra_headers: &HashMap<String, String>,
    ) -> Result<Client<OpenAIConfig>, AppError> {
        match self {
            Self::OpenAiCompatible { api_base, api_key } => {
                build_oai_client(api_base, api_key, extra_headers)
            }
            Self::VertexAI {
                auth_manager,
                project_id,
                location,
            } => {
                let token = auth_manager
                    .token(&[GCP_SCOPE_CLOUD_PLATFORM])
                    .await
                    .map_err(|e| {
                        AppError::Internal(format!("failed to acquire Vertex AI access token: {e}"))
                    })?;
                let api_base = format!(
                    "https://{location}-aiplatform.googleapis.com/v1beta1/projects/{project_id}/locations/{location}/endpoints/openapi"
                );

                build_oai_client(&api_base, token.as_str(), extra_headers)
            }
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::OpenAiCompatible { .. } => "openai_compatible",
            Self::VertexAI { .. } => "vertex_ai",
        }
    }
}

fn non_empty_config(value: &str) -> Option<String> {
    Some(value.trim().to_string()).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RagConfig;

    #[test]
    fn trims_empty_config_values() {
        assert_eq!(non_empty_config("  value  ").as_deref(), Some("value"));
        assert_eq!(non_empty_config("   "), None);
    }

    fn make_rag_config() -> RagConfig {
        RagConfig {
            qdrant_url: "http://localhost:6334".into(),
            qdrant_collection: "docs".into(),
            embedding_url: "http://localhost:11434/v1".into(),
            embedding_model: "nomic-embed-text".into(),
            embedding_dimensions: 768,
            embedding_api_key: String::new(),
            chat_url: "http://localhost:11434/v1".into(),
            chat_model: "llama3.1".into(),
            chat_api_key: String::new(),
            vertex_project_id: String::new(),
            vertex_location: String::new(),
            system_prompt_template: "Context: {{ context }}".into(),
            rewrite_model: String::new(),
            rewrite_max_tokens: 80,
            chat_headers: HashMap::new(),
            embedding_headers: HashMap::new(),
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
            expand_to_parent: false,
        }
    }

    #[tokio::test]
    async fn initialize_uses_configured_chat_url_for_openai_compatible_provider() {
        let config = make_rag_config();

        let provider = LlmProvider::initialize(&config)
            .await
            .expect("provider should initialize");

        match provider {
            LlmProvider::OpenAiCompatible { api_base, api_key } => {
                assert_eq!(api_base, "http://localhost:11434/v1");
                assert!(api_key.is_empty());
            }
            LlmProvider::VertexAI { .. } => panic!("expected OpenAI-compatible provider"),
        }
    }

    #[tokio::test]
    async fn initialize_requires_chat_url_when_vertex_is_not_configured() {
        let mut config = make_rag_config();
        config.chat_url = "   ".into();

        let error = match LlmProvider::initialize(&config).await {
            Ok(_) => panic!("provider should reject missing chat_url"),
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("rag.chat_url is required when Vertex AI is not configured"));
    }
}
