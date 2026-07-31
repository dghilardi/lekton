use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::db::models::{Asset, DocumentReference, ExtractionStatus};
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

    /// Fetch every asset whose key is in `keys`, in a single query. Lets
    /// callers batch lookups (e.g. `check-hashes`) instead of issuing one query
    /// per key.
    async fn find_by_keys(&self, keys: &[String]) -> Result<Vec<Asset>, AppError>;

    /// List all assets, sorted by key.
    async fn list_all(&self) -> Result<Vec<Asset>, AppError>;

    /// List assets whose key starts with the given prefix, sorted by key.
    async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Asset>, AppError>;

    /// Delete an asset by key.
    async fn delete(&self, key: &str) -> Result<(), AppError>;

    /// Update only the extraction-tracking fields of an asset. A no-op when the
    /// key does not exist.
    async fn update_extraction(&self, key: &str, update: ExtractionUpdate) -> Result<(), AppError>;

    /// Reconcile references for an unversioned document slug.
    async fn set_references(
        &self,
        source_slug: &str,
        keys: &[String],
    ) -> Result<Vec<String>, AppError>;

    /// Reconcile `referenced_by` so that `source` references exactly `keys`:
    /// add the release-aware reference to those assets and remove it from every
    /// other asset that still lists it. Idempotent. Returns the keys whose
    /// reference set actually changed (added or removed), so callers can
    /// recompute derived state.
    async fn set_release_references(
        &self,
        source: &DocumentReference,
        keys: &[String],
    ) -> Result<Vec<String>, AppError> {
        if source.release.is_none() {
            self.set_references(&source.slug, keys).await
        } else {
            Err(AppError::Internal(
                "release-aware asset references are not implemented".into(),
            ))
        }
    }

    /// List assets whose extraction was left unfinished — `Pending` (never
    /// started) or `InProgress` (interrupted mid-flight). Used by a startup
    /// sweep to re-enqueue work that would otherwise be lost across a restart,
    /// since the extraction queue is in-memory.
    async fn list_unfinished_extractions(&self) -> Result<Vec<Asset>, AppError>;
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

    async fn find_by_keys(&self, keys: &[String]) -> Result<Vec<Asset>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        if keys.is_empty() {
            return Ok(vec![]);
        }
        let mut cursor = self
            .collection
            .find(doc! { "key": { "$in": keys } })
            .await?;
        let mut assets = Vec::new();
        while let Some(asset) = cursor.try_next().await? {
            assets.push(asset);
        }
        Ok(assets)
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

    async fn set_references(
        &self,
        source_slug: &str,
        keys: &[String],
    ) -> Result<Vec<String>, AppError> {
        self.set_release_references(&source_slug.into(), keys).await
    }

    async fn set_release_references(
        &self,
        source: &DocumentReference,
        keys: &[String],
    ) -> Result<Vec<String>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, to_bson};

        let source = to_bson(source).map_err(|e| {
            AppError::Internal(format!("failed to serialize asset document reference: {e}"))
        })?;

        // Keys currently referencing this exact document release, to compute the
        // exact diff without detaching references from older releases.
        let mut cursor = self
            .collection
            .find(doc! { "referenced_by": &source })
            .await?;
        let mut current = Vec::new();
        while let Some(asset) = cursor.try_next().await? {
            current.push(asset.key);
        }

        let removed: Vec<String> = current
            .iter()
            .filter(|k| !keys.contains(k))
            .cloned()
            .collect();
        let added: Vec<String> = keys
            .iter()
            .filter(|k| !current.contains(k))
            .cloned()
            .collect();

        if !removed.is_empty() {
            let removed_refs: Vec<&str> = removed.iter().map(|s| s.as_str()).collect();
            self.collection
                .update_many(
                    doc! { "key": { "$in": &removed_refs } },
                    doc! { "$pull": { "referenced_by": &source } },
                )
                .await?;
        }

        if !added.is_empty() {
            let added_refs: Vec<&str> = added.iter().map(|s| s.as_str()).collect();
            self.collection
                .update_many(
                    doc! { "key": { "$in": &added_refs } },
                    doc! { "$addToSet": { "referenced_by": &source } },
                )
                .await?;
        }

        let mut affected = removed;
        affected.extend(added);
        Ok(affected)
    }

    async fn list_unfinished_extractions(&self) -> Result<Vec<Asset>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, to_bson};

        let serialize = |s: &ExtractionStatus| {
            to_bson(s).map_err(|e| {
                AppError::Internal(format!("failed to serialize extraction status: {e}"))
            })
        };
        let pending = serialize(&ExtractionStatus::Pending)?;
        let in_progress = serialize(&ExtractionStatus::InProgress)?;

        let mut cursor = self
            .collection
            .find(doc! { "extraction_status": { "$in": [pending, in_progress] } })
            .await?;

        let mut assets = Vec::new();
        while let Some(asset) = cursor.try_next().await? {
            assets.push(asset);
        }
        Ok(assets)
    }
}
