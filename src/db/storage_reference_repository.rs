//! Read-only listing of every S3 object a MongoDB row points at, for the
//! storage consistency report.

use async_trait::async_trait;

use crate::error::AppError;

/// An S3 key and the row that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageReference {
    pub key: String,
    /// Collection and identity of the owning row, e.g. `documents guides/intro`.
    pub owner: String,
}

#[async_trait]
pub trait StorageReferenceRepository: Send + Sync {
    /// Every S3 key referenced by documents, document revisions, assets,
    /// schema versions, prompts, prompt versions and active import jobs.
    async fn list_storage_references(&self) -> Result<Vec<StorageReference>, AppError>;
}

pub struct MongoStorageReferenceRepository {
    db: mongodb::Database,
}

impl MongoStorageReferenceRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self { db: db.clone() }
    }

    /// The `key_field` of every row matching `filter` in `collection`, each
    /// labelled by `owner`. Only the projected fields are read.
    async fn references(
        &self,
        collection: &str,
        filter: bson::Document,
        projection: bson::Document,
        key_field: &str,
        owner: impl Fn(&bson::Document) -> String,
    ) -> Result<Vec<StorageReference>, AppError> {
        use futures::TryStreamExt;

        let mut cursor = self
            .db
            .collection::<bson::Document>(collection)
            .find(filter)
            .projection(projection)
            .await?;
        let mut references = Vec::new();
        while let Some(row) = cursor.try_next().await? {
            if let Ok(key) = row.get_str(key_field) {
                if !key.is_empty() {
                    references.push(StorageReference {
                        key: key.to_string(),
                        owner: owner(&row),
                    });
                }
            }
        }
        Ok(references)
    }
}

fn field(row: &bson::Document, name: &str) -> String {
    match row.get(name) {
        Some(bson::Bson::String(s)) => s.clone(),
        Some(bson::Bson::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[async_trait]
impl StorageReferenceRepository for MongoStorageReferenceRepository {
    async fn list_storage_references(&self) -> Result<Vec<StorageReference>, AppError> {
        use bson::doc;

        let mut references = self
            .references(
                "documents",
                doc! {},
                doc! { "slug": 1, "release": 1, "s3_key": 1 },
                "s3_key",
                |row| match field(row, "release") {
                    release if release.is_empty() => format!("documents {}", field(row, "slug")),
                    release => format!("documents {}@{release}", field(row, "slug")),
                },
            )
            .await?;
        references.extend(
            self.references(
                "document_revisions",
                doc! {},
                doc! { "slug": 1, "revision": 1, "s3_key": 1 },
                "s3_key",
                |row| {
                    format!(
                        "document_revisions {} #{}",
                        field(row, "slug"),
                        field(row, "revision")
                    )
                },
            )
            .await?,
        );
        references.extend(
            self.references(
                "assets",
                doc! {},
                doc! { "key": 1, "s3_key": 1 },
                "s3_key",
                |row| format!("assets {}", field(row, "key")),
            )
            .await?,
        );
        references.extend(
            self.references(
                "prompts",
                doc! {},
                doc! { "slug": 1, "s3_key": 1 },
                "s3_key",
                |row| format!("prompts {}", field(row, "slug")),
            )
            .await?,
        );
        references.extend(
            self.references(
                "prompt_versions",
                doc! {},
                doc! { "slug": 1, "version": 1, "s3_key": 1 },
                "s3_key",
                |row| {
                    format!(
                        "prompt_versions {} #{}",
                        field(row, "slug"),
                        field(row, "version")
                    )
                },
            )
            .await?,
        );
        // A finished job deletes its staging object (or leaves a harmless
        // leftover), so only active jobs own one.
        references.extend(
            self.references(
                "ingest_jobs",
                doc! { "active": true },
                doc! { "id": 1, "staging_key": 1 },
                "staging_key",
                |row| format!("ingest_jobs {}", field(row, "id")),
            )
            .await?,
        );

        // Schema versions are an array inside each schema.
        {
            use futures::TryStreamExt;
            let mut cursor = self
                .db
                .collection::<bson::Document>("schemas")
                .find(doc! {})
                .projection(doc! { "name": 1, "versions.version": 1, "versions.s3_key": 1 })
                .await?;
            while let Some(row) = cursor.try_next().await? {
                let name = field(&row, "name");
                let Ok(versions) = row.get_array("versions") else {
                    continue;
                };
                for version in versions.iter().filter_map(bson::Bson::as_document) {
                    if let Ok(key) = version.get_str("s3_key") {
                        references.push(StorageReference {
                            key: key.to_string(),
                            owner: format!("schemas {name} {}", field(version, "version")),
                        });
                    }
                }
            }
        }

        Ok(references)
    }
}
