use async_trait::async_trait;
use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, PointStruct,
    SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::{Payload, Qdrant};

use crate::config::RagConfig;
use crate::error::AppError;

// ── Data types ───────────────────────────────────────────────────────────────

/// Metadata stored alongside each vector in Qdrant.
#[derive(Debug, Clone)]
pub struct ChunkPayload {
    pub chunk_text: String,
    pub document_slug: String,
    pub document_title: String,
    pub access_level: String,
    pub is_draft: bool,
    pub tags: Vec<String>,
    pub chunk_index: u32,
    /// Heading hierarchy above this chunk (e.g. `["Architecture", "Storage Layer"]`).
    pub section_path: Vec<String>,
    /// URL-safe anchor for the deepest heading (e.g. `"storage-layer"`).
    pub section_anchor: String,
}

/// A vector point ready for upsert into Qdrant.
#[derive(Debug, Clone)]
pub struct VectorPoint {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: ChunkPayload,
}

/// A single search hit returned from the vector store.
#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    /// Stable identifier of the underlying vector point. Used by offline
    /// evaluation tooling to match retrieved chunks against an expected set;
    /// not consumed by the chat-time pipeline. Empty when the backend did not
    /// return an id.
    pub point_id: String,
    pub chunk_text: String,
    pub document_slug: String,
    pub document_title: String,
    pub score: f32,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Create the collection if it does not already exist.
    async fn ensure_collection(&self, dimensions: u32) -> Result<(), AppError>;

    /// Insert (or replace) a batch of vector points.
    async fn upsert_chunks(&self, points: Vec<VectorPoint>) -> Result<(), AppError>;

    /// Delete all chunks that belong to a given document slug.
    async fn delete_by_slug(&self, slug: &str) -> Result<(), AppError>;

    /// Semantic search filtered by access levels and draft visibility.
    ///
    /// * `access_levels` — `None` means unrestricted (admin), `Some([])` means no access.
    /// * `include_draft` — whether to include `is_draft = true` documents.
    async fn search(
        &self,
        vector: Vec<f32>,
        limit: usize,
        access_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<VectorSearchResult>, AppError>;
}

// ── Qdrant implementation ────────────────────────────────────────────────────

pub struct QdrantVectorStore {
    client: Qdrant,
    collection: String,
}

impl QdrantVectorStore {
    pub fn from_rag_config(config: &RagConfig) -> Result<Self, AppError> {
        if config.qdrant_url.is_empty() {
            return Err(AppError::Internal("qdrant_url is required for RAG".into()));
        }
        let client = Qdrant::from_url(&config.qdrant_url)
            .build()
            .map_err(|e| AppError::Internal(format!("failed to build Qdrant client: {e}")))?;
        Ok(Self {
            client,
            collection: config.qdrant_collection.clone(),
        })
    }
}

#[async_trait]
impl VectorStore for QdrantVectorStore {
    async fn ensure_collection(&self, dimensions: u32) -> Result<(), AppError> {
        let exists = self
            .client
            .collection_exists(&self.collection)
            .await
            .map_err(|e| AppError::Internal(format!("qdrant collection_exists: {e}")))?;

        if !exists {
            self.client
                .create_collection(
                    CreateCollectionBuilder::new(&self.collection).vectors_config(
                        VectorParamsBuilder::new(dimensions as u64, Distance::Cosine),
                    ),
                )
                .await
                .map_err(|e| AppError::Internal(format!("qdrant create_collection: {e}")))?;
            tracing::info!(
                collection = %self.collection,
                dimensions,
                "created Qdrant collection"
            );
        }

        Ok(())
    }

    async fn upsert_chunks(&self, points: Vec<VectorPoint>) -> Result<(), AppError> {
        if points.is_empty() {
            return Ok(());
        }

        let qdrant_points: Vec<PointStruct> = points
            .into_iter()
            .map(|p| {
                let mut payload = Payload::new();
                payload.insert("chunk_text", p.payload.chunk_text);
                payload.insert("document_slug", p.payload.document_slug);
                payload.insert("document_title", p.payload.document_title);
                payload.insert("access_level", p.payload.access_level);
                payload.insert("is_draft", p.payload.is_draft);
                payload.insert("chunk_index", p.payload.chunk_index as i64);
                payload.insert("section_anchor", p.payload.section_anchor);
                let tag_values: Vec<qdrant_client::qdrant::Value> =
                    p.payload.tags.into_iter().map(|t| t.into()).collect();
                payload.insert(
                    "tags",
                    qdrant_client::qdrant::Value {
                        kind: Some(qdrant_client::qdrant::value::Kind::ListValue(
                            qdrant_client::qdrant::ListValue { values: tag_values },
                        )),
                    },
                );
                let section_values: Vec<qdrant_client::qdrant::Value> = p
                    .payload
                    .section_path
                    .into_iter()
                    .map(|s| s.into())
                    .collect();
                payload.insert(
                    "section_path",
                    qdrant_client::qdrant::Value {
                        kind: Some(qdrant_client::qdrant::value::Kind::ListValue(
                            qdrant_client::qdrant::ListValue {
                                values: section_values,
                            },
                        )),
                    },
                );

                PointStruct::new(p.id, p.vector, payload)
            })
            .collect();

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection, qdrant_points).wait(true))
            .await
            .map_err(|e| AppError::Internal(format!("qdrant upsert_points: {e}")))?;

        Ok(())
    }

    async fn delete_by_slug(&self, slug: &str) -> Result<(), AppError> {
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection)
                    .points(Filter::must([Condition::matches(
                        "document_slug",
                        slug.to_string(),
                    )]))
                    .wait(true),
            )
            .await
            .map_err(|e| AppError::Internal(format!("qdrant delete_points: {e}")))?;

        Ok(())
    }

    async fn search(
        &self,
        vector: Vec<f32>,
        limit: usize,
        access_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        let mut conditions: Vec<Condition> = Vec::new();

        // Filter by access levels (skip for admins where access_levels is None)
        if let Some(levels) = access_levels {
            if levels.is_empty() {
                // No access → return empty results immediately
                return Ok(Vec::new());
            }
            conditions.push(Condition::matches(
                "access_level",
                levels.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            ));
        }

        // Exclude drafts unless explicitly included
        if !include_draft {
            conditions.push(Condition::matches("is_draft", false));
        }

        let mut builder =
            SearchPointsBuilder::new(&self.collection, vector, limit as u64).with_payload(true);

        if !conditions.is_empty() {
            builder = builder.filter(Filter::must(conditions));
        }

        let response = self
            .client
            .search_points(builder)
            .await
            .map_err(|e| AppError::Internal(format!("qdrant search_points: {e}")))?;

        let results = response
            .result
            .into_iter()
            .map(|scored| {
                let point_id = scored
                    .id
                    .and_then(|id| id.point_id_options)
                    .map(|opt| match opt {
                        qdrant_client::qdrant::point_id::PointIdOptions::Uuid(s) => s,
                        qdrant_client::qdrant::point_id::PointIdOptions::Num(n) => n.to_string(),
                    })
                    .unwrap_or_default();
                let chunk_text = scored
                    .payload
                    .get("chunk_text")
                    .and_then(|v| v.as_str())
                    .cloned()
                    .unwrap_or_default();
                let document_slug = scored
                    .payload
                    .get("document_slug")
                    .and_then(|v| v.as_str())
                    .cloned()
                    .unwrap_or_default();
                let document_title = scored
                    .payload
                    .get("document_title")
                    .and_then(|v| v.as_str())
                    .cloned()
                    .unwrap_or_default();

                VectorSearchResult {
                    point_id,
                    chunk_text,
                    document_slug,
                    document_title,
                    score: scored.score,
                }
            })
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rag_config_fails_with_empty_url() {
        let config = RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: "test".into(),
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
            reranker_url: String::new(),
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
        };
        assert!(QdrantVectorStore::from_rag_config(&config).is_err());
    }

    #[test]
    fn from_rag_config_succeeds_with_url() {
        let config = RagConfig {
            qdrant_url: "http://localhost:6334".into(),
            qdrant_collection: "test_collection".into(),
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
            reranker_url: String::new(),
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
        };
        assert!(QdrantVectorStore::from_rag_config(&config).is_ok());
    }

    #[test]
    fn search_returns_empty_when_no_access() {
        // Synchronous check: empty access_levels should short-circuit
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let config = RagConfig {
            qdrant_url: "http://localhost:6334".into(),
            qdrant_collection: "test".into(),
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
            reranker_url: String::new(),
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
        };
        let store = QdrantVectorStore::from_rag_config(&config).unwrap();

        let result =
            rt.block_on(async { store.search(vec![0.0; 768], 10, Some(&[]), false).await });
        assert!(result.unwrap().is_empty());
    }
}
