//! Lexical document discovery plus visibility-filtered, relevance-ranked passage
//! retrieval. This uses existing indexes and never turns search previews into context.
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    time::Duration,
};

use super::vectorstore::{SourceKind, VectorSearchResult, VectorStore};
use crate::{error::AppError, search::client::SearchService};

/// The standalone original plus up to four analyzer sub-queries.
pub(super) const MAX_QUERY_VARIANTS: usize = 5;
pub(super) const MAX_LEXICAL_DOCUMENTS: usize = 8;
pub(super) const PASSAGES_PER_DOCUMENT: usize = 3;
const HITS_PER_QUERY: usize = 20;

struct DocumentMatch {
    score: f64,
    query_index: usize,
    rank: usize,
}

/// Deadline for each of the two sequential phases (discovery, passage lookup),
/// so the lexical branch adds at most ~10 seconds to a turn.
const PHASE_TIMEOUT: Duration = Duration::from_secs(5);

async fn bounded<T>(future: impl Future<Output = Result<T, AppError>>) -> Result<T, AppError> {
    tokio::time::timeout(PHASE_TIMEOUT, future)
        .await
        .map_err(|_| AppError::Internal("hybrid lookup timed out".into()))?
}

/// Reuse the same bounded natural-language variants for both retrieval branches.
/// Original intent and exact identifiers survive decomposition and HyDE.
pub(super) fn natural_queries(original: &str, sub_queries: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    std::iter::once(original)
        .chain(sub_queries.iter().map(String::as_str))
        .map(str::trim)
        .filter(|q| !q.is_empty() && seen.insert((*q).to_string()))
        .take(MAX_QUERY_VARIANTS)
        .map(str::to_string)
        .collect()
}

pub(super) async fn lexical_candidates(
    search: &dyn SearchService,
    store: &dyn VectorStore,
    queries: &[String],
    vectors: &[Vec<f32>],
    levels: Option<&[String]>,
    include_draft: bool,
    include_attachments: bool,
) -> Vec<VectorSearchResult> {
    // A balanced document RRF merges lexical query results. Keep the strongest
    // matching query for passage selection, breaking ties by query order.
    let searches = queries
        .iter()
        .take(MAX_QUERY_VARIANTS)
        .map(|q| bounded(search.search(q, levels, include_draft)));
    let responses = futures::future::join_all(searches).await;
    let mut documents: HashMap<String, DocumentMatch> = HashMap::new();
    for (query_index, response) in responses.into_iter().enumerate() {
        let hits = match response {
            Ok(hits) => hits,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "hybrid lexical search failed; retaining other retrieval results"
                );
                continue;
            }
        };
        let mut seen = HashSet::new();
        let hits = hits
            .into_iter()
            .filter(|hit| include_attachments || hit.attachment_key.is_none())
            .filter(|hit| !hit.slug.is_empty() && seen.insert(hit.slug.clone()))
            .take(HITS_PER_QUERY);
        for (rank, hit) in hits.enumerate() {
            let entry = documents.entry(hit.slug).or_insert(DocumentMatch {
                score: 0.0,
                query_index,
                rank,
            });
            entry.score += 1.0 / (60 + rank + 1) as f64;
            if rank < entry.rank {
                entry.query_index = query_index;
                entry.rank = rank;
            }
        }
    }
    let mut documents: Vec<_> = documents.into_iter().collect();
    documents.sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then_with(|| a.0.cmp(&b.0)));
    let lookups = documents
        .iter()
        .take(MAX_LEXICAL_DOCUMENTS)
        .map(|(slug, matched)| {
            document_passages(
                store,
                vectors.get(matched.query_index),
                slug,
                levels,
                include_draft,
                include_attachments,
            )
        });
    let passages = futures::future::join_all(lookups).await;
    // Document rank first, passage relevance within each document. Round robin
    // gives every discovered document a chance before taking second passages.
    let mut ranked = Vec::new();
    for rank in 0..PASSAGES_PER_DOCUMENT {
        for chunks in &passages {
            if let Some(chunk) = chunks.get(rank) {
                ranked.push(chunk.clone());
            }
        }
    }
    ranked
}

async fn document_passages(
    store: &dyn VectorStore,
    vector: Option<&Vec<f32>>,
    slug: &str,
    levels: Option<&[String]>,
    include_draft: bool,
    include_attachments: bool,
) -> Vec<VectorSearchResult> {
    let Some(vector) = vector.filter(|v| !v.is_empty()) else {
        return Vec::new();
    };
    let lookup = store.search_document_chunks(
        vector.clone(),
        slug,
        PASSAGES_PER_DOCUMENT,
        levels,
        include_draft,
        include_attachments,
    );
    match bounded(lookup).await {
        Ok(chunks) => chunks
            .into_iter()
            // Defense in depth for custom backends: never accept chunks
            // for a different document or a disabled attachment source.
            .filter(|c| {
                c.document_slug == slug
                    && (include_attachments || c.source_kind != SourceKind::Attachment)
            })
            .take(PASSAGES_PER_DOCUMENT)
            .collect(),
        Err(error) => {
            tracing::warn!(
                %error,
                "hybrid passage lookup failed; retaining other retrieval results"
            );
            Vec::new()
        }
    }
}
