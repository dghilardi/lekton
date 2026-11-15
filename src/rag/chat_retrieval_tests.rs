//! Regression tests exercise the real retrieve_only pipeline without external services.
use super::*;
use crate::auth::models::AuthenticatedUser;
use crate::config::{AppConfig, LlmStepConfig};
use crate::rag::vectorstore::VectorPoint;
use crate::search::client::{SearchDocument, SearchHit};
use async_trait::async_trait;
use std::sync::Mutex;

mockall::mock! { Repo {}
    #[async_trait]
    impl ChatRepository for Repo {
    async fn create_session(&self, session: ChatSession) -> Result<(), AppError>;

    async fn get_session(&self, id: &str) -> Result<Option<ChatSession>, AppError>;

    async fn list_sessions_for_user(&self, user_id: &str) -> Result<Vec<ChatSession>, AppError>;

    async fn update_session_title(&self, id: &str, title: &str) -> Result<(), AppError>;

    async fn touch_session(&self, id: &str) -> Result<(), AppError>;

    async fn add_message(&self, msg: ChatMessage) -> Result<(), AppError>;

    async fn get_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, AppError>;

    async fn delete_session(&self, id: &str) -> Result<(), AppError>;

    async fn get_message_by_id(&self, id: &str) -> Result<Option<ChatMessage>, AppError>;

    async fn delete_message(&self, id: &str) -> Result<(), AppError>;
    }
}

struct TestStore {
    lists: Vec<Vec<VectorSearchResult>>,
    expected_limit: usize,
}
#[async_trait]
impl VectorStore for TestStore {
    async fn ensure_collection(&self, _dimensions: u32) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn upsert_chunks(&self, _points: Vec<VectorPoint>) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn delete_by_slug(&self, _slug: &str) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn delete_stale_chunks(&self, _slug: &str, _keep_ids: &[String]) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn delete_by_attachment_key(&self, _attachment_key: &str) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn delete_stale_attachment_chunks(
        &self,
        _attachment_key: &str,
        _keep_ids: &[String],
    ) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn set_attachment_access_levels(
        &self,
        _attachment_key: &str,
        _access_levels: &[String],
    ) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn get_section_chunks(
        &self,
        _document_slug: &str,
        _section_anchor: &str,
        _access_levels: Option<&[String]>,
        _include_draft: bool,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn health_check(&self) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn search(
        &self,
        vector: Vec<f32>,
        limit: usize,
        levels: Option<&[String]>,
        draft: bool,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        assert_eq!(limit, self.expected_limit);
        let levels = levels.expect("non-admin must retain its visibility filter");
        for level in ["engineering", "public", "loggeduser"] {
            assert!(levels.iter().any(|l| l == level));
        }
        assert!(!draft);
        Ok(self.lists[vector[0] as usize]
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }
}
#[derive(Default)]
struct TestSearch {
    hits: Vec<SearchHit>,
    fail: bool,
    calls: std::sync::atomic::AtomicUsize,
    forbidden: bool,
}
#[async_trait]
impl SearchService for TestSearch {
    async fn index_document(&self, _doc: &SearchDocument) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn delete_document(&self, _slug: &str) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn configure_index(&self) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn health_check(&self) -> Result<(), AppError> {
        unreachable!("not used in retrieval-only tests")
    }
    async fn search(
        &self,
        query: &str,
        levels: Option<&[String]>,
        draft: bool,
    ) -> Result<Vec<SearchHit>, AppError> {
        assert!(!self.forbidden, "thrifty mode must skip lexical API work");
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(query, "original question");
        assert!(levels.is_some());
        assert!(!draft);
        if self.fail {
            Err(AppError::Internal("mock lexical failure".into()))
        } else {
            Ok(self.hits.clone())
        }
    }
}
#[derive(Default)]
struct RecordingEmbedding(Mutex<Vec<String>>);
#[async_trait]
impl EmbeddingService for RecordingEmbedding {
    async fn embed(&self, _: &UsageKey, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        self.0.lock().unwrap().extend_from_slice(texts);
        Ok(texts
            .iter()
            .enumerate()
            .map(|(i, _)| vec![i as f32])
            .collect())
    }
}

#[derive(Default)]
struct RecordingReranker {
    seen: Mutex<Vec<(String, Vec<String>, usize)>>,
    promote: String,
    fail: bool,
}
#[async_trait]
impl Reranker for RecordingReranker {
    async fn rerank(
        &self,
        query: &str,
        mut chunks: Vec<VectorSearchResult>,
        top_n: usize,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        self.seen.lock().unwrap().push((
            query.into(),
            chunks.iter().map(chunk_identity).collect(),
            top_n,
        ));
        if self.fail {
            return Err(AppError::Internal("mock reranker unavailable".into()));
        }
        chunks.sort_by_key(|c| c.point_id != self.promote);
        // Intentionally return the entire pool: the caller must still enforce
        // its final budget even for a provider that ignores top_n.
        Ok(chunks)
    }
}

fn config() -> RagConfig {
    let mut app: AppConfig = config::Config::builder()
        .add_source(config::File::from_str(
            include_str!("../../config/default.toml"),
            config::FileFormat::Toml,
        ))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap();
    app.rag.llm.url = "http://unused.invalid/v1".into();
    app.rag.llm.model = "unused-chat".into();
    app.rag
}
fn user() -> UserContext {
    let mut user = UserContext::for_user(
        AuthenticatedUser {
            user_id: "retrieval-test".into(),
            email: "test@example.invalid".into(),
            name: None,
            is_admin: false,
        },
        None,
    );
    user.effective_access_levels = vec!["engineering".into()];
    user
}
fn chunk(i: usize) -> VectorSearchResult {
    VectorSearchResult {
        point_id: format!("p{i}"),
        document_slug: format!("doc-{i}"),
        document_title: format!("Doc {i}"),
        chunk_text: format!("stored passage {i}"),
        chunk_index: i as u32,
        score: 1.0 / (i + 1) as f32,
        ..Default::default()
    }
}
fn store(lists: Vec<Vec<VectorSearchResult>>, expected_limit: usize) -> TestStore {
    TestStore {
        lists,
        expected_limit,
    }
}
async fn service(
    config: &RagConfig,
    store: TestStore,
    embedding: Arc<RecordingEmbedding>,
    search: Option<Arc<dyn SearchService>>,
    reranker: Option<Arc<dyn Reranker>>,
    attachments: bool,
) -> ChatService {
    ChatService::from_rag_config(
        config,
        Arc::new(MockRepo::new()),
        embedding,
        Arc::new(store),
        search,
        reranker,
        attachments,
    )
    .await
    .unwrap()
}
async fn retrieve(service: &ChatService, mode: RetrievalMode) -> RetrievalOutput {
    service
        .retrieve_only(&user(), "original question", &[], "test", mode)
        .await
        .unwrap()
}

#[tokio::test]
async fn candidate_below_fifth_reaches_final_context_in_retrieval_only_path() {
    let reranker = Arc::new(RecordingReranker {
        promote: "p49".into(),
        ..Default::default()
    });
    let service = service(
        &config(),
        store(vec![(0..80).map(chunk).collect()], 50),
        Arc::default(),
        None,
        Some(reranker.clone()),
        true,
    )
    .await;
    let result = retrieve(&service, RetrievalMode::Full).await;
    assert_eq!(result.pre_rerank.len(), 50);
    assert_eq!(result.pre_rerank[49].point_id, "p49");
    assert_eq!(result.post_rerank.len(), 5);
    assert_eq!(result.post_rerank[0].point_id, "p49");
    let seen = reranker.seen.lock().unwrap();
    assert_eq!(seen[0].1.len(), 50);
    assert_eq!(seen[0].2, 5);
}

#[tokio::test]
async fn reranker_failure_returns_final_budget_in_original_candidate_order() {
    let reranker = Arc::new(RecordingReranker {
        fail: true,
        ..Default::default()
    });
    let mut config = config();
    config.retrieval_candidate_limit = 100;
    config.context_chunk_limit = 7;
    let service = service(
        &config,
        store(vec![(0..150).map(chunk).collect()], 100),
        Arc::default(),
        None,
        Some(reranker),
        true,
    )
    .await;
    let result = retrieve(&service, RetrievalMode::Full).await;
    assert_eq!(result.pre_rerank.len(), 100);
    assert_eq!(
        result
            .post_rerank
            .iter()
            .map(|c| c.point_id.clone())
            .collect::<Vec<_>>(),
        (0..7).map(|i| format!("p{i}")).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn no_reranker_and_thrifty_keep_small_vector_work_and_final_context() {
    for (with_reranker, mode) in [(false, RetrievalMode::Full), (true, RetrievalMode::Thrifty)] {
        let reranker = Arc::new(RecordingReranker::default());
        let service = service(
            &config(),
            store(vec![(0..80).map(chunk).collect()], 5),
            Arc::default(),
            None,
            with_reranker.then(|| reranker.clone() as Arc<dyn Reranker>),
            true,
        )
        .await;
        let result = retrieve(&service, mode).await;
        assert_eq!(result.pre_rerank.len(), 5);
        assert_eq!(result.post_rerank.len(), 5);
        assert!(reranker.seen.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn hybrid_boosts_vector_hits_before_cap_but_does_not_invent_lexical_only_chunks() {
    let mut config = config();
    config.hybrid_search_enabled = true;
    config.retrieval_candidate_limit = 10;
    config.context_chunk_limit = 3;
    let search = Arc::new(TestSearch {
        hits: ["lexical-only", "doc-9"]
            .into_iter()
            .map(|slug| SearchHit {
                slug: slug.into(),
                title: slug.into(),
                tags: vec![],
                content_preview: "preview".into(),
                attachment_key: None,
                page: None,
            })
            .collect(),
        ..Default::default()
    });
    let reranker = Arc::new(RecordingReranker::default());
    let service = service(
        &config,
        store(vec![(0..20).map(chunk).collect()], 10),
        Arc::default(),
        Some(search.clone()),
        Some(reranker),
        true,
    )
    .await;
    let result = retrieve(&service, RetrievalMode::Full).await;
    assert_eq!(result.pre_rerank[0].point_id, "p9");
    assert!(result
        .pre_rerank
        .iter()
        .all(|c| c.document_slug != "lexical-only"));
    assert_eq!(result.post_rerank.len(), 3);
}

#[tokio::test]
async fn hybrid_without_reranker_keeps_existing_small_overfetch_and_thrifty_skips_search() {
    for (mode, fail) in [
        (RetrievalMode::Full, false),
        (RetrievalMode::Full, true),
        (RetrievalMode::Thrifty, false),
    ] {
        let mut config = config();
        config.hybrid_search_enabled = true;
        let search = Arc::new(TestSearch {
            fail,
            forbidden: mode.is_thrifty(),
            ..Default::default()
        });
        let limit = if mode.is_thrifty() { 5 } else { 15 };
        let service = service(
            &config,
            store(vec![(0..80).map(chunk).collect()], limit),
            Arc::default(),
            Some(search.clone()),
            None,
            true,
        )
        .await;
        let result = retrieve(&service, mode).await;
        assert_eq!(result.pre_rerank.len(), 5);
        assert_eq!(result.post_rerank.len(), 5);
        assert_eq!(
            search.calls.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(!mode.is_thrifty())
        );
    }
}

#[test]
fn merged_pool_is_unique_bounded_and_preserves_subquery_top_hits_after_hybrid_fusion() {
    for limit in [1, 5, 50, 100] {
        let lists = vec![
            (0..75).map(chunk).collect(),
            (50..125).map(chunk).collect(),
            (100..175).map(chunk).collect(),
        ];
        let result = select_candidates(lists, &["doc-74".into(), "doc-124".into()], limit);
        assert_eq!(result.len(), limit);
        assert_eq!(
            result
                .iter()
                .map(chunk_identity)
                .collect::<HashSet<_>>()
                .len(),
            limit
        );
        for id in ["p0", "p50", "p100"].into_iter().take(limit) {
            assert!(result.iter().any(|c| c.point_id == id));
        }
    }
    let mut legacy = chunk(1);
    legacy.point_id.clear();
    let mut other = chunk(2);
    other.point_id.clear();
    let result = select_candidates(vec![vec![legacy.clone(), legacy, other]], &[], 50);
    assert_eq!(result.len(), 2);
}

#[test]
fn validates_budget_defaults_boundaries_and_legacy_configuration() {
    let mut config = config();
    assert_eq!(
        (config.retrieval_candidate_limit, config.context_chunk_limit),
        (50, 5)
    );
    for (candidates, context, valid) in [
        (0, 5, false),
        (101, 5, false),
        (50, 0, false),
        (5, 6, false),
        (100, 100, true),
        (1, 1, true),
    ] {
        config.retrieval_candidate_limit = candidates;
        config.context_chunk_limit = context;
        assert_eq!(config.validate().is_ok(), valid, "{candidates}/{context}");
    }
    let mut legacy: serde_json::Value = config::Config::builder()
        .add_source(config::File::from_str(
            include_str!("../../config/default.toml"),
            config::FileFormat::Toml,
        ))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap();
    for key in ["retrieval_candidate_limit", "context_chunk_limit"] {
        legacy["rag"].as_object_mut().unwrap().remove(key);
    }
    let app: AppConfig = serde_json::from_value(legacy).unwrap();
    assert_eq!(
        (
            app.rag.retrieval_candidate_limit,
            app.rag.context_chunk_limit
        ),
        (50, 5)
    );
}

#[tokio::test]
async fn decomposition_and_hyde_only_expand_queries_and_filter_attachments_before_reranking() {
    use axum::{routing::post, Json, Router};
    use serde_json::{json, Value};
    let app = Router::new().route("/v1/chat/completions", post(|Json(body): Json<Value>| async move {
        let content = if body["model"] == "analyzer" {
            json!({"complexity":"multi_entity","sub_queries":["one", "two"]}).to_string()
        } else {
            format!("hypothetical answer to {}", body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap())
        };
        Json(json!({"id":"test","object":"chat.completion","created":0,"model":"mock","choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":"stop"}]}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = config();
    config.llm.url = format!("http://{}/v1", listener.local_addr().unwrap());
    config.analyzer =
        Some(serde_json::from_value::<LlmStepConfig>(json!({"model":"analyzer"})).unwrap());
    config.hyde = Some(serde_json::from_value::<LlmStepConfig>(json!({"model":"hyde"})).unwrap());
    config.retrieval_candidate_limit = 10;
    config.context_chunk_limit = 3;
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let embedding = Arc::new(RecordingEmbedding::default());
    let reranker = Arc::new(RecordingReranker {
        promote: "p8".into(),
        ..Default::default()
    });
    let mut stale = chunk(999);
    stale.source_kind = SourceKind::Attachment;
    let lists = vec![
        std::iter::once(stale.clone())
            .chain((0..9).map(chunk))
            .collect(),
        std::iter::once(stale).chain((7..16).map(chunk)).collect(),
    ];
    let chat = service(
        &config,
        store(lists, 10),
        embedding.clone(),
        None,
        Some(reranker.clone()),
        false,
    )
    .await;
    let result = retrieve(&chat, RetrievalMode::Full).await;
    task.abort();
    assert_eq!(result.query_plan.sub_queries, ["one", "two"]);
    let embedded = embedding.0.lock().unwrap();
    assert_eq!(embedded.len(), 2);
    assert!(embedded.iter().all(|q| q.contains("hypothetical answer")));
    assert_eq!(result.pre_rerank.len(), 10);
    assert_eq!(
        result
            .pre_rerank
            .iter()
            .map(chunk_identity)
            .collect::<HashSet<_>>()
            .len(),
        10
    );
    assert!(result.pre_rerank.iter().any(|c| c.point_id == "p0"));
    assert!(result.pre_rerank.iter().any(|c| c.point_id == "p7"));
    assert!(result
        .pre_rerank
        .iter()
        .chain(&result.post_rerank)
        .all(|c| c.source_kind != SourceKind::Attachment
            && c.chunk_text.starts_with("stored passage")));
    assert_eq!(reranker.seen.lock().unwrap()[0].0, "original question");
    assert_eq!(result.post_rerank.len(), 3);
    assert_eq!(result.post_rerank[0].point_id, "p8");
    drop(embedded);
    let thrifty_embedding = Arc::new(RecordingEmbedding::default());
    let thrifty_reranker = Arc::new(RecordingReranker::default());
    let thrifty = service(
        &config,
        store(vec![(0..20).map(chunk).collect()], 3),
        thrifty_embedding.clone(),
        None,
        Some(thrifty_reranker.clone()),
        false,
    )
    .await;
    let result = retrieve(&thrifty, RetrievalMode::Thrifty).await;
    assert_eq!(result.query_plan.complexity, Complexity::Simple);
    assert_eq!(result.queries_embedded, ["original question"]);
    assert_eq!(result.pre_rerank.len(), 3);
    assert_eq!(result.post_rerank.len(), 3);
    assert!(thrifty_reranker.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn invalid_evaluation_budgets_fail_before_backend_initialization() {
    let mut config = config();
    config.context_chunk_limit = 0;
    let error = crate::rag::eval::RagEvalContext::from_rag_config(&config, None)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("context_chunk_limit"));
}
