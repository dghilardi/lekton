use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

// ── Model ─────────────────────────────────────────────────────────────────────

/// A cached embedding entry stored in MongoDB.
///
/// Keyed on `(hash, model)` with a unique compound index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingCacheEntry {
    /// SHA-256 hex digest of the normalised chunk text.
    pub hash: String,
    /// Embedding model identifier (e.g. `"nomic-embed-text"`).
    pub model: String,
    /// The embedding vector.
    pub embedding: Vec<f32>,
    /// When this entry was generated.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub generated_at: DateTime<Utc>,
    /// Original chunk text, stored only when `embedding_cache_store_text = true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Entries stored under one cache namespace (see
/// [`crate::rag::cached_embedding::cache_namespace`]).
#[derive(Debug, Clone, PartialEq)]
pub struct NamespaceStats {
    /// The namespace, as stored in the `model` field.
    pub namespace: String,
    pub entries: u64,
    /// `entries` times the BSON size of one sampled entry: entries of one
    /// namespace share their vector length, so this is close unless
    /// `embedding_cache_store_text` stored texts of very different lengths.
    pub approx_bytes: u64,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait EmbeddingCacheRepository: Send + Sync {
    /// Batch lookup by `(hash, model)` pairs. Returns only the entries that exist.
    async fn get_many(
        &self,
        queries: &[(String, String)],
    ) -> Result<Vec<EmbeddingCacheEntry>, AppError>;

    /// Batch upsert (insert or replace) by `(hash, model)`.
    async fn upsert_many(&self, entries: Vec<EmbeddingCacheEntry>) -> Result<(), AppError>;

    /// Every namespace present, with its entry count, ordered by namespace.
    /// Reads no vectors.
    async fn namespace_stats(&self) -> Result<Vec<NamespaceStats>, AppError>;

    /// Delete every entry of the given namespaces; returns how many.
    async fn delete_namespaces(&self, namespaces: &[String]) -> Result<u64, AppError>;
}

// ── MongoDB implementation ────────────────────────────────────────────────────

/// The unique `(hash, model)` index of migration 010. The namespace queries
/// hint it: `model` is not its prefix, but scanning the whole index is a few
/// megabytes, while the collection scan the planner would pick otherwise reads
/// every stored vector.
#[cfg(feature = "ssr")]
const HASH_MODEL_INDEX: &str = "hash_1_model_1";

#[cfg(feature = "ssr")]
pub struct MongoEmbeddingCacheRepository {
    collection: mongodb::Collection<EmbeddingCacheEntry>,
}

#[cfg(feature = "ssr")]
impl MongoEmbeddingCacheRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("embedding_cache"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl EmbeddingCacheRepository for MongoEmbeddingCacheRepository {
    async fn get_many(
        &self,
        queries: &[(String, String)],
    ) -> Result<Vec<EmbeddingCacheEntry>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        if queries.is_empty() {
            return Ok(Vec::new());
        }

        // Build an $or query: [{ hash: h1, model: m }, { hash: h2, model: m }, ...]
        let conditions: Vec<mongodb::bson::Document> = queries
            .iter()
            .map(|(hash, model)| doc! { "hash": hash, "model": model })
            .collect();

        let filter = doc! { "$or": conditions };
        let cursor = self
            .collection
            .find(filter)
            .await
            .map_err(|e| AppError::Internal(format!("embedding cache find failed: {e}")))?;

        cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Internal(format!("embedding cache collect failed: {e}")))
    }

    async fn upsert_many(&self, entries: Vec<EmbeddingCacheEntry>) -> Result<(), AppError> {
        use mongodb::bson::{self, doc};

        if entries.is_empty() {
            return Ok(());
        }

        for entry in entries {
            let filter = doc! { "hash": &entry.hash, "model": &entry.model };
            let update_doc = bson::to_document(&entry)
                .map_err(|e| AppError::Internal(format!("embedding cache serialize error: {e}")))?;
            self.collection
                .update_one(filter, doc! { "$set": update_doc })
                .upsert(true)
                .await
                .map_err(|e| AppError::Internal(format!("embedding cache upsert failed: {e}")))?;
        }
        Ok(())
    }

    async fn namespace_stats(&self) -> Result<Vec<NamespaceStats>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, Document};
        use mongodb::options::Hint;

        let collection = self.collection.clone_with_type::<Document>();
        let hint = || Hint::Name(HASH_MODEL_INDEX.to_string());
        let query_error = |e: mongodb::error::Error| {
            AppError::Internal(format!("embedding cache namespace query failed: {e}"))
        };

        // Only `model` is needed, and the hinted index holds it: a covered
        // index scan that never fetches a document.
        let counts: Vec<Document> = collection
            .aggregate(vec![
                doc! { "$group": { "_id": "$model", "entries": { "$sum": 1 } } },
                doc! { "$sort": { "_id": 1 } },
            ])
            .hint(hint())
            .await
            .map_err(query_error)?
            .try_collect()
            .await
            .map_err(query_error)?;

        let mut stats = Vec::with_capacity(counts.len());
        for count in counts {
            let Ok(namespace) = count.get_str("_id") else {
                continue;
            };
            let entries = match count.get("entries") {
                Some(mongodb::bson::Bson::Int32(n)) => *n as u64,
                Some(mongodb::bson::Bson::Int64(n)) => *n as u64,
                _ => 0,
            };
            // One entry per namespace for its size: the index finds it, so
            // this reads a single document.
            let sample: Option<Document> = collection
                .aggregate(vec![
                    doc! { "$match": { "model": namespace } },
                    doc! { "$limit": 1 },
                    doc! { "$project": { "_id": 0, "size": { "$bsonSize": "$$ROOT" } } },
                ])
                .hint(hint())
                .await
                .map_err(query_error)?
                .try_next()
                .await
                .map_err(query_error)?;
            let entry_bytes = sample
                .and_then(|s| s.get_i32("size").ok())
                .map_or(0, |size| size.max(0) as u64);
            stats.push(NamespaceStats {
                namespace: namespace.to_string(),
                entries,
                approx_bytes: entries * entry_bytes,
            });
        }
        Ok(stats)
    }

    async fn delete_namespaces(&self, namespaces: &[String]) -> Result<u64, AppError> {
        use mongodb::bson::doc;
        use mongodb::options::Hint;

        if namespaces.is_empty() {
            return Ok(0);
        }
        // Hinted for the same reason as `namespace_stats`: only the matching
        // documents are fetched, instead of every vector in the collection.
        let result = self
            .collection
            .delete_many(doc! { "model": { "$in": namespaces } })
            .hint(Hint::Name(HASH_MODEL_INDEX.to_string()))
            .await
            .map_err(|e| AppError::Internal(format!("embedding cache purge failed: {e}")))?;
        Ok(result.deleted_count)
    }
}
