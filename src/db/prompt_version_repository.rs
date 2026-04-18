use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// A historical version of a prompt body, created when the body content changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptVersion {
    pub id: String,
    pub slug: String,
    pub version: u64,
    pub content_hash: String,
    pub s3_key: String,
    pub updated_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait PromptVersionRepository: Send + Sync {
    async fn create(&self, version: PromptVersion) -> Result<(), AppError>;
    async fn find_latest(&self, slug: &str) -> Result<Option<PromptVersion>, AppError>;
    async fn list_by_slug(&self, slug: &str) -> Result<Vec<PromptVersion>, AppError>;
    async fn next_version_number(&self, slug: &str) -> Result<u64, AppError>;
}

#[cfg(feature = "ssr")]
pub struct MongoPromptVersionRepository {
    collection: mongodb::Collection<PromptVersion>,
}

#[cfg(feature = "ssr")]
impl MongoPromptVersionRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("prompt_versions"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl PromptVersionRepository for MongoPromptVersionRepository {
    async fn create(&self, version: PromptVersion) -> Result<(), AppError> {
        self.collection.insert_one(&version).await?;
        Ok(())
    }

    async fn find_latest(&self, slug: &str) -> Result<Option<PromptVersion>, AppError> {
        use mongodb::bson::doc;
        use mongodb::options::FindOneOptions;

        let options = FindOneOptions::builder()
            .sort(doc! { "version": -1 })
            .build();

        Ok(self
            .collection
            .find_one(doc! { "slug": slug })
            .with_options(options)
            .await?)
    }

    async fn list_by_slug(&self, slug: &str) -> Result<Vec<PromptVersion>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder().sort(doc! { "version": -1 }).build();

        let mut cursor = self
            .collection
            .find(doc! { "slug": slug })
            .with_options(options)
            .await?;

        let mut versions = Vec::new();
        while let Some(version) = cursor.try_next().await? {
            versions.push(version);
        }
        Ok(versions)
    }

    async fn next_version_number(&self, slug: &str) -> Result<u64, AppError> {
        let latest = self.find_latest(slug).await?;
        Ok(latest.map_or(1, |version| version.version + 1))
    }
}
