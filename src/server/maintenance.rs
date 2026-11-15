//! Server functions for the admin data-maintenance page.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

// ── Embedding cache ──────────────────────────────────────────────────────────

/// One embedding-cache namespace: `model|dimensions|endpoint-hash`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingCacheNamespace {
    pub namespace: String,
    pub entries: u64,
    /// Estimated from one sampled entry, see `NamespaceStats::approx_bytes`.
    pub approx_bytes: u64,
    /// Used by the running configuration, so it cannot be purged.
    pub in_use: bool,
}

impl EmbeddingCacheNamespace {
    /// The model and vector dimensions encoded in the namespace, for display.
    pub fn model_and_dimensions(&self) -> (&str, &str) {
        let mut parts = self.namespace.split('|');
        (parts.next().unwrap_or(""), parts.next().unwrap_or(""))
    }
}

#[server(ListEmbeddingCacheNamespaces, "/api")]
pub async fn list_embedding_cache_namespaces() -> Result<Vec<EmbeddingCacheNamespace>, ServerFnError>
{
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;
    let repo = state
        .embedding_cache_repo
        .as_ref()
        .ok_or_else(|| ServerFnError::new("The embedding cache is not available"))?;

    let rows = crate::jobs::embedding_cache::list_namespaces(
        repo.as_ref(),
        &state.embedding_cache_namespaces,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|(stats, in_use)| EmbeddingCacheNamespace {
            namespace: stats.namespace,
            entries: stats.entries,
            approx_bytes: stats.approx_bytes,
            in_use,
        })
        .collect())
}

/// Delete the given namespaces. Refused, deleting nothing, when any of them is
/// in use by the running configuration. Returns the number of entries deleted.
#[server(PurgeEmbeddingCacheNamespaces, "/api")]
pub async fn purge_embedding_cache_namespaces(
    namespaces: Vec<String>,
) -> Result<u64, ServerFnError> {
    let state = expect_context::<AppState>();
    let admin = require_admin_user(&state).await?;
    let repo = state
        .embedding_cache_repo
        .as_ref()
        .ok_or_else(|| ServerFnError::new("The embedding cache is not available"))?;

    tracing::info!(admin = %admin.email, ?namespaces, "embedding cache purge requested");
    crate::jobs::embedding_cache::purge_namespaces(
        repo.as_ref(),
        &state.embedding_cache_namespaces,
        &namespaces,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))
}

// ── Consistency report ───────────────────────────────────────────────────────

/// One category of the consistency report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsistencyFinding {
    /// Stable identifier, e.g. `s3_orphans`.
    pub id: String,
    pub title: String,
    /// What the category means and what to do about it.
    pub description: String,
    pub count: u64,
    /// At most [`CONSISTENCY_SAMPLE_LIMIT`] examples.
    pub samples: Vec<String>,
    /// Expected by design rather than a problem (shown for information).
    pub informational: bool,
}

/// Examples kept per category of the consistency report.
pub const CONSISTENCY_SAMPLE_LIMIT: usize = 20;

/// The read-only comparison of MongoDB, S3, Meilisearch and Qdrant.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConsistencyReport {
    /// What was compared, e.g. `("S3 objects", 1234)`.
    pub totals: Vec<(String, u64)>,
    pub findings: Vec<ConsistencyFinding>,
    /// Stores that were not compared, and why (feature disabled, listing
    /// failed).
    pub skipped: Vec<String>,
}

/// Compare MongoDB with S3, Meilisearch and Qdrant. Reads only: nothing is
/// deleted or modified.
#[server(RunConsistencyReport, "/api")]
pub async fn run_consistency_report() -> Result<ConsistencyReport, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    crate::jobs::consistency_report::build_report(crate::jobs::consistency_report::Sources {
        references: state.storage_reference_repo.as_ref(),
        documents: state.document_repo.as_ref(),
        assets: state.asset_repo.as_ref(),
        storage: state.storage_client.as_ref(),
        search: state.search_service.as_deref(),
        attachment_search: state.attachment_search_service.as_deref(),
        rag: state.rag_service.as_deref(),
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Bytes as a short human-readable size (`1.5 MB`), decimal units.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_uses_decimal_units() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(985_000_000), "985.0 MB");
    }

    #[test]
    fn namespace_exposes_model_and_dimensions() {
        let ns = EmbeddingCacheNamespace {
            namespace: "nomic-embed-text|768|0123456789abcdef".into(),
            entries: 1,
            approx_bytes: 1,
            in_use: false,
        };
        assert_eq!(ns.model_and_dimensions(), ("nomic-embed-text", "768"));
    }
}
