use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::config::RagConfig;
use crate::error::AppError;

use super::embedding::{EmbeddingService, OpenAICompatibleEmbedding};
use super::splitter::split_document;
use super::vectorstore::{ChunkPayload, QdrantVectorStore, VectorPoint, VectorStore};

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait RagService: Send + Sync {
    /// Index (or re-index) a document: delete old chunks, split, embed, upsert.
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
}

impl DefaultRagService {
    /// Create from pre-built service components.
    pub fn new(embedding: Arc<dyn EmbeddingService>, vectorstore: Arc<dyn VectorStore>) -> Self {
        Self {
            embedding,
            vectorstore,
        }
    }

    /// Build from application config.  Returns `Err` when required URLs are missing.
    pub fn from_rag_config(config: &RagConfig) -> Result<Self, AppError> {
        let embedding = OpenAICompatibleEmbedding::from_rag_config(config)?;
        let vectorstore = QdrantVectorStore::from_rag_config(config)?;
        Ok(Self {
            embedding: Arc::new(embedding),
            vectorstore: Arc::new(vectorstore),
        })
    }
}

#[async_trait]
impl RagService for DefaultRagService {
    async fn index_document(
        &self,
        slug: &str,
        title: &str,
        content: &str,
        access_level: &str,
        is_draft: bool,
        tags: &[String],
    ) -> Result<(), AppError> {
        // 1. Remove previous chunks for this document
        self.vectorstore.delete_by_slug(slug).await?;

        // 2. Split content into chunks
        let chunks = split_document(content);
        if chunks.is_empty() {
            return Ok(());
        }

        // 3. Embed all chunks
        let vectors = self.embedding.embed(&chunks).await?;

        // 4. Build Qdrant points, skipping any chunk whose embedding is empty.
        // Some embedding backends (e.g. Ollama) return [] for whitespace-only
        // or otherwise problematic inputs; sending a zero-dim vector to Qdrant
        // causes a hard error ("expected dim: 768, got 0").
        let num_chunks = chunks.len();
        let points: Vec<VectorPoint> = chunks
            .into_iter()
            .zip(vectors)
            .enumerate()
            .filter_map(|(idx, (text, vector))| {
                if vector.is_empty() {
                    tracing::warn!(
                        slug,
                        idx,
                        "RAG: embedding returned empty vector for chunk, skipping"
                    );
                    return None;
                }
                Some(VectorPoint {
                    id: Uuid::new_v4().to_string(),
                    vector,
                    payload: ChunkPayload {
                        chunk_text: text,
                        document_slug: slug.to_string(),
                        document_title: title.to_string(),
                        access_level: access_level.to_string(),
                        is_draft,
                        tags: tags.to_vec(),
                        chunk_index: idx as u32,
                    },
                })
            })
            .collect();

        // 5. Upsert into vector store
        self.vectorstore.upsert_chunks(points).await?;

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

    #[test]
    fn from_rag_config_fails_when_not_configured() {
        let config = RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: "test".into(),
            embedding_url: String::new(),
            embedding_model: "nomic-embed-text".into(),
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
        };
        assert!(DefaultRagService::from_rag_config(&config).is_err());
    }
}
