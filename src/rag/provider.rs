use std::{collections::HashMap, env, sync::Arc};

use async_openai::{config::OpenAIConfig, Client};
use gcp_auth::TokenProvider;

use crate::error::AppError;

use super::client::build_oai_client;

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";
const DEFAULT_VERTEX_LOCATION: &str = "us-central1";
const GCP_SCOPE_CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Clone)]
pub enum LlmProvider {
    OpenRouter,
    VertexAI {
        auth_manager: Arc<dyn TokenProvider>,
        project_id: String,
        location: String,
    },
}

impl LlmProvider {
    pub async fn initialize() -> Result<Self, AppError> {
        match non_empty_env("VERTEX_PROJECT_ID") {
            Some(project_id) => {
                let location = non_empty_env("VERTEX_LOCATION")
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
            None => Ok(Self::OpenRouter),
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
            Self::OpenRouter => {
                let api_key = non_empty_env("OPENROUTER_API_KEY").ok_or_else(|| {
                    AppError::Internal(
                        "OPENROUTER_API_KEY is required when Vertex AI is not configured".into(),
                    )
                })?;
                build_oai_client(OPENROUTER_API_BASE, &api_key, extra_headers)
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
            Self::OpenRouter => "openrouter",
            Self::VertexAI { .. } => "vertex_ai",
        }
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_empty_env_values() {
        std::env::set_var("LKN_TEST_ENV", "  value  ");
        assert_eq!(non_empty_env("LKN_TEST_ENV").as_deref(), Some("value"));
        std::env::set_var("LKN_TEST_ENV", "   ");
        assert_eq!(non_empty_env("LKN_TEST_ENV"), None);
        std::env::remove_var("LKN_TEST_ENV");
    }
}
