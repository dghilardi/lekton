use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// A historical revision of a document, created when its content changes.
///
/// Distinct from a *release*: a revision is a fact ("this file was edited"),
/// a release is a decision ("this set is 1.2.0").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRevision {
    /// Internal UUID.
    pub id: String,
    /// The document slug this revision belongs to.
    pub slug: String,
    /// Auto-incrementing revision number (1-based, per slug).
    pub revision: u64,
    /// SHA-256 hash of the content at this revision.
    pub content_hash: String,
    /// S3 key where the historical content is stored.
    /// Format: `docs/history/{slug_escaped}/{revision}.md` — the S3 layout is
    /// unchanged by the rename, so stored objects keep resolving.
    pub s3_key: String,
    /// Who triggered this revision (token name or "legacy").
    pub updated_by: String,
    /// When this revision was created.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Repository trait for document revision history.
#[async_trait]
pub trait DocumentRevisionRepository: Send + Sync {
    /// Insert a new revision record.
    async fn create(&self, revision: DocumentRevision) -> Result<(), AppError>;

    /// Find the latest (highest-numbered) revision for a slug.
    async fn find_latest(&self, slug: &str) -> Result<Option<DocumentRevision>, AppError>;

    /// List all revisions for a slug, newest first.
    async fn list_by_slug(&self, slug: &str) -> Result<Vec<DocumentRevision>, AppError>;

    /// Return the next revision number for a slug (max + 1, or 1 when none exist).
    async fn next_revision_number(&self, slug: &str) -> Result<u64, AppError>;
}

/// MongoDB implementation of [`DocumentRevisionRepository`].
#[cfg(feature = "ssr")]
pub struct MongoDocumentRevisionRepository {
    collection: mongodb::Collection<DocumentRevision>,
}

#[cfg(feature = "ssr")]
impl MongoDocumentRevisionRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("document_revisions"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl DocumentRevisionRepository for MongoDocumentRevisionRepository {
    async fn create(&self, revision: DocumentRevision) -> Result<(), AppError> {
        self.collection.insert_one(&revision).await?;
        Ok(())
    }

    async fn find_latest(&self, slug: &str) -> Result<Option<DocumentRevision>, AppError> {
        use mongodb::bson::doc;
        use mongodb::options::FindOneOptions;

        let options = FindOneOptions::builder()
            .sort(doc! { "revision": -1 })
            .build();

        Ok(self
            .collection
            .find_one(doc! { "slug": slug })
            .with_options(options)
            .await?)
    }

    async fn list_by_slug(&self, slug: &str) -> Result<Vec<DocumentRevision>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder().sort(doc! { "revision": -1 }).build();

        let mut cursor = self
            .collection
            .find(doc! { "slug": slug })
            .with_options(options)
            .await?;

        let mut revisions = Vec::new();
        while let Some(revision) = cursor.try_next().await? {
            revisions.push(revision);
        }
        Ok(revisions)
    }

    async fn next_revision_number(&self, slug: &str) -> Result<u64, AppError> {
        let latest = self.find_latest(slug).await?;
        Ok(latest.map_or(1, |r| r.revision + 1))
    }
}
