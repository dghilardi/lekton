use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::db::models::{Asset, ExtractionStatus};
use crate::error::AppError;

/// The extraction-tracking fields to write, leaving the rest of the asset
/// (notably `referenced_by`) untouched. Fields left `None` are cleared.
#[derive(Debug, Clone)]
pub struct ExtractionUpdate {
    pub status: ExtractionStatus,
    pub error: Option<String>,
    pub extracted_content_hash: Option<String>,
    pub extracted_at: Option<DateTime<Utc>>,
    pub indexed_chunks: Option<u32>,
}

/// Repository trait for asset operations.
///
/// This trait allows mocking the database layer in tests.
#[async_trait]
pub trait AssetRepository: Send + Sync {
    /// Create a new asset or update an existing one (matched by key).
    async fn create_or_update(&self, asset: Asset) -> Result<(), AppError>;

    /// Find an asset by its key.
    async fn find_by_key(&self, key: &str) -> Result<Option<Asset>, AppError>;

    /// List all assets, sorted by key.
    async fn list_all(&self) -> Result<Vec<Asset>, AppError>;

    /// List assets whose key starts with the given prefix, sorted by key.
    async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Asset>, AppError>;

    /// Delete an asset by key.
    async fn delete(&self, key: &str) -> Result<(), AppError>;

    /// Update only the extraction-tracking fields of an asset. A no-op when the
    /// key does not exist.
    async fn update_extraction(&self, key: &str, update: ExtractionUpdate) -> Result<(), AppError>;
}

/// MongoDB implementation of the AssetRepository.
#[cfg(feature = "ssr")]
pub struct MongoAssetRepository {
    collection: mongodb::Collection<Asset>,
}

#[cfg(feature = "ssr")]
impl MongoAssetRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("assets"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl AssetRepository for MongoAssetRepository {
    async fn create_or_update(&self, asset: Asset) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::ReplaceOptions;

        let filter = doc! { "key": &asset.key };
        let options = ReplaceOptions::builder().upsert(true).build();

        self.collection
            .replace_one(filter, &asset)
            .with_options(options)
            .await?;

        Ok(())
    }

    async fn find_by_key(&self, key: &str) -> Result<Option<Asset>, AppError> {
        use mongodb::bson::doc;

        Ok(self.collection.find_one(doc! { "key": key }).await?)
    }

    async fn list_all(&self) -> Result<Vec<Asset>, AppError> {
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder().sort(doc! { "key": 1 }).build();

        let mut cursor = self.collection.find(doc! {}).with_options(options).await?;

        let mut assets = Vec::new();
        use futures::TryStreamExt;
        while let Some(asset) = cursor.try_next().await? {
            assets.push(asset);
        }

        Ok(assets)
    }

    async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Asset>, AppError> {
        use mongodb::bson::{doc, Regex};
        use mongodb::options::FindOptions;

        // Escape regex metacharacters in the prefix
        let escaped_prefix: String = prefix
            .chars()
            .flat_map(|c| {
                if ".*+?^${}()|[]\\".contains(c) {
                    vec!['\\', c]
                } else {
                    vec![c]
                }
            })
            .collect();
        let regex = Regex {
            pattern: format!("^{}", escaped_prefix),
            options: String::new(),
        };

        let options = FindOptions::builder().sort(doc! { "key": 1 }).build();

        let mut cursor = self
            .collection
            .find(doc! { "key": regex })
            .with_options(options)
            .await?;

        let mut assets = Vec::new();
        use futures::TryStreamExt;
        while let Some(asset) = cursor.try_next().await? {
            assets.push(asset);
        }

        Ok(assets)
    }

    async fn delete(&self, key: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let result = self.collection.delete_one(doc! { "key": key }).await?;

        if result.deleted_count == 0 {
            return Err(AppError::NotFound(format!("Asset '{}' not found", key)));
        }

        Ok(())
    }

    async fn update_extraction(&self, key: &str, update: ExtractionUpdate) -> Result<(), AppError> {
        use mongodb::bson::{doc, to_bson, DateTime as BsonDateTime};

        let status = to_bson(&update.status).map_err(|e| {
            AppError::Internal(format!("failed to serialize extraction status: {e}"))
        })?;
        let extracted_at = update.extracted_at.map(BsonDateTime::from_chrono);

        let set = doc! {
            "extraction_status": status,
            "extraction_error": update.error,
            "extracted_content_hash": update.extracted_content_hash,
            "extracted_at": extracted_at,
            "indexed_chunks": update.indexed_chunks.map(|c| c as i64),
        };

        self.collection
            .update_one(doc! { "key": key }, doc! { "$set": set })
            .await?;

        Ok(())
    }
}
