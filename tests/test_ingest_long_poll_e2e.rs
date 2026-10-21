mod common;

use axum::{http::StatusCode, middleware, routing::post, Router};
use lekton::{
    config::IngestConfig,
    db::ingest_job_repository::{IngestJobRepository, IngestJobState, MongoIngestJobRepository},
    jobs::ingest::IngestQueue,
    storage::client::StorageClient,
};
use std::{sync::Arc, time::Duration};

/// Keep real S3 storage, but make processing exceed the actual HTTP wait budget.
struct SlowStagingStorage(Arc<dyn StorageClient>);

#[async_trait::async_trait]
impl StorageClient for SlowStagingStorage {
    async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), lekton::error::AppError> {
        self.0.put_object(key, content).await
    }

    async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, lekton::error::AppError> {
        if key.starts_with("ingest-staging/") {
            tokio::time::sleep(Duration::from_secs(25)).await;
        }
        self.0.get_object(key).await
    }

    async fn delete_object(&self, key: &str) -> Result<(), lekton::error::AppError> {
        self.0.delete_object(key).await
    }
}

struct Workspace(std::path::PathBuf);

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn run_cli(
    binary: &std::path::Path,
    root: &std::path::Path,
    url: &str,
) -> std::process::Output {
    tokio::time::timeout(
        Duration::from_secs(90),
        tokio::process::Command::new(binary)
            .arg(root)
            .args(["--verbose", "--ingest-timeout-seconds", "60"])
            .env("LEKTON_URL", url)
            .env("LEKTON_TOKEN", "test-token")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("CLI exceeded the end-to-end deadline")
    .expect("Could not launch lekton-sync")
}

/// Build the real CLI first, then run this explicitly with its absolute path:
/// LEKTON_SYNC_TEST_BINARY=/path/to/lekton-sync cargo test --features ssr \
///   --test test_ingest_long_poll_e2e -- --ignored --nocapture
#[tokio::test]
#[ignore = "requires Docker and LEKTON_SYNC_TEST_BINARY pointing to the built CLI"]
async fn real_cli_polls_past_twenty_seconds_and_legacy_requests_fail_explicitly() {
    let binary = std::path::PathBuf::from(
        std::env::var("LEKTON_SYNC_TEST_BINARY").expect("Set LEKTON_SYNC_TEST_BINARY"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let env = common::TestEnv::start().await;
    lekton::db::migrations::build_plan()
        .run(env.db.clone())
        .await
        .unwrap();
    let repository = Arc::new(MongoIngestJobRepository::new(&env.db));
    let queue = Arc::new(IngestQueue {
        repository: repository.clone(),
        config: IngestConfig::default(),
    });
    assert_eq!(queue.config.wait_seconds, 20);
    let mut state = env.app_state.clone();
    state.ingest_queue = Some(queue.clone());
    state.storage_client = Arc::new(SlowStagingStorage(env.storage.clone()));
    let responses = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = responses.clone();
    let router = Router::new()
        .route("/api/v1/ingest", post(lekton::api::ingest::ingest_handler))
        .route(
            "/api/v1/ingest/jobs/{id}/poll",
            post(lekton::api::ingest_jobs::poll_handler),
        )
        .route("/api/v1/sync", post(lekton::api::sync::sync_handler))
        .layer(middleware::from_fn(
            move |request: axum::extract::Request, next: middleware::Next| {
                let recorded = recorded.clone();
                async move {
                    let path = request.uri().path().to_owned();
                    let response = next.run(request).await;
                    recorded.lock().unwrap().push((path, response.status()));
                    response
                }
            },
        ))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let worker = tokio::spawn(queue.run(state));
    let workspace =
        Workspace(std::env::temp_dir().join(format!("lekton-e2e-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&workspace.0).unwrap();
    std::fs::write(
        workspace.0.join(".lekton.yml"),
        "id: e2e-source\ndefault_access_level: developer\ndefault_service_owner: test-team\n",
    )
    .unwrap();
    std::fs::write(
        workspace.0.join("modern.md"),
        "---\nlekton-import: true\nslug: e2e/modern\ntitle: Modern import\n---\n# Actual CLI import\n",
    )
    .unwrap();

    let started = std::time::Instant::now();
    let output = run_cli(&binary, &workspace.0, &url).await;
    let elapsed = started.elapsed();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("Modern CLI completed in {elapsed:.2?}\n{stdout}\n{stderr}");
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(elapsed >= Duration::from_secs(25));
    assert!(stdout.contains("1 uploaded"));
    {
        let responses = responses.lock().unwrap();
        assert!(responses.contains(&("/api/v1/ingest".into(), StatusCode::ACCEPTED)));
        assert!(responses
            .iter()
            .any(|(path, status)| { path.ends_with("/poll") && *status == StatusCode::OK }));
    }
    let document = env.repo.find_by_slug("e2e/modern").await.unwrap().unwrap();
    assert!(!document.needs_reindex);
    assert!(env
        .storage
        .get_object(&document.s3_key)
        .await
        .unwrap()
        .is_some());
    let repeat = run_cli(&binary, &workspace.0, &url).await;
    let repeat_stdout = String::from_utf8_lossy(&repeat.stdout);
    assert!(repeat.status.success());
    assert!(
        repeat_stdout.contains("0 uploaded, 1 unchanged"),
        "{repeat_stdout}"
    );
    println!("Repeat CLI sync: {repeat_stdout}");

    // This is the legacy wire contract: no capability header. It must return
    // an error within the load-balancer budget while durable work continues.
    let started = std::time::Instant::now();
    let response = reqwest::Client::new()
        .post(format!("{url}/api/v1/ingest"))
        .timeout(Duration::from_secs(30))
        .json(&serde_json::json!({
            "service_token": "test-token", "slug": "e2e/legacy", "source_id": "legacy-source",
            "source_path": "legacy.md", "title": "Legacy import", "content": "# Legacy content",
            "access_level": "developer", "service_owner": "test-team"
        }))
        .send()
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(elapsed >= Duration::from_secs(20));
    assert!(elapsed < Duration::from_secs(25));
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["code"], "ingest_pending");
    println!("Legacy request returned 409 ingest_pending after {elapsed:.2?}");
    let id = body["job_id"].as_str().unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let job = repository.get(id).await.unwrap().unwrap();
            if !job.active {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(finished.state, IngestJobState::Succeeded);
    assert!(finished.result.unwrap().indexed);
    assert!(
        !env.repo
            .find_by_slug("e2e/legacy")
            .await
            .unwrap()
            .unwrap()
            .needs_reindex
    );
    println!("Legacy background job completed successfully and needs_reindex=false");
    worker.abort();
    server.abort();
    let _ = tokio::join!(worker, server);
}
