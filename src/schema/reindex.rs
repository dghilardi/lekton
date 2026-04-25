use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::db::schema_repository::SchemaRepository;
use crate::storage::client::StorageClient;

/// Shared state for tracking a background schema endpoint re-index operation.
#[derive(Default)]
pub struct SchemaEndpointReindexState {
    pub is_running: AtomicBool,
    /// Progress percentage (0–100).
    pub progress: AtomicU32,
}

/// Re-extract and persist API endpoints for every non-archived schema version.
///
/// Fetches each version's content from S3, runs endpoint extraction, and
/// writes the result back to MongoDB via `create_or_update`. Versions whose
/// S3 content cannot be retrieved are skipped without failing the whole job.
pub async fn run_schema_endpoint_reindex(
    reindex: Arc<SchemaEndpointReindexState>,
    schema_repo: Arc<dyn SchemaRepository>,
    storage: Arc<dyn StorageClient>,
) {
    reindex.progress.store(0, Ordering::Relaxed);

    let mut schemas = match schema_repo.list_all().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Schema endpoint reindex: failed to list schemas: {e}");
            reindex.is_running.store(false, Ordering::Release);
            return;
        }
    };

    let total = schemas.len();
    if total == 0 {
        tracing::info!("Schema endpoint reindex: no schemas found");
        reindex.progress.store(100, Ordering::Relaxed);
        reindex.is_running.store(false, Ordering::Release);
        return;
    }

    tracing::info!(total, "Schema endpoint reindex: starting");

    for (i, schema) in schemas.iter_mut().enumerate() {
        for version in schema.versions.iter_mut().filter(|v| !v.is_archived) {
            let content = match storage.get_object(&version.s3_key).await {
                Ok(Some(bytes)) => match String::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(
                            schema = %schema.name,
                            version = %version.version,
                            "Schema endpoint reindex: invalid UTF-8, skipping: {e}"
                        );
                        continue;
                    }
                },
                Ok(None) => {
                    tracing::warn!(
                        schema = %schema.name,
                        version = %version.version,
                        "Schema endpoint reindex: content not found in storage, skipping"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        schema = %schema.name,
                        version = %version.version,
                        "Schema endpoint reindex: storage error, skipping: {e}"
                    );
                    continue;
                }
            };

            version.endpoints =
                crate::api::schemas::extract_schema_endpoints(&schema.schema_type, &content);
        }

        if let Err(e) = schema_repo.create_or_update(schema.clone()).await {
            tracing::warn!(
                schema = %schema.name,
                "Schema endpoint reindex: failed to persist updated schema: {e}"
            );
        }

        update_progress(&reindex, i, total);
    }

    tracing::info!(total, "Schema endpoint reindex: complete");
    reindex.progress.store(100, Ordering::Relaxed);
    reindex.is_running.store(false, Ordering::Release);
}

fn update_progress(reindex: &SchemaEndpointReindexState, index: usize, total: usize) {
    let pct = ((index + 1) * 100 / total) as u32;
    reindex.progress.store(pct, Ordering::Relaxed);
}
