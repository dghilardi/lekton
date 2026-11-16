//! Regression tests exercise the real retrieve_only pipeline without external services.
use super::*;
use crate::auth::models::AuthenticatedUser;
use crate::config::{AppConfig, LlmStepConfig};
use crate::rag::analyzer::Complexity;
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

/// Recorded passage lookups: (document slug, limit, query vector).
type LexicalCalls = Arc<Mutex<Vec<(String, usize, Vec<f32>)>>>;

struct TestStore {
    lists: Vec<Vec<VectorSearchResult>>,
    expected_limit: usize,
    lexical: HashMap<String, Vec<VectorSearchResult>>,
    lexical_calls: LexicalCalls,
    lexical_failure: bool,
    lexical_hang: bool,
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
    async fn search_document_chunks(
        &self,
        vector: Vec<f32>,
        slug: &str,
        limit: usize,
        levels: Option<&[String]>,
        draft: bool,
        attachments: bool,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        assert_eq!(limit, super::super::hybrid::PASSAGES_PER_DOCUMENT);
        let levels = levels.expect("lexical passage lookup must retain visibility");
        for level in ["engineering", "public", "loggeduser"] {
            assert!(levels.iter().any(|l| l == level));
        }
        assert!(!draft);
        self.lexical_calls
            .lock()
            .unwrap()
            .push((slug.into(), limit, vector));
        if self.lexical_hang {
            return std::future::pending().await;
        }
        if self.lexical_failure {
            return Err(AppError::Internal("mock passage lookup unavailable".into()));
        }
        Ok(self
            .lexical
            .get(slug)
            .into_iter()
            .flatten()
            .filter(|c| attachments || c.source_kind != SourceKind::Attachment)
            .take(limit)
            .cloned()
            .collect())
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
    query_hits: HashMap<String, Vec<SearchHit>>,
    queries: Mutex<Vec<String>>,
    fail: bool,
    calls: std::sync::atomic::AtomicUsize,
    forbidden: bool,
    hang: bool,
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
        self.queries.lock().unwrap().push(query.to_string());
        assert!(levels.is_some());
        assert!(!draft);
        if self.hang {
            return std::future::pending().await;
        }
        if self.fail {
            Err(AppError::Internal("mock lexical failure".into()))
        } else {
            Ok(self.query_hits.get(query).unwrap_or(&self.hits).clone())
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
        lexical: HashMap::new(),
        lexical_calls: Default::default(),
        lexical_failure: false,
        lexical_hang: false,
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
    assert_eq!(seen[0].2, 50, "the whole pool is ranked");
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
async fn lexical_only_document_reaches_reranker_and_final_context() {
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
                content_preview: "preview".into(),
                ..SearchHit::default()
            })
            .collect(),
        ..Default::default()
    });
    let reranker = Arc::new(RecordingReranker {
        promote: "p99".into(),
        ..Default::default()
    });
    let mut backend = store(vec![(0..20).map(chunk).collect()], 10);
    let mut lexical = chunk(99);
    lexical.document_slug = "lexical-only".into();
    lexical.chunk_index = 999;
    backend.lexical.insert("lexical-only".into(), vec![lexical]);
    backend.lexical.insert("doc-9".into(), vec![chunk(9)]);
    let calls = backend.lexical_calls.clone();
    let service = service(
        &config,
        backend,
        Arc::default(),
        Some(search.clone()),
        Some(reranker.clone()),
        true,
    )
    .await;
    let result = retrieve(&service, RetrievalMode::Full).await;
    assert!(result
        .pre_rerank
        .iter()
        .any(|c| c.document_slug == "lexical-only"));
    assert!(result.pre_rerank.iter().any(|c| c.point_id == "p0"));
    assert_eq!(
        result
            .pre_rerank
            .iter()
            .filter(|c| c.point_id == "p9")
            .count(),
        1
    );
    assert!(reranker.seen.lock().unwrap()[0]
        .1
        .contains(&"p99".to_string()));
    assert_eq!(result.post_rerank[0].point_id, "p99");
    assert_eq!(result.post_rerank[0].chunk_index, 999);
    assert_eq!(result.post_rerank.len(), 3);
    assert_eq!(calls.lock().unwrap().len(), 2);
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
        // The fetched pool reaches the final selection, which keeps five.
        assert_eq!(result.pre_rerank.len(), limit);
        assert_eq!(result.post_rerank.len(), 5);
        assert_eq!(
            search.calls.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(!mode.is_thrifty())
        );
    }
}

#[tokio::test]
async fn hybrid_without_reranker_admits_lexical_passage_beside_top_vector_hit() {
    let mut config = config();
    config.hybrid_search_enabled = true;
    let search = Arc::new(TestSearch {
        hits: vec![hit("lexical-only")],
        ..Default::default()
    });
    let mut backend = store(vec![(0..80).map(chunk).collect()], 15);
    let mut lexical = chunk(99);
    lexical.document_slug = "lexical-only".into();
    backend.lexical.insert("lexical-only".into(), vec![lexical]);
    let chat = service(&config, backend, Arc::default(), Some(search), None, true).await;
    let result = retrieve(&chat, RetrievalMode::Full).await;
    let ids = |chunks: &[VectorSearchResult]| {
        chunks
            .iter()
            .map(|c| c.point_id.clone())
            .collect::<Vec<_>>()
    };
    // Without a reranker the final context keeps the top vector hit first,
    // then the top lexical passage, then the fused ranking.
    assert_eq!(
        ids(&result.pre_rerank[..5]),
        ["p0", "p99", "p1", "p2", "p3"]
    );
    assert_eq!(ids(&result.post_rerank), ids(&result.pre_rerank[..5]));
}

#[test]
fn merged_pool_is_unique_bounded_and_preserves_subquery_top_hits_after_hybrid_fusion() {
    for limit in [1, 5, 50, 100] {
        let lists = vec![
            (0..75).map(chunk).collect(),
            (50..125).map(chunk).collect(),
            (100..175).map(chunk).collect(),
        ];
        let result = select_candidates(lists, vec![chunk(74), chunk(124)], limit);
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
    let result = select_candidates(vec![vec![legacy.clone(), legacy, other]], vec![], 50);
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
            assert!(body["messages"][0]["content"].as_str().unwrap().contains("Preserve exact product names, API paths and error identifiers verbatim"));
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
    config.hybrid_search_enabled = true;
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
        vec![stale.clone(), chunk(50)],
        std::iter::once(stale.clone())
            .chain((0..9).map(chunk))
            .collect(),
        std::iter::once(stale).chain((7..16).map(chunk)).collect(),
    ];
    let search = Arc::new(TestSearch {
        query_hits: ["original question", "one", "two"]
            .into_iter()
            .enumerate()
            .map(|(i, q)| (q.into(), vec![hit(&format!("doc-{}", 90 + i))]))
            .collect(),
        ..Default::default()
    });
    let mut backend = store(lists, 10);
    for i in 90..93 {
        backend.lexical.insert(format!("doc-{i}"), vec![chunk(i)]);
    }
    let lookups = backend.lexical_calls.clone();
    let chat = service(
        &config,
        backend,
        embedding.clone(),
        Some(search.clone()),
        Some(reranker.clone()),
        false,
    )
    .await;
    let result = retrieve(&chat, RetrievalMode::Full).await;
    task.abort();
    assert_eq!(result.query_plan.sub_queries, ["one", "two"]);
    let embedded = embedding.0.lock().unwrap().clone();
    assert_eq!(embedded.len(), 3);
    assert_eq!(
        *search.queries.lock().unwrap(),
        ["original question", "one", "two"]
    );
    assert!(search
        .queries
        .lock()
        .unwrap()
        .iter()
        .all(|q| !q.contains("hypothetical")));
    let lookups = lookups.lock().unwrap().clone();
    assert_eq!(lookups.len(), 3);
    for i in 0..3 {
        assert!(lookups.iter().any(
            |(slug, _, vector)| slug == &format!("doc-{}", 90 + i) && vector == &vec![i as f32]
        ));
    }
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

fn hit(slug: &str) -> SearchHit {
    SearchHit {
        slug: slug.into(),
        title: slug.into(),
        content_preview: "untrusted search preview".into(),
        ..SearchHit::default()
    }
}

#[test]
fn natural_query_fanout_is_bounded_deduplicated_and_keeps_identifiers() {
    let original = "TP-200 POST /api/v1/devices ERR_DEVICE_42";
    let mut subqueries = vec![
        original.into(),
        "   ".into(),
        " TP-200 ".into(),
        "TP-200".into(),
    ];
    subqueries.extend((0..100).map(|i| format!("question {i}")));
    let queries = super::super::hybrid::natural_queries(original, &subqueries);
    assert_eq!(
        queries,
        [original, "TP-200", "question 0", "question 1", "question 2"]
    );
}

#[tokio::test]
async fn lexical_queries_preserve_identifiers_and_document_passage_work_is_bounded() {
    let query = "TP-200 POST /api/v1/devices ERR_DEVICE_42";
    let queries: Vec<_> = std::iter::once(query.to_string())
        .chain((0..20).map(|i| format!("variant {i}")))
        .collect();
    let search = Arc::new(TestSearch {
        hits: (0..100).map(|i| hit(&format!("doc-{i}"))).collect(),
        ..Default::default()
    });
    let mut backend = store(vec![], 0);
    for i in 0..100 {
        backend.lexical.insert(
            format!("doc-{i}"),
            (0..20)
                .map(|j| {
                    let mut c = chunk(i * 100 + j);
                    c.document_slug = format!("doc-{i}");
                    c
                })
                .collect(),
        );
    }
    let candidates = super::super::hybrid::lexical_candidates(
        search.as_ref(),
        &backend,
        &queries,
        &vec![vec![0.0]; queries.len()],
        Some(&user().document_visibility().0.unwrap()),
        false,
        false,
    )
    .await;
    assert_eq!(search.calls.load(std::sync::atomic::Ordering::SeqCst), 5);
    assert_eq!(search.queries.lock().unwrap()[0], query);
    let lookups = backend.lexical_calls.lock().unwrap();
    assert_eq!(lookups.len(), 8);
    assert!(lookups.iter().all(|(_, limit, _)| *limit == 3));
    assert_eq!(candidates.len(), 24);
    let mut per_doc = HashMap::new();
    for c in &candidates {
        *per_doc.entry(&c.document_slug).or_insert(0) += 1;
    }
    assert!(per_doc.values().all(|count| *count == 3));
    let fused = select_candidates(vec![(0..100).map(chunk).collect()], candidates, 50);
    assert_eq!(fused.len(), 50);
    assert_eq!(
        fused
            .iter()
            .map(chunk_identity)
            .collect::<HashSet<_>>()
            .len(),
        50
    );
    assert!(fused.iter().any(|c| c.point_id == "p0"));
}

#[tokio::test]
async fn unavailable_or_unreadable_lexical_passages_fall_back_without_trusting_previews() {
    for unavailable in [false, true] {
        let mut config = config();
        config.hybrid_search_enabled = true;
        let search = Arc::new(TestSearch {
            hits: vec![hit("forbidden"), hit("draft"), hit("visible")],
            ..Default::default()
        });
        let mut backend = store(vec![(0..50).map(chunk).collect()], 50);
        backend.lexical_failure = unavailable;
        let mut attachment = chunk(1000);
        attachment.source_kind = SourceKind::Attachment;
        attachment.document_slug = "visible".into();
        let mut visible = chunk(1001);
        visible.document_slug = "visible".into();
        // Unreadable documents return nothing from the visibility-enforcing store.
        backend
            .lexical
            .insert("visible".into(), vec![attachment, visible]);
        let lookups = backend.lexical_calls.clone();
        let reranker = Arc::new(RecordingReranker {
            promote: "p1001".into(),
            ..Default::default()
        });
        let chat = service(
            &config,
            backend,
            Arc::default(),
            Some(search),
            Some(reranker),
            false,
        )
        .await;
        let result = retrieve(&chat, RetrievalMode::Full).await;
        assert_eq!(lookups.lock().unwrap().len(), 3);
        assert!(result
            .pre_rerank
            .iter()
            .all(|c| c.document_slug != "forbidden"
                && c.document_slug != "draft"
                && c.source_kind != SourceKind::Attachment));
        assert!(result
            .post_rerank
            .iter()
            .all(|c| c.chunk_text.starts_with("stored passage")));
        if unavailable {
            assert_eq!(result.post_rerank[0].point_id, "p0");
        } else {
            assert_eq!(result.post_rerank[0].point_id, "p1001");
        }
    }
}

#[tokio::test]
async fn lexical_passages_work_with_no_global_vector_hits() {
    let mut config = config();
    config.hybrid_search_enabled = true;
    let search = Arc::new(TestSearch {
        hits: vec![hit("doc-99")],
        ..Default::default()
    });
    let mut backend = store(vec![vec![]], 50);
    backend.lexical.insert("doc-99".into(), vec![chunk(99)]);
    let chat = service(
        &config,
        backend,
        Arc::default(),
        Some(search),
        Some(Arc::new(RecordingReranker::default())),
        true,
    )
    .await;
    let result = retrieve(&chat, RetrievalMode::Full).await;
    assert_eq!(result.pre_rerank.len(), 1);
    assert_eq!(result.post_rerank[0].point_id, "p99");
}

#[tokio::test(start_paused = true)]
async fn hanging_lexical_discovery_or_passage_lookup_has_bounded_vector_fallback() {
    for discovery_hangs in [true, false] {
        let mut config = config();
        config.hybrid_search_enabled = true;
        let search = Arc::new(TestSearch {
            hits: vec![hit("doc-999")],
            hang: discovery_hangs,
            ..Default::default()
        });
        let mut backend = store(vec![(0..50).map(chunk).collect()], 50);
        backend.lexical_hang = !discovery_hangs;
        let chat = service(
            &config,
            backend,
            Arc::default(),
            Some(search),
            Some(Arc::new(RecordingReranker::default())),
            true,
        )
        .await;
        let start = tokio::time::Instant::now();
        let result = retrieve(&chat, RetrievalMode::Full).await;
        assert!(start.elapsed() <= std::time::Duration::from_secs(10));
        assert_eq!(result.post_rerank.len(), 5);
        assert_eq!(result.post_rerank[0].point_id, "p0");
    }
}

#[test]
fn mixed_branch_fusion_caps_unique_chunks_and_reserves_vector_and_lexical_evidence() {
    for limit in [1, 2, 5, 50, 100] {
        let lexical: Vec<_> = (100..124)
            .map(chunk)
            .chain(std::iter::once(chunk(7)))
            .collect();
        let candidates = select_candidates(vec![(0..100).map(chunk).collect()], lexical, limit);
        assert_eq!(candidates.len(), limit);
        assert_eq!(
            candidates
                .iter()
                .map(chunk_identity)
                .collect::<HashSet<_>>()
                .len(),
            limit
        );
        assert!(candidates.iter().any(|c| c.point_id == "p0"));
        if limit > 1 {
            assert!(candidates.iter().any(|c| c.point_id == "p100"));
        }
    }
}

fn adr_chunk(id: &str, slug: &str, text: &str, score: f32) -> VectorSearchResult {
    VectorSearchResult {
        point_id: id.into(),
        document_slug: slug.into(),
        document_title: slug.into(),
        chunk_text: text.into(),
        score,
        ..Default::default()
    }
}

fn attached_copy(id: &str, key: &str, text: &str, score: f32) -> VectorSearchResult {
    VectorSearchResult {
        source_kind: SourceKind::Attachment,
        attachment_key: Some(key.into()),
        ..adr_chunk(id, "", text, score)
    }
}

/// Ranked pool reproducing the mediator ADR question: ADR-0001 is indexed as a
/// page, as a `.md` attachment of ADR-0002 (with relative links) and under a
/// duplicate slug, so its copies crowd ADR-0002 out of a five-chunk context.
fn adr_pool() -> Vec<VectorSearchResult> {
    let adr1 = [
        "ADR-0001 decision",
        "ADR-0001 context, extended by [ADR-0002](/api/v1/assets/adr-0001/adr-0002.md)",
        "ADR-0001 consequences",
    ];
    vec![
        adr_chunk("a1-0", "adr-0001", adr1[0], 0.95),
        attached_copy("att-0", "adr-0002/adr-0001.md", adr1[0], 0.94),
        adr_chunk(
            "dup-0",
            "adr-0001-copy",
            &format!("  **{}**\n", adr1[0]),
            0.93,
        ),
        adr_chunk("a1-1", "adr-0001", adr1[1], 0.92),
        // The page rewrote the attached copy's relative link to an asset URL.
        attached_copy(
            "att-1",
            "adr-0002/adr-0001.md",
            "ADR-0001 context, extended by [ADR-0002](adr-0002.md)",
            0.91,
        ),
        adr_chunk("a1-2", "adr-0001", adr1[2], 0.90),
        adr_chunk("a3-0", "adr-0003", "ADR-0003 decision", 0.89),
        adr_chunk("a3-1", "adr-0003", "ADR-0003 context", 0.88),
        adr_chunk("a2-0", "adr-0002", "ADR-0002 decision", 0.87),
        adr_chunk("a2-1", "adr-0002", "ADR-0002 context", 0.86),
    ]
}

fn ids(chunks: &[VectorSearchResult]) -> Vec<&str> {
    chunks.iter().map(|c| c.point_id.as_str()).collect()
}

#[test]
fn duplicate_text_is_collapsed_before_the_candidate_cutoff() {
    let candidates = select_candidates(vec![adr_pool()], vec![], 6);
    assert_eq!(
        ids(&candidates),
        ["a1-0", "a1-1", "a1-2", "a3-0", "a3-1", "a2-0"]
    );
}

#[test]
fn collapsed_subquery_top_hit_is_represented_by_its_surviving_copy() {
    let first = vec![
        adr_chunk("x0", "x", "shared passage", 0.9),
        adr_chunk("x1", "x", "x only", 0.8),
        adr_chunk("x2", "x", "x more", 0.7),
    ];
    // The second subquery's top hit is a copy of a passage from the first one.
    let second = vec![
        attached_copy("copy", "y/x.md", "shared passage", 0.6),
        adr_chunk("y1", "y", "y only", 0.5),
    ];
    let result = select_candidates(vec![first, second], vec![], 2);
    assert_eq!(ids(&result), ["x0", "x1"]);
}

#[test]
fn context_selection_caps_chunks_per_document() {
    let candidates = select_candidates(vec![adr_pool()], vec![], 50);
    let context = select_context(candidates, 5);
    assert_eq!(ids(&context), ["a1-0", "a1-1", "a3-0", "a3-1", "a2-0"]);
}

#[test]
fn context_selection_fills_free_slots_from_capped_documents() {
    let only_two_documents = vec![
        adr_chunk("a0", "a", "a0", 0.9),
        adr_chunk("a1", "a", "a1", 0.8),
        adr_chunk("a2", "a", "a2", 0.7),
        adr_chunk("a3", "a", "a3", 0.6),
        adr_chunk("b0", "b", "b0", 0.5),
    ];
    let context = select_context(only_two_documents, 4);
    assert_eq!(ids(&context), ["a0", "a1", "a2", "b0"]);
}

#[test]
fn per_document_cap_grows_with_the_context_budget() {
    assert_eq!([1, 3, 5, 8, 10].map(per_document_cap), [2, 2, 2, 3, 4]);
}

#[tokio::test]
async fn duplicate_copies_do_not_crowd_a_relevant_document_out_of_the_context() {
    let reranker = Arc::new(RecordingReranker::default());
    let service = service(
        &config(),
        store(vec![adr_pool()], 50),
        Arc::default(),
        None,
        Some(reranker.clone()),
        true,
    )
    .await;
    let result = retrieve(&service, RetrievalMode::Full).await;
    assert_eq!(
        result.pre_rerank.len(),
        7,
        "text copies are not reranked twice"
    );
    assert_eq!(
        ids(&result.post_rerank),
        ["a1-0", "a1-1", "a3-0", "a3-1", "a2-0"]
    );
    // The whole pool is ranked so the cap can reach past the first five.
    assert_eq!(reranker.seen.lock().unwrap()[0].2, 7);
}

#[test]
fn code_that_differs_only_in_markup_like_text_is_not_collapsed() {
    let fenced = |ty: &str| format!("Identifiers:\n\n```rust\ntype Id = {ty};\n```\n");
    let inline = |ty: &str| format!("Use `{ty}` for identifiers.");
    let pool = vec![
        adr_chunk("u8", "a", &fenced("Vec<u8>"), 0.9),
        adr_chunk("u64", "b", &fenced("Vec<u64>"), 0.8),
        adr_chunk("i-u8", "c", &inline("Vec<u8>"), 0.7),
        adr_chunk("i-u64", "d", &inline("Vec<u64>"), 0.6),
        // Copies still collapse when only the prose markup differs.
        adr_chunk(
            "copy",
            "e",
            &format!("**Identifiers:**\n\n```rust\ntype Id = Vec<u8>;\n```"),
            0.5,
        ),
    ];
    let candidates = select_candidates(vec![pool], vec![], 10);
    assert_eq!(ids(&candidates), ["u8", "u64", "i-u8", "i-u64"]);
}

#[tokio::test]
async fn per_document_cap_applies_with_and_without_a_reranker() {
    let mut pool: Vec<_> = (0..5)
        .map(|i| {
            adr_chunk(
                &format!("a{i}"),
                "a",
                &format!("a passage {i}"),
                0.9 - i as f32 / 100.0,
            )
        })
        .collect();
    pool.push(adr_chunk("b0", "b", "b passage", 0.5));
    pool.push(adr_chunk("c0", "c", "c passage", 0.4));
    for with_reranker in [false, true] {
        let mut config = config();
        config.hybrid_search_enabled = true;
        // The vector overfetch sees B and C; lexical search finds nothing.
        let expected_limit = if with_reranker { 50 } else { 15 };
        let service = service(
            &config,
            store(vec![pool.clone()], expected_limit),
            Arc::default(),
            Some(Arc::new(TestSearch::default())),
            with_reranker.then(|| Arc::new(RecordingReranker::default()) as Arc<dyn Reranker>),
            true,
        )
        .await;
        let result = retrieve(&service, RetrievalMode::Full).await;
        assert_eq!(
            ids(&result.post_rerank),
            ["a0", "a1", "a2", "b0", "c0"],
            "reranker: {with_reranker}"
        );
    }
}

#[tokio::test]
async fn per_document_cap_sees_the_whole_fused_pool_without_hybrid_or_reranker() {
    use axum::{routing::post, Json, Router};
    use serde_json::{json, Value};
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|Json(_): Json<Value>| async move {
            let content = json!({"complexity":"multi_entity","sub_queries":["two"]}).to_string();
            Json(json!({"id":"test","object":"chat.completion","created":0,"model":"mock","choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":"stop"}]}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = config();
    config.llm.url = format!("http://{}/v1", listener.local_addr().unwrap());
    config.analyzer =
        Some(serde_json::from_value::<LlmStepConfig>(json!({"model":"analyzer"})).unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let a = |i: usize| adr_chunk(&format!("a{i}"), "a", &format!("a passage {i}"), 0.9);
    let lists = vec![
        (0..5).map(a).collect(),
        vec![
            a(0),
            a(1),
            a(2),
            adr_chunk("b0", "b", "b passage", 0.5),
            adr_chunk("c0", "c", "c passage", 0.4),
        ],
    ];
    let chat = service(&config, store(lists, 5), Arc::default(), None, None, true).await;
    let result = retrieve(&chat, RetrievalMode::Full).await;
    task.abort();
    assert_eq!(result.queries_embedded.len(), 2);
    assert_eq!(
        result.pre_rerank.len(),
        7,
        "every fused passage is a candidate"
    );
    assert_eq!(ids(&result.post_rerank), ["a0", "a1", "a2", "b0", "c0"]);
}
