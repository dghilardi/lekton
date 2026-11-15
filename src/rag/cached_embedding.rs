use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::db::embedding_cache_repository::{EmbeddingCacheEntry, EmbeddingCacheRepository};
use crate::error::AppError;
use crate::usage::UsageKey;

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
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// Builds the cache namespace that scopes every embedding entry.
///
/// Two cached vectors only collide when the model name, vector dimensions, AND
/// backend endpoint all match. Without the dimensions/endpoint, repointing
/// `embedding_url` (or a Vertex project) to a different backend — or changing
/// dimensions for a Matryoshka model — while keeping the model name would return
/// stale vectors from the old backend (dimension mismatch vs Qdrant or
/// semantically wrong results). Changing any of the three components naturally
/// busts the cache, so it doubles as a bumpable cache-version namespace.
pub fn cache_namespace(model: &str, dimensions: u32, endpoint: &str) -> String {
    let endpoint_hash = sha256_hex(endpoint);
    format!("{model}|{dimensions}|{}", &endpoint_hash[..16])
}

/// Every cache namespace this RAG configuration writes and reads.
///
/// The cache wraps exactly one embedding backend: the configured
/// `embedding_model` behind `embedding_url` or a Vertex project, used for
/// chunks and, with `embedding_cache_query`, for chat queries too. The
/// reranker is a cross-encoder that does not embed, and the evaluation tools
/// bypass the cache, so neither adds a namespace. Startup builds the cache
/// from this, and the admin purge refuses to delete what it returns.
pub fn configured_cache_namespaces(rag: &crate::config::RagConfig) -> Vec<String> {
    // Endpoint identity: distinct per backend so repointing the URL (or Vertex
    // project) busts the cache instead of returning stale vectors from the old
    // backend.
    let endpoint = if rag.embedding_vertex_project_id.is_empty() {
        format!("oai:{}", rag.embedding_url)
    } else {
        format!(
            "vertex:{}/{}",
            rag.embedding_vertex_project_id, rag.embedding_vertex_location
        )
    };
    vec![cache_namespace(
        &rag.embedding_model,
        rag.embedding_dimensions,
        &endpoint,
    )]
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
    /// Cache namespace (model + dimensions + endpoint) used as part of the cache key.
    /// Build it with [`cache_namespace`].
    namespace: String,
    /// Whether to persist the original normalised text alongside the embedding.
    store_text: bool,
}

impl CachedEmbeddingService {
    pub fn new(
        inner: Arc<dyn EmbeddingService>,
        cache: Arc<dyn EmbeddingCacheRepository>,
        namespace: String,
        store_text: bool,
    ) -> Self {
        Self {
            inner,
            cache,
            namespace,
            store_text,
        }
    }
}

#[async_trait]
impl EmbeddingService for CachedEmbeddingService {
    /// The spend ceiling is checked by the inner service, not here: a cache hit
    /// costs nothing, so an exhausted ceiling must not refuse one.
    async fn embed(&self, key: &UsageKey, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Normalise and hash every input text.
        let normalised: Vec<String> = texts.iter().map(|t| normalize(t)).collect();
        let hashes: Vec<String> = normalised.iter().map(|t| sha256_hex(t)).collect();

        // 2. Batch-fetch whatever is already in the cache.
        let pairs: Vec<(String, String)> = hashes
            .iter()
            .map(|h| (h.clone(), self.namespace.clone()))
            .collect();
        let cached_entries = self.cache.get_many(&pairs).await?;

        let mut cache_map: HashMap<String, Vec<f32>> = cached_entries
            .into_iter()
            .map(|e| (e.hash, e.embedding))
            .collect();

        // 3. Identify which inputs are still missing from the cache, once per
        //    distinct text: a batch may repeat a text, and each copy needs the
        //    same vector, not a second paid embedding.
        let mut seen = std::collections::HashSet::new();
        let missing_indices: Vec<usize> = hashes
            .iter()
            .enumerate()
            .filter(|(_, h)| !cache_map.contains_key(*h) && seen.insert(*h))
            .map(|(i, _)| i)
            .collect();

        if !missing_indices.is_empty() {
            // 4. Embed only the missing texts.
            let missing_texts: Vec<String> = missing_indices
                .iter()
                .map(|&i| normalised[i].clone())
                .collect();
            let new_vectors = self.inner.embed(key, &missing_texts).await?;

            // 5. Persist the newly generated embeddings (skip empty vectors —
            //    some backends return [] for degenerate inputs).
            let entries_to_store: Vec<EmbeddingCacheEntry> = missing_indices
                .iter()
                .zip(new_vectors.iter())
                .filter(|(_, v)| !v.is_empty())
                .map(|(&i, v)| EmbeddingCacheEntry {
                    hash: hashes[i].clone(),
                    model: self.namespace.clone(),
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
            for (&i, v) in missing_indices.iter().zip(new_vectors) {
                cache_map.insert(hashes[i].clone(), v);
            }
        }

        // 7. Reconstruct the result slice in the original input order. Cloned,
        //    not taken: a repeated text needs its vector more than once.
        let result: Vec<Vec<f32>> = hashes
            .iter()
            .map(|h| cache_map.get(h).cloned().unwrap_or_default())
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
        /// Total number of texts sent to the backend.
        embedded: Mutex<usize>,
        keys: Mutex<Vec<UsageKey>>,
        dims: usize,
    }

    impl CountingEmbedding {
        fn new(dims: usize) -> Self {
            Self {
                call_count: Mutex::new(0),
                embedded: Mutex::new(0),
                keys: Mutex::new(Vec::new()),
                dims,
            }
        }
        fn calls(&self) -> usize {
            *self.call_count.lock().unwrap()
        }
        fn keys(&self) -> Vec<UsageKey> {
            self.keys.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EmbeddingService for CountingEmbedding {
        async fn embed(&self, key: &UsageKey, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
            *self.call_count.lock().unwrap() += 1;
            *self.embedded.lock().unwrap() += texts.len();
            self.keys.lock().unwrap().push(key.clone());
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

        async fn namespace_stats(
            &self,
        ) -> Result<Vec<crate::db::embedding_cache_repository::NamespaceStats>, AppError> {
            unimplemented!("not used by these tests")
        }

        async fn delete_namespaces(&self, _: &[String]) -> Result<u64, AppError> {
            unimplemented!("not used by these tests")
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
        let result = svc.embed(&UsageKey::System, &[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn duplicate_texts_in_one_batch_all_get_their_vector() {
        // Two chunks with the same text (a repeated boilerplate paragraph, two
        // identical table rows) must both be indexed, whether the text was
        // cached already or not.
        let (inner, _, svc) = make_service(4);
        let texts = vec![
            "same text".to_string(),
            "other".to_string(),
            "same  text".to_string(), // normalises to the first
        ];

        let first = svc.embed(&UsageKey::System, &texts).await.unwrap();
        assert!(
            first.iter().all(|v| v.len() == 4),
            "uncached duplicates: {first:?}"
        );
        assert_eq!(
            *inner.embedded.lock().unwrap(),
            2,
            "each distinct text embedded once"
        );

        let cached = svc.embed(&UsageKey::System, &texts).await.unwrap();
        assert!(
            cached.iter().all(|v| v.len() == 4),
            "cached duplicates: {cached:?}"
        );
    }

    #[tokio::test]
    async fn first_call_goes_to_inner() {
        let (inner, _, svc) = make_service(4);
        let texts = vec!["hello world".to_string()];
        let result = svc.embed(&UsageKey::System, &texts).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 4);
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn the_caller_is_forwarded_to_the_inner_service_unchanged() {
        // The cache sits between every embedding caller and the accounting, so
        // a hardcoded key here would misattribute the entire embedding spend —
        // silently, since the vectors would still be correct.
        let (inner, _, svc) = make_service(4);
        let caller = UsageKey::User("u-42".into());

        svc.embed(&caller, &["a query".to_string()]).await.unwrap();

        assert_eq!(inner.keys(), vec![caller]);
    }

    #[tokio::test]
    async fn second_call_hits_cache() {
        let (inner, _, svc) = make_service(4);
        let texts = vec!["hello world".to_string()];
        svc.embed(&UsageKey::System, &texts).await.unwrap();
        svc.embed(&UsageKey::System, &texts).await.unwrap();
        // Inner service should only have been called once
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn normalisation_produces_cache_hit() {
        let (inner, _, svc) = make_service(4);
        svc.embed(&UsageKey::System, &["hello   world".to_string()])
            .await
            .unwrap();
        // Trailing/multiple spaces get collapsed — same hash expected
        svc.embed(&UsageKey::System, &["hello world".to_string()])
            .await
            .unwrap();
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn batch_partial_cache_hit() {
        let (inner, _, svc) = make_service(4);
        // Warm up cache with first text
        svc.embed(&UsageKey::System, &["text one".to_string()])
            .await
            .unwrap();
        assert_eq!(inner.calls(), 1);

        // Second call: "text one" (cached) + "text two" (missing)
        let result = svc
            .embed(
                &UsageKey::System,
                &["text one".to_string(), "text two".to_string()],
            )
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        // Inner was called again, but only for 1 text
        assert_eq!(inner.calls(), 2);
    }

    #[tokio::test]
    async fn different_namespace_misses_cache() {
        // Same text + same backing cache, but two services scoped to different
        // namespaces (e.g. different dimensions or endpoint) must not share entries.
        let inner = Arc::new(CountingEmbedding::new(4));
        let cache = Arc::new(InMemoryCache::new());
        let svc_a = CachedEmbeddingService::new(
            inner.clone(),
            cache.clone(),
            cache_namespace("m", 768, "oai:http://a"),
            false,
        );
        let svc_b = CachedEmbeddingService::new(
            inner.clone(),
            cache.clone(),
            cache_namespace("m", 1024, "oai:http://a"),
            false,
        );
        let texts = vec!["hello world".to_string()];
        svc_a.embed(&UsageKey::System, &texts).await.unwrap();
        svc_b.embed(&UsageKey::System, &texts).await.unwrap();
        // Distinct namespaces → both went to the inner service.
        assert_eq!(inner.calls(), 2);
    }

    #[test]
    fn cache_namespace_varies_with_each_component() {
        let base = cache_namespace("model-x", 768, "oai:http://host");
        assert_ne!(base, cache_namespace("model-y", 768, "oai:http://host"));
        assert_ne!(base, cache_namespace("model-x", 1024, "oai:http://host"));
        assert_ne!(base, cache_namespace("model-x", 768, "oai:http://other"));
        // Stable for identical inputs.
        assert_eq!(base, cache_namespace("model-x", 768, "oai:http://host"));
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
        svc.embed(&UsageKey::System, &["store me".to_string()])
            .await
            .unwrap();

        let _entries = cache
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
