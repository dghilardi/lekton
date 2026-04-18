use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::db::embedding_cache_repository::{EmbeddingCacheEntry, EmbeddingCacheRepository};
use crate::error::AppError;

use super::embedding::EmbeddingService;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Collapse whitespace and trim so that minor formatting differences don't
/// produce distinct cache keys for semantically identical text.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// SHA-256 hex digest of the given string.
fn sha256_hex(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    format!("{:x}", hash)
}

// ── CachedEmbeddingService ────────────────────────────────────────────────────

/// Wraps any [`EmbeddingService`] with a MongoDB-backed cache.
///
/// Before calling the inner service the cache is consulted for all requested
/// texts in a single batch query; only the missing vectors are forwarded to
/// the inner service, then persisted for future reuse.
pub struct CachedEmbeddingService {
    inner: Arc<dyn EmbeddingService>,
    cache: Arc<dyn EmbeddingCacheRepository>,
    /// Model name used as part of the cache key.
    model: String,
    /// Whether to persist the original normalised text alongside the embedding.
    store_text: bool,
}

impl CachedEmbeddingService {
    pub fn new(
        inner: Arc<dyn EmbeddingService>,
        cache: Arc<dyn EmbeddingCacheRepository>,
        model: String,
        store_text: bool,
    ) -> Self {
        Self {
            inner,
            cache,
            model,
            store_text,
        }
    }
}

#[async_trait]
impl EmbeddingService for CachedEmbeddingService {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Normalise and hash every input text.
        let normalised: Vec<String> = texts.iter().map(|t| normalize(t)).collect();
        let hashes: Vec<String> = normalised.iter().map(|t| sha256_hex(t)).collect();

        // 2. Batch-fetch whatever is already in the cache.
        let pairs: Vec<(String, String)> = hashes
            .iter()
            .map(|h| (h.clone(), self.model.clone()))
            .collect();
        let cached_entries = self.cache.get_many(&pairs).await?;

        let mut cache_map: HashMap<String, Vec<f32>> = cached_entries
            .into_iter()
            .map(|e| (e.hash, e.embedding))
            .collect();

        // 3. Identify which inputs are still missing from the cache.
        let missing_indices: Vec<usize> = hashes
            .iter()
            .enumerate()
            .filter(|(_, h)| !cache_map.contains_key(*h))
            .map(|(i, _)| i)
            .collect();

        if !missing_indices.is_empty() {
            // 4. Embed only the missing texts.
            let missing_texts: Vec<String> = missing_indices
                .iter()
                .map(|&i| normalised[i].clone())
                .collect();
            let new_vectors = self.inner.embed(&missing_texts).await?;

            // 5. Persist the newly generated embeddings (skip empty vectors —
            //    some backends return [] for degenerate inputs).
            let entries_to_store: Vec<EmbeddingCacheEntry> = missing_indices
                .iter()
                .zip(new_vectors.iter())
                .filter(|(_, v)| !v.is_empty())
                .map(|(&i, v)| EmbeddingCacheEntry {
                    hash: hashes[i].clone(),
                    model: self.model.clone(),
                    embedding: v.clone(),
                    generated_at: Utc::now(),
                    text: if self.store_text {
                        Some(normalised[i].clone())
                    } else {
                        None
                    },
                })
                .collect();

            if !entries_to_store.is_empty() {
                if let Err(e) = self.cache.upsert_many(entries_to_store).await {
                    // Cache write failures are non-fatal: log and continue.
                    tracing::warn!("embedding cache write failed (non-fatal): {e}");
                }
            }

            // 6. Merge new vectors into the local map.
            for (&i, v) in missing_indices.iter().zip(new_vectors.into_iter()) {
                cache_map.insert(hashes[i].clone(), v);
            }
        }

        // 7. Reconstruct the result slice in the original input order.
        let result: Vec<Vec<f32>> = hashes
            .iter()
            .map(|h| cache_map.remove(h).unwrap_or_default())
            .collect();

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ── Helpers ──────────────────────────────────────────────────────────────

    struct CountingEmbedding {
        call_count: Mutex<usize>,
        dims: usize,
    }

    impl CountingEmbedding {
        fn new(dims: usize) -> Self {
            Self {
                call_count: Mutex::new(0),
                dims,
            }
        }
        fn calls(&self) -> usize {
            *self.call_count.lock().unwrap()
        }
    }

    #[async_trait]
    impl EmbeddingService for CountingEmbedding {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
            *self.call_count.lock().unwrap() += 1;
            Ok(texts.iter().map(|_| vec![0.1f32; self.dims]).collect())
        }
    }

    struct InMemoryCache {
        store: Mutex<HashMap<(String, String), EmbeddingCacheEntry>>,
    }

    impl InMemoryCache {
        fn new() -> Self {
            Self {
                store: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl EmbeddingCacheRepository for InMemoryCache {
        async fn get_many(
            &self,
            queries: &[(String, String)],
        ) -> Result<Vec<EmbeddingCacheEntry>, AppError> {
            let store = self.store.lock().unwrap();
            Ok(queries
                .iter()
                .filter_map(|q| store.get(q).cloned())
                .collect())
        }

        async fn upsert_many(&self, entries: Vec<EmbeddingCacheEntry>) -> Result<(), AppError> {
            let mut store = self.store.lock().unwrap();
            for e in entries {
                store.insert((e.hash.clone(), e.model.clone()), e);
            }
            Ok(())
        }
    }

    fn make_service(
        dims: usize,
    ) -> (
        Arc<CountingEmbedding>,
        Arc<InMemoryCache>,
        CachedEmbeddingService,
    ) {
        let inner = Arc::new(CountingEmbedding::new(dims));
        let cache = Arc::new(InMemoryCache::new());
        let svc = CachedEmbeddingService::new(
            inner.clone(),
            cache.clone(),
            "test-model".to_string(),
            false,
        );
        (inner, cache, svc)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn empty_input_returns_empty() {
        let (_, _, svc) = make_service(4);
        let result = svc.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn first_call_goes_to_inner() {
        let (inner, _, svc) = make_service(4);
        let texts = vec!["hello world".to_string()];
        let result = svc.embed(&texts).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 4);
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn second_call_hits_cache() {
        let (inner, _, svc) = make_service(4);
        let texts = vec!["hello world".to_string()];
        svc.embed(&texts).await.unwrap();
        svc.embed(&texts).await.unwrap();
        // Inner service should only have been called once
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn normalisation_produces_cache_hit() {
        let (inner, _, svc) = make_service(4);
        svc.embed(&["hello   world".to_string()]).await.unwrap();
        // Trailing/multiple spaces get collapsed — same hash expected
        svc.embed(&["hello world".to_string()]).await.unwrap();
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn batch_partial_cache_hit() {
        let (inner, _, svc) = make_service(4);
        // Warm up cache with first text
        svc.embed(&["text one".to_string()]).await.unwrap();
        assert_eq!(inner.calls(), 1);

        // Second call: "text one" (cached) + "text two" (missing)
        let result = svc
            .embed(&["text one".to_string(), "text two".to_string()])
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        // Inner was called again, but only for 1 text
        assert_eq!(inner.calls(), 2);
    }

    #[tokio::test]
    async fn store_text_flag_persists_text() {
        let inner = Arc::new(CountingEmbedding::new(4));
        let cache = Arc::new(InMemoryCache::new());
        let svc = CachedEmbeddingService::new(
            inner.clone(),
            cache.clone(),
            "test-model".to_string(),
            true, // store_text = true
        );
        svc.embed(&["store me".to_string()]).await.unwrap();

        let entries = cache
            .get_many(&[("*".to_string(), "*".to_string())])
            .await
            .unwrap();
        // get_many with wildcard returns nothing — retrieve by actual hash
        let hash = sha256_hex(&normalize("store me"));
        let entries = cache
            .get_many(&[(hash, "test-model".to_string())])
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text.as_deref(), Some("store me"));
    }
}
