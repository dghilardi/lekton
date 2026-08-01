use std::sync::Arc;

use async_openai::{
    config::OpenAIConfig,
    types::embeddings::{CreateEmbeddingRequest, EmbeddingInput},
    Client,
};
use async_trait::async_trait;
use gcp_auth::TokenProvider;
use serde::{Deserialize, Serialize};

use crate::config::RagConfig;
use crate::error::AppError;
use crate::rag::{build_oai_client, client::format_llm_error};
use crate::usage;
use crate::usage::UsageKey;

const DEFAULT_VERTEX_LOCATION: &str = "us-central1";
const GCP_SCOPE_CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";
/// Most OpenAI-compatible embedding APIs (e.g. OpenAI itself) cap batch requests at 100 inputs.
const OPENAI_EMBEDDING_BATCH_SIZE: usize = 100;

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

        let mut result = Vec::with_capacity(texts.len());

        for batch in texts.chunks(OPENAI_EMBEDDING_BATCH_SIZE) {
            let request = CreateEmbeddingRequest {
                model: self.model.clone(),
                input: EmbeddingInput::StringArray(batch.to_vec()),
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

            usage::record(
                // Embeddings are billed to the system: `EmbeddingService::embed`
                // serves both ingest (no caller) and chat queries (a user),
                // and threading a key through the trait would touch reindex,
                // MCP and the eval binaries. Ingest dominates the volume, so
                // system is the better approximation until the trait changes.
                &UsageKey::System,
                usage::LlmFeature::Embedding,
                &self.model,
                usage::TokenUsage {
                    prompt: u64::from(response.usage.prompt_tokens),
                    completion: 0,
                    estimated: false,
                },
            );

            // Sort by index to guarantee ordering matches input
            let mut embeddings = response.data;
            embeddings.sort_by_key(|e| e.index);

            result.extend(embeddings.into_iter().map(|e| e.embedding));
        }

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
    client: reqwest::Client,
    project_id: String,
    location: String,
    model: String,
    dimensions: u32,
    endpoint_url: String,
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
            client: crate::net::http_client(),
            project_id: config.embedding_vertex_project_id.clone(),
            location: location.clone(),
            model: vertex_embedding_model_id(&config.embedding_model),
            dimensions: config.embedding_dimensions,
            endpoint_url: vertex_embedding_endpoint_url(
                &config.embedding_vertex_project_id,
                &location,
                &config.embedding_model,
            ),
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

        let batch_size = vertex_embedding_batch_size(&self.model);
        let mut vectors = Vec::with_capacity(texts.len());

        for batch in texts.chunks(batch_size) {
            let request = vertex_embedding_request(batch, self.dimensions);
            let response = self
                .client
                .post(&self.endpoint_url)
                .bearer_auth(token.as_str())
                .json(&request)
                .send()
                .await
                .map_err(|e| {
                    AppError::Internal(format!(
                        "Vertex AI embedding request failed for project '{}' in '{}': {e}",
                        self.project_id, self.location
                    ))
                })?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(AppError::Internal(format!(
                    "Vertex AI embedding request failed for model '{}' with {status}: {body}",
                    self.model
                )));
            }

            let response: VertexEmbeddingResponse = response.json().await.map_err(|e| {
                AppError::Internal(format!("Vertex AI embedding response parse error: {e}"))
            })?;
            usage::record(
                // Embeddings are billed to the system: `EmbeddingService::embed`
                // serves both ingest (no caller) and chat queries (a user),
                // and threading a key through the trait would touch reindex,
                // MCP and the eval binaries. Ingest dominates the volume, so
                // system is the better approximation until the trait changes.
                &UsageKey::System,
                usage::LlmFeature::Embedding,
                &self.model,
                match response.token_count() {
                    Some(prompt) => usage::TokenUsage {
                        prompt,
                        completion: 0,
                        estimated: false,
                    },
                    None => usage::estimate(batch.iter().map(String::len).sum(), 0),
                },
            );

            let mut batch_vectors = response.into_vectors()?;
            if batch_vectors.len() != batch.len() {
                return Err(AppError::Internal(format!(
                    "Vertex AI embedding returned {} vectors for {} inputs",
                    batch_vectors.len(),
                    batch.len()
                )));
            }
            vectors.append(&mut batch_vectors);
        }

        Ok(vectors)
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct VertexEmbeddingRequest {
    instances: Vec<VertexEmbeddingInstance>,
    parameters: VertexEmbeddingParameters,
}

#[derive(Debug, Serialize, PartialEq)]
struct VertexEmbeddingInstance {
    content: String,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct VertexEmbeddingParameters {
    auto_truncate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dimensionality: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct VertexEmbeddingResponse {
    predictions: Vec<VertexEmbeddingPrediction>,
}

#[derive(Debug, Deserialize)]
struct VertexEmbeddingPrediction {
    embeddings: Option<VertexEmbeddingValues>,
}

#[derive(Debug, Deserialize)]
struct VertexEmbeddingValues {
    values: Vec<f32>,
    #[serde(default)]
    statistics: Option<VertexEmbeddingStatistics>,
}

/// Per-prediction counters returned by the Vertex predict API. Verified against
/// `gemini-embedding-001` (2026-08-01): `embeddings.statistics.token_count`.
#[derive(Debug, Deserialize)]
struct VertexEmbeddingStatistics {
    #[serde(default)]
    token_count: Option<u64>,
}

impl VertexEmbeddingResponse {
    /// Tokens billed for this batch, or `None` when the API reported none —
    /// in which case the caller estimates rather than counting the batch free.
    fn token_count(&self) -> Option<u64> {
        let counts: Vec<u64> = self
            .predictions
            .iter()
            .filter_map(|prediction| prediction.embeddings.as_ref())
            .filter_map(|embedding| embedding.statistics.as_ref())
            .filter_map(|statistics| statistics.token_count)
            .collect();

        (!counts.is_empty()).then(|| counts.into_iter().sum())
    }

    fn into_vectors(self) -> Result<Vec<Vec<f32>>, AppError> {
        self.predictions
            .into_iter()
            .enumerate()
            .map(|(index, prediction)| {
                prediction
                    .embeddings
                    .map(|embedding| embedding.values)
                    .ok_or_else(|| {
                        AppError::Internal(format!(
                            "Vertex AI embedding response missing embeddings at index {index}"
                        ))
                    })
            })
            .collect()
    }
}

fn vertex_embedding_request(texts: &[String], dimensions: u32) -> VertexEmbeddingRequest {
    VertexEmbeddingRequest {
        instances: texts
            .iter()
            .map(|content| VertexEmbeddingInstance {
                content: content.clone(),
            })
            .collect(),
        parameters: VertexEmbeddingParameters {
            auto_truncate: true,
            output_dimensionality: (dimensions > 0).then_some(dimensions),
        },
    }
}

fn vertex_embedding_endpoint_url(project_id: &str, location: &str, model: &str) -> String {
    let host = if location == "global" {
        "https://aiplatform.googleapis.com".to_string()
    } else {
        format!("https://{location}-aiplatform.googleapis.com")
    };
    let model = vertex_embedding_model_id(model);

    format!(
        "{host}/v1/projects/{project_id}/locations/{location}/publishers/google/models/{model}:predict"
    )
}

fn vertex_embedding_model_id(model: &str) -> String {
    let model = model.trim();
    if let Some((_, model_id)) = model.rsplit_once("/models/") {
        return model_id.to_string();
    }

    model.strip_prefix("google/").unwrap_or(model).to_string()
}

fn vertex_embedding_batch_size(model: &str) -> usize {
    let model = vertex_embedding_model_id(model);
    if model.starts_with("gemini-embedding-") {
        1
    } else {
        5
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
    use crate::config::{ChatStepConfig, LlmConfig, RagConfig};

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
                max_output_tokens: 2048,
            },
            analyzer: None,
            hyde: None,
            rewriter: None,
            vlm: None,
            attachment_page_text_threshold: 100,
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

    #[test]
    fn vertex_embedding_endpoint_uses_native_predict_api() {
        let url =
            vertex_embedding_endpoint_url("test-project", "europe-west1", "gemini-embedding-001");

        assert_eq!(
            url,
            "https://europe-west1-aiplatform.googleapis.com/v1/projects/test-project/locations/europe-west1/publishers/google/models/gemini-embedding-001:predict"
        );
    }

    #[test]
    fn vertex_embedding_endpoint_uses_global_host_for_global_location() {
        let url =
            vertex_embedding_endpoint_url("test-project", "global", "google/gemini-embedding-001");

        assert_eq!(
            url,
            "https://aiplatform.googleapis.com/v1/projects/test-project/locations/global/publishers/google/models/gemini-embedding-001:predict"
        );
    }

    #[test]
    fn vertex_embedding_model_id_accepts_openai_compatible_prefix() {
        assert_eq!(
            vertex_embedding_model_id("google/gemini-embedding-001"),
            "gemini-embedding-001"
        );
        assert_eq!(
            vertex_embedding_model_id("publishers/google/models/text-embedding-005"),
            "text-embedding-005"
        );
    }

    #[test]
    fn vertex_embedding_request_uses_vertex_schema_and_dimensions() {
        let request = vertex_embedding_request(&["hello".to_string(), "world".to_string()], 768);
        let value = serde_json::to_value(request).expect("request should serialize");

        assert_eq!(
            value,
            serde_json::json!({
                "instances": [
                    { "content": "hello" },
                    { "content": "world" }
                ],
                "parameters": {
                    "autoTruncate": true,
                    "outputDimensionality": 768
                }
            })
        );
    }

    #[test]
    fn vertex_embedding_batch_size_respects_vertex_limits() {
        assert_eq!(vertex_embedding_batch_size("gemini-embedding-001"), 1);
        assert_eq!(
            vertex_embedding_batch_size("google/gemini-embedding-001"),
            1
        );
        assert_eq!(vertex_embedding_batch_size("text-embedding-005"), 5);
    }

    #[test]
    fn vertex_embedding_response_extracts_vectors_in_provider_order() {
        let response: VertexEmbeddingResponse = serde_json::from_value(serde_json::json!({
            "predictions": [
                { "embeddings": { "values": [1.0, 2.0] } },
                { "embeddings": { "values": [3.0, 4.0] } }
            ]
        }))
        .expect("response should parse");

        assert_eq!(
            response.into_vectors().expect("vectors should extract"),
            vec![vec![1.0, 2.0], vec![3.0, 4.0]]
        );
    }

    #[test]
    fn vertex_embedding_response_sums_reported_token_counts() {
        // Shape verified against gemini-embedding-001 on 2026-08-01.
        let response: VertexEmbeddingResponse = serde_json::from_value(serde_json::json!({
            "predictions": [
                { "embeddings": { "statistics": { "truncated": false, "token_count": 5 },
                                  "values": [1.0] } },
                { "embeddings": { "statistics": { "truncated": false, "token_count": 7 },
                                  "values": [2.0] } }
            ]
        }))
        .expect("response should parse");

        assert_eq!(response.token_count(), Some(12));
    }

    #[test]
    fn vertex_embedding_response_reports_no_token_count_when_absent() {
        // Without statistics the caller must estimate rather than bill zero.
        let response: VertexEmbeddingResponse = serde_json::from_value(serde_json::json!({
            "predictions": [{ "embeddings": { "values": [1.0] } }]
        }))
        .expect("response should parse");

        assert_eq!(response.token_count(), None);
    }

    #[test]
    fn vertex_embedding_response_rejects_missing_embeddings() {
        let response: VertexEmbeddingResponse = serde_json::from_value(serde_json::json!({
            "predictions": [
                { "embeddings": { "values": [1.0, 2.0] } },
                {}
            ]
        }))
        .expect("response should parse");

        let error = response
            .into_vectors()
            .expect_err("missing embeddings should fail");

        assert!(error
            .to_string()
            .contains("Vertex AI embedding response missing embeddings at index 1"));
    }
}
