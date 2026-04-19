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

    /// List non-archived schemas whose name matches the provided exact or prefix scope.
    async fn find_by_name_prefix(&self, prefix: &str) -> Result<Vec<Schema>, AppError>;

    /// Add a new version to an existing schema.
    /// Returns an error if the schema doesn't exist or the version already exists.
    async fn add_version(&self, schema_name: &str, version: SchemaVersion) -> Result<(), AppError>;

    /// Set the archived flag on a specific schema version.
    async fn set_version_archived(
        &self,
        schema_name: &str,
        version: &str,
        archived: bool,
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
            .await?;

        Ok(())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, AppError> {
        use mongodb::bson::doc;

        Ok(self.collection.find_one(doc! { "name": name }).await?)
    }

    async fn list_all(&self) -> Result<Vec<Schema>, AppError> {
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder().sort(doc! { "name": 1 }).build();

        let mut cursor = self.collection.find(doc! {}).with_options(options).await?;

        let mut schemas = Vec::new();
        use futures::TryStreamExt;
        while let Some(schema) = cursor.try_next().await? {
            schemas.push(schema);
        }

        Ok(schemas)
    }

    async fn find_by_name_prefix(&self, prefix: &str) -> Result<Vec<Schema>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        let filter = if prefix.is_empty() {
            doc! {}
        } else {
            doc! {
                "$or": [
                    { "name": prefix },
                    { "name": { "$regex": format!("^{}/", regex_escape(prefix)) } }
                ]
            }
        };

        let mut cursor = self.collection.find(filter).await?;
        let mut schemas = Vec::new();
        while let Some(schema) = cursor.try_next().await? {
            schemas.push(schema);
        }

        Ok(schemas)
    }

    async fn add_version(&self, schema_name: &str, version: SchemaVersion) -> Result<(), AppError> {
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

        let version_bson = to_bson(&version)?;

        self.collection
            .update_one(
                doc! { "name": schema_name },
                doc! { "$push": { "versions": version_bson } },
            )
            .await?;

        Ok(())
    }

    async fn set_version_archived(
        &self,
        schema_name: &str,
        version: &str,
        archived: bool,
    ) -> Result<(), AppError> {
        let mut schema = self
            .find_by_name(schema_name)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Schema '{}' not found", schema_name)))?;

        let schema_version = schema
            .versions
            .iter_mut()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "Version '{}' not found for schema '{}'",
                    version, schema_name
                ))
            })?;

        schema_version.is_archived = archived;
        self.create_or_update(schema).await
    }

    async fn delete(&self, name: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let result = self.collection.delete_one(doc! { "name": name }).await?;

        if result.deleted_count == 0 {
            return Err(AppError::NotFound(format!("Schema '{}' not found", name)));
        }

        Ok(())
    }
}

#[cfg(feature = "ssr")]
fn regex_escape(s: &str) -> String {
    let special = [
        '.', '*', '+', '?', '(', ')', '[', ']', '{', '}', '\\', '^', '$', '|',
    ];
    let mut escaped = String::with_capacity(s.len());
    for c in s.chars() {
        if special.contains(&c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}
