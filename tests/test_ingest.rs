mod common;

use lekton::auth::models::AccessLevel;
use lekton::db::repository::DocumentRepository;
use lekton::storage::client::StorageClient;

#[tokio::test]
async fn ingest_creates_document() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("ingest-create-{}", uuid::Uuid::new_v4());

    env.ingest(&server, &slug, "Test Document", "# Hello\nWorld", "developer")
        .await;

    // Verify document exists in MongoDB
    let doc = env.repo.find_by_slug(&slug).await.unwrap();
    assert!(doc.is_some(), "Document should exist in MongoDB");
    let doc = doc.unwrap();
    assert_eq!(doc.title, "Test Document");
    assert_eq!(doc.access_level, AccessLevel::Developer);
    assert_eq!(doc.service_owner, "test-team");

    // Verify content is stored in S3
    let content = env.storage.get_object(&doc.s3_key).await.unwrap();
    assert!(content.is_some(), "Content should exist in S3");
    assert_eq!(
        String::from_utf8(content.unwrap()).unwrap(),
        "# Hello\nWorld"
    );
}

#[tokio::test]
async fn ingest_rejects_invalid_token() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "wrong-token",
            "slug": "test-doc",
            "title": "Test",
            "content": "content",
            "access_level": "public",
            "service_owner": "team",
            "tags": [],
            "order": 0,
            "is_hidden": false
        }))
        .await;

    response.assert_status_unauthorized();
}

#[tokio::test]
async fn ingest_rejects_empty_slug() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": "",
            "title": "Test",
            "content": "content",
            "access_level": "public",
            "service_owner": "team",
            "tags": [],
            "order": 0,
            "is_hidden": false
        }))
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn ingest_rejects_invalid_access_level() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": "test-doc",
            "title": "Test",
            "content": "content",
            "access_level": "superadmin",
            "service_owner": "team",
            "tags": [],
            "order": 0,
            "is_hidden": false
        }))
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn ingest_upsert_updates_existing() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("ingest-upsert-{}", uuid::Uuid::new_v4());

    // First ingest
    env.ingest(&server, &slug, "Original Title", "# Original", "developer")
        .await;

    // Second ingest (update)
    env.ingest(&server, &slug, "Updated Title", "# Updated", "developer")
        .await;

    // Should have only one document with the updated title
    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    assert_eq!(doc.title, "Updated Title");

    // Content in S3 should be updated
    let content = env.storage.get_object(&doc.s3_key).await.unwrap().unwrap();
    assert_eq!(String::from_utf8(content).unwrap(), "# Updated");
}

#[tokio::test]
async fn ingest_preserves_hierarchy_on_update() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("ingest-hierarchy-{}", uuid::Uuid::new_v4());

    // First ingest with parent_slug
    server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": slug,
            "title": "Child Doc",
            "content": "# Child",
            "access_level": "developer",
            "service_owner": "test-team",
            "tags": ["test"],
            "parent_slug": "parent-doc",
            "order": 5,
            "is_hidden": false
        }))
        .await;

    // Verify initial hierarchy
    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    assert_eq!(doc.parent_slug.as_deref(), Some("parent-doc"));
    assert_eq!(doc.order, 5);

    // Re-ingest without parent_slug (should preserve existing)
    env.ingest(&server, &slug, "Child Doc Updated", "# Updated Child", "developer")
        .await;

    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    assert_eq!(doc.title, "Child Doc Updated");
    assert_eq!(
        doc.parent_slug.as_deref(),
        Some("parent-doc"),
        "parent_slug should be preserved on update"
    );
}

#[tokio::test]
async fn ingest_extracts_and_stores_links() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("ingest-links-{}", uuid::Uuid::new_v4());
    let content = "# Doc with links\n\nSee [setup guide](/docs/setup) and [deploy guide](/docs/deploy).";

    env.ingest(&server, &slug, "Doc with Links", content, "developer")
        .await;

    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    assert!(
        doc.links_out.contains(&"setup".to_string()),
        "links_out should contain 'setup', got: {:?}",
        doc.links_out
    );
    assert!(
        doc.links_out.contains(&"deploy".to_string()),
        "links_out should contain 'deploy', got: {:?}",
        doc.links_out
    );
}

#[tokio::test]
async fn ingest_updates_backlinks() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let target_slug = format!("backlink-target-{}", uuid::Uuid::new_v4());
    let source_slug = format!("backlink-source-{}", uuid::Uuid::new_v4());

    // Create the target document first
    env.ingest(&server, &target_slug, "Target Doc", "# Target", "public")
        .await;

    // Create a source document that links to the target
    let content = format!(
        "# Source\n\nSee [target](/docs/{}) for details.",
        target_slug
    );
    env.ingest(&server, &source_slug, "Source Doc", &content, "public")
        .await;

    // Target document should now have a backlink from source
    let target = env.repo.find_by_slug(&target_slug).await.unwrap().unwrap();
    assert!(
        target.backlinks.contains(&source_slug),
        "Target should have backlink from source. Backlinks: {:?}",
        target.backlinks
    );
}

#[tokio::test]
async fn ingest_indexes_in_meilisearch() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("ingest-search-{}", uuid::Uuid::new_v4());

    env.ingest(
        &server,
        &slug,
        "Unique Searchable Document",
        "# Unique Searchable Document\n\nThis contains unique findable content.",
        "public",
    )
    .await;

    // Wait for Meilisearch async indexing
    env.wait_for_search_indexing().await;

    // Search should find the document
    let results = env
        .search
        .search("Unique Searchable", AccessLevel::Public)
        .await
        .unwrap();

    assert!(
        results.iter().any(|r| r.slug == slug),
        "Document should be findable via Meilisearch. Results: {:?}",
        results
    );
}
