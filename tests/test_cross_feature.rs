mod common;

use axum_test::multipart::{MultipartForm, Part};
use serde_json::json;

#[tokio::test]
async fn ingest_with_custom_access_level_then_search() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let admin = env.create_test_user("admin-1", "admin@test.com", true).await;

    // Create a custom access level
    server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "secret",
            "label": "Secret",
            "description": "Top secret docs",
            "sort_order": 100
        }))
        .await;

    // Ingest a doc at the "secret" level
    env.ingest(&server, "secret-doc", "Secret Document", "This is classified content", "secret")
        .await;

    env.wait_for_search_indexing().await;

    // Admin can search and find it (bypasses RBAC)
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "classified")
        .add_query_param("access_levels", "secret")
        .await;

    let hits: Vec<serde_json::Value> = response.json();
    assert!(!hits.is_empty(), "admin should find the secret doc");

    // Search with only "public" access level should NOT find it
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "classified")
        .add_query_param("access_levels", "public")
        .await;

    let hits: Vec<serde_json::Value> = response.json();
    assert!(hits.is_empty(), "public-only search should not find secret doc");
}

#[tokio::test]
async fn admin_creates_level_assigns_permission_user_sees_doc() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let admin = env.create_test_user("admin-1", "admin@test.com", true).await;
    env.create_test_user("user-1", "user@test.com", false).await;

    // 1. Create custom access level
    server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "team-alpha",
            "label": "Team Alpha",
            "description": "Alpha team docs",
            "sort_order": 50
        }))
        .await;

    // 2. Assign read permission to user-1 for team-alpha
    server
        .put("/api/v1/admin/users/user-1/permissions")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "permissions": [{
                "access_level_name": "team-alpha",
                "can_read": true,
                "can_write": false,
                "can_read_draft": false,
                "can_write_draft": false
            }]
        }))
        .await;

    // 3. Ingest a doc at team-alpha level
    env.ingest(&server, "alpha-doc", "Alpha Doc", "Alpha team content", "team-alpha")
        .await;

    env.wait_for_search_indexing().await;

    // 4. Verify the doc is findable with team-alpha access
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "Alpha team")
        .add_query_param("access_levels", "team-alpha")
        .await;

    let hits: Vec<serde_json::Value> = response.json();
    assert!(!hits.is_empty(), "should find doc with correct access level");

    // 5. Verify the permission was persisted
    let perms = env.user_repo.get_permissions("user-1").await.unwrap();
    assert_eq!(perms.len(), 1);
    assert_eq!(perms[0].access_level_name, "team-alpha");
    assert!(perms[0].can_read);
}

#[tokio::test]
async fn settings_custom_css_roundtrip() {
    let env = common::TestEnv::start().await;

    let custom_css = ":root { --primary: #ff0000; }";

    // Save custom CSS
    env.settings_repo
        .set_custom_css(custom_css)
        .await
        .unwrap();

    // Read it back
    let settings = env.settings_repo.get_settings().await.unwrap();
    assert_eq!(settings.custom_css, custom_css);

    // Update it
    let updated_css = "body { background: blue; }";
    env.settings_repo
        .set_custom_css(updated_css)
        .await
        .unwrap();

    let settings = env.settings_repo.get_settings().await.unwrap();
    assert_eq!(settings.custom_css, updated_css);
}

#[tokio::test]
async fn ingest_doc_then_update_content() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    // Ingest initial doc
    env.ingest(&server, "evolving-doc", "First Title", "Initial content here", "public")
        .await;

    // Verify initial content is in S3
    let doc = env.repo.find_by_slug("evolving-doc").await.unwrap().unwrap();
    let content = String::from_utf8(env.storage.get_object(&doc.s3_key).await.unwrap().unwrap()).unwrap();
    assert!(content.contains("Initial content"));

    // Update via re-ingest
    env.ingest(&server, "evolving-doc", "Updated Title", "Updated content now", "public")
        .await;

    env.wait_for_search_indexing().await;

    // Verify updated content
    let doc = env.repo.find_by_slug("evolving-doc").await.unwrap().unwrap();
    assert_eq!(doc.title, "Updated Title");

    let content = String::from_utf8(env.storage.get_object(&doc.s3_key).await.unwrap().unwrap()).unwrap();
    assert!(content.contains("Updated content"));

    // Verify search picks up the update
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "Updated content")
        .add_query_param("access_levels", "public")
        .await;

    let hits: Vec<serde_json::Value> = response.json();
    assert!(!hits.is_empty(), "search should find the updated content");
}

#[tokio::test]
async fn asset_lifecycle_with_document() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    // Upload an asset via the API (multipart form like the existing upload pattern)
    let form = MultipartForm::new()
        .add_text("service_token", "test-token")
        .add_part(
            "file",
            Part::bytes(b"asset-content-here".to_vec())
                .file_name("config.yaml")
                .mime_type("text/yaml"),
        );

    let response = server
        .put("/api/v1/assets/project-x/config.yaml")
        .multipart(form)
        .await;

    response.assert_status_ok();

    // Serve the asset back
    let response = server.get("/api/v1/assets/project-x/config.yaml").await;
    response.assert_status_ok();
    let body = response.text();
    assert_eq!(body, "asset-content-here");

    // List assets
    let response = server.get("/api/v1/assets").await;
    response.assert_status_ok();
    let assets: Vec<serde_json::Value> = response.json();
    assert!(!assets.is_empty());
}

#[tokio::test]
async fn seed_defaults_idempotent() {
    let env = common::TestEnv::start().await;

    // Seed twice
    env.access_level_repo.seed_defaults().await.unwrap();
    env.access_level_repo.seed_defaults().await.unwrap();

    // Should still have exactly 4 default levels, not 8
    let levels = env.access_level_repo.list_all().await.unwrap();
    assert_eq!(levels.len(), 4, "seed_defaults should be idempotent");
}
