//! Google standalone Ranking API. Authentication follows the Vertex embedding/chat
//! providers: ask gcp_auth for a valid token on every call (cached/refreshed by it).
use std::{collections::HashSet, sync::Arc, time::Duration};

use async_trait::async_trait;
use gcp_auth::TokenProvider;
use serde::{Deserialize, Serialize};

use crate::{config::RagConfig, error::AppError};

use super::{reranker::Reranker, vectorstore::VectorSearchResult};

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const DEFAULT_MODEL: &str = "semantic-ranker-default@latest";
const MAX_RECORDS: usize = 1000;
const TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct VertexReranker {
    auth: Arc<dyn TokenProvider>,
    client: reqwest::Client,
    endpoint: String,
    project: String,
    model: String,
    timeout: Duration,
}

impl VertexReranker {
    pub(super) async fn from_rag_config(config: &RagConfig) -> Result<Self, AppError> {
        let project = config.reranker_vertex_project_id.trim();
        // Keep the configured project inside one URL path segment and header.
        if project.is_empty()
            || !project
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(AppError::Internal(
                "invalid Google reranker project ID".into(),
            ));
        }
        let auth = tokio::time::timeout(TIMEOUT, gcp_auth::provider())
            .await
            .map_err(|_| {
                AppError::Internal("Google reranker auth initialization timed out".into())
            })?
            .map_err(|_| {
                AppError::Internal("failed to initialize Google reranker ADC authentication".into())
            })?;
        Self::new(config, auth)
    }

    fn new(config: &RagConfig, auth: Arc<dyn TokenProvider>) -> Result<Self, AppError> {
        let project = config.reranker_vertex_project_id.trim();
        let model = config.reranker_vertex_model.trim();
        Ok(Self {
            auth,
            client: crate::net::http_client_builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| AppError::Internal("failed to build Google reranker client".into()))?,
            endpoint: format!("https://discoveryengine.googleapis.com/v1/projects/{project}/locations/global/rankingConfigs/default_ranking_config:rank"),
            project: project.into(),
            model: if model.trim().is_empty() {
                DEFAULT_MODEL.into()
            } else {
                model.trim().into()
            },
            timeout: TIMEOUT,
        })
    }

    async fn rank(
        &self,
        query: &str,
        chunks: Vec<VectorSearchResult>,
        top_n: usize,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        if chunks.is_empty() || top_n == 0 {
            return Ok(Vec::new());
        }
        // Do not silently drop candidates or compare scores from independent batches.
        if chunks.len() > MAX_RECORDS {
            return Err(AppError::Internal(
                "Google reranker supports at most 1000 records".into(),
            ));
        }
        let top_n = top_n.min(chunks.len());
        let request = RankRequest {
            query,
            model: &self.model,
            top_n,
            ignore_record_details_in_response: true,
            records: chunks
                .iter()
                .enumerate()
                .map(|(i, c)| RankRecord {
                    id: format!("chunk-{i}"),
                    content: &c.chunk_text,
                    title: &c.document_title,
                })
                .collect(),
        };
        let token = self
            .auth
            .token(&[CLOUD_PLATFORM_SCOPE])
            .await
            .map_err(|_| {
                AppError::Internal("failed to acquire Google reranker access token".into())
            })?;
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(token.as_str())
            .header("x-goog-user-project", &self.project)
            .json(&request)
            .send()
            .await
            .map_err(|_| AppError::Internal("Google reranker request failed".into()))?;
        if !response.status().is_success() {
            // Provider bodies can echo queries, document text, or credentials.
            return Err(AppError::Internal(format!(
                "Google reranker returned {}",
                response.status()
            )));
        }
        let response: RankResponse = response
            .json()
            .await
            .map_err(|_| AppError::Internal("invalid Google reranker response JSON".into()))?;
        map_response(response, chunks, top_n)
    }
}

#[async_trait]
impl Reranker for VertexReranker {
    async fn rerank(
        &self,
        query: &str,
        chunks: Vec<VectorSearchResult>,
        top_n: usize,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        // Bound token refresh as well as HTTP send/body consumption.
        tokio::time::timeout(self.timeout, self.rank(query, chunks, top_n))
            .await
            .map_err(|_| AppError::Internal("Google reranker timed out".into()))?
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RankRequest<'a> {
    query: &'a str,
    model: &'a str,
    top_n: usize,
    ignore_record_details_in_response: bool,
    records: Vec<RankRecord<'a>>,
}

#[derive(Serialize)]
struct RankRecord<'a> {
    id: String,
    content: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    title: &'a str,
}

#[derive(Deserialize)]
struct RankResponse {
    records: Vec<ScoredRecord>,
}

#[derive(Deserialize)]
struct ScoredRecord {
    id: String,
    // Protobuf JSON can omit scalar fields with their default value (zero).
    #[serde(default)]
    score: f32,
}

fn map_response(
    response: RankResponse,
    chunks: Vec<VectorSearchResult>,
    top_n: usize,
) -> Result<Vec<VectorSearchResult>, AppError> {
    let invalid = || AppError::Internal("invalid Google reranker records".into());
    if response.records.len() != top_n {
        return Err(invalid());
    }
    let mut seen = HashSet::new();
    let mut scored = Vec::with_capacity(top_n);
    for record in response.records {
        let i = record
            .id
            .strip_prefix("chunk-")
            .and_then(|i| i.parse::<usize>().ok())
            .ok_or_else(invalid)?;
        if i >= chunks.len()
            || record.id != format!("chunk-{i}")
            || !seen.insert(i)
            || !record.score.is_finite()
            || !(0.0..=1.0).contains(&record.score)
        {
            return Err(invalid());
        }
        scored.push((i, record.score));
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    // Preserve original metadata and retrieval scores, matching the URL provider.
    Ok(scored.into_iter().map(|(i, _)| chunks[i].clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    #[derive(Default)]
    struct FakeAuth {
        calls: AtomicUsize,
        fail: bool,
        hang: bool,
    }

    #[async_trait]
    impl TokenProvider for FakeAuth {
        async fn token(&self, scopes: &[&str]) -> Result<Arc<gcp_auth::Token>, gcp_auth::Error> {
            assert_eq!(scopes, &[CLOUD_PLATFORM_SCOPE]);
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.hang {
                std::future::pending::<()>().await;
            }
            if self.fail {
                return Err(gcp_auth::Error::Str("test auth failure"));
            }
            Ok(Arc::new(
                serde_json::from_value(json!({
                    "access_token": format!("mock-token-{call}"), "expires_in": 3600
                }))
                .unwrap(),
            ))
        }
        async fn project_id(&self) -> Result<Arc<str>, gcp_auth::Error> {
            Ok("test-project".into())
        }
    }

    type Requests = Arc<Mutex<Vec<(HeaderMap, Value)>>>;
    struct Server {
        endpoint: String,
        requests: Requests,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn server(status: StatusCode, response: Value, delay: Duration) -> Server {
        let requests: Requests = Default::default();
        let app = Router::new()
            .route(
                "/rank",
                post(
                    move |State(requests): State<Requests>,
                          headers: HeaderMap,
                          Json(body): Json<Value>| {
                        let response = response.clone();
                        async move {
                            requests.lock().unwrap().push((headers, body));
                            tokio::time::sleep(delay).await;
                            (status, Json(response))
                        }
                    },
                ),
            )
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/rank", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Server {
            endpoint,
            requests,
            task,
        }
    }

    fn reranker(server: &Server, auth: Arc<FakeAuth>) -> VertexReranker {
        let mut reranker = VertexReranker::new(&make_config("test-project"), auth).unwrap();
        reranker.endpoint = server.endpoint.clone();
        reranker
    }
    fn chunks() -> Vec<VectorSearchResult> {
        (0..3)
            .map(|i| VectorSearchResult {
                // Point IDs are deliberately identical: request IDs must be per candidate.
                point_id: "same-point".into(),
                document_slug: format!("doc-{i}"),
                chunk_text: format!("text-{i}"),
                document_title: format!("title-{i}"),
                score: 0.75,
                attachment_key: Some("asset.pdf".into()),
                source_page: Some(2),
                ..Default::default()
            })
            .collect()
    }

    fn make_config(project: &str) -> RagConfig {
        let config = config::Config::builder()
            .add_source(config::File::from_str(
                include_str!("../../config/default.toml"),
                config::FileFormat::Toml,
            ))
            .build()
            .unwrap();
        let mut config: crate::config::AppConfig = config.try_deserialize().unwrap();
        config.rag.reranker_vertex_project_id = project.into();
        config.rag
    }

    #[test]
    fn google_model_is_independent_of_legacy_url_model() {
        let auth = Arc::new(FakeAuth::default());
        let mut config = make_config("project-123");
        config.reranker_model = "BAAI/bge-reranker-v2-m3".into();
        let default = VertexReranker::new(&config, auth.clone()).unwrap();
        assert_eq!(default.endpoint, "https://discoveryengine.googleapis.com/v1/projects/project-123/locations/global/rankingConfigs/default_ranking_config:rank");
        assert_eq!(default.model, DEFAULT_MODEL);
        config.reranker_vertex_model = " semantic-ranker-default-004 ".into();
        let pinned = VertexReranker::new(&config, auth).unwrap();
        assert_eq!(pinned.model, "semantic-ranker-default-004");
        assert_eq!(config.reranker_model, "BAAI/bge-reranker-v2-m3");
    }

    #[tokio::test]
    async fn http_contract_mapping_and_fresh_tokens_each_call() {
        let server = server(
            StatusCode::OK,
            json!({"records":[
                {"id":"chunk-0","score":0.1}, {"id":"chunk-2","score":0.9}
            ]}),
            Duration::ZERO,
        )
        .await;
        let auth = Arc::new(FakeAuth::default());
        let reranker = reranker(&server, auth.clone());
        for _ in 0..2 {
            let result = reranker.rerank("question", chunks(), 2).await.unwrap();
            assert_eq!(
                result
                    .iter()
                    .map(|c| c.document_slug.as_str())
                    .collect::<Vec<_>>(),
                ["doc-2", "doc-0"]
            );
            assert_eq!(result[0].attachment_key.as_deref(), Some("asset.pdf"));
            assert_eq!(result[0].source_page, Some(2));
            assert_eq!(result[0].score, 0.75);
        }
        assert_eq!(auth.calls.load(Ordering::SeqCst), 2);
        for (i, (headers, body)) in server.requests.lock().unwrap().iter().enumerate() {
            assert_eq!(headers["authorization"], format!("Bearer mock-token-{i}"));
            assert_eq!(headers["x-goog-user-project"], "test-project");
            assert_eq!(
                *body,
                json!({
                    "query":"question", "model":DEFAULT_MODEL, "topN":2,
                    "ignoreRecordDetailsInResponse":true,
                    "records":[
                        {"id":"chunk-0","content":"text-0","title":"title-0"},
                        {"id":"chunk-1","content":"text-1","title":"title-1"},
                        {"id":"chunk-2","content":"text-2","title":"title-2"}
                    ]
                })
            );
        }
    }

    #[tokio::test]
    async fn empty_zero_and_over_limit_do_not_acquire_tokens_or_call_http() {
        let server = server(StatusCode::OK, json!({}), Duration::ZERO).await;
        let auth = Arc::new(FakeAuth::default());
        let reranker = reranker(&server, auth.clone());
        assert!(reranker.rerank("q", vec![], 5).await.unwrap().is_empty());
        assert!(reranker.rerank("q", chunks(), 0).await.unwrap().is_empty());
        assert!(reranker
            .rerank("q", vec![chunks()[0].clone(); 1001], 5)
            .await
            .is_err());
        assert_eq!(auth.calls.load(Ordering::SeqCst), 0);
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn clamps_top_n_and_accepts_proto_zero_score() {
        let server = server(
            StatusCode::OK,
            json!({"records":[
                {"id":"chunk-2"}, {"id":"chunk-1","score":0.5}, {"id":"chunk-0","score":1.0}
            ]}),
            Duration::ZERO,
        )
        .await;
        let result = reranker(&server, Arc::new(FakeAuth::default()))
            .rerank("q", chunks(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[2].document_slug, "doc-2");
        assert_eq!(server.requests.lock().unwrap()[0].1["topN"], 3);
    }

    #[tokio::test]
    async fn malformed_responses_fall_back_without_losing_original_order() {
        for response in [
            json!({}),
            json!("not an object"),
            json!({"records":[]}),
            json!({"records":[{"id":"chunk-0","score":0.5}]}),
            json!({"records":[{"id":"chunk-0"},{"id":"chunk-0"}]}),
            json!({"records":[{"id":"chunk-0"},{"id":"chunk-99"}]}),
            json!({"records":[{"id":"chunk-0"},{"id":"chunk-01"}]}),
            json!({"records":[{"id":"chunk-0"},{"id":"bogus"}]}),
            json!({"records":[{"id":"chunk-0"},{"score":0.5}]}),
            json!({"records":[{"id":"chunk-0"},{"id":"chunk-1","score":null}]}),
            json!({"records":[{"id":"chunk-0"},{"id":"chunk-1","score":1.5}]}),
            json!({"records":[{"id":"chunk-0"},{"id":"chunk-1","score":-0.1}]}),
        ] {
            let server = server(StatusCode::OK, response, Duration::ZERO).await;
            let reranker = reranker(&server, Arc::new(FakeAuth::default()));
            assert!(reranker.rerank("q", chunks(), 2).await.is_err());
            let fallback =
                super::super::reranker::rerank_or_fallback(&reranker, "q", &chunks(), 2).await;
            assert_eq!(
                fallback
                    .iter()
                    .map(|c| c.document_slug.as_str())
                    .collect::<Vec<_>>(),
                ["doc-0", "doc-1"]
            );
        }
    }

    #[tokio::test]
    async fn http_errors_redact_response_body_and_fall_back() {
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            let server = server(
                status,
                json!({"error":"sensitive-document-or-token"}),
                Duration::ZERO,
            )
            .await;
            let reranker = reranker(&server, Arc::new(FakeAuth::default()));
            let error = reranker
                .rerank("q", chunks(), 2)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(status.as_str()));
            assert!(!error.contains("sensitive-document-or-token"));
            let fallback =
                super::super::reranker::rerank_or_fallback(&reranker, "q", &chunks(), 2).await;
            assert_eq!(fallback[0].document_slug, "doc-0");
        }
    }

    #[tokio::test]
    async fn connection_failure_preserves_retrieval_order() {
        let mut server = server(StatusCode::OK, json!({}), Duration::ZERO).await;
        let reranker = reranker(&server, Arc::new(FakeAuth::default()));
        server.task.abort();
        // Join before requesting so the listener has certainly closed.
        let _ = (&mut server.task).await;
        let fallback =
            super::super::reranker::rerank_or_fallback(&reranker, "q", &chunks(), 2).await;
        assert_eq!(fallback.len(), 2);
        assert_eq!(fallback[0].document_slug, "doc-0");
        assert_eq!(fallback[1].document_slug, "doc-1");
    }

    #[tokio::test]
    async fn bounds_http_and_auth_delays_and_handles_auth_failure() {
        let server = server(StatusCode::OK, json!({}), Duration::from_secs(10)).await;
        for auth in [
            FakeAuth::default(),
            FakeAuth {
                hang: true,
                ..Default::default()
            },
            FakeAuth {
                fail: true,
                ..Default::default()
            },
        ] {
            let mut reranker = reranker(&server, Arc::new(auth));
            reranker.timeout = Duration::from_millis(30);
            assert!(reranker.rerank("q", chunks(), 2).await.is_err());
        }
    }
}
