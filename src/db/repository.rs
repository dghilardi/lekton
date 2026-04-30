use async_trait::async_trait;

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

    /// List every document regardless of access level, draft, hidden, or archive state.
    ///
    /// This is intended for administrative maintenance jobs that must reconcile
    /// derived stores with the canonical document metadata.
    async fn list_all(&self) -> Result<Vec<Document>, AppError>;

    /// List documents the caller is allowed to see.
    ///
    /// - `allowed_levels`: the set of `access_level` names the caller can read
    ///   (e.g. `["public", "internal"]`).
    ///   Pass an empty slice to return only documents with no access-level restriction
    ///   (i.e. only `"public"` level documents when the caller is unauthenticated).
    ///   Admin callers should pass `None` to receive *all* documents.
    /// - `include_draft`: when `true`, draft documents are included in the result.
    ///   Admin callers and users with `can_read_draft` should set this to `true`.
    ///
    /// Hidden documents (`is_hidden = true`) are always excluded — they can only
    /// be fetched by slug.
    async fn list_by_access_levels(
        &self,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<Document>, AppError>;

    /// Update backlinks when a document's outgoing links change.
    ///
    /// Removes `source_slug` from backlinks of targets no longer linked,
    /// and adds `source_slug` to backlinks of newly linked targets.
    async fn update_backlinks(
        &self,
        source_slug: &str,
        old_links: &[String],
        new_links: &[String],
    ) -> Result<(), AppError>;

    /// Find all non-archived documents whose slug starts with `prefix`.
    ///
    /// If `prefix` is empty, returns all non-archived documents.
    async fn find_by_slug_prefix(&self, prefix: &str) -> Result<Vec<Document>, AppError>;

    /// Set the `is_archived` flag on a document.
    async fn set_archived(&self, slug: &str, archived: bool) -> Result<(), AppError>;

    /// Find a document by its source file path (e.g. `docs/guides/intro.md`).
    ///
    /// Returns `None` for documents ingested before `source_path` was introduced.
    async fn find_by_source_path(&self, source_path: &str) -> Result<Option<Document>, AppError>;
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
            .await?;

        Ok(())
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<Document>, AppError> {
        use mongodb::bson::doc;

        Ok(self.collection.find_one(doc! { "slug": slug }).await?)
    }

    async fn list_all(&self) -> Result<Vec<Document>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder()
            .sort(doc! { "order": 1, "slug": 1 })
            .build();
        let mut cursor = self.collection.find(doc! {}).with_options(options).await?;

        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            documents.push(document);
        }

        Ok(documents)
    }

    async fn list_by_access_levels(
        &self,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<Document>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, Bson};
        use mongodb::options::FindOptions;

        // Build the access-level filter.
        // `None` means admin — no restriction on level.
        let mut filter_parts: Vec<mongodb::bson::Document> = vec![
            // Exclude hidden documents
            doc! {
                "$or": [
                    { "is_hidden": { "$exists": false } },
                    { "is_hidden": false }
                ]
            },
        ];

        if let Some(levels) = allowed_levels {
            let bson_levels: Vec<Bson> = levels.iter().map(|l| Bson::String(l.clone())).collect();
            filter_parts.push(doc! { "access_level": { "$in": bson_levels } });
        }

        if !include_draft {
            filter_parts.push(doc! {
                "$or": [
                    { "is_draft": { "$exists": false } },
                    { "is_draft": false }
                ]
            });
        }

        let filter = doc! { "$and": filter_parts };

        let options = FindOptions::builder()
            .sort(doc! { "order": 1, "slug": 1 })
            .build();

        let mut cursor = self.collection.find(filter).with_options(options).await?;

        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            documents.push(document);
        }

        Ok(documents)
    }

    async fn update_backlinks(
        &self,
        source_slug: &str,
        old_links: &[String],
        new_links: &[String],
    ) -> Result<(), AppError> {
        use mongodb::bson::doc;

        // Targets that lost a link from this source
        let removed: Vec<&String> = old_links
            .iter()
            .filter(|link| !new_links.contains(link))
            .collect();

        // Targets that gained a link from this source
        let added: Vec<&String> = new_links
            .iter()
            .filter(|link| !old_links.contains(link))
            .collect();

        for slug in removed {
            self.collection
                .update_one(
                    doc! { "slug": slug },
                    doc! { "$pull": { "backlinks": source_slug } },
                )
                .await?;
        }

        for slug in added {
            self.collection
                .update_one(
                    doc! { "slug": slug },
                    doc! { "$addToSet": { "backlinks": source_slug } },
                )
                .await?;
        }

        Ok(())
    }

    async fn find_by_slug_prefix(&self, prefix: &str) -> Result<Vec<Document>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        let filter = if prefix.is_empty() {
            doc! {
                "$or": [
                    { "is_archived": { "$exists": false } },
                    { "is_archived": false }
                ]
            }
        } else {
            doc! {
                "$and": [
                    {
                        "$or": [
                            { "slug": prefix },
                            { "slug": { "$regex": format!("^{}/", regex_escape(prefix)) } }
                        ]
                    },
                    {
                        "$or": [
                            { "is_archived": { "$exists": false } },
                            { "is_archived": false }
                        ]
                    }
                ]
            }
        };

        let mut cursor = self.collection.find(filter).await?;
        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            documents.push(document);
        }
        Ok(documents)
    }

    async fn set_archived(&self, slug: &str, archived: bool) -> Result<(), AppError> {
        use mongodb::bson::doc;

        self.collection
            .update_one(
                doc! { "slug": slug },
                doc! { "$set": { "is_archived": archived } },
            )
            .await?;
        Ok(())
    }

    async fn find_by_source_path(&self, source_path: &str) -> Result<Option<Document>, AppError> {
        use mongodb::bson::doc;
        Ok(self
            .collection
            .find_one(doc! { "source_path": source_path })
            .await?)
    }
}

/// Escape special regex characters in a string for use in MongoDB regex queries.
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
