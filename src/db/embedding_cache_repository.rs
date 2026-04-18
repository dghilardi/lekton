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
}

// ── MongoDB implementation ────────────────────────────────────────────────────

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

    /// Ensure the unique compound index on `(hash, model)` exists.
    pub async fn ensure_index(&self) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let opts = IndexOptions::builder().unique(true).build();
        let model = IndexModel::builder()
            .keys(doc! { "hash": 1, "model": 1 })
            .options(opts)
            .build();
        self.collection.create_index(model).await.map_err(|e| {
            AppError::Internal(format!("embedding_cache index creation failed: {e}"))
        })?;
        Ok(())
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
}
