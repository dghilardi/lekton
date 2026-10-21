//! Serial, durable document imports. HTTP cancellation never cancels a job.
use std::{sync::Arc, time::Duration};

use chrono::Utc;
use futures::FutureExt;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::{
    api::ingest::{process_authorized_ingest, spawn_attachment_acl_recompute, IngestContext},
    app::AppState,
    config::IngestConfig,
    db::{
        ingest_job_repository::{IngestJob, IngestJobRepository, IngestJobState},
        models::IngestRequest,
    },
    error::AppError,
    storage::client::StorageClient,
};

pub struct IngestQueue {
    pub repository: Arc<dyn IngestJobRepository>,
    pub config: IngestConfig,
}

pub fn context(state: &AppState) -> IngestContext<'_> {
    IngestContext {
        repo: state.document_repo.as_ref(),
        asset_repo: state.asset_repo.as_ref(),
        storage: state.storage_client.as_ref(),
        search: state.search_service.as_deref(),
        access_level_repo: state.access_level_repo.as_ref(),
        service_token_repo: state.service_token_repo.as_ref(),
        revision_repo: state.document_revision_repo.as_ref(),
        release_repo: state.release_repo.as_ref(),
        rag: state.rag_service.as_deref(),
        legacy_token: Some(&state.service_token),
    }
}

impl IngestQueue {
    pub async fn enqueue(
        &self,
        storage: &dyn StorageClient,
        mut request: IngestRequest,
        actor: String,
    ) -> Result<IngestJob, AppError> {
        request.service_token.clear();
        // Include all request fields (content, metadata, source and release), but
        // never credentials, in the idempotency key.
        let dedup_key =
            crate::auth::token_service::TokenService::hash_token(&serde_json::to_string(&request)?);
        let id = Uuid::new_v4().to_string();
        let staging_key = format!("{}/{}.md", self.config.staging_prefix.trim_matches('/'), id);
        storage
            .put_object(
                &staging_key,
                std::mem::take(&mut request.content).into_bytes(),
            )
            .await?;
        let job = IngestJob {
            id: id.clone(),
            sequence: 0,
            dedup_key,
            active: true,
            state: IngestJobState::Queued,
            request,
            actor,
            staging_key: staging_key.clone(),
            worker: None,
            attempts: 0,
            created_at: Utc::now(),
            result: None,
            error: None,
            expires_at: None,
        };
        match self.repository.enqueue(job).await {
            Ok(existing) => {
                if existing.id != id {
                    let _ = storage.delete_object(&staging_key).await;
                }
                Ok(existing)
            }
            // On an ambiguous database timeout, the insert may have committed.
            // Leave staging intact so a committed job can still be recovered.
            Err(e) => Err(e),
        }
    }

    pub async fn wait(&self, mut job: IngestJob) -> Result<IngestJob, AppError> {
        let id = job.id.clone();
        let waiting = async {
            loop {
                if !job.active {
                    return Ok::<(), AppError>(());
                }
                sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                job = self
                    .repository
                    .get(&id)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Import job not found".into()))?;
            }
        };
        // Keep the latest observed state without an unbounded extra database
        // call after the HTTP wait deadline has elapsed.
        if let Ok(result) = timeout(Duration::from_secs(self.config.wait_seconds), waiting).await {
            result?;
        }
        Ok(job)
    }

    /// Start one candidate worker per replica. The MongoDB lease allows only
    /// one to claim work; abandoned processing jobs are retried first.
    pub async fn run(self: Arc<Self>, state: AppState) {
        loop {
            // An idle replica only reads the queue, rather than writing a lease
            // on every scan. Claiming is still serialized after this hint.
            match self.repository.has_work().await {
                Ok(true) => (),
                other => {
                    if let Err(error) = other {
                        tracing::warn!(%error, "Could not inspect import queue");
                    }
                    sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                    continue;
                }
            }
            let owner = Uuid::new_v4().to_string();
            match self
                .repository
                .acquire(&owner, self.config.lease_seconds)
                .await
            {
                Ok(true) => {
                    let work = std::panic::AssertUnwindSafe(self.process_next(&state, &owner))
                        .catch_unwind();
                    let heartbeat = async {
                        loop {
                            sleep(Duration::from_secs(self.config.lease_seconds / 3)).await;
                            // Fail closed: losing the lease drops in-flight work.
                            match timeout(
                                Duration::from_secs(self.config.lease_seconds / 3),
                                self.repository.renew(&owner, self.config.lease_seconds),
                            )
                            .await
                            {
                                Ok(Ok(true)) => (),
                                _ => break,
                            }
                        }
                    };
                    tokio::select! {
                        result = work => match result {
                            Ok(Ok(())) => (),
                            Ok(Err(error)) => tracing::error!(%error, "Import worker failed; job will be recovered"),
                            Err(_) => tracing::error!("Import worker panicked; job will be recovered"),
                        },
                        _ = heartbeat => tracing::warn!("Import lease lost; stopping current attempt"),
                    }
                    if let Err(error) = self.repository.release(&owner).await {
                        tracing::warn!(%error, "Could not release import lease");
                    }
                }
                Ok(false) => (),
                Err(error) => tracing::warn!(%error, "Could not acquire import lease"),
            }
            sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
        }
    }

    async fn process_next(&self, state: &AppState, owner: &str) -> Result<(), AppError> {
        let Some(mut job) = self.repository.claim(owner).await? else {
            return Ok(());
        };
        tracing::info!(job_id = %job.id, slug = %job.request.slug, attempt = job.attempts, "Processing document import");
        let result = timeout(
            Duration::from_secs(self.config.job_timeout_seconds),
            async {
                if job.attempts > self.config.max_attempts {
                    return Err(AppError::Internal(
                        "Import recovery attempts exhausted".into(),
                    ));
                }
                let bytes = state
                    .storage_client
                    .get_object(&job.staging_key)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Staged import content missing".into()))?;
                let mut request = job.request.clone();
                request.content =
                    String::from_utf8(bytes).map_err(|e| AppError::BadRequest(e.to_string()))?;
                process_authorized_ingest(&context(state), request, &job.actor).await
            },
        )
        .await
        .unwrap_or_else(|_| {
            Err(AppError::Internal(
                "Import execution deadline exceeded".into(),
            ))
        });
        match result {
            Ok(outcome) => {
                job.state = if outcome.response.indexed {
                    IngestJobState::Succeeded
                } else {
                    IngestJobState::Failed
                };
                if !outcome.response.indexed {
                    job.error = Some(
                        "Document saved, but search/RAG indexing failed; retry the upload".into(),
                    );
                }
                job.result = Some(outcome.response);
                spawn_attachment_acl_recompute(
                    state.rag_service.clone(),
                    state.asset_repo.clone(),
                    state.document_repo.clone(),
                    state.storage_client.clone(),
                    state.attachment_search_service.clone(),
                    outcome.assets_to_recompute,
                );
            }
            Err(error) => {
                tracing::error!(job_id = %job.id, %error, "Document import failed");
                job.state = IngestJobState::Failed;
                job.error = Some(match error {
                    AppError::BadRequest(message)
                    | AppError::Forbidden(message)
                    | AppError::NotFound(message) => message,
                    _ => "Import failed; check server logs and retry the upload".into(),
                });
            }
        }
        job.active = false;
        job.expires_at =
            Some(Utc::now() + chrono::Duration::days(i64::from(self.config.retention_days)));
        self.repository.finish(&job).await?;
        // Durable completion comes first. A failed delete is harmless; lifecycle
        // rules can clean up leftover staging objects after the recovery window.
        match timeout(
            Duration::from_secs(5),
            state.storage_client.delete_object(&job.staging_key),
        )
        .await
        {
            Ok(Ok(())) => (),
            other => {
                tracing::warn!(job_id = %job.id, error = ?other, "Could not remove import staging object")
            }
        }
        Ok(())
    }
}
