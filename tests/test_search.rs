mod common;

use lekton::auth::models::AccessLevel;

#[tokio::test]
async fn search_returns_matching_documents() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("search-match-{}", uuid::Uuid::new_v4());

    env.ingest(
        &server,
        &slug,
        "Kubernetes Deployment Guide",
        "# Kubernetes Deployment\n\nHow to deploy services to k8s clusters.",
        "public",
    )
    .await;

    env.wait_for_search_indexing().await;

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "Kubernetes Deployment")
        .add_query_param("access_level", "public")
        .await;

    response.assert_status_ok();
    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results.iter().any(|r| r["slug"].as_str() == Some(&slug)),
        "Search should return the ingested document. Results: {:?}",
        results
    );
}

#[tokio::test]
async fn search_respects_access_level_filtering() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let public_slug = format!("search-rbac-public-{}", uuid::Uuid::new_v4());
    let admin_slug = format!("search-rbac-admin-{}", uuid::Uuid::new_v4());

    // Ingest a public doc and an admin-only doc with a shared keyword
    let keyword = format!("rbacfilter{}", uuid::Uuid::new_v4().simple());
    env.ingest(
        &server,
        &public_slug,
        &format!("Public {keyword}"),
        &format!("# Public {keyword}\n\nPublic content."),
        "public",
    )
    .await;

    env.ingest(
        &server,
        &admin_slug,
        &format!("Admin {keyword}"),
        &format!("# Admin {keyword}\n\nSecret admin content."),
        "admin",
    )
    .await;

    env.wait_for_search_indexing().await;

    // Search as public user — should only see public doc
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_query_param("access_level", "public")
        .await;

    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results.iter().any(|r| r["slug"].as_str() == Some(&public_slug)),
        "Public doc should appear in public search"
    );
    assert!(
        !results.iter().any(|r| r["slug"].as_str() == Some(&admin_slug)),
        "Admin doc should NOT appear in public search"
    );

    // Search as admin — should see both
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_query_param("access_level", "admin")
        .await;

    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results.iter().any(|r| r["slug"].as_str() == Some(&public_slug)),
        "Public doc should appear in admin search"
    );
    assert!(
        results.iter().any(|r| r["slug"].as_str() == Some(&admin_slug)),
        "Admin doc should appear in admin search"
    );
}

#[tokio::test]
async fn search_returns_empty_for_no_match() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "xyznonexistent99999")
        .add_query_param("access_level", "admin")
        .await;

    response.assert_status_ok();
    let results: Vec<serde_json::Value> = response.json();
    assert!(results.is_empty(), "No results expected for gibberish query");
}

#[tokio::test]
async fn search_returns_content_preview() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("search-preview-{}", uuid::Uuid::new_v4());
    let keyword = format!("previewkw{}", uuid::Uuid::new_v4().simple());

    env.ingest(
        &server,
        &slug,
        &format!("Preview {keyword}"),
        &format!("# Preview {keyword}\n\nThis document has meaningful content for preview extraction."),
        "public",
    )
    .await;

    env.wait_for_search_indexing().await;

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_query_param("access_level", "public")
        .await;

    let results: Vec<serde_json::Value> = response.json();
    let hit = results
        .iter()
        .find(|r| r["slug"].as_str() == Some(&slug))
        .expect("Document should be found");

    let preview = hit["content_preview"].as_str().unwrap();
    assert!(
        !preview.is_empty(),
        "Content preview should not be empty"
    );
    assert!(
        !preview.contains('#'),
        "Preview should have markdown stripped"
    );
}

#[tokio::test]
async fn search_fails_when_service_unavailable() {
    let env = common::TestEnv::start().await;
    let server = common::server_without_search(&env);

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "anything")
        .await;

    response.assert_status_internal_server_error();
}
