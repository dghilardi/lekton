//! Best-effort history retention. MongoDB relinquishes ownership before S3
//! deletion, so a storage failure leaves an object the consistency report finds.
use crate::{
    db::document_revision_repository::DocumentRevisionRepository, storage::client::StorageClient,
};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PruneResult {
    pub rows_deleted: u64,
    pub failures: u64,
}

pub async fn prune_slug(
    repo: &dyn DocumentRevisionRepository,
    storage: &dyn StorageClient,
    slug: &str,
    keep: usize,
) -> PruneResult {
    let mut result = PruneResult::default();
    if keep == 0 {
        return result;
    }
    let revisions = match repo.list_by_slug(slug).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(%slug, "Cannot list history for retention: {e}");
            result.failures += 1;
            return result;
        }
    };
    for row in revisions.into_iter().skip(keep) {
        match repo.delete(&row.id).await {
            Ok(true) => {
                result.rows_deleted += 1;
                // Historical keys flattened slashes to underscores. Another
                // slug may still own the same key: preserve shared objects.
                match repo.key_is_referenced(&row.s3_key).await {
                    Ok(false) => {}
                    Ok(true) => continue,
                    Err(e) => {
                        result.failures += 1;
                        tracing::warn!(key = %row.s3_key, "Cannot check remaining history owners: {e}");
                        continue;
                    }
                }
                if let Err(e) = storage.delete_object(&row.s3_key).await {
                    result.failures += 1;
                    tracing::warn!(key = %row.s3_key, "Retention left an orphan object: {e}");
                }
            }
            Ok(false) => {}
            Err(e) => {
                result.failures += 1;
                tracing::warn!(id = %row.id, "Retention could not delete row: {e}");
            }
        }
    }
    result
}

pub async fn count_candidates(
    repo: &dyn DocumentRevisionRepository,
    keep: usize,
) -> Result<u64, crate::error::AppError> {
    if keep == 0 {
        return Ok(0);
    }
    let mut count = 0;
    for slug in repo.list_slugs().await? {
        count += repo.list_by_slug(&slug).await?.len().saturating_sub(keep) as u64;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::document_revision_repository::DocumentRevision;
    use crate::error::AppError;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct Repo {
        rows: Mutex<Vec<DocumentRevision>>,
        fail: bool,
    }
    #[async_trait]
    impl DocumentRevisionRepository for Repo {
        async fn key_is_referenced(&self, key: &str) -> Result<bool, AppError> {
            Ok(self.rows.lock().unwrap().iter().any(|r| r.s3_key == key))
        }
        async fn create(&self, row: DocumentRevision) -> Result<(), AppError> {
            self.rows.lock().unwrap().push(row);
            Ok(())
        }
        async fn list_slugs(&self) -> Result<Vec<String>, AppError> {
            Ok(vec!["doc".into()])
        }
        async fn list_by_slug(&self, _: &str) -> Result<Vec<DocumentRevision>, AppError> {
            Ok(self.rows.lock().unwrap().clone())
        }
        async fn find_latest(&self, _: &str) -> Result<Option<DocumentRevision>, AppError> {
            Ok(self.rows.lock().unwrap().first().cloned())
        }
        async fn next_revision_number(&self, _: &str) -> Result<u64, AppError> {
            Ok(4)
        }
        async fn delete(&self, id: &str) -> Result<bool, AppError> {
            if self.fail {
                return Err(AppError::Internal("delete failed".into()));
            }
            self.rows.lock().unwrap().retain(|r| r.id != id);
            Ok(true)
        }
    }
    struct Storage<'a>(&'a Repo);
    #[async_trait]
    impl StorageClient for Storage<'_> {
        async fn put_object(&self, _: &str, _: Vec<u8>) -> Result<(), AppError> {
            unreachable!()
        }
        async fn get_object(&self, _: &str) -> Result<Option<Vec<u8>>, AppError> {
            unreachable!()
        }
        async fn list(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> Result<crate::storage::client::ListPage, AppError> {
            unreachable!()
        }
        async fn delete_object(&self, key: &str) -> Result<(), AppError> {
            assert!(
                !self.0.rows.lock().unwrap().iter().any(|r| r.s3_key == key),
                "Mongo row must be removed first"
            );
            Err(AppError::Storage("unavailable".into()))
        }
    }
    #[tokio::test]
    async fn deletes_mongo_before_s3_and_survives_failures() {
        for fail in [false, true] {
            let repo = Repo {
                fail,
                rows: Mutex::new(
                    (1..=3)
                        .rev()
                        .map(|n| DocumentRevision {
                            id: n.to_string(),
                            slug: "doc".into(),
                            revision: n,
                            content_hash: "hash".into(),
                            s3_key: format!("history/{n}"),
                            updated_by: "test".into(),
                            created_at: chrono::Utc::now(),
                        })
                        .collect(),
                ),
            };
            let result = prune_slug(&repo, &Storage(&repo), "doc", 1).await;
            assert_eq!(result.failures, 2);
            assert_eq!(result.rows_deleted, if fail { 0 } else { 2 });
            assert_eq!(repo.rows.lock().unwrap().len(), if fail { 3 } else { 1 });
        }
    }
}
