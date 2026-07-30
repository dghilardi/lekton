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

/// Pick, among every release of one slug, the copy a reader at `pins` sees.
///
/// The single-document counterpart of [`release_resolution_clause`], kept as a
/// pure function so both share one definition of "which release wins" and it can
/// be tested without a database.
///
/// A pin naming a release that does not exist for this slug falls through to
/// `latest`, so a stale shared link degrades instead of 404-ing.
pub fn resolve_by_release(
    candidates: Vec<Document>,
    pins: &crate::versioning::ReleasePins,
) -> Option<Document> {
    let pinned = candidates.iter().position(|doc| {
        let Some(source_id) = doc.source_id.as_deref() else {
            return false;
        };
        match (pins.release_for(source_id), doc.release.as_deref()) {
            (Some(wanted), Some(actual)) => wanted == actual,
            _ => false,
        }
    });

    let chosen = pinned.or_else(|| candidates.iter().position(|doc| doc.is_latest));

    chosen.map(|idx| candidates.into_iter().nth(idx).expect("index just found"))
}

/// Repository trait for document operations.
///
/// This trait allows mocking the database layer in tests.
#[async_trait]
pub trait DocumentRepository: Send + Sync {
    /// Create a new document or update an existing one (matched by slug).
    async fn create_or_update(&self, doc: Document) -> Result<(), AppError>;

    /// Find a document by its slug.
    ///
    /// Returns an arbitrary copy when the slug exists in several releases. Use
    /// it only where that cannot happen or does not matter (an unversioned
    /// source, or a check that any copy exists); reader and writer paths that
    /// mean a specific release go through [`Self::find_all_by_slug`] plus
    /// [`resolve_by_release`].
    async fn find_by_slug(&self, slug: &str) -> Result<Option<Document>, AppError>;

    /// Every release of one slug.
    ///
    /// The single primitive behind release-aware lookup: callers pick the copy
    /// they mean — [`resolve_by_release`] for readers, an exact `release` match
    /// for the ingest write path.
    ///
    /// Includes archived documents, like [`Self::find_by_slug`], because callers
    /// apply their own visibility rules.
    async fn find_all_by_slug(&self, slug: &str) -> Result<Vec<Document>, AppError> {
        self.find_by_slugs(std::slice::from_ref(&slug.to_string()))
            .await
    }

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

    /// Set the `is_archived` flag on one release of a document.
    ///
    /// `release` identifies which copy to touch; `None` is the unversioned
    /// bucket. Archiving by slug alone would hit an arbitrary release.
    async fn set_archived(
        &self,
        slug: &str,
        release: Option<&str>,
        archived: bool,
    ) -> Result<(), AppError>;

    /// Rename a document's slug in-place, preserving all other fields and history.
    ///
    /// Does nothing if `old_slug` is not found.
    async fn rename_slug(&self, old_slug: &str, new_slug: &str) -> Result<(), AppError>;

    /// Point the denormalized `is_latest` flag of one source at `release`.
    ///
    /// Sets it on every document of that release and clears it on the source's
    /// other releases, so the flag keeps matching the alias it mirrors. Returns
    /// the slugs whose flag changed, which is what needs re-indexing: search and
    /// RAG only carry `latest`, so a promotion has to add the new release's
    /// documents and drop the old one's.
    /// Defaults to a no-op returning no affected slugs so test mocks need not
    /// implement it; the MongoDB backend overrides it.
    async fn promote_release(
        &self,
        _source_id: &str,
        _release: &str,
    ) -> Result<Vec<String>, AppError> {
        Ok(vec![])
    }

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
        use mongodb::bson::{doc, Bson};
        use mongodb::options::ReplaceOptions;

        // Matched on (slug, release), mirroring the unique index: a slug may now
        // exist in several releases, and replacing by slug alone would overwrite
        // whichever copy Mongo happened to return.
        let release_match = match doc.release.as_deref() {
            Some(r) => Bson::String(r.to_string()),
            None => Bson::Null,
        };
        let filter = doc! { "slug": &doc.slug, "release": release_match };
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

    async fn find_all_by_slug(&self, slug: &str) -> Result<Vec<Document>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        // Served by the unique (slug, release) index.
        let mut cursor = self.collection.find(doc! { "slug": slug }).await?;
        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            documents.push(document);
        }
        Ok(documents)
    }

    async fn promote_release(
        &self,
        source_id: &str,
        release: &str,
    ) -> Result<Vec<String>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        // Collect the affected slugs before writing: afterwards the two sets are
        // no longer distinguishable by `is_latest`.
        let mut affected = Vec::new();
        let mut cursor = self
            .collection
            .find(doc! {
                "source_id": source_id,
                "$or": [
                    // Gaining the flag.
                    { "release": release, "is_latest": { "$ne": true } },
                    // Losing it.
                    { "release": { "$ne": release }, "is_latest": true },
                ]
            })
            .await?;
        while let Some(document) = cursor.try_next().await? {
            affected.push(document.slug);
        }

        // Both updates are conditioned on the flag actually changing, so
        // re-promoting the release that is already latest is a no-op instead of
        // marking the whole release stale for re-indexing.
        self.collection
            .update_many(
                doc! { "source_id": source_id, "release": release, "is_latest": { "$ne": true } },
                doc! { "$set": { "is_latest": true, "needs_reindex": true } },
            )
            .await?;
        // Losing the alias clears the flag rather than setting it: only `latest`
        // is ever indexed, so a demoted release is not *stale* — it is out of
        // scope. Flagging it would leave a false positive nothing can clear, and
        // every promotion would accumulate more. Its slug still needs its index
        // entry revisited, which is what the returned list is for.
        self.collection
            .update_many(
                doc! { "source_id": source_id, "release": { "$ne": release }, "is_latest": true },
                doc! { "$set": { "is_latest": false, "needs_reindex": false } },
            )
            .await?;

        Ok(affected)
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

    async fn set_archived(
        &self,
        slug: &str,
        release: Option<&str>,
        archived: bool,
    ) -> Result<(), AppError> {
        use mongodb::bson::{doc, Bson};

        let release_match = match release {
            Some(r) => Bson::String(r.to_string()),
            None => Bson::Null,
        };

        self.collection
            .update_one(
                doc! { "slug": slug, "release": release_match },
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

    fn doc_in(slug: &str, source: &str, release: Option<&str>, is_latest: bool) -> Document {
        Document {
            slug: slug.to_string(),
            title: slug.to_string(),
            summary: None,
            s3_key: format!("docs/{slug}.md"),
            access_level: "public".to_string(),
            is_draft: false,
            service_owner: "team".to_string(),
            last_updated: chrono::Utc::now(),
            tags: vec![],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            content_hash: None,
            metadata_hash: None,
            is_archived: false,
            source_path: None,
            source_id: Some(source.to_string()),
            release: release.map(str::to_string),
            is_latest,
            needs_reindex: false,
            skip_rag: false,
        }
    }

    #[test]
    fn resolves_to_latest_without_pins() {
        let candidates = vec![
            doc_in("api/auth", "svc", Some("1.0.0"), false),
            doc_in("api/auth", "svc", Some("1.2.0"), true),
        ];

        let resolved =
            resolve_by_release(candidates, &ReleasePins::default()).expect("latest must resolve");
        assert_eq!(resolved.release.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn a_pin_wins_over_latest() {
        let candidates = vec![
            doc_in("api/auth", "svc", Some("1.0.0"), false),
            doc_in("api/auth", "svc", Some("1.2.0"), true),
        ];
        let mut pins = ReleasePins::default();
        pins.set("svc", "1.0.0");

        let resolved = resolve_by_release(candidates, &pins).expect("pin must resolve");
        assert_eq!(resolved.release.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn a_pin_for_another_source_does_not_apply() {
        let candidates = vec![doc_in("api/auth", "svc", Some("1.2.0"), true)];
        let mut pins = ReleasePins::default();
        pins.set("other-svc", "9.9.9");

        let resolved = resolve_by_release(candidates, &pins).expect("falls back to latest");
        assert_eq!(resolved.release.as_deref(), Some("1.2.0"));
    }

    /// A shared link whose release was deleted must degrade, not 404.
    #[test]
    fn a_pin_naming_a_missing_release_falls_back_to_latest() {
        let candidates = vec![doc_in("api/auth", "svc", Some("1.2.0"), true)];
        let mut pins = ReleasePins::default();
        pins.set("svc", "0.9.0");

        let resolved = resolve_by_release(candidates, &pins).expect("must not vanish");
        assert_eq!(resolved.release.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn an_unversioned_document_resolves_as_itself() {
        let candidates = vec![doc_in("guide", "svc", None, true)];

        let resolved =
            resolve_by_release(candidates, &ReleasePins::default()).expect("must resolve");
        assert_eq!(resolved.release, None);
    }

    #[test]
    fn nothing_resolves_when_there_is_no_candidate() {
        assert!(resolve_by_release(vec![], &ReleasePins::default()).is_none());
    }

    /// Defensive: if no copy carries the alias (an interrupted promotion), the
    /// lookup reports nothing rather than silently serving an arbitrary release.
    #[test]
    fn no_latest_and_no_matching_pin_resolves_to_nothing() {
        let candidates = vec![
            doc_in("api/auth", "svc", Some("1.0.0"), false),
            doc_in("api/auth", "svc", Some("1.2.0"), false),
        ];

        assert!(resolve_by_release(candidates, &ReleasePins::default()).is_none());
    }
}
