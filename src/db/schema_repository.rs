use async_trait::async_trait;

use crate::db::models::{Schema, SchemaVersion};
use crate::error::AppError;

/// Repository trait for schema operations.
///
/// This trait allows mocking the database layer in tests.
#[async_trait]
pub trait SchemaRepository: Send + Sync {
    /// Create a new schema or update an existing one (matched by name).
    async fn create_or_update(&self, schema: Schema) -> Result<(), AppError>;

    /// Find a schema by its name.
    async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, AppError>;

    /// List all schemas.
    async fn list_all(&self) -> Result<Vec<Schema>, AppError>;

    /// Add a new version to an existing schema.
    /// Returns an error if the schema doesn't exist or the version already exists.
    async fn add_version(
        &self,
        schema_name: &str,
        version: SchemaVersion,
    ) -> Result<(), AppError>;

    /// Delete a schema by name.
    async fn delete(&self, name: &str) -> Result<(), AppError>;
}

/// MongoDB implementation of the SchemaRepository.
#[cfg(feature = "ssr")]
pub struct MongoSchemaRepository {
    collection: mongodb::Collection<Schema>,
}

#[cfg(feature = "ssr")]
impl MongoSchemaRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("schemas"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl SchemaRepository for MongoSchemaRepository {
    async fn create_or_update(&self, schema: Schema) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::ReplaceOptions;

        let filter = doc! { "name": &schema.name };
        let options = ReplaceOptions::builder().upsert(true).build();

        self.collection
            .replace_one(filter, &schema)
            .with_options(options)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, AppError> {
        use mongodb::bson::doc;

        self.collection
            .find_one(doc! { "name": name })
            .await
            .map_err(|e| AppError::Database(e.to_string()))
    }

    async fn list_all(&self) -> Result<Vec<Schema>, AppError> {
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder()
            .sort(doc! { "name": 1 })
            .build();

        let mut cursor = self
            .collection
            .find(doc! {})
            .with_options(options)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut schemas = Vec::new();
        use futures::TryStreamExt;
        while let Some(schema) = cursor
            .try_next()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        {
            schemas.push(schema);
        }

        Ok(schemas)
    }

    async fn add_version(
        &self,
        schema_name: &str,
        version: SchemaVersion,
    ) -> Result<(), AppError> {
        use mongodb::bson::{doc, to_bson};

        // Check if the schema exists
        let existing = self.find_by_name(schema_name).await?;
        let Some(existing) = existing else {
            return Err(AppError::NotFound(format!(
                "Schema '{}' not found",
                schema_name
            )));
        };

        // Check if version already exists
        if existing
            .versions
            .iter()
            .any(|v| v.version == version.version)
        {
            return Err(AppError::BadRequest(format!(
                "Version '{}' already exists for schema '{}'",
                version.version, schema_name
            )));
        }

        let version_bson =
            to_bson(&version).map_err(|e| AppError::Database(e.to_string()))?;

        self.collection
            .update_one(
                doc! { "name": schema_name },
                doc! { "$push": { "versions": version_bson } },
            )
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let result = self
            .collection
            .delete_one(doc! { "name": name })
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        if result.deleted_count == 0 {
            return Err(AppError::NotFound(format!(
                "Schema '{}' not found",
                name
            )));
        }

        Ok(())
    }
}
