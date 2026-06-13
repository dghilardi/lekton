use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::config::RagConfig;
use crate::error::AppError;

use super::embedding::{build_embedding_service, EmbeddingService};
use super::splitter::split_document;
use super::vectorstore::{ChunkPayload, QdrantVectorStore, VectorPoint, VectorStore};

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait RagService: Send + Sync {
    /// Index (or re-index) a document: split, embed, upsert new chunks, then
    /// delete stale ones (upsert-then-delete-stale, so a failed embedding never
    /// leaves the document missing from the store).
    async fn index_document(
        &self,
        slug: &str,
        title: &str,
        content: &str,
        access_level: &str,
        is_draft: bool,
        tags: &[String],
    ) -> Result<(), AppError>;

    /// Remove all chunks for a document.
    async fn delete_document(&self, slug: &str) -> Result<(), AppError>;
}

// ── Default implementation ───────────────────────────────────────────────────

pub struct DefaultRagService {
    embedding: Arc<dyn EmbeddingService>,
    vectorstore: Arc<dyn VectorStore>,
    chunk_size_tokens: usize,
    chunk_overlap_tokens: usize,
}

impl DefaultRagService {
    /// Create from pre-built service components with explicit chunk sizing.
    pub fn new(
        embedding: Arc<dyn EmbeddingService>,
        vectorstore: Arc<dyn VectorStore>,
        chunk_size_tokens: usize,
        chunk_overlap_tokens: usize,
    ) -> Self {
        Self {
            embedding,
            vectorstore,
            chunk_size_tokens,
            chunk_overlap_tokens,
        }
    }

    /// Build from application config. Returns `Err` when required config is missing.
    pub async fn from_rag_config(config: &RagConfig) -> Result<Self, AppError> {
        let embedding = build_embedding_service(config).await?;
        let vectorstore = QdrantVectorStore::from_rag_config(config)?;
        Ok(Self {
            embedding,
            vectorstore: Arc::new(vectorstore),
            chunk_size_tokens: config.chunk_size_tokens as usize,
            chunk_overlap_tokens: config.chunk_overlap_tokens as usize,
        })
    }
}

#[async_trait]
impl RagService for DefaultRagService {
    #[tracing::instrument(skip(self, content, tags), fields(slug = %slug, access_level = %access_level))]
    async fn index_document(
        &self,
        slug: &str,
        title: &str,
        content: &str,
        access_level: &str,
        is_draft: bool,
        tags: &[String],
    ) -> Result<(), AppError> {
        // 1. Split content into token-aware chunks
        let chunks = split_document(content, self.chunk_size_tokens, self.chunk_overlap_tokens);
        if chunks.is_empty() {
            // Document has no indexable content: remove any previously indexed chunks.
            self.vectorstore.delete_by_slug(slug).await?;
            return Ok(());
        }

        // 2. Build enriched embedding texts: "Title > Section\n\nChunk text"
        // The embedding vector is computed on the enriched text for better recall of
        // context-ambiguous chunks. The display text (chunk.text) stays clean for prompt
        // injection and UI rendering; only embedding_text is sent to the embedder.
        let embedding_texts: Vec<String> = chunks
            .iter()
            .map(|c| {
                let mut prefix = title.to_string();
                if !c.section_path.is_empty() {
                    prefix.push_str(" > ");
                    prefix.push_str(&c.section_path.join(" > "));
                }
                format!("{}\n\n{}", prefix, c.text)
            })
            .collect();
        let vectors = self.embedding.embed(&embedding_texts).await?;

        // 3. Build Qdrant points, skipping any chunk whose embedding is empty.
        // Some embedding backends (e.g. Ollama) return [] for whitespace-only
        // or otherwise problematic inputs; sending a zero-dim vector to Qdrant
        // causes a hard error ("expected dim: 768, got 0").
        let points: Vec<VectorPoint> = chunks
            .into_iter()
            .zip(vectors)
            .filter_map(|(chunk, vector)| {
                if vector.is_empty() {
                    tracing::warn!(
                        slug,
                        idx = chunk.chunk_index,
                        "RAG: embedding returned empty vector for chunk, skipping"
                    );
                    return None;
                }
                Some(VectorPoint {
                    id: Uuid::new_v4().to_string(),
                    vector,
                    payload: ChunkPayload {
                        chunk_text: chunk.text,
                        section_path: chunk.section_path,
                        section_anchor: chunk.section_anchor,
                        document_slug: slug.to_string(),
                        document_title: title.to_string(),
                        access_level: access_level.to_string(),
                        is_draft,
                        tags: tags.to_vec(),
                        chunk_index: chunk.chunk_index,
                    },
                })
            })
            .collect();

        // All chunks failed to embed: leave the existing index untouched rather
        // than wiping it on a degenerate embedding run (avoids silent data loss).
        if points.is_empty() {
            tracing::warn!(
                slug,
                "RAG: no embeddable chunks produced; existing index left intact"
            );
            return Ok(());
        }

        // 4. Upsert the new chunks, then 5. delete the stale ones (everything for
        // this slug except the ids just written). Upsert-then-delete means the
        // document is never absent from the store: embedding is computed before
        // any destructive write, and a failure after upsert leaves duplicates
        // (corrected on the next reindex) instead of a gap.
        let num_chunks = points.len();
        let new_ids: Vec<String> = points.iter().map(|p| p.id.clone()).collect();
        self.vectorstore.upsert_chunks(points).await?;
        self.vectorstore.delete_stale_chunks(slug, &new_ids).await?;

        tracing::debug!(slug, chunks = num_chunks, "RAG: indexed document");
        Ok(())
    }

    async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
        self.vectorstore.delete_by_slug(slug).await?;
        tracing::debug!(slug, "RAG: deleted document chunks");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChatStepConfig, LlmConfig};

    #[tokio::test]
    async fn from_rag_config_fails_when_not_configured() {
        let config = RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: "test".into(),
            embedding_url: String::new(),
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
            },
            analyzer: None,
            hyde: None,
            rewriter: None,
        };
        assert!(DefaultRagService::from_rag_config(&config).await.is_err());
    }
}
