use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::RagConfig;
use crate::error::AppError;
use crate::rag::vectorstore::VectorSearchResult;

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait Reranker: Send + Sync {
    /// Re-rank `chunks` against `query` and return at most `top_n` results,
    /// ordered by descending relevance score.
    async fn rerank(
        &self,
        query: &str,
        chunks: Vec<VectorSearchResult>,
        top_n: usize,
    ) -> Result<Vec<VectorSearchResult>, AppError>;
}

/// Select Google only when explicitly configured; legacy URL configuration is unchanged.
pub async fn build_reranker(
    config: &RagConfig,
) -> Result<Option<std::sync::Arc<dyn Reranker>>, AppError> {
    if !config.reranker_vertex_project_id.trim().is_empty() {
        return Ok(Some(std::sync::Arc::new(
            super::vertex_reranker::VertexReranker::from_rag_config(config).await?,
        )));
    }
    Ok(CrossEncoderReranker::from_rag_config(config)
        .map(|r| std::sync::Arc::new(r) as std::sync::Arc<dyn Reranker>))
}

/// Preserve the retrieval order on any provider failure.
pub(crate) async fn rerank_or_fallback(
    reranker: &dyn Reranker,
    query: &str,
    chunks: &[VectorSearchResult],
    top_n: usize,
) -> Vec<VectorSearchResult> {
    match reranker.rerank(query, chunks.to_vec(), top_n).await {
        Ok(results) => results.into_iter().take(top_n).collect(),
        Err(error) => {
            tracing::warn!(%error, "reranker failed, falling back to retrieval order");
            chunks.iter().take(top_n).cloned().collect()
        }
    }
}

// ── Jina / Infinity / Cohere-compatible cross-encoder reranker ────────────────
//
// Request format:
//   POST <endpoint>
//   {"query": "...", "documents": ["doc1", ...], "model": "...", "top_n": N}
//
// Compatible with: Infinity server, Jina AI API, Cohere API.
// For HuggingFace TEI, set `reranker_url` to the `/rerank` path and leave
// `reranker_model` empty — TEI accepts the same `documents` key via its
// OpenAPI-compatible mode.

#[derive(Serialize)]
struct RerankRequest {
    query: String,
    documents: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    model: String,
    top_n: usize,
}

#[derive(Deserialize)]
struct RerankResponse {
    results: Vec<RerankResult>,
}

#[derive(Deserialize)]
struct RerankResult {
    index: usize,
    relevance_score: f32,
}

pub struct CrossEncoderReranker {
    client: reqwest::Client,
    endpoint: String,
    model: String,
}

impl CrossEncoderReranker {
    pub fn from_rag_config(config: &RagConfig) -> Option<Self> {
        if config.reranker_url.is_empty() {
            return None;
        }

        let mut headers = reqwest::header::HeaderMap::new();
        if !config.reranker_api_key.is_empty() {
            let value = reqwest::header::HeaderValue::from_str(&format!(
                "Bearer {}",
                config.reranker_api_key
            ))
            .expect("invalid reranker API key characters");
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        for (name, value) in &config.reranker_headers {
            if let (Ok(header_name), Ok(header_value)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(header_name, header_value);
            }
        }

        let client = crate::net::http_client_builder()
            .default_headers(headers)
            .build()
            .expect("failed to build reranker HTTP client");

        Some(Self {
            client,
            endpoint: config.reranker_url.clone(),
            model: config.reranker_model.clone(),
        })
    }
}

#[async_trait]
impl Reranker for CrossEncoderReranker {
    async fn rerank(
        &self,
        query: &str,
        chunks: Vec<VectorSearchResult>,
        top_n: usize,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        if chunks.is_empty() || top_n == 0 {
            return Ok(Vec::new());
        }
        let top_n = top_n.min(chunks.len());

        let documents: Vec<String> = chunks.iter().map(|c| c.chunk_text.clone()).collect();

        let request = RerankRequest {
            query: query.to_string(),
            documents,
            model: self.model.clone(),
            top_n,
        };

        let response = self
            .client
            .post(&self.endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("reranker request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "reranker returned {status}: {body}"
            )));
        }

        let rerank_response: RerankResponse = response
            .json()
            .await
            .map_err(|e| AppError::Internal(format!("reranker response parse error: {e}")))?;

        let mut scored: Vec<(usize, f32)> = rerank_response
            .results
            .into_iter()
            .filter(|r| r.index < chunks.len())
            .map(|r| (r.index, r.relevance_score))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));

        let mut seen = std::collections::HashSet::new();
        let reranked = scored
            .into_iter()
            .filter(|(i, _)| seen.insert(*i))
            .take(top_n)
            .filter_map(|(i, _)| chunks.get(i).cloned())
            .collect();

        Ok(reranked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChatStepConfig, LlmConfig, RagConfig};

    fn make_config(url: &str) -> RagConfig {
        RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: String::new(),
            embedding_url: String::new(),
            embedding_model: String::new(),
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
            retrieval_candidate_limit: 50,
            context_chunk_limit: 5,
            reranker_vertex_project_id: String::new(),
            reranker_vertex_model: String::new(),
            reranker_url: url.to_string(),
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

    #[tokio::test]
    async fn factory_preserves_disabled_and_url_configuration() {
        assert!(build_reranker(&make_config("")).await.unwrap().is_none());
        assert!(build_reranker(&make_config("http://localhost:7997/rerank"))
            .await
            .unwrap()
            .is_some());
        let mut config = make_config("http://localhost:7997/rerank");
        config.reranker_vertex_project_id = "invalid/project".into();
        // Google takes precedence: reject invalid project before any ADC lookup.
        assert!(build_reranker(&config)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("project ID"));
    }

    #[tokio::test]
    async fn legacy_http_provider_keeps_auth_headers_and_wire_format() {
        use axum::{http::HeaderMap, routing::post, Json, Router};
        use serde_json::{json, Value};
        let app = Router::new().route("/rerank", post(|headers: HeaderMap, Json(body): Json<Value>| async move {
            assert_eq!(headers["authorization"], "Bearer legacy-key");
            assert_eq!(headers["x-custom"], "legacy-header");
            assert_eq!(body, json!({"query":"q","documents":["first","second"],"model":"legacy-model","top_n":1}));
            Json(json!({"results":[{"index":1,"relevance_score":0.9}]}))
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = make_config(&format!("http://{}/rerank", listener.local_addr().unwrap()));
        config.reranker_api_key = "legacy-key".into();
        config.reranker_model = "legacy-model".into();
        config
            .reranker_headers
            .insert("x-custom".into(), "legacy-header".into());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let reranker = build_reranker(&config).await.unwrap().unwrap();
        let result = reranker
            .rerank(
                "q",
                vec![
                    VectorSearchResult {
                        chunk_text: "first".into(),
                        ..Default::default()
                    },
                    VectorSearchResult {
                        chunk_text: "second".into(),
                        ..Default::default()
                    },
                ],
                1,
            )
            .await;
        task.abort();
        assert_eq!(result.unwrap()[0].chunk_text, "second");
    }

    #[tokio::test]
    async fn url_provider_clamps_top_n_and_deduplicates_response_indices() {
        use axum::{routing::post, Json, Router};
        use serde_json::{json, Value};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let received = calls.clone();
        let app = Router::new().route(
            "/rerank",
            post(move |Json(body): Json<Value>| {
                let received = received.clone();
                async move {
                    received.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(body["top_n"], 2);
                    assert_eq!(body["documents"].as_array().unwrap().len(), 2);
                    Json(json!({"results":[
                        {"index":1,"relevance_score":0.9},
                        {"index":1,"relevance_score":0.8},
                        {"index":0,"relevance_score":0.7},
                        {"index":99,"relevance_score":0.99}
                    ]}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = make_config(&format!("http://{}/rerank", listener.local_addr().unwrap()));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let provider = build_reranker(&config).await.unwrap().unwrap();
        let chunks = vec![
            VectorSearchResult {
                point_id: "first".into(),
                chunk_text: "first".into(),
                ..Default::default()
            },
            VectorSearchResult {
                point_id: "second".into(),
                chunk_text: "second".into(),
                ..Default::default()
            },
        ];
        let result = provider.rerank("q", chunks.clone(), 100).await.unwrap();
        assert_eq!(
            result
                .iter()
                .map(|c| c.point_id.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert!(provider.rerank("q", chunks, 0).await.unwrap().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[test]
    fn old_config_deserializes_without_google_field() {
        let config = config::Config::builder()
            .add_source(config::File::from_str(
                include_str!("../../config/default.toml"),
                config::FileFormat::Toml,
            ))
            .set_override("rag.reranker_url", "http://legacy/rerank")
            .unwrap()
            .build()
            .unwrap();
        let mut json: serde_json::Value = config.try_deserialize().unwrap();
        json["rag"]
            .as_object_mut()
            .unwrap()
            .remove("reranker_vertex_project_id");
        json["rag"]
            .as_object_mut()
            .unwrap()
            .remove("reranker_vertex_model");
        let parsed: crate::config::AppConfig = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.rag.reranker_url, "http://legacy/rerank");
        assert!(parsed.rag.reranker_vertex_project_id.is_empty());
        assert!(parsed.rag.reranker_vertex_model.is_empty());
    }

    #[test]
    fn from_rag_config_returns_none_when_url_empty() {
        assert!(CrossEncoderReranker::from_rag_config(&make_config("")).is_none());
    }

    #[test]
    fn from_rag_config_returns_some_when_url_set() {
        assert!(CrossEncoderReranker::from_rag_config(&make_config(
            "http://localhost:7997/rerank"
        ))
        .is_some());
    }
}
