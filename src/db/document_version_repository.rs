use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// A historical version of a document, created when content changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVersion {
    /// Internal UUID.
    pub id: String,
    /// The document slug this version belongs to.
    pub slug: String,
    /// Auto-incrementing version number (1-based, per slug).
    pub version: u64,
    /// SHA-256 hash of the content at this version.
    pub content_hash: String,
    /// S3 key where the historical content is stored.
    /// Format: `docs/history/{slug_escaped}/{version}.md`
    pub s3_key: String,
    /// Who triggered this version (token name or "legacy").
    pub updated_by: String,
    /// When this version was created.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Repository trait for document version history.
#[async_trait]
pub trait DocumentVersionRepository: Send + Sync {
    /// Insert a new version record.
    async fn create(&self, version: DocumentVersion) -> Result<(), AppError>;

    /// Find the latest (highest-numbered) version for a slug.
    async fn find_latest(&self, slug: &str) -> Result<Option<DocumentVersion>, AppError>;

    /// List all versions for a slug, ordered by version descending.
    async fn list_by_slug(&self, slug: &str) -> Result<Vec<DocumentVersion>, AppError>;

    /// Return the next version number for a slug (max + 1, or 1 if no versions exist).
    async fn next_version_number(&self, slug: &str) -> Result<u64, AppError>;
}

/// MongoDB implementation of [`DocumentVersionRepository`].
#[cfg(feature = "ssr")]
pub struct MongoDocumentVersionRepository {
    collection: mongodb::Collection<DocumentVersion>,
}

#[cfg(feature = "ssr")]
impl MongoDocumentVersionRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("document_versions"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl DocumentVersionRepository for MongoDocumentVersionRepository {
    async fn create(&self, version: DocumentVersion) -> Result<(), AppError> {
        self.collection.insert_one(&version).await?;
        Ok(())
    }

    async fn find_latest(&self, slug: &str) -> Result<Option<DocumentVersion>, AppError> {
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

    async fn list_by_slug(&self, slug: &str) -> Result<Vec<DocumentVersion>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder()
            .sort(doc! { "version": -1 })
            .build();

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
        Ok(latest.map_or(1, |v| v.version + 1))
    }
}
