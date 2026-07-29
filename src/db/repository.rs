use async_trait::async_trait;

use crate::db::models::Document;
use crate::error::AppError;

/// A distinct import source discovered from the documents collection, together
/// with how many documents carry that `source_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDocCount {
    pub source_id: String,
    pub document_count: u64,
}

/// The Mongo clause selecting which release of each source a reader sees.
///
/// Unpinned sources resolve to whichever release currently carries the `latest`
/// alias, which is denormalized onto the document as `is_latest`. A pinned
/// source resolves to its exact release instead.
///
/// `is_latest` is matched permissively (`$exists: false` counts as `true`) to
/// match how the neighbouring `is_hidden` / `is_archived` / `is_draft` filters
/// treat documents written before their field existed.
///
/// `$nin` also matches documents with no `source_id` at all, so documents
/// ingested before source tracking keep resolving.
#[cfg(feature = "ssr")]
pub(crate) fn release_resolution_clause(
    pins: &crate::versioning::ReleasePins,
) -> mongodb::bson::Document {
    use mongodb::bson::{doc, Bson};

    let at_latest = doc! {
        "$or": [
            { "is_latest": { "$exists": false } },
            { "is_latest": true },
        ]
    };

    if pins.is_empty() {
        return at_latest;
    }

    let pinned_sources: Vec<Bson> = pins
        .source_ids()
        .into_iter()
        .map(|s| Bson::String(s.to_string()))
        .collect();

    let mut branches = vec![doc! {
        "$and": [
            { "source_id": { "$nin": pinned_sources } },
            at_latest,
        ]
    }];

    for pin in pins.iter() {
        branches.push(doc! {
            "source_id": &pin.source_id,
            "release": &pin.release,
        });
    }

    doc! { "$or": branches }
}

/// Repository trait for document operations.
///
/// This trait allows mocking the database layer in tests.
#[async_trait]
pub trait DocumentRepository: Send + Sync {
    /// Create a new document or update an existing one (matched by slug).
    async fn create_or_update(&self, doc: Document) -> Result<(), AppError>;

    /// Find a document by its slug.
    async fn find_by_slug(&self, slug: &str) -> Result<Option<Document>, AppError>;

    /// Find all documents whose slug is in `slugs`.
    ///
    /// Missing slugs are ignored. Callers that need lookup-by-slug semantics can
    /// index the returned vector into a map.
    async fn find_by_slugs(&self, slugs: &[String]) -> Result<Vec<Document>, AppError>;

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
    /// - `pins`: which release of each source to resolve. Pass
    ///   [`ReleasePins::default()`](crate::versioning::ReleasePins) for the
    ///   default view, where every source resolves to its `latest` release —
    ///   that is the right choice for indexing and machine-facing callers.
    async fn list_by_access_levels(
        &self,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
        pins: &crate::versioning::ReleasePins,
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

    /// Rename a document's slug in-place, preserving all other fields and history.
    ///
    /// Does nothing if `old_slug` is not found.
    async fn rename_slug(&self, old_slug: &str, new_slug: &str) -> Result<(), AppError>;

    /// Find a document by its source file path (e.g. `docs/guides/intro.md`).
    ///
    /// Returns `None` for documents ingested before `source_path` was introduced.
    /// Includes archived documents — callers must check `is_archived` if needed.
    async fn find_by_source_path(&self, source_path: &str) -> Result<Option<Document>, AppError>;

    /// Return all non-archived documents belonging to the given import source.
    ///
    /// Used at render time to build a `source_path → slug` map for relative
    /// link resolution. Returns an empty vec for unknown source ids.
    async fn find_all_by_source_id(&self, source_id: &str) -> Result<Vec<Document>, AppError>;

    /// Return the non-archived documents of one source belonging to exactly one
    /// release.
    ///
    /// `release: None` selects the *unversioned* bucket — documents with no
    /// release at all — not "any release". This is the scoping primitive the
    /// sync protocol needs: archiving must only consider the release being
    /// synced, so dropping a document in 1.2.0 leaves the 1.0.0 copy alone.
    ///
    /// The default implementation filters in memory on top of
    /// [`Self::find_all_by_source_id`]; the MongoDB backend overrides it with an
    /// indexed query.
    async fn find_all_by_source_id_and_release(
        &self,
        source_id: &str,
        release: Option<&str>,
    ) -> Result<Vec<Document>, AppError> {
        let documents = self.find_all_by_source_id(source_id).await?;
        Ok(documents
            .into_iter()
            .filter(|d| d.release.as_deref() == release)
            .collect())
    }

    /// List distinct non-empty `source_id` values across all documents, each
    /// with its document count. Used by the source registry to discover which
    /// import sources exist.
    ///
    /// Defaults to an empty list so test mocks need not implement it; the
    /// MongoDB backend overrides it with the real aggregation.
    async fn list_source_ids(&self) -> Result<Vec<SourceDocCount>, AppError> {
        Ok(vec![])
    }
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

    async fn find_by_slugs(&self, slugs: &[String]) -> Result<Vec<Document>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, Bson};

        if slugs.is_empty() {
            return Ok(vec![]);
        }

        let bson_slugs: Vec<Bson> = slugs.iter().cloned().map(Bson::String).collect();
        let mut cursor = self
            .collection
            .find(doc! { "slug": { "$in": bson_slugs } })
            .await?;

        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            documents.push(document);
        }

        Ok(documents)
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
        pins: &crate::versioning::ReleasePins,
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
            // Exclude archived documents
            doc! {
                "$or": [
                    { "is_archived": { "$exists": false } },
                    { "is_archived": false }
                ]
            },
            // Resolve each source to its pinned release, or to `latest`.
            release_resolution_clause(pins),
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

    async fn rename_slug(&self, old_slug: &str, new_slug: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;
        self.collection
            .update_one(
                doc! { "slug": old_slug },
                doc! { "$set": { "slug": new_slug } },
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

    async fn find_all_by_source_id(&self, source_id: &str) -> Result<Vec<Document>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        let filter = doc! {
            "source_id": source_id,
            "$or": [
                { "is_archived": { "$exists": false } },
                { "is_archived": false }
            ]
        };

        let mut cursor = self.collection.find(filter).await?;
        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            documents.push(document);
        }
        Ok(documents)
    }

    async fn find_all_by_source_id_and_release(
        &self,
        source_id: &str,
        release: Option<&str>,
    ) -> Result<Vec<Document>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, Bson};

        // `Bson::Null` matches both an absent field and an explicit null, which
        // together form the unversioned bucket.
        let release_match = match release {
            Some(r) => Bson::String(r.to_string()),
            None => Bson::Null,
        };

        let filter = doc! {
            "source_id": source_id,
            "release": release_match,
            "$or": [
                { "is_archived": { "$exists": false } },
                { "is_archived": false }
            ]
        };

        let mut cursor = self.collection.find(filter).await?;
        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            documents.push(document);
        }
        Ok(documents)
    }

    async fn list_source_ids(&self) -> Result<Vec<SourceDocCount>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        let pipeline = vec![
            doc! { "$match": { "source_id": { "$type": "string", "$ne": "" } } },
            doc! { "$group": { "_id": "$source_id", "count": { "$sum": 1 } } },
            doc! { "$sort": { "_id": 1 } },
        ];

        let mut cursor = self.collection.aggregate(pipeline).await?;
        let mut sources = Vec::new();
        while let Some(doc) = cursor.try_next().await? {
            let source_id = match doc.get_str("_id") {
                Ok(id) if !id.is_empty() => id.to_string(),
                _ => continue,
            };
            let document_count = doc
                .get_i64("count")
                .map(|v| v as u64)
                .or_else(|_| doc.get_i32("count").map(|v| v as u64))
                .unwrap_or(0);
            sources.push(SourceDocCount {
                source_id,
                document_count,
            });
        }
        Ok(sources)
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

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;
    use crate::versioning::ReleasePins;
    use mongodb::bson::doc;

    #[test]
    fn unpinned_resolution_accepts_latest_and_pre_field_documents() {
        let clause = release_resolution_clause(&ReleasePins::default());

        assert_eq!(
            clause,
            doc! {
                "$or": [
                    { "is_latest": { "$exists": false } },
                    { "is_latest": true },
                ]
            },
            "with no pins every source must resolve to its latest release"
        );
    }

    #[test]
    fn a_pin_scopes_only_its_own_source() {
        let mut pins = ReleasePins::default();
        pins.set("assets-manager", "1.1.0");

        let clause = release_resolution_clause(&pins);
        let branches = clause
            .get_array("$or")
            .expect("pinned resolution is a disjunction");

        assert_eq!(
            branches.len(),
            2,
            "one branch for the unpinned sources, one for the pin"
        );

        let unpinned = branches[0].as_document().expect("branch is a document");
        let and = unpinned
            .get_array("$and")
            .expect("unpinned branch is a conjunction");
        assert_eq!(
            and[0].as_document().unwrap(),
            &doc! { "source_id": { "$nin": ["assets-manager"] } },
            "the unpinned branch must exclude exactly the pinned sources"
        );

        assert_eq!(
            branches[1].as_document().unwrap(),
            &doc! { "source_id": "assets-manager", "release": "1.1.0" },
            "the pinned branch must select the exact release"
        );
    }

    #[test]
    fn every_pin_gets_its_own_branch() {
        let mut pins = ReleasePins::default();
        pins.set("a", "1.0.0");
        pins.set("b", "2.0.0");

        let clause = release_resolution_clause(&pins);
        let branches = clause.get_array("$or").expect("disjunction");

        assert_eq!(branches.len(), 3, "one unpinned branch plus one per pin");

        let unpinned = branches[0].as_document().unwrap();
        let and = unpinned.get_array("$and").unwrap();
        let nin = and[0]
            .as_document()
            .unwrap()
            .get_document("source_id")
            .unwrap()
            .get_array("$nin")
            .unwrap();
        assert_eq!(nin.len(), 2, "both pinned sources must be excluded");
    }
}
