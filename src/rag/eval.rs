//! Shared types and helpers for offline RAG evaluation.
//!
//! Used by both `rag-eval` (single-run metrics) and `rag-bench` (multi-config
//! benchmark with automated ingest). The core evaluation logic lives here so
//! the two binaries stay in sync on how relevance and metrics are computed.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::models::{AuthenticatedUser, UserContext};
use crate::config::{AppConfig, RagConfig, SearchConfig};
use crate::db::chat_models::{ChatMessage, ChatSession};
use crate::db::chat_repository::ChatRepository;
use crate::error::AppError;
use crate::rag::chat::ChatService;
use crate::rag::embedding::{EmbeddingService, OpenAICompatibleEmbedding};
use crate::rag::reranker::{CrossEncoderReranker, Reranker};
use crate::rag::vectorstore::{QdrantVectorStore, VectorSearchResult, VectorStore};
use crate::search::client::{MeilisearchService, SearchService};

// ── Shared eval types ─────────────────────────────────────────────────────────

/// One record from a JSONL eval set.
#[derive(Debug, Deserialize)]
pub struct EvalQuery {
    #[serde(default)]
    pub id: Option<String>,
    pub query: String,
    /// Document slugs that must appear in the top-k results.
    #[serde(default)]
    pub expected_doc_slugs: Vec<String>,
    /// Specific Qdrant point ids that must appear in the top-k. Prefer slugs
    /// over ids because ids change when documents are re-indexed.
    #[serde(default)]
    pub expected_chunk_ids: Vec<String>,
    /// Text fragments that must appear in the chunk_text of at least one top-k
    /// result. Case-insensitive substring match. Each fragment is an independent
    /// recall expectation (adds 1 to the denominator).
    #[serde(default)]
    pub expected_text_fragments: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Metrics {
    pub recall_at_k: f64,
    pub mrr: f64,
    pub ndcg_at_k: f64,
}

#[derive(Debug, Serialize)]
pub struct RetrievedChunk {
    pub rank: usize,
    pub point_id: String,
    pub document_slug: String,
    pub chunk_text: String,
    pub score: f32,
    pub relevant: bool,
}

pub struct ScoredCandidates {
    pub retrieved: Vec<RetrievedChunk>,
    pub metrics: Metrics,
}

// ── Scoring logic ─────────────────────────────────────────────────────────────

/// Score one candidate list against the expected set.
///
/// A chunk is relevant when:
/// - its `point_id` is in `expected_chunk_ids`, OR
/// - its `document_slug` is in `expected_doc_slugs`, OR
/// - its `chunk_text` contains any entry from `expected_text_fragments`
///   (case-insensitive substring).
///
/// Recall denominator = distinct expected slugs + expected chunk ids +
/// expected text fragments (each is treated as an independent expectation).
/// Slug-level matching deduplicates: two chunks from the same expected doc
/// do not double-count.
pub fn score(
    candidates: &[VectorSearchResult],
    query: &EvalQuery,
    top_k: usize,
) -> ScoredCandidates {
    let expected_slugs: HashSet<&str> = query
        .expected_doc_slugs
        .iter()
        .map(|s| s.as_str())
        .collect();
    let expected_ids: HashSet<&str> = query
        .expected_chunk_ids
        .iter()
        .map(|s| s.as_str())
        .collect();
    let fragments_lower: Vec<String> = query
        .expected_text_fragments
        .iter()
        .map(|f| f.to_lowercase())
        .collect();

    let topk: Vec<&VectorSearchResult> = candidates.iter().take(top_k).collect();

    let mut retrieved = Vec::with_capacity(topk.len());
    let mut covered_slugs: HashSet<&str> = HashSet::new();
    let mut covered_ids: HashSet<&str> = HashSet::new();
    let mut covered_fragments: HashSet<usize> = HashSet::new();
    let mut first_relevant_rank: Option<usize> = None;
    let mut dcg: f64 = 0.0;

    for (i, c) in topk.iter().enumerate() {
        let rank = i + 1;
        let id_match = !c.point_id.is_empty() && expected_ids.contains(c.point_id.as_str());
        let slug_match = expected_slugs.contains(c.document_slug.as_str());
        let chunk_lower = c.chunk_text.to_lowercase();
        let fragment_matches: Vec<usize> = fragments_lower
            .iter()
            .enumerate()
            .filter(|(_, f)| chunk_lower.contains(f.as_str()))
            .map(|(idx, _)| idx)
            .collect();
        let relevant = id_match || slug_match || !fragment_matches.is_empty();

        // A chunk contributes to DCG only when it covers an *expected item that
        // has not been covered yet*. Without this guard, retrieving N chunks of
        // the same expected doc would N-count it in DCG while IDCG counts each
        // expectation at most once, pushing nDCG above 1.0.
        let mut new_cover = false;

        if relevant {
            if first_relevant_rank.is_none() {
                first_relevant_rank = Some(rank);
            }
            if id_match && covered_ids.insert(c.point_id.as_str()) {
                new_cover = true;
            }
            if slug_match && covered_slugs.insert(c.document_slug.as_str()) {
                new_cover = true;
            }
            for idx in fragment_matches {
                if covered_fragments.insert(idx) {
                    new_cover = true;
                }
            }
            if new_cover {
                dcg += 1.0 / ((rank + 1) as f64).log2();
            }
        }

        retrieved.push(RetrievedChunk {
            rank,
            point_id: c.point_id.clone(),
            document_slug: c.document_slug.clone(),
            chunk_text: c.chunk_text.clone(),
            score: c.score,
            relevant,
        });
    }

    let denom = expected_slugs.len() + expected_ids.len() + fragments_lower.len();
    let recall = if denom == 0 {
        0.0
    } else {
        (covered_slugs.len() + covered_ids.len() + covered_fragments.len()) as f64 / denom as f64
    };

    let mrr = first_relevant_rank.map(|r| 1.0 / r as f64).unwrap_or(0.0);

    let ideal_hits = denom.min(top_k);
    let idcg: f64 = (1..=ideal_hits)
        .map(|r| 1.0 / ((r + 1) as f64).log2())
        .sum();
    let ndcg = if idcg > 0.0 { dcg / idcg } else { 0.0 };

    ScoredCandidates {
        retrieved,
        metrics: Metrics {
            recall_at_k: recall,
            mrr,
            ndcg_at_k: ndcg,
        },
    }
}

pub fn mean_metrics<'a>(it: impl Iterator<Item = &'a Metrics>) -> Metrics {
    let mut count = 0_usize;
    let mut acc = Metrics::default();
    for m in it {
        acc.recall_at_k += m.recall_at_k;
        acc.mrr += m.mrr;
        acc.ndcg_at_k += m.ndcg_at_k;
        count += 1;
    }
    if count == 0 {
        return acc;
    }
    Metrics {
        recall_at_k: acc.recall_at_k / count as f64,
        mrr: acc.mrr / count as f64,
        ndcg_at_k: acc.ndcg_at_k / count as f64,
    }
}

/// Build an admin [`UserContext`] suitable for evaluation (unrestricted access).
pub fn admin_context() -> UserContext {
    UserContext {
        user: AuthenticatedUser {
            user_id: "rag-eval".into(),
            email: "rag-eval@local".into(),
            name: Some("rag-eval".into()),
            is_admin: true,
        },
        effective_access_levels: vec![],
        can_write: true,
        can_read_draft: true,
        can_write_draft: true,
    }
}

/// Read and parse a JSONL eval query file.
pub fn read_queries(path: &Path) -> Result<Vec<EvalQuery>, String> {
    let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (n, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| format!("read error on line {}: {e}", n + 1))?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let q: EvalQuery = serde_json::from_str(trimmed)
            .map_err(|e| format!("parse error on line {}: {e}", n + 1))?;
        if q.expected_doc_slugs.is_empty()
            && q.expected_chunk_ids.is_empty()
            && q.expected_text_fragments.is_empty()
        {
            return Err(format!(
                "line {}: query must specify at least one of \
                 expected_doc_slugs, expected_chunk_ids, or expected_text_fragments",
                n + 1
            ));
        }
        out.push(q);
    }
    Ok(out)
}

// ── RagEvalContext ─────────────────────────────────────────────────────────────

/// A wired-up retrieval stack ready to answer [`ChatService::retrieve_only`]
/// calls without any chat persistence.
pub struct RagEvalContext {
    pub chat_service: Arc<ChatService>,
    pub embedding: Arc<dyn EmbeddingService>,
    pub vectorstore: Arc<dyn VectorStore>,
}

impl RagEvalContext {
    /// Build the retrieval stack from a full application config.
    pub async fn from_config(config: &AppConfig) -> Result<Self, AppError> {
        Self::from_rag_config(&config.rag, Some(&config.search)).await
    }

    /// Build the retrieval stack directly from a [`RagConfig`].
    ///
    /// Pass `Some(search_config)` to enable hybrid RRF search; `None` disables it.
    pub async fn from_rag_config(
        rag: &RagConfig,
        search: Option<&SearchConfig>,
    ) -> Result<Self, AppError> {
        if !rag.is_enabled() {
            return Err(AppError::Internal(
                "RAG is not enabled — set rag.qdrant_url, rag.embedding_url".into(),
            ));
        }

        let embedding: Arc<dyn EmbeddingService> =
            Arc::new(OpenAICompatibleEmbedding::from_rag_config(rag)?);

        let vectorstore: Arc<dyn VectorStore> = Arc::new(QdrantVectorStore::from_rag_config(rag)?);

        let search_service: Option<Arc<dyn SearchService>> =
            search.and_then(|sc| match MeilisearchService::from_app_config(sc) {
                Ok(svc) => Some(Arc::new(svc) as Arc<dyn SearchService>),
                Err(e) => {
                    tracing::warn!(
                        "Meilisearch not available for eval: {e} — hybrid RRF will be skipped"
                    );
                    None
                }
            });

        let reranker: Option<Arc<dyn Reranker>> =
            CrossEncoderReranker::from_rag_config(rag).map(|r| Arc::new(r) as Arc<dyn Reranker>);

        let chat_repo: Arc<dyn ChatRepository> = Arc::new(NoopChatRepository);

        let chat_service = ChatService::from_rag_config(
            rag,
            chat_repo,
            embedding.clone(),
            vectorstore.clone(),
            search_service,
            reranker,
        )
        .await?;

        Ok(Self {
            chat_service: Arc::new(chat_service),
            embedding,
            vectorstore,
        })
    }
}

// ── NoopChatRepository ────────────────────────────────────────────────────────

/// A no-op [`ChatRepository`] for eval contexts that never persist chat sessions.
struct NoopChatRepository;

#[async_trait]
impl ChatRepository for NoopChatRepository {
    async fn create_session(&self, _session: ChatSession) -> Result<(), AppError> {
        Err(AppError::Internal(
            "NoopChatRepository: create_session called outside retrieve-only context".into(),
        ))
    }

    async fn get_session(&self, _id: &str) -> Result<Option<ChatSession>, AppError> {
        Ok(None)
    }

    async fn list_sessions_for_user(&self, _user_id: &str) -> Result<Vec<ChatSession>, AppError> {
        Ok(Vec::new())
    }

    async fn update_session_title(&self, _id: &str, _title: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn touch_session(&self, _id: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn add_message(&self, _msg: ChatMessage) -> Result<(), AppError> {
        Ok(())
    }

    async fn get_messages(
        &self,
        _session_id: &str,
        _limit: usize,
    ) -> Result<Vec<ChatMessage>, AppError> {
        Ok(Vec::new())
    }

    async fn delete_session(&self, _id: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn get_message_by_id(&self, _id: &str) -> Result<Option<ChatMessage>, AppError> {
        Ok(None)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn vsr(point_id: &str, slug: &str, chunk_text: &str, score: f32) -> VectorSearchResult {
        VectorSearchResult {
            point_id: point_id.into(),
            chunk_text: chunk_text.into(),
            document_slug: slug.into(),
            document_title: slug.into(),
            chunk_index: 0,
            section_path: Vec::new(),
            section_anchor: String::new(),
            score,
        }
    }

    fn query(slugs: &[&str]) -> EvalQuery {
        EvalQuery {
            id: None,
            query: "irrelevant".into(),
            expected_doc_slugs: slugs.iter().map(|s| s.to_string()).collect(),
            expected_chunk_ids: vec![],
            expected_text_fragments: vec![],
        }
    }

    #[test]
    fn perfect_retrieval_yields_full_metrics() {
        let candidates = vec![
            vsr("p1", "docs/a", "text a", 0.9),
            vsr("p2", "docs/b", "text b", 0.8),
        ];
        let q = query(&["docs/a", "docs/b"]);
        let s = score(&candidates, &q, 10);
        assert!((s.metrics.recall_at_k - 1.0).abs() < 1e-9);
        assert!((s.metrics.mrr - 1.0).abs() < 1e-9);
        assert!((s.metrics.ndcg_at_k - 1.0).abs() < 1e-9);
    }

    #[test]
    fn empty_retrieval_yields_zero_metrics() {
        let candidates: Vec<VectorSearchResult> = vec![];
        let q = query(&["docs/a"]);
        let s = score(&candidates, &q, 10);
        assert_eq!(s.metrics.recall_at_k, 0.0);
        assert_eq!(s.metrics.mrr, 0.0);
        assert_eq!(s.metrics.ndcg_at_k, 0.0);
    }

    #[test]
    fn first_relevant_at_rank_two_gives_mrr_half() {
        let candidates = vec![
            vsr("p0", "docs/x", "irrelevant", 0.9),
            vsr("p1", "docs/a", "target", 0.8),
        ];
        let q = query(&["docs/a"]);
        let s = score(&candidates, &q, 10);
        assert!((s.metrics.mrr - 0.5).abs() < 1e-9);
        assert!((s.metrics.recall_at_k - 1.0).abs() < 1e-9);
    }

    #[test]
    fn top_k_cutoff_is_respected() {
        let candidates = vec![
            vsr("p0", "docs/x", "x", 0.9),
            vsr("p1", "docs/y", "y", 0.8),
            vsr("p2", "docs/a", "target", 0.7),
        ];
        let q = query(&["docs/a"]);
        let s = score(&candidates, &q, 2);
        assert_eq!(s.metrics.recall_at_k, 0.0);
        assert_eq!(s.metrics.mrr, 0.0);
    }

    #[test]
    fn duplicate_slug_hits_do_not_double_count() {
        let candidates = vec![
            vsr("p0", "docs/a", "chunk 1 of a", 0.9),
            vsr("p1", "docs/a", "chunk 2 of a", 0.8),
            vsr("p2", "docs/b", "chunk of b", 0.7),
        ];
        let q = query(&["docs/a", "docs/b"]);
        let s = score(&candidates, &q, 10);
        assert!((s.metrics.recall_at_k - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ndcg_stays_in_unit_range_when_slug_repeats() {
        // Two expected slugs but five chunks of the same expected doc.
        // The duplicate hits must NOT inflate DCG beyond IDCG.
        let candidates = vec![
            vsr("p0", "docs/a", "x", 0.9),
            vsr("p1", "docs/a", "x", 0.8),
            vsr("p2", "docs/a", "x", 0.7),
            vsr("p3", "docs/a", "x", 0.6),
            vsr("p4", "docs/a", "x", 0.5),
        ];
        let q = query(&["docs/a", "docs/b"]);
        let s = score(&candidates, &q, 10);
        // Only docs/a is covered → nDCG should be the partial-coverage ratio,
        // strictly less than 1.0 and never above it.
        assert!(s.metrics.ndcg_at_k <= 1.0 + 1e-9);
        assert!(s.metrics.ndcg_at_k > 0.0);
    }

    #[test]
    fn text_fragment_match_counts_as_relevant() {
        let candidates = vec![
            vsr("p0", "docs/x", "The RBAC system controls access", 0.9),
            vsr("p1", "docs/y", "unrelated content", 0.8),
        ];
        let q = EvalQuery {
            id: None,
            query: "irrelevant".into(),
            expected_doc_slugs: vec![],
            expected_chunk_ids: vec![],
            expected_text_fragments: vec!["rbac system".into()],
        };
        let s = score(&candidates, &q, 10);
        assert!((s.metrics.recall_at_k - 1.0).abs() < 1e-9);
        assert!((s.metrics.mrr - 1.0).abs() < 1e-9);
        assert!(s.retrieved[0].relevant);
        assert!(!s.retrieved[1].relevant);
    }

    #[test]
    fn multiple_fragment_expectations_tracked_independently() {
        // Two fragments, only one found → recall = 0.5
        let candidates = vec![vsr("p0", "docs/x", "The RBAC system controls access", 0.9)];
        let q = EvalQuery {
            id: None,
            query: "irrelevant".into(),
            expected_doc_slugs: vec![],
            expected_chunk_ids: vec![],
            expected_text_fragments: vec!["rbac system".into(), "jwt token".into()],
        };
        let s = score(&candidates, &q, 10);
        assert!((s.metrics.recall_at_k - 0.5).abs() < 1e-9);
    }
}
