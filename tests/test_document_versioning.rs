mod common;

use lekton::db::document_version_repository::DocumentVersionRepository;
use lekton::db::repository::DocumentRepository;
use lekton::storage::client::StorageClient;

// ── Content hash integration tests ───────────────────────────────────────────

#[tokio::test]
async fn ingest_stores_content_hash() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("hash-test-{}", uuid::Uuid::new_v4());
    env.ingest(&server, &slug, "Doc", "# Content", "public")
        .await;

    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    assert!(doc.content_hash.is_some());
    assert!(doc.content_hash.unwrap().starts_with("sha256:"));
}

#[tokio::test]
async fn ingest_unchanged_content_returns_not_changed() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("unchanged-{}", uuid::Uuid::new_v4());

    // First ingest
    env.ingest(&server, &slug, "Doc", "# Same content", "public")
        .await;

    // Second ingest with same content and metadata
    let response = server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": slug,
            "source_path": format!("docs/{}.md", slug),
            "title": "Doc",
            "summary": format!("Test summary for Doc used by automated ingestion checks."),
            "content": "# Same content",
            "access_level": "public",
            "service_owner": "test-team",
            "tags": ["test"],
            "order": 0,
            "is_hidden": false
        }))
        .await;

    let body: serde_json::Value = response.json();
    assert_eq!(body["changed"], false);
}

#[tokio::test]
async fn ingest_changed_content_returns_changed() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("changed-{}", uuid::Uuid::new_v4());
    env.ingest(&server, &slug, "Doc", "# Original", "public")
        .await;

    let response = server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": slug,
            "source_path": format!("docs/{}.md", slug),
            "title": "Doc",
            "content": "# Updated content",
            "access_level": "public",
            "service_owner": "test-team",
            "tags": ["test"],
            "order": 0,
            "is_hidden": false
        }))
        .await;

    let body: serde_json::Value = response.json();
    assert_eq!(body["changed"], true);
}

// ── Document versioning integration tests ────────────────────────────────────

#[tokio::test]
async fn ingest_content_change_creates_version() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("version-test-{}", uuid::Uuid::new_v4());

    // First ingest
    env.ingest(&server, &slug, "Doc v1", "# Version 1", "public")
        .await;

    // Verify no versions yet (first ingest doesn't create a version record)
    let versions = env.document_version_repo.list_by_slug(&slug).await.unwrap();
    assert!(
        versions.is_empty(),
        "No versions should exist after first ingest"
    );

    // Second ingest with different content
    env.ingest(&server, &slug, "Doc v2", "# Version 2", "public")
        .await;

    // Now there should be a version record for the old content
    let versions = env.document_version_repo.list_by_slug(&slug).await.unwrap();
    assert_eq!(versions.len(), 1, "One version should exist after update");
    assert_eq!(versions[0].version, 1);
    assert!(versions[0].content_hash.starts_with("sha256:"));
    assert_eq!(versions[0].updated_by, "legacy");

    // Third ingest with yet another change
    env.ingest(&server, &slug, "Doc v3", "# Version 3", "public")
        .await;

    let versions = env.document_version_repo.list_by_slug(&slug).await.unwrap();
    assert_eq!(versions.len(), 2, "Two versions after two updates");
    // Versions should be ordered descending
    assert_eq!(versions[0].version, 2);
    assert_eq!(versions[1].version, 1);
}

#[tokio::test]
async fn ingest_unchanged_content_does_not_create_version() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("no-version-{}", uuid::Uuid::new_v4());

    // Ingest twice with same content
    env.ingest(&server, &slug, "Doc", "# Same", "public").await;
    env.ingest(&server, &slug, "Doc", "# Same", "public").await;

    let versions = env.document_version_repo.list_by_slug(&slug).await.unwrap();
    assert!(versions.is_empty(), "No versions for unchanged content");
}

#[tokio::test]
async fn version_old_content_copied_to_s3_history() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("history-s3-{}", uuid::Uuid::new_v4());

    // First ingest
    env.ingest(&server, &slug, "Doc", "# Original content", "public")
        .await;

    // Second ingest triggers version
    env.ingest(&server, &slug, "Doc", "# New content", "public")
        .await;

    // Check the version record has an S3 key in history path
    let versions = env.document_version_repo.list_by_slug(&slug).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert!(
        versions[0].s3_key.contains("docs/history/"),
        "Version S3 key should be in history path, got: {}",
        versions[0].s3_key
    );

    // Verify old content is actually stored at the history path
    let old_content = env.storage.get_object(&versions[0].s3_key).await.unwrap();
    assert!(
        old_content.is_some(),
        "Old content should exist in S3 history"
    );
    assert_eq!(
        String::from_utf8(old_content.unwrap()).unwrap(),
        "# Original content"
    );
}

#[tokio::test]
async fn version_next_version_number() {
    let env = common::TestEnv::start().await;

    // No versions: should return 1
    let num = env
        .document_version_repo
        .next_version_number("nonexistent-slug")
        .await
        .unwrap();
    assert_eq!(num, 1);
}

// ── find_by_slug_prefix and set_archived ─────────────────────────────────────

#[tokio::test]
async fn find_by_slug_prefix_returns_matching_docs() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let prefix = format!("prefix-{}", uuid::Uuid::new_v4());
    let slug1 = format!("{prefix}/doc1");
    let slug2 = format!("{prefix}/doc2");
    let slug_other = format!("other-{}/doc", uuid::Uuid::new_v4());

    env.ingest(&server, &slug1, "Doc 1", "# One", "public")
        .await;
    env.ingest(&server, &slug2, "Doc 2", "# Two", "public")
        .await;
    env.ingest(&server, &slug_other, "Other", "# Other", "public")
        .await;

    let results = env.repo.find_by_slug_prefix(&prefix).await.unwrap();
    let slugs: Vec<&str> = results.iter().map(|d| d.slug.as_str()).collect();

    assert!(slugs.contains(&slug1.as_str()));
    assert!(slugs.contains(&slug2.as_str()));
    assert!(!slugs.contains(&slug_other.as_str()));
}

#[tokio::test]
async fn find_by_slug_prefix_excludes_archived() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let prefix = format!("archived-prefix-{}", uuid::Uuid::new_v4());
    let slug1 = format!("{prefix}/live");
    let slug2 = format!("{prefix}/archived");

    env.ingest(&server, &slug1, "Live", "# Live", "public")
        .await;
    env.ingest(&server, &slug2, "Archived", "# Archived", "public")
        .await;

    env.repo.set_archived(&slug2, true).await.unwrap();

    let results = env.repo.find_by_slug_prefix(&prefix).await.unwrap();
    let slugs: Vec<&str> = results.iter().map(|d| d.slug.as_str()).collect();

    assert!(slugs.contains(&slug1.as_str()));
    assert!(
        !slugs.contains(&slug2.as_str()),
        "Archived doc should be excluded"
    );
}

#[tokio::test]
async fn set_archived_toggles_flag() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("archive-toggle-{}", uuid::Uuid::new_v4());
    env.ingest(&server, &slug, "Doc", "# Doc", "public").await;

    // Archive
    env.repo.set_archived(&slug, true).await.unwrap();
    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    assert!(doc.is_archived);

    // Unarchive
    env.repo.set_archived(&slug, false).await.unwrap();
    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    assert!(!doc.is_archived);
}
