use async_trait::async_trait;

use crate::db::source_models::DocumentSource;
use crate::error::AppError;

/// Persistence for admin-curated [`DocumentSource`] metadata.
#[async_trait]
pub trait DocumentSourceRepository: Send + Sync {
    /// Create or update the metadata for a source, keyed by `id`.
    ///
    /// `created_at` is preserved on update; `updated_at` is taken from the
    /// supplied record.
    async fn upsert(&self, source: DocumentSource) -> Result<(), AppError>;

    /// Fetch the metadata for a single source id.
    async fn find_by_id(&self, id: &str) -> Result<Option<DocumentSource>, AppError>;

    /// List all stored source-metadata records, ordered by id.
    async fn list(&self) -> Result<Vec<DocumentSource>, AppError>;

    /// Remove the metadata for a source id (does not touch its documents).
    async fn delete(&self, id: &str) -> Result<(), AppError>;
}

#[cfg(feature = "ssr")]
pub struct MongoDocumentSourceRepository {
    collection: mongodb::Collection<DocumentSource>,
}

#[cfg(feature = "ssr")]
impl MongoDocumentSourceRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("document_sources"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl DocumentSourceRepository for MongoDocumentSourceRepository {
    async fn upsert(&self, source: DocumentSource) -> Result<(), AppError> {
        use mongodb::bson::{doc, to_bson, DateTime};
        use mongodb::options::UpdateOptions;

        let maintainers = to_bson(&source.maintainers)
            .map_err(|e| AppError::Database(format!("serialize maintainers: {e}")))?;
        let created_at = DateTime::from_millis(source.created_at.timestamp_millis());
        let updated_at = DateTime::from_millis(source.updated_at.timestamp_millis());

        // `$set` the mutable metadata; `$setOnInsert` keeps `created_at` stable
        // across updates so we never need a read-before-write.
        let update = doc! {
            "$set": {
                "display_name": source.display_name,
                "repo_url": source.repo_url,
                "mainline_branch": source.mainline_branch,
                "maintainers": maintainers,
                "description": source.description,
                "review_enabled": source.review_enabled,
                "updated_at": updated_at,
            },
            "$setOnInsert": {
                "id": &source.id,
                "created_at": created_at,
            },
        };

        self.collection
            .update_one(doc! { "id": &source.id }, update)
            .with_options(UpdateOptions::builder().upsert(true).build())
            .await
            .map_err(|e| AppError::Database(format!("upsert document_source: {e}")))?;
        Ok(())
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<DocumentSource>, AppError> {
        use mongodb::bson::doc;
        self.collection
            .find_one(doc! { "id": id })
            .await
            .map_err(|e| AppError::Database(format!("find document_source by id: {e}")))
    }

    async fn list(&self) -> Result<Vec<DocumentSource>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        let cursor = self
            .collection
            .find(doc! {})
            .sort(doc! { "id": 1 })
            .await
            .map_err(|e| AppError::Database(format!("list document_sources: {e}")))?;

        cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Database(format!("collect document_sources: {e}")))
    }

    async fn delete(&self, id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;
        self.collection
            .delete_one(doc! { "id": id })
            .await
            .map_err(|e| AppError::Database(format!("delete document_source: {e}")))?;
        Ok(())
    }
}
