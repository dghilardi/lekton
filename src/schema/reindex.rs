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
    /// Per-run failed/skipped counters and last error.
    pub outcome: crate::jobs::JobOutcome,
}

impl crate::jobs::RunningFlag for SchemaEndpointReindexState {
    fn is_running(&self) -> &AtomicBool {
        &self.is_running
    }
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
    // Reset `is_running` unconditionally when this task ends, even on an early
    // return or panic, so a crashed reindex cannot block all future runs.
    let _guard = crate::jobs::RunningGuard::new(reindex.clone());

    reindex.progress.store(0, Ordering::Relaxed);
    reindex.outcome.reset();

    let mut schemas = match schema_repo.list_all().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Schema endpoint reindex: failed to list schemas: {e}");
            return;
        }
    };

    let total = schemas.len();
    if total == 0 {
        tracing::info!("Schema endpoint reindex: no schemas found");
        reindex.progress.store(100, Ordering::Relaxed);
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
                        reindex.outcome.record_skip();
                        continue;
                    }
                },
                Ok(None) => {
                    tracing::warn!(
                        schema = %schema.name,
                        version = %version.version,
                        "Schema endpoint reindex: content not found in storage, skipping"
                    );
                    reindex.outcome.record_skip();
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        schema = %schema.name,
                        version = %version.version,
                        "Schema endpoint reindex: storage error, skipping: {e}"
                    );
                    reindex
                        .outcome
                        .record_failure(format!("read {}@{}: {e}", schema.name, version.version));
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
            reindex
                .outcome
                .record_failure(format!("persist {}: {e}", schema.name));
        }

        update_progress(&reindex, i, total);
    }

    tracing::info!(total, "Schema endpoint reindex: complete");
    reindex.progress.store(100, Ordering::Relaxed);
}

fn update_progress(reindex: &SchemaEndpointReindexState, index: usize, total: usize) {
    let pct = ((index + 1) * 100 / total) as u32;
    reindex.progress.store(pct, Ordering::Relaxed);
}
