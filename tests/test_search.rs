mod common;

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
        .add_query_param("access_levels", "public")
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
    let arch_slug = format!("search-rbac-arch-{}", uuid::Uuid::new_v4());

    // Ingest a public doc and an architect-only doc with a shared keyword
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
        &arch_slug,
        &format!("Architect {keyword}"),
        &format!("# Architect {keyword}\n\nRestricted architect content."),
        "architect",
    )
    .await;

    env.wait_for_search_indexing().await;

    // Search as public user — should only see public doc
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_query_param("access_levels", "public")
        .await;

    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results.iter().any(|r| r["slug"].as_str() == Some(&public_slug)),
        "Public doc should appear in public search"
    );
    assert!(
        !results.iter().any(|r| r["slug"].as_str() == Some(&arch_slug)),
        "Architect doc should NOT appear in public search"
    );

    // Search with all levels — should see both
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_query_param("access_levels", "public,internal,developer,architect")
        .await;

    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results.iter().any(|r| r["slug"].as_str() == Some(&public_slug)),
        "Public doc should appear in full-access search"
    );
    assert!(
        results.iter().any(|r| r["slug"].as_str() == Some(&arch_slug)),
        "Architect doc should appear in full-access search"
    );
}

#[tokio::test]
async fn search_returns_empty_for_no_match() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "xyznonexistent99999")
        .add_query_param("access_levels", "public,internal,developer,architect")
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
        .add_query_param("access_levels", "public")
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
async fn search_excludes_archived_documents() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("search-archive-{}", uuid::Uuid::new_v4());
    let keyword = format!("archivekw{}", uuid::Uuid::new_v4().simple());

    // Ingest the document so it appears in search
    env.ingest(
        &server,
        &slug,
        &format!("Archive test {keyword}"),
        &format!("# Archive test {keyword}\n\nThis document will be archived."),
        "public",
    )
    .await;

    env.wait_for_search_indexing().await;

    // Confirm it's searchable before archiving
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_query_param("access_levels", "public")
        .await;
    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results.iter().any(|r| r["slug"].as_str() == Some(&slug)),
        "Document should appear in search before archiving"
    );

    // Archive via sync with archive_missing: true and an empty client list
    let sync_response = server
        .post("/api/v1/sync")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "documents": [],
            "archive_missing": true
        }))
        .await;
    sync_response.assert_status_ok();

    env.wait_for_search_indexing().await;

    // Document should no longer appear in search results
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_query_param("access_levels", "public")
        .await;
    let results: Vec<serde_json::Value> = response.json();
    assert!(
        !results.iter().any(|r| r["slug"].as_str() == Some(&slug)),
        "Archived document must not appear in search results"
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
