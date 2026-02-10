use async_trait::async_trait;

use crate::auth::models::AccessLevel;
use crate::db::models::Document;
use crate::error::AppError;

/// Repository trait for document operations.
///
/// This trait allows mocking the database layer in tests.
#[async_trait]
pub trait DocumentRepository: Send + Sync {
    /// Create a new document or update an existing one (matched by slug).
    async fn create_or_update(&self, doc: Document) -> Result<(), AppError>;

    /// Find a document by its slug.
    async fn find_by_slug(&self, slug: &str) -> Result<Option<Document>, AppError>;

    /// List all documents accessible at or below the given access level.
    async fn list_accessible(&self, max_level: AccessLevel) -> Result<Vec<Document>, AppError>;
}

/// MongoDB implementation of the DocumentRepository.
///
/// This is only available when the `ssr` feature is enabled (i.e., server-side).
#[cfg(feature = "ssr")]
pub struct MongoDocumentRepository {
    collection: mongodb::Collection<Document>,
}

#[cfg(feature = "ssr")]
impl MongoDocumentRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("documents"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl DocumentRepository for MongoDocumentRepository {
    async fn create_or_update(&self, doc: Document) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::ReplaceOptions;

        let filter = doc! { "slug": &doc.slug };
        let options = ReplaceOptions::builder().upsert(true).build();

        self.collection
            .replace_one(filter, &doc)
            .with_options(options)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<Document>, AppError> {
        use mongodb::bson::doc;

        self.collection
            .find_one(doc! { "slug": slug })
            .await
            .map_err(|e| AppError::Database(e.to_string()))
    }

    async fn list_accessible(&self, max_level: AccessLevel) -> Result<Vec<Document>, AppError> {
        use mongodb::bson::doc;

        // AccessLevel is serialized as a string, so we need to filter by the
        // known levels up to and including max_level.
        let allowed_levels: Vec<String> = [
            AccessLevel::Public,
            AccessLevel::Developer,
            AccessLevel::Architect,
            AccessLevel::Admin,
        ]
        .iter()
        .filter(|level| **level <= max_level)
        .map(|level| level.to_string())
        .collect();

        let filter = doc! {
            "access_level": { "$in": &allowed_levels }
        };

        let mut cursor = self
            .collection
            .find(filter)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut documents = Vec::new();
        use futures::TryStreamExt;
        while let Some(doc) = cursor
            .try_next()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        {
            documents.push(doc);
        }

        Ok(documents)
    }
}
