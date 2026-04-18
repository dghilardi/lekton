mod common;

use lekton::db::repository::DocumentRepository;

#[tokio::test]
async fn sync_identifies_new_docs_to_upload() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    // Server has no docs. Client sends one.
    let response = server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "documents": [
                { "slug": "docs/new", "content_hash": "sha256:abc" }
            ],
            "archive_missing": false
        }))
        .await;

    let body: serde_json::Value = response.json();
    let to_upload = body["to_upload"].as_array().unwrap();
    let unchanged = body["unchanged"].as_array().unwrap();
    let to_archive = body["to_archive"].as_array().unwrap();

    assert_eq!(to_upload.len(), 1);
    assert_eq!(to_upload[0], "docs/new");
    assert!(unchanged.is_empty());
    assert!(to_archive.is_empty());
}

#[tokio::test]
async fn sync_identifies_unchanged_docs() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("sync-unchanged-{}", uuid::Uuid::new_v4());
    env.ingest(&server, &slug, "Doc", "# Content", "public")
        .await;

    // Get the stored content hash
    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    let hash = doc.content_hash.unwrap();

    let response = server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "documents": [
                { "slug": slug, "content_hash": hash }
            ],
            "archive_missing": false
        }))
        .await;

    let body: serde_json::Value = response.json();
    assert_eq!(body["unchanged"].as_array().unwrap().len(), 1);
    assert!(body["to_upload"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn sync_identifies_changed_docs() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("sync-changed-{}", uuid::Uuid::new_v4());
    env.ingest(&server, &slug, "Doc", "# Original", "public")
        .await;

    let response = server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "documents": [
                { "slug": slug, "content_hash": "sha256:different" }
            ],
            "archive_missing": false
        }))
        .await;

    let body: serde_json::Value = response.json();
    assert_eq!(body["to_upload"].as_array().unwrap().len(), 1);
    assert!(body["unchanged"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn sync_identifies_docs_to_archive() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug_keep = format!("sync-keep-{}", uuid::Uuid::new_v4());
    let slug_remove = format!("sync-remove-{}", uuid::Uuid::new_v4());

    env.ingest(&server, &slug_keep, "Keep", "# Keep", "public")
        .await;
    env.ingest(&server, &slug_remove, "Remove", "# Remove", "public")
        .await;

    let doc = env.repo.find_by_slug(&slug_keep).await.unwrap().unwrap();
    let hash = doc.content_hash.unwrap();

    // Only send slug_keep; slug_remove is missing from client
    let response = server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "documents": [
                { "slug": slug_keep, "content_hash": hash }
            ],
            "archive_missing": false
        }))
        .await;

    let body: serde_json::Value = response.json();
    let to_archive: Vec<String> = body["to_archive"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(to_archive.contains(&slug_remove));
}

#[tokio::test]
async fn sync_archive_missing_sets_flag() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug_keep = format!("sync-arch-keep-{}", uuid::Uuid::new_v4());
    let slug_archive = format!("sync-arch-gone-{}", uuid::Uuid::new_v4());

    env.ingest(&server, &slug_keep, "Keep", "# Keep", "public")
        .await;
    env.ingest(&server, &slug_archive, "Gone", "# Gone", "public")
        .await;

    let doc = env.repo.find_by_slug(&slug_keep).await.unwrap().unwrap();
    let hash = doc.content_hash.unwrap();

    server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "documents": [
                { "slug": slug_keep, "content_hash": hash }
            ],
            "archive_missing": true
        }))
        .await;

    // Verify archived flag
    let archived = env.repo.find_by_slug(&slug_archive).await.unwrap().unwrap();
    assert!(archived.is_archived, "Missing doc should be archived");

    let kept = env.repo.find_by_slug(&slug_keep).await.unwrap().unwrap();
    assert!(!kept.is_archived, "Kept doc should not be archived");
}

#[tokio::test]
async fn sync_with_scoped_token() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let raw = env
        .create_service_token("sync-scoped", vec!["sync-ns/*".to_string()], true)
        .await;

    // Ingest a doc with the legacy token first
    let slug = format!("sync-ns/doc-{}", uuid::Uuid::new_v4());
    env.ingest(&server, &slug, "Doc", "# Doc", "public").await;

    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    let hash = doc.content_hash.unwrap();

    // Sync with scoped token
    let response = server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": raw,
            "documents": [
                { "slug": slug, "content_hash": hash }
            ],
            "archive_missing": false
        }))
        .await;

    let body: serde_json::Value = response.json();
    assert_eq!(body["unchanged"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn sync_rejects_out_of_scope_slug() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let raw = env
        .create_service_token("sync-scope-reject", vec!["allowed/*".to_string()], true)
        .await;

    let response = server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": raw,
            "documents": [
                { "slug": "forbidden/doc", "content_hash": "sha256:abc" }
            ],
            "archive_missing": false
        }))
        .await;

    response.assert_status_forbidden();
}

#[tokio::test]
async fn sync_rejects_invalid_token() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": "invalid-token",
            "documents": [],
            "archive_missing": false
        }))
        .await;

    response.assert_status_unauthorized();
}
