//! Registry of per-source documentation releases and the movable `latest` alias.
//!
//! A source becomes *release-managed* the first time it is synced with an
//! explicit release. From then on its documents are partitioned by `release`,
//! and exactly one release carries the `latest` alias that unpinned readers
//! resolve to.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// One document expected to exist before a staged release can be finalized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseDocumentExpectation {
    pub slug: String,
    pub source_path: String,
    pub content_hash: String,
    pub metadata_hash: Option<String>,
}

/// One staged or finalized release of a source's documentation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRelease {
    /// Matches `Document.source_id`.
    pub source_id: String,
    /// The release tag as passed to `lekton-sync --version` (e.g. `"1.2.0"`).
    pub release: String,
    /// When this release was first synced.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub first_synced_at: DateTime<Utc>,
    /// When this release was last re-synced.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_synced_at: DateTime<Utc>,
    /// Snapshot declared by the sync that staged this release.
    #[serde(default)]
    pub expected_documents: Vec<ReleaseDocumentExpectation>,
    /// Set only after every expected document has been persisted with matching
    /// hashes. Unfinalized releases are not selectable or promotable.
    #[serde(
        default,
        with = "bson::serde_helpers::chrono_datetime_as_bson_datetime_optional"
    )]
    pub finalized_at: Option<DateTime<Utc>>,
}

/// Persistence for the release catalogue and the `latest` alias.
#[async_trait]
pub trait ReleaseRepository: Send + Sync {
    /// Record a sync of `release` for `source_id`, creating the catalogue entry
    /// on first sight and refreshing `last_synced_at` afterwards.
    async fn register(&self, source_id: &str, release: &str) -> Result<(), AppError>;

    /// Stage a release and its complete expected document snapshot.
    ///
    /// Restaging the same tag replaces the expectation and clears finalization
    /// until the new snapshot has been verified.
    async fn stage(
        &self,
        source_id: &str,
        release: &str,
        _expected_documents: &[ReleaseDocumentExpectation],
    ) -> Result<(), AppError> {
        self.register(source_id, release).await
    }

    /// Return one staged or finalized release.
    async fn find(
        &self,
        source_id: &str,
        release: &str,
    ) -> Result<Option<SourceRelease>, AppError> {
        Ok(self
            .list_by_source(source_id)
            .await?
            .into_iter()
            .find(|candidate| candidate.release == release))
    }

    /// Mark a staged release complete after its expected snapshot is verified.
    async fn finalize(&self, source_id: &str, release: &str) -> Result<(), AppError> {
        self.register(source_id, release).await
    }

    /// Releases of a source, most recently first *published* first.
    ///
    /// Ordered by `first_synced_at` rather than by parsing the tag: release
    /// strings are free-form (`1.2.0`, `2024-06`, `v3-rc1`), so publication
    /// order is the only ordering that is always meaningful.
    async fn list_by_source(&self, source_id: &str) -> Result<Vec<SourceRelease>, AppError>;

    /// Whether the source has at least one release — i.e. whether a sync of it
    /// must carry an explicit release.
    async fn is_release_managed(&self, source_id: &str) -> Result<bool, AppError>;

    /// The release currently aliased `latest`, if the alias has been set.
    async fn latest(&self, source_id: &str) -> Result<Option<String>, AppError>;

    /// Point `latest` at `release`.
    ///
    /// A single-document upsert, so the alias is never briefly absent or
    /// duplicated — which is why it lives here and not as a boolean spread over
    /// the catalogue rows.
    async fn set_latest(&self, source_id: &str, release: &str) -> Result<(), AppError>;

    /// Point `latest` at `release` and durably enqueue the slugs whose search
    /// and RAG state must be reconciled.
    async fn set_latest_with_pending(
        &self,
        source_id: &str,
        release: &str,
        _pending_slugs: &[String],
    ) -> Result<(), AppError> {
        self.set_latest(source_id, release).await
    }

    /// Slugs still awaiting search/RAG reconciliation for this source.
    async fn pending_reindex(&self, _source_id: &str) -> Result<Vec<String>, AppError> {
        Ok(vec![])
    }

    /// Acknowledge one successfully reconciled slug.
    async fn clear_reindex_pending(&self, _source_id: &str, _slug: &str) -> Result<(), AppError> {
        Ok(())
    }
}

/// Deserialization view over a `source_release_aliases` row.
///
/// Only the field we read: `source_id` comes from the query filter, and writes
/// go through a raw update document, so there is nothing else to model.
#[cfg(feature = "ssr")]
#[derive(Deserialize)]
struct ReleaseAlias {
    latest_release: String,
    #[serde(default)]
    reindex_pending: Vec<String>,
}

#[cfg(feature = "ssr")]
pub struct MongoReleaseRepository {
    releases: mongodb::Collection<SourceRelease>,
    aliases: mongodb::Collection<ReleaseAlias>,
}

#[cfg(feature = "ssr")]
impl MongoReleaseRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            releases: db.collection("source_releases"),
            aliases: db.collection("source_release_aliases"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl ReleaseRepository for MongoReleaseRepository {
    async fn register(&self, source_id: &str, release: &str) -> Result<(), AppError> {
        self.stage(source_id, release, &[]).await?;
        self.finalize(source_id, release).await
    }

    async fn stage(
        &self,
        source_id: &str,
        release: &str,
        expected_documents: &[ReleaseDocumentExpectation],
    ) -> Result<(), AppError> {
        use mongodb::bson::{doc, DateTime};
        use mongodb::options::UpdateOptions;

        let now = DateTime::from_millis(Utc::now().timestamp_millis());
        let expected_documents = mongodb::bson::to_bson(expected_documents)
            .map_err(|e| AppError::Database(format!("serialize release manifest: {e}")))?;
        let update = doc! {
            "$set": {
                "last_synced_at": now,
                "expected_documents": expected_documents,
                "finalized_at": mongodb::bson::Bson::Null,
            },
            "$setOnInsert": {
                "source_id": source_id,
                "release": release,
                "first_synced_at": now,
            },
        };

        self.releases
            .update_one(doc! { "source_id": source_id, "release": release }, update)
            .with_options(UpdateOptions::builder().upsert(true).build())
            .await
            .map_err(|e| AppError::Database(format!("stage source release: {e}")))?;
        Ok(())
    }

    async fn find(
        &self,
        source_id: &str,
        release: &str,
    ) -> Result<Option<SourceRelease>, AppError> {
        use mongodb::bson::doc;

        self.releases
            .find_one(doc! { "source_id": source_id, "release": release })
            .await
            .map_err(|e| AppError::Database(format!("find source release: {e}")))
    }

    async fn finalize(&self, source_id: &str, release: &str) -> Result<(), AppError> {
        use mongodb::bson::{doc, DateTime};

        let now = DateTime::from_millis(Utc::now().timestamp_millis());
        let result = self
            .releases
            .update_one(
                doc! { "source_id": source_id, "release": release },
                doc! { "$set": { "finalized_at": now, "last_synced_at": now } },
            )
            .await
            .map_err(|e| AppError::Database(format!("finalize source release: {e}")))?;
        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "staged release '{release}' for source '{source_id}'"
            )));
        }
        Ok(())
    }

    async fn list_by_source(&self, source_id: &str) -> Result<Vec<SourceRelease>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        self.releases
            .find(doc! {
                "source_id": source_id,
                "finalized_at": { "$type": "date" },
            })
            .sort(doc! { "first_synced_at": -1 })
            .await
            .map_err(|e| AppError::Database(format!("list source releases: {e}")))?
            .try_collect()
            .await
            .map_err(|e| AppError::Database(format!("collect source releases: {e}")))
    }

    async fn is_release_managed(&self, source_id: &str) -> Result<bool, AppError> {
        use mongodb::bson::doc;

        let count = self
            .releases
            .count_documents(doc! { "source_id": source_id })
            .await
            .map_err(|e| AppError::Database(format!("count source releases: {e}")))?;
        Ok(count > 0)
    }

    async fn latest(&self, source_id: &str) -> Result<Option<String>, AppError> {
        use mongodb::bson::doc;

        Ok(self
            .aliases
            .find_one(doc! { "source_id": source_id })
            .await
            .map_err(|e| AppError::Database(format!("find latest alias: {e}")))?
            .map(|a| a.latest_release))
    }

    async fn set_latest(&self, source_id: &str, release: &str) -> Result<(), AppError> {
        self.set_latest_with_pending(source_id, release, &[]).await
    }

    async fn set_latest_with_pending(
        &self,
        source_id: &str,
        release: &str,
        pending_slugs: &[String],
    ) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::UpdateOptions;

        self.aliases
            .update_one(
                doc! { "source_id": source_id },
                doc! {
                    "$set": { "latest_release": release },
                    "$addToSet": { "reindex_pending": { "$each": pending_slugs } },
                },
            )
            .with_options(UpdateOptions::builder().upsert(true).build())
            .await
            .map_err(|e| AppError::Database(format!("set latest alias: {e}")))?;
        Ok(())
    }

    async fn pending_reindex(&self, source_id: &str) -> Result<Vec<String>, AppError> {
        use mongodb::bson::doc;

        Ok(self
            .aliases
            .find_one(doc! { "source_id": source_id })
            .await
            .map_err(|e| AppError::Database(format!("find promotion reindex backlog: {e}")))?
            .map(|alias| alias.reindex_pending)
            .unwrap_or_default())
    }

    async fn clear_reindex_pending(&self, source_id: &str, slug: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        self.aliases
            .update_one(
                doc! { "source_id": source_id },
                doc! { "$pull": { "reindex_pending": slug } },
            )
            .await
            .map_err(|e| AppError::Database(format!("clear promotion reindex backlog: {e}")))?;
        Ok(())
    }
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;

    #[test]
    fn legacy_alias_deserializes_with_an_empty_reindex_backlog() {
        let alias: ReleaseAlias =
            bson::from_document(bson::doc! { "latest_release": "1.0.0" }).unwrap();

        assert_eq!(alias.latest_release, "1.0.0");
        assert!(alias.reindex_pending.is_empty());
    }
}
