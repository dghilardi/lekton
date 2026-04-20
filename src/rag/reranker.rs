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

        let client = reqwest::Client::builder()
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
        if chunks.is_empty() {
            return Ok(chunks);
        }

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

        let reranked = scored
            .into_iter()
            .take(top_n)
            .filter_map(|(i, _)| chunks.get(i).cloned())
            .collect();

        Ok(reranked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(url: &str) -> RagConfig {
        RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: String::new(),
            embedding_url: String::new(),
            embedding_model: String::new(),
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
            reranker_url: url.to_string(),
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
