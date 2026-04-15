use std::{collections::HashMap, sync::Arc};

use async_openai::{config::OpenAIConfig, Client};
use gcp_auth::TokenProvider;

use crate::config::RagConfig;
use crate::error::AppError;

use super::client::build_oai_client;

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";
const DEFAULT_VERTEX_LOCATION: &str = "us-central1";
const GCP_SCOPE_CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Clone)]
pub enum LlmProvider {
    OpenRouter {
        api_key: String,
    },
    VertexAI {
        auth_manager: Arc<dyn TokenProvider>,
        project_id: String,
        location: String,
    },
}

impl LlmProvider {
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
                let api_key = non_empty_config(&config.chat_api_key).ok_or_else(|| {
                    AppError::Internal(
                        "rag.chat_api_key is required when Vertex AI is not configured".into(),
                    )
                })?;
                Ok(Self::OpenRouter { api_key })
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
            Self::OpenRouter { api_key } => {
                build_oai_client(OPENROUTER_API_BASE, api_key, extra_headers)
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
            Self::OpenRouter { .. } => "openrouter",
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

    #[test]
    fn trims_empty_config_values() {
        assert_eq!(non_empty_config("  value  ").as_deref(), Some("value"));
        assert_eq!(non_empty_config("   "), None);
    }
}
