mod common;

use axum::{routing::post, Router};
use lekton::{
    config::IngestConfig,
    db::ingest_job_repository::{IngestJobRepository, IngestJobState, MongoIngestJobRepository},
    jobs::ingest::IngestQueue,
};
use std::{sync::Arc, time::Duration};

/// Storage reads can stall independently of embedding calls. The job deadline
/// must cover fetching the staged content as well as the ingest pipeline.
struct HangingReadStorage(Arc<dyn lekton::storage::client::StorageClient>);

#[async_trait::async_trait]
impl lekton::storage::client::StorageClient for HangingReadStorage {
    async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), lekton::error::AppError> {
        self.0.put_object(key, content).await
    }
    async fn get_object(&self, _: &str) -> Result<Option<Vec<u8>>, lekton::error::AppError> {
        std::future::pending().await
    }
    async fn delete_object(&self, key: &str) -> Result<(), lekton::error::AppError> {
        self.0.delete_object(key).await
    }
}

/// Exercise the HTTP contract, real unique indexes, durable recovery and the
/// existing ingest pipeline together, without waiting twenty seconds per test.
#[tokio::test]
async fn imports_survive_http_timeouts_and_are_polled_with_scope_checks() {
    let env = common::TestEnv::start().await;
    lekton::db::migrations::build_plan()
        .run(env.db.clone())
        .await
        .unwrap();
    let repository = Arc::new(MongoIngestJobRepository::new(&env.db));
    let queue = Arc::new(IngestQueue {
        repository: repository.clone(),
        config: IngestConfig {
            wait_seconds: 0,
            poll_interval_ms: 10,
            ..Default::default()
        },
    });
    let mut state = env.app_state.clone();
    state.ingest_queue = Some(queue.clone());
    let router = Router::new()
        .route("/api/v1/ingest", post(lekton::api::ingest::ingest_handler))
        .route(
            "/api/v1/ingest/jobs/{id}/poll",
            post(lekton::api::ingest_jobs::poll_handler),
        )
        .route("/api/v1/sync", post(lekton::api::sync::sync_handler))
        .with_state(state.clone());
    let server = axum_test::TestServer::builder().build(router);
    let mut body = serde_json::json!({
        "service_token": "test-token", "slug": "queue/doc", "source_id": "queue-source",
        "source_path": "doc.md", "title": "Queued document", "content": "# New content",
        "access_level": "developer", "service_owner": "test-team"
    });

    // Unauthorized uploads never create work or staging objects.
    body["service_token"] = "invalid".into();
    server
        .post("/api/v1/ingest")
        .json(&body)
        .await
        .assert_status_unauthorized();
    body["service_token"] = "test-token".into();
    let old = server.post("/api/v1/ingest").json(&body).await;
    old.assert_status(axum::http::StatusCode::CONFLICT);
    let pending: serde_json::Value = old.json();
    assert_eq!(pending["code"], "ingest_pending");
    let id = pending["job_id"].as_str().unwrap();
    let poll_url = pending["status_url"].as_str().unwrap();

    // Header negotiation changes only the pending response. Identical uploads
    // reuse the same durable job even when submitted concurrently.
    let modern = server
        .post("/api/v1/ingest")
        .add_header("Lekton-Long-Polling", "true")
        .json(&body)
        .await;
    modern.assert_status(axum::http::StatusCode::ACCEPTED);
    assert_eq!(modern.json::<serde_json::Value>()["job_id"], id);
    // Polling is scoped to the document, not merely to knowledge of a job ID.
    let scoped = lekton::db::service_token_models::ServiceToken {
        id: "other-ci".into(),
        name: "other-ci".into(),
        token_hash: lekton::auth::token_service::TokenService::hash_token("other-token"),
        allowed_scopes: vec!["another/*".into()],
        can_write: true,
        token_type: "service".into(),
        user_id: None,
        access_levels: None,
        created_by: "test".into(),
        created_at: chrono::Utc::now(),
        last_used_at: None,
        is_active: true,
    };
    env.service_token_repo.create(scoped).await.unwrap();
    server
        .post(poll_url)
        .json(&serde_json::json!({ "service_token": "other-token" }))
        .await
        .assert_status_forbidden();
    let job = repository.get(id).await.unwrap().unwrap();
    assert!(job.request.service_token.is_empty());
    assert!(job.request.content.is_empty());
    assert_eq!(
        env.storage
            .get_object(&job.staging_key)
            .await
            .unwrap()
            .unwrap(),
        b"# New content"
    );
    server
        .post(poll_url)
        .json(&serde_json::json!({ "service_token": "invalid" }))
        .await
        .assert_status_unauthorized();
    server.post("/api/v1/sync").json(&serde_json::json!({
        "service_token": "test-token", "source_id": "queue-source", "documents": [], "archive_missing": true
    })).await.assert_status_bad_request();

    // Simulate a dead worker with an expired lease; a new worker must recover
    // the claimed head item instead of dropping it or advancing past it.
    assert!(repository.acquire("dead-worker", 0).await.unwrap());
    let abandoned = repository.claim("dead-worker").await.unwrap().unwrap();
    assert_eq!(abandoned.id, id);
    assert!(repository.acquire("new-worker", 60).await.unwrap());
    assert!(!repository.acquire("other-worker", 60).await.unwrap());
    assert!(!repository.renew("dead-worker", 60).await.unwrap());
    let reclaimed = repository.claim("new-worker").await.unwrap().unwrap();
    assert_eq!(reclaimed.id, id);
    let mut stale = abandoned;
    stale.active = false;
    assert!(repository.finish(&stale).await.is_err());
    repository.release("new-worker").await.unwrap();

    let worker = tokio::spawn(queue.clone().run(state));
    let finished = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let job = repository.get(id).await.unwrap().unwrap();
            if !job.active {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(finished.state, IngestJobState::Succeeded);
    assert!(finished.result.as_ref().unwrap().indexed);
    tokio::time::timeout(Duration::from_secs(5), async {
        while env
            .storage
            .get_object(&finished.staging_key)
            .await
            .unwrap()
            .is_some()
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let response = server
        .post(poll_url)
        .json(&serde_json::json!({ "service_token": "test-token" }))
        .await;
    response.assert_status_ok();
    assert_eq!(response.json::<serde_json::Value>()["state"], "succeeded");
    assert!(
        !env.repo
            .find_by_slug("queue/doc")
            .await
            .unwrap()
            .unwrap()
            .needs_reindex
    );
    worker.abort();
    let _ = worker.await;
    // The worker may have started an empty queue scan after finishing the job.
    // Release the actual lease left by the aborted test task, not an old owner.
    let lease = env
        .db
        .collection::<mongodb::bson::Document>("ingest_worker_locks")
        .find_one(mongodb::bson::doc! { "_id": "global" })
        .await
        .unwrap()
        .unwrap();
    repository
        .release(lease.get_str("owner").unwrap())
        .await
        .unwrap();

    // Concurrent equivalent uploads share one job; a later different update
    // remains a distinct FIFO item even when timestamps coincide.
    let mut next: lekton::db::models::IngestRequest = serde_json::from_value(body.clone()).unwrap();
    next.slug = "queue/next".into();
    next.source_path = "next.md".into();
    let (left, right) = tokio::join!(
        queue.enqueue(env.storage.as_ref(), next.clone(), "ci".into()),
        queue.enqueue(env.storage.as_ref(), next.clone(), "ci".into()),
    );
    let first = left.unwrap();
    assert_eq!(first.id, right.unwrap().id);
    next.content = "# Later revision".into();
    let second = queue
        .enqueue(env.storage.as_ref(), next, "ci".into())
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    assert!(first.sequence < second.sequence);
    assert!(repository.acquire("fifo", 60).await.unwrap());
    let mut claimed = repository.claim("fifo").await.unwrap().unwrap();
    assert_eq!(claimed.id, first.id);
    claimed.active = false;
    claimed.state = IngestJobState::Failed;
    claimed.error = Some("test failure".into());
    repository.finish(&claimed).await.unwrap();
    let failure_url = format!("/api/v1/ingest/jobs/{}/poll", claimed.id);
    let failure = server
        .post(&failure_url)
        .json(&serde_json::json!({"service_token":"test-token"}))
        .await;
    failure.assert_status(axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(failure.json::<serde_json::Value>()["error"], "test failure");
    assert_eq!(
        repository.claim("fifo").await.unwrap().unwrap().id,
        second.id
    );
    repository.release("fifo").await.unwrap();
    // Business failures from the real ingest pipeline become terminal job
    // failures, rather than a successful acceptance or a vanished task.
    let mut conflict = body.clone();
    conflict["source_id"] = "another-source".into();
    conflict["content"] = "# Conflicting import".into();
    let accepted = server
        .post("/api/v1/ingest")
        .add_header("Lekton-Long-Polling", "true")
        .json(&conflict)
        .await;
    accepted.assert_status(axum::http::StatusCode::ACCEPTED);
    let failed_id = accepted.json::<serde_json::Value>()["job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let worker = tokio::spawn(queue.run(env.app_state.clone()));
    let failed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let job = repository.get(&failed_id).await.unwrap().unwrap();
            if !job.active {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    worker.abort();
    let _ = worker.await;
    assert_eq!(failed.state, IngestJobState::Failed);
    assert!(failed
        .error
        .as_deref()
        .unwrap()
        .contains("cannot be overwritten"));
    server
        .post(&format!("/api/v1/ingest/jobs/{failed_id}/poll"))
        .json(&serde_json::json!({"service_token":"test-token"}))
        .await
        .assert_status(axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    let lease = env
        .db
        .collection::<mongodb::bson::Document>("ingest_worker_locks")
        .find_one(mongodb::bson::doc! { "_id": "global" })
        .await
        .unwrap()
        .unwrap();
    repository
        .release(lease.get_str("owner").unwrap())
        .await
        .unwrap();
    let deadline_queue = Arc::new(IngestQueue {
        repository: repository.clone(),
        config: IngestConfig {
            wait_seconds: 0,
            job_timeout_seconds: 1,
            poll_interval_ms: 10,
            ..Default::default()
        },
    });
    let mut request: lekton::db::models::IngestRequest = serde_json::from_value(body).unwrap();
    request.slug = "queue/hanging-read".into();
    request.source_path = "hanging-read.md".into();
    let queued = deadline_queue
        .enqueue(env.storage.as_ref(), request, "ci".into())
        .await
        .unwrap();
    let mut worker_state = env.app_state.clone();
    worker_state.storage_client = Arc::new(HangingReadStorage(env.storage.clone()));
    let worker = tokio::spawn(deadline_queue.run(worker_state));
    let timed_out = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let job = repository.get(&queued.id).await.unwrap().unwrap();
            if !job.active {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    worker.abort();
    let _ = worker.await;
    assert_eq!(timed_out.state, IngestJobState::Failed);
    assert!(timed_out.expires_at.is_some());
}
