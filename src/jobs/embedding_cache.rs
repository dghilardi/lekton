//! Admin purge of embedding-cache namespaces the configuration no longer uses.
//!
//! Each cached vector is stored under the namespace of the backend that
//! produced it (see [`crate::rag::cached_embedding::cache_namespace`]).
//! Changing the model, dimensions or endpoint starts a new namespace and
//! leaves the old one behind, unread, for good.

use crate::db::embedding_cache_repository::{EmbeddingCacheRepository, NamespaceStats};
use crate::error::AppError;

/// Every namespace present, with whether the running configuration uses it.
pub async fn list_namespaces(
    repo: &dyn EmbeddingCacheRepository,
    in_use: &[String],
) -> Result<Vec<(NamespaceStats, bool)>, AppError> {
    Ok(repo
        .namespace_stats()
        .await?
        .into_iter()
        .map(|stats| {
            let used = in_use.contains(&stats.namespace);
            (stats, used)
        })
        .collect())
}

/// Delete the `requested` namespaces, refusing the whole request when any of
/// them is in use: deleting a live namespace would re-embed, and pay for,
/// every chunk on the next reindex.
pub async fn purge_namespaces(
    repo: &dyn EmbeddingCacheRepository,
    in_use: &[String],
    requested: &[String],
) -> Result<u64, AppError> {
    if requested.is_empty() {
        return Err(AppError::BadRequest("select at least one namespace".into()));
    }
    let protected: Vec<&str> = requested
        .iter()
        .filter(|namespace| in_use.contains(namespace))
        .map(String::as_str)
        .collect();
    if !protected.is_empty() {
        return Err(AppError::BadRequest(format!(
            "in use by the current configuration, not deleted: {}",
            protected.join(", ")
        )));
    }
    let deleted = repo.delete_namespaces(requested).await?;
    tracing::info!(
        namespaces = ?requested,
        deleted,
        "embedding cache: purged unused namespaces"
    );
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::embedding_cache_repository::EmbeddingCacheEntry;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Entry counts per namespace.
    struct FakeCache(Mutex<Vec<(String, u64)>>);

    impl FakeCache {
        fn new(namespaces: &[(&str, u64)]) -> Self {
            Self(Mutex::new(
                namespaces
                    .iter()
                    .map(|(n, c)| (n.to_string(), *c))
                    .collect(),
            ))
        }

        fn namespaces(&self) -> Vec<String> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .map(|(n, _)| n.clone())
                .collect()
        }
    }

    #[async_trait]
    impl EmbeddingCacheRepository for FakeCache {
        async fn get_many(
            &self,
            _: &[(String, String)],
        ) -> Result<Vec<EmbeddingCacheEntry>, AppError> {
            Ok(Vec::new())
        }

        async fn upsert_many(&self, _: Vec<EmbeddingCacheEntry>) -> Result<(), AppError> {
            Ok(())
        }

        async fn namespace_stats(&self) -> Result<Vec<NamespaceStats>, AppError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|(namespace, entries)| NamespaceStats {
                    namespace: namespace.clone(),
                    entries: *entries,
                    approx_bytes: entries * 100,
                })
                .collect())
        }

        async fn delete_namespaces(&self, namespaces: &[String]) -> Result<u64, AppError> {
            let mut store = self.0.lock().unwrap();
            let deleted = store
                .iter()
                .filter(|(n, _)| namespaces.contains(n))
                .map(|(_, c)| c)
                .sum();
            store.retain(|(n, _)| !namespaces.contains(n));
            Ok(deleted)
        }
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[tokio::test]
    async fn lists_every_namespace_and_marks_the_configured_ones() {
        let cache = FakeCache::new(&[("old|768|aa", 5), ("new|1024|bb", 7)]);

        let rows = list_namespaces(&cache, &names(&["new|1024|bb"]))
            .await
            .unwrap();

        let marked: Vec<(&str, bool)> = rows
            .iter()
            .map(|(stats, used)| (stats.namespace.as_str(), *used))
            .collect();
        assert_eq!(marked, vec![("old|768|aa", false), ("new|1024|bb", true)]);
    }

    #[tokio::test]
    async fn purges_only_the_selected_unused_namespaces() {
        let cache = FakeCache::new(&[("a|768|aa", 5), ("b|768|bb", 3), ("live|768|cc", 9)]);

        let deleted = purge_namespaces(&cache, &names(&["live|768|cc"]), &names(&["a|768|aa"]))
            .await
            .unwrap();

        assert_eq!(deleted, 5);
        assert_eq!(cache.namespaces(), names(&["b|768|bb", "live|768|cc"]));
    }

    #[tokio::test]
    async fn refuses_a_request_that_includes_a_namespace_in_use() {
        let cache = FakeCache::new(&[("a|768|aa", 5), ("live|768|cc", 9)]);

        let err = purge_namespaces(
            &cache,
            &names(&["live|768|cc"]),
            &names(&["a|768|aa", "live|768|cc"]),
        )
        .await
        .expect_err("an in-use namespace must not be deletable");

        assert!(err.to_string().contains("live|768|cc"), "got: {err}");
        // Nothing at all is deleted, not even the unused one.
        assert_eq!(cache.namespaces(), names(&["a|768|aa", "live|768|cc"]));
    }

    #[tokio::test]
    async fn refuses_an_empty_selection() {
        let cache = FakeCache::new(&[("a|768|aa", 5)]);
        assert!(purge_namespaces(&cache, &[], &[]).await.is_err());
        assert_eq!(cache.namespaces(), names(&["a|768|aa"]));
    }
}
