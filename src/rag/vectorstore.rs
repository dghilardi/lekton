use async_trait::async_trait;
use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, PointStruct,
    ScrollPointsBuilder, SearchPointsBuilder, SetPayloadPointsBuilder, UpsertPointsBuilder,
    VectorParamsBuilder,
};
use qdrant_client::{Payload, Qdrant};

use crate::config::RagConfig;
use crate::error::AppError;

// ── Data types ───────────────────────────────────────────────────────────────

/// Origin of an indexed chunk: the body of a document, or an attachment
/// (PDF/text file) referenced by one or more documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceKind {
    #[default]
    Document,
    Attachment,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Document => "document",
            SourceKind::Attachment => "attachment",
        }
    }

    /// Parse the value stored in the Qdrant payload, defaulting to `Document`.
    pub fn from_payload(s: &str) -> Self {
        match s {
            "attachment" => SourceKind::Attachment,
            _ => SourceKind::Document,
        }
    }
}

/// Metadata stored alongside each vector in Qdrant.
#[derive(Debug, Clone)]
pub struct ChunkPayload {
    pub chunk_text: String,
    pub document_slug: String,
    pub document_title: String,
    /// Whether this chunk comes from a document body or an attachment.
    pub source_kind: SourceKind,
    /// Asset key of the originating attachment (`None` for document chunks).
    /// Used to build citations and resolve referencing documents.
    pub attachment_key: Option<String>,
    /// 1-based page number within the attachment (`None` for document chunks or
    /// non-paginated sources).
    pub source_page: Option<u32>,
    /// Access levels under which this chunk is visible. For document chunks this
    /// is a single-element list; attachment chunks may carry several (one per
    /// referencing document). A search matches when this list intersects the
    /// caller's access levels.
    pub access_levels: Vec<String>,
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
#[derive(Debug, Clone, Default)]
pub struct VectorSearchResult {
    /// Stable identifier of the underlying vector point. Used by offline
    /// evaluation tooling to match retrieved chunks against an expected set;
    /// not consumed by the chat-time pipeline. Empty when the backend did not
    /// return an id.
    pub point_id: String,
    pub chunk_text: String,
    pub document_slug: String,
    pub document_title: String,
    pub chunk_index: u32,
    pub section_path: Vec<String>,
    pub section_anchor: String,
    pub score: f32,
    /// Whether this hit comes from a document body or an attachment.
    pub source_kind: SourceKind,
    /// Asset key of the originating attachment (`None` for document hits).
    pub attachment_key: Option<String>,
    /// 1-based page number within the attachment (`None` for document hits).
    pub source_page: Option<u32>,
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

    /// Delete chunks for a slug **except** the given point ids.
    ///
    /// Used after an upsert to remove stale chunks left by a previous indexing
    /// pass (upsert-then-delete-stale), so the document is never absent from the
    /// store. An empty `keep_ids` deletes every chunk for the slug.
    async fn delete_stale_chunks(&self, slug: &str, keep_ids: &[String]) -> Result<(), AppError>;

    /// Delete all chunks belonging to an attachment.
    async fn delete_by_attachment_key(&self, attachment_key: &str) -> Result<(), AppError>;

    /// Delete chunks for an attachment **except** the given point ids
    /// (upsert-then-delete-stale, mirroring [`delete_stale_chunks`]).
    async fn delete_stale_attachment_chunks(
        &self,
        attachment_key: &str,
        keep_ids: &[String],
    ) -> Result<(), AppError>;

    /// Overwrite the `access_levels` payload of every chunk for an attachment
    /// without re-embedding. Used when the set of referencing documents (or
    /// their access levels) changes.
    async fn set_attachment_access_levels(
        &self,
        attachment_key: &str,
        access_levels: &[String],
    ) -> Result<(), AppError>;

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

    /// Return all chunks that belong to the given document section.
    async fn get_section_chunks(
        &self,
        document_slug: &str,
        section_anchor: &str,
        access_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<VectorSearchResult>, AppError>;

    /// Cheap reachability probe for readiness checks.
    async fn health_check(&self) -> Result<(), AppError>;
}

// ── Qdrant implementation ────────────────────────────────────────────────────

pub struct QdrantVectorStore {
    client: Qdrant,
    collection: String,
}

impl QdrantVectorStore {
    pub fn new(url: &str, collection: impl Into<String>) -> Result<Self, AppError> {
        let client = Qdrant::from_url(url)
            .build()
            .map_err(|e| AppError::Internal(format!("failed to build Qdrant client: {e}")))?;
        Ok(Self {
            client,
            collection: collection.into(),
        })
    }

    pub fn from_rag_config(config: &RagConfig) -> Result<Self, AppError> {
        if config.qdrant_url.is_empty() {
            return Err(AppError::Internal("qdrant_url is required for RAG".into()));
        }
        Self::new(&config.qdrant_url, config.qdrant_collection.clone())
    }

    fn visibility_conditions(
        access_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<Condition>, AppError> {
        let mut conditions: Vec<Condition> = Vec::new();

        if let Some(levels) = access_levels {
            if levels.is_empty() {
                return Err(AppError::Internal(
                    "section lookup requested with empty access levels".into(),
                ));
            }
            // `matches` against a list payload field is a "match any": the chunk
            // passes when its `access_levels` list intersects the caller's levels.
            conditions.push(Condition::matches(
                "access_levels",
                levels.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            ));
        }

        if !include_draft {
            conditions.push(Condition::matches("is_draft", false));
        }

        Ok(conditions)
    }

    /// Drop the entire collection, ignoring "not found" errors.
    pub async fn delete_collection(&self) -> Result<(), AppError> {
        let exists = self
            .client
            .collection_exists(&self.collection)
            .await
            .map_err(|e| AppError::Internal(format!("qdrant collection_exists: {e}")))?;
        if exists {
            self.client
                .delete_collection(&self.collection)
                .await
                .map_err(|e| AppError::Internal(format!("qdrant delete_collection: {e}")))?;
        }
        Ok(())
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
                let access_level_values: Vec<qdrant_client::qdrant::Value> = p
                    .payload
                    .access_levels
                    .into_iter()
                    .map(|a| a.into())
                    .collect();
                payload.insert(
                    "access_levels",
                    qdrant_client::qdrant::Value {
                        kind: Some(qdrant_client::qdrant::value::Kind::ListValue(
                            qdrant_client::qdrant::ListValue {
                                values: access_level_values,
                            },
                        )),
                    },
                );
                payload.insert("is_draft", p.payload.is_draft);
                payload.insert("source_kind", p.payload.source_kind.as_str());
                if let Some(key) = p.payload.attachment_key {
                    payload.insert("attachment_key", key);
                }
                if let Some(page) = p.payload.source_page {
                    payload.insert("source_page", page as i64);
                }
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

    async fn delete_stale_chunks(&self, slug: &str, keep_ids: &[String]) -> Result<(), AppError> {
        let mut must_not = Vec::new();
        if !keep_ids.is_empty() {
            must_not.push(Condition::has_id(keep_ids.iter().cloned()));
        }
        let filter = Filter {
            must: vec![Condition::matches("document_slug", slug.to_string())],
            must_not,
            ..Default::default()
        };

        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection)
                    .points(filter)
                    .wait(true),
            )
            .await
            .map_err(|e| AppError::Internal(format!("qdrant delete_stale_chunks: {e}")))?;

        Ok(())
    }

    async fn delete_by_attachment_key(&self, attachment_key: &str) -> Result<(), AppError> {
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection)
                    .points(Filter::must([Condition::matches(
                        "attachment_key",
                        attachment_key.to_string(),
                    )]))
                    .wait(true),
            )
            .await
            .map_err(|e| AppError::Internal(format!("qdrant delete_points (attachment): {e}")))?;

        Ok(())
    }

    async fn delete_stale_attachment_chunks(
        &self,
        attachment_key: &str,
        keep_ids: &[String],
    ) -> Result<(), AppError> {
        let mut must_not = Vec::new();
        if !keep_ids.is_empty() {
            must_not.push(Condition::has_id(keep_ids.iter().cloned()));
        }
        let filter = Filter {
            must: vec![Condition::matches(
                "attachment_key",
                attachment_key.to_string(),
            )],
            must_not,
            ..Default::default()
        };

        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection)
                    .points(filter)
                    .wait(true),
            )
            .await
            .map_err(|e| {
                AppError::Internal(format!("qdrant delete_stale_attachment_chunks: {e}"))
            })?;

        Ok(())
    }

    async fn set_attachment_access_levels(
        &self,
        attachment_key: &str,
        access_levels: &[String],
    ) -> Result<(), AppError> {
        let mut payload = Payload::new();
        let values: Vec<qdrant_client::qdrant::Value> =
            access_levels.iter().map(|a| a.clone().into()).collect();
        payload.insert(
            "access_levels",
            qdrant_client::qdrant::Value {
                kind: Some(qdrant_client::qdrant::value::Kind::ListValue(
                    qdrant_client::qdrant::ListValue { values },
                )),
            },
        );

        self.client
            .set_payload(
                SetPayloadPointsBuilder::new(&self.collection, payload)
                    .points_selector(Filter::must([Condition::matches(
                        "attachment_key",
                        attachment_key.to_string(),
                    )]))
                    .wait(true),
            )
            .await
            .map_err(|e| {
                AppError::Internal(format!(
                    "qdrant set_payload (attachment access_levels): {e}"
                ))
            })?;

        Ok(())
    }

    async fn search(
        &self,
        vector: Vec<f32>,
        limit: usize,
        access_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        let conditions = match Self::visibility_conditions(access_levels, include_draft) {
            Ok(conditions) => conditions,
            Err(_) => return Ok(Vec::new()),
        };

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
                let chunk_index = scored
                    .payload
                    .get("chunk_index")
                    .and_then(|v| v.as_integer())
                    .map(|n| n as u32)
                    .unwrap_or_default();
                let section_anchor = scored
                    .payload
                    .get("section_anchor")
                    .and_then(|v| v.as_str())
                    .cloned()
                    .unwrap_or_default();
                let section_path = scored
                    .payload
                    .get("section_path")
                    .and_then(|v| v.as_list())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|v| v.as_str().cloned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let source_kind = scored
                    .payload
                    .get("source_kind")
                    .and_then(|v| v.as_str())
                    .map(|s| SourceKind::from_payload(s))
                    .unwrap_or_default();
                let attachment_key = scored
                    .payload
                    .get("attachment_key")
                    .and_then(|v| v.as_str())
                    .cloned();
                let source_page = scored
                    .payload
                    .get("source_page")
                    .and_then(|v| v.as_integer())
                    .map(|n| n as u32);

                VectorSearchResult {
                    point_id,
                    chunk_text,
                    document_slug,
                    document_title,
                    chunk_index,
                    section_path,
                    section_anchor,
                    score: scored.score,
                    source_kind,
                    attachment_key,
                    source_page,
                }
            })
            .collect();

        Ok(results)
    }

    async fn get_section_chunks(
        &self,
        document_slug: &str,
        section_anchor: &str,
        access_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        let mut conditions = match Self::visibility_conditions(access_levels, include_draft) {
            Ok(conditions) => conditions,
            Err(_) => return Ok(Vec::new()),
        };
        conditions.push(Condition::matches(
            "document_slug",
            document_slug.to_string(),
        ));
        conditions.push(Condition::matches(
            "section_anchor",
            section_anchor.to_string(),
        ));

        let response = self
            .client
            .scroll(
                ScrollPointsBuilder::new(&self.collection)
                    .filter(Filter::must(conditions))
                    .limit(256)
                    .with_payload(true)
                    .with_vectors(false),
            )
            .await
            .map_err(|e| AppError::Internal(format!("qdrant scroll section chunks: {e}")))?;

        let mut results: Vec<VectorSearchResult> = response
            .result
            .into_iter()
            .map(|point| {
                let point_id = point
                    .id
                    .and_then(|id| id.point_id_options)
                    .map(|opt| match opt {
                        qdrant_client::qdrant::point_id::PointIdOptions::Uuid(s) => s,
                        qdrant_client::qdrant::point_id::PointIdOptions::Num(n) => n.to_string(),
                    })
                    .unwrap_or_default();
                let chunk_text = point
                    .payload
                    .get("chunk_text")
                    .and_then(|v| v.as_str())
                    .cloned()
                    .unwrap_or_default();
                let document_title = point
                    .payload
                    .get("document_title")
                    .and_then(|v| v.as_str())
                    .cloned()
                    .unwrap_or_default();
                let chunk_index = point
                    .payload
                    .get("chunk_index")
                    .and_then(|v| v.as_integer())
                    .map(|n| n as u32)
                    .unwrap_or_default();
                let section_path = point
                    .payload
                    .get("section_path")
                    .and_then(|v| v.as_list())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|v| v.as_str().cloned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                VectorSearchResult {
                    point_id,
                    chunk_text,
                    document_slug: document_slug.to_string(),
                    document_title,
                    chunk_index,
                    section_path,
                    section_anchor: section_anchor.to_string(),
                    score: 0.0,
                    ..Default::default()
                }
            })
            .collect();

        results.sort_by_key(|chunk| chunk.chunk_index);
        Ok(results)
    }

    async fn health_check(&self) -> Result<(), AppError> {
        // `collection_exists` is a cheap round-trip that fails if Qdrant is
        // unreachable, which is what a readiness probe needs to detect.
        self.client
            .collection_exists(&self.collection)
            .await
            .map(|_| ())
            .map_err(|e| AppError::Internal(format!("qdrant health check: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChatStepConfig, LlmConfig};

    fn make_config(qdrant_url: &str) -> RagConfig {
        RagConfig {
            qdrant_url: qdrant_url.into(),
            qdrant_collection: "test".into(),
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
            vlm: None,
            attachment_page_text_threshold: 100,
        }
    }

    #[test]
    fn from_rag_config_fails_with_empty_url() {
        assert!(QdrantVectorStore::from_rag_config(&make_config("")).is_err());
    }

    #[test]
    fn from_rag_config_succeeds_with_url() {
        let mut config = make_config("http://localhost:6334");
        config.qdrant_collection = "test_collection".into();
        assert!(QdrantVectorStore::from_rag_config(&config).is_ok());
    }

    #[test]
    fn search_returns_empty_when_no_access() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let store =
            QdrantVectorStore::from_rag_config(&make_config("http://localhost:6334")).unwrap();
        let result =
            rt.block_on(async { store.search(vec![0.0; 768], 10, Some(&[]), false).await });
        assert!(result.unwrap().is_empty());
    }
}
