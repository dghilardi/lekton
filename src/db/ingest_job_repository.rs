//! Durable import queue. A renewable, database-wide lease serializes workers.
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mongodb::{bson::doc, options::ReturnDocument, Collection, Database};
use serde::{Deserialize, Serialize};

use crate::db::models::{IngestRequest, IngestResponse};
use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestJobState {
    Queued,
    Processing,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestJob {
    pub id: String,
    pub sequence: i64,
    pub dedup_key: String,
    pub active: bool,
    pub state: IngestJobState,
    /// Metadata only: content and service_token are empty. The body is in S3.
    pub request: IngestRequest,
    pub actor: String,
    pub staging_key: String,
    pub worker: Option<String>,
    pub attempts: u32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    pub result: Option<IngestResponse>,
    pub error: Option<String>,
    #[serde(
        default,
        with = "bson::serde_helpers::chrono_datetime_as_bson_datetime_optional"
    )]
    pub expires_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait IngestJobRepository: Send + Sync {
    /// Atomic insert-or-reuse for an identical active request.
    async fn enqueue(&self, job: IngestJob) -> Result<IngestJob, AppError>;
    async fn has_work(&self) -> Result<bool, AppError>;
    async fn has_active_source(&self, source_id: &str) -> Result<bool, AppError>;
    async fn get(&self, id: &str) -> Result<Option<IngestJob>, AppError>;
    async fn acquire(&self, owner: &str, seconds: u64) -> Result<bool, AppError>;
    async fn renew(&self, owner: &str, seconds: u64) -> Result<bool, AppError>;
    async fn release(&self, owner: &str) -> Result<(), AppError>;
    async fn claim(&self, owner: &str) -> Result<Option<IngestJob>, AppError>;
    async fn finish(&self, job: &IngestJob) -> Result<(), AppError>;
}

pub struct MongoIngestJobRepository {
    jobs: Collection<IngestJob>,
    locks: Collection<mongodb::bson::Document>,
}

impl MongoIngestJobRepository {
    pub fn new(db: &Database) -> Self {
        Self {
            jobs: db.collection("ingest_jobs"),
            locks: db.collection("ingest_worker_locks"),
        }
    }
}

fn lease_update(owner: &str, seconds: u64) -> Vec<mongodb::bson::Document> {
    vec![doc! { "$set": {
        "owner": owner,
        "lease_until": { "$add": ["$$NOW", (seconds as i64) * 1000] }
    } }]
}

#[async_trait]
impl IngestJobRepository for MongoIngestJobRepository {
    async fn enqueue(&self, mut job: IngestJob) -> Result<IngestJob, AppError> {
        let counter = self
            .locks
            .find_one_and_update(
                doc! { "_id": "sequence" },
                doc! { "$inc": { "value": 1_i64 } },
            )
            .return_document(ReturnDocument::After)
            .await?
            .ok_or_else(|| AppError::Internal("Import sequence is not initialized".into()))?;
        job.sequence = counter
            .get_i64("value")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let value =
            mongodb::bson::to_document(&job).map_err(|e| AppError::Internal(e.to_string()))?;
        // The partial unique index also protects concurrent upserts on replicas.
        match self
            .jobs
            .find_one_and_update(
                doc! { "dedup_key": &job.dedup_key, "active": true },
                doc! { "$setOnInsert": value },
            )
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await
        {
            Ok(Some(job)) => Ok(job),
            Err(e) if e.to_string().contains("E11000") => self
                .jobs
                .find_one(doc! { "dedup_key": &job.dedup_key, "active": true })
                .await?
                .ok_or_else(|| {
                    AppError::Internal("Concurrent import completed; retry upload".into())
                }),
            Err(e) => Err(e.into()),
            _ => Err(AppError::Internal("Could not enqueue import".into())),
        }
    }

    async fn has_work(&self) -> Result<bool, AppError> {
        Ok(self.jobs.find_one(doc! { "active": true }).await?.is_some())
    }

    async fn has_active_source(&self, source_id: &str) -> Result<bool, AppError> {
        Ok(self
            .jobs
            .find_one(doc! { "active": true, "request.source_id": source_id })
            .await?
            .is_some())
    }

    async fn get(&self, id: &str) -> Result<Option<IngestJob>, AppError> {
        Ok(self.jobs.find_one(doc! { "id": id }).await?)
    }

    async fn acquire(&self, owner: &str, seconds: u64) -> Result<bool, AppError> {
        Ok(self
            .locks
            .find_one_and_update(
                doc! { "_id": "global", "$expr": { "$lte": ["$lease_until", "$$NOW"] } },
                lease_update(owner, seconds),
            )
            .await?
            .is_some())
    }

    async fn renew(&self, owner: &str, seconds: u64) -> Result<bool, AppError> {
        Ok(self.locks.update_one(
            doc! { "_id": "global", "owner": owner, "$expr": { "$gt": ["$lease_until", "$$NOW"] } },
            lease_update(owner, seconds),
        ).await?.matched_count == 1)
    }

    async fn release(&self, owner: &str) -> Result<(), AppError> {
        self.locks
            .update_one(
                doc! { "_id": "global", "owner": owner },
                doc! { "$set": { "lease_until": bson::DateTime::from_millis(0) } },
            )
            .await?;
        Ok(())
    }

    async fn claim(&self, owner: &str) -> Result<Option<IngestJob>, AppError> {
        // A processing job left by a dead worker stays at the head of the queue.
        Ok(self.jobs.find_one_and_update(doc! { "active": true },
            doc! { "$set": { "state": "processing", "worker": owner }, "$inc": { "attempts": 1 } })
            .sort(doc! { "sequence": 1 })
            .return_document(ReturnDocument::After).await?)
    }

    async fn finish(&self, job: &IngestJob) -> Result<(), AppError> {
        let updated = self
            .jobs
            .replace_one(
                doc! { "id": &job.id, "worker": &job.worker, "active": true },
                job,
            )
            .await?;
        if updated.matched_count != 1 {
            return Err(AppError::Internal("Import worker lost ownership".into()));
        }
        Ok(())
    }
}
