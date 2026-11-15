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
    let architect = env
        .create_test_user("architect-1", "architect@test.com", false)
        .await;
    let admin = env
        .create_test_user("search-admin", "admin@test.com", true)
        .await;

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

    server
        .put("/api/v1/admin/users/architect-1/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&serde_json::json!({
            "assigned_access_levels": ["architect"],
            "can_write": false,
            "can_read_draft": false,
            "can_write_draft": false
        }))
        .await
        .assert_status_ok();

    env.wait_for_search_indexing().await;

    // Search as anonymous user — should only see the public doc.
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .await;

    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results
            .iter()
            .any(|r| r["slug"].as_str() == Some(&public_slug)),
        "Public doc should appear in public search"
    );
    assert!(
        !results
            .iter()
            .any(|r| r["slug"].as_str() == Some(&arch_slug)),
        "Architect doc should NOT appear in public search"
    );

    // Search as an authenticated architect — should see both.
    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_cookie(env.auth_cookie(&architect))
        .await;

    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results
            .iter()
            .any(|r| r["slug"].as_str() == Some(&public_slug)),
        "Public doc should appear in full-access search"
    );
    assert!(
        results
            .iter()
            .any(|r| r["slug"].as_str() == Some(&arch_slug)),
        "Architect doc should appear in full-access search"
    );
}

#[tokio::test]
async fn search_ignores_client_supplied_access_levels() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("search-hidden-{}", uuid::Uuid::new_v4());
    let keyword = format!("hiddenkw{}", uuid::Uuid::new_v4().simple());

    env.ingest(
        &server,
        &slug,
        &format!("Hidden {keyword}"),
        &format!("# Hidden {keyword}\n\nRestricted content."),
        "architect",
    )
    .await;

    env.wait_for_search_indexing().await;

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .add_query_param("access_levels", "architect")
        .await;

    response.assert_status_ok();
    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results.is_empty(),
        "Anonymous callers must not escalate visibility via query params"
    );
}

#[tokio::test]
async fn search_returns_empty_for_no_match() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", "xyznonexistent99999")
        .await;

    response.assert_status_ok();
    let results: Vec<serde_json::Value> = response.json();
    assert!(
        results.is_empty(),
        "No results expected for gibberish query"
    );
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
        &format!(
            "# Preview {keyword}\n\nThis document has meaningful content for preview extraction."
        ),
        "public",
    )
    .await;

    env.wait_for_search_indexing().await;

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .await;

    let results: Vec<serde_json::Value> = response.json();
    let hit = results
        .iter()
        .find(|r| r["slug"].as_str() == Some(&slug))
        .expect("Document should be found");

    let preview = hit["content_preview"].as_str().unwrap();
    assert!(!preview.is_empty(), "Content preview should not be empty");
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
            "source_id": "test-source",
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

/// A body far past the 200-character preview, with an identifier in prose and
/// another in a code block.
fn long_body(title: &str, keyword: &str, code_identifier: &str) -> String {
    format!(
        "# {title}\n\n{}\n\nWhen the device reports `{keyword}`, reboot it.\n\n{}\n\n\
         ```rust\nfn {code_identifier}() {{}}\n```\n\nbodytailmarker {}",
        "Opening filler sentence about devices. ".repeat(10),
        "Middle filler sentence about firmware. ".repeat(10),
        "Closing filler sentence. ".repeat(40),
    )
}

#[tokio::test]
async fn search_finds_body_only_terms_with_a_bounded_snippet_and_no_body() {
    let env = common::TestEnv::start().await;
    let server = env.server();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let keyword = format!("bodyonlykw{id}");
    let code_identifier = format!("reset_device_{id}");

    let public_slug = format!("search-body-public-{id}");
    let restricted_slug = format!("search-body-architect-{id}");
    let draft_slug = format!("search-body-draft-{id}");
    let body = long_body("Device guide", &keyword, &code_identifier);
    assert!(body.find(&keyword).unwrap() > 300);
    env.ingest(&server, &public_slug, "Device guide", &body, "public")
        .await
        .assert_status_ok();
    env.ingest(
        &server,
        &restricted_slug,
        "Device guide",
        &body,
        "architect",
    )
    .await
    .assert_status_ok();
    server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": draft_slug,
            "source_path": format!("docs/{draft_slug}.md"),
            "source_id": "test-source",
            "title": "Device guide",
            "content": body,
            "access_level": "public",
            "is_draft": true,
            "service_owner": "test-team",
            "tags": ["test"],
        }))
        .await
        .assert_status_ok();
    env.wait_for_search_indexing().await;

    let response = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .await;
    response.assert_status_ok();
    let raw = response.text();
    let results: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
    let slugs: Vec<_> = results.iter().filter_map(|r| r["slug"].as_str()).collect();
    assert_eq!(slugs, [public_slug.as_str()], "only the visible document");

    // The response carries a snippet around the body match, never the body.
    let hit = &results[0];
    assert!(hit.get("content").is_none(), "{hit}");
    assert!(!raw.contains("bodytailmarker") && !raw.contains("Closing filler"));
    let preview = hit["content_preview"].as_str().unwrap();
    assert!(preview.contains(&keyword), "{preview}");
    assert!(preview.split_whitespace().count() <= 30, "{preview}");
    let [start, end] =
        serde_json::from_value::<Vec<[usize; 2]>>(hit["preview_matches"].clone()).unwrap()[0];
    let matched: String = preview.chars().skip(start).take(end - start).collect();
    assert_eq!(matched, keyword);

    // Identifiers inside code blocks are searchable too.
    let results: Vec<serde_json::Value> = server
        .get("/api/v1/search")
        .add_query_param("q", &code_identifier)
        .await
        .json();
    assert!(results
        .iter()
        .any(|r| r["slug"].as_str() == Some(public_slug.as_str())));

    // Hybrid discovery: a natural-language question, the reader's visibility,
    // slugs only.
    let levels = vec!["public".to_string()];
    let question = format!("what should I do when the device reports {keyword}");
    let hits = env
        .search
        .discover(&question, Some(&levels), false)
        .await
        .unwrap();
    assert!(hits.iter().any(|h| h.slug == public_slug), "{hits:?}");
    assert!(hits
        .iter()
        .all(|h| h.slug != restricted_slug && h.slug != draft_slug));
    assert!(hits
        .iter()
        .all(|h| h.title.is_empty() && h.content_preview.is_empty()));
    let hits = env.search.search(&keyword, None, true).await.unwrap();
    assert_eq!(hits.len(), 3, "admins see every copy, one hit per document");
}

#[tokio::test]
async fn search_ranks_metadata_matches_above_body_only_matches() {
    let env = common::TestEnv::start().await;
    let server = env.server();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let keyword = format!("rankkw{id}");
    let body_slug = format!("search-rank-body-{id}");
    let title_slug = format!("search-rank-title-{id}");

    // Ingest the body-only match first so ties cannot explain the order.
    env.ingest(
        &server,
        &body_slug,
        "Firmware notes",
        &long_body("Firmware notes", &keyword, "unused_identifier"),
        "public",
    )
    .await
    .assert_status_ok();
    env.ingest(
        &server,
        &title_slug,
        &format!("Reference {keyword}"),
        "# Reference\n\nShort page.",
        "public",
    )
    .await
    .assert_status_ok();
    env.wait_for_search_indexing().await;

    let results: Vec<serde_json::Value> = server
        .get("/api/v1/search")
        .add_query_param("q", &keyword)
        .await
        .json();
    let slugs: Vec<_> = results.iter().filter_map(|r| r["slug"].as_str()).collect();
    assert_eq!(slugs, [title_slug.as_str(), body_slug.as_str()]);
}

#[tokio::test]
async fn search_keeps_entries_indexed_before_full_body_search() {
    let env = common::TestEnv::start().await;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let slug = format!("search-legacy-{id}");
    // A pre-upgrade entry has no body; it stays findable by its old fields.
    let legacy = lekton::search::client::SearchDocument {
        id: lekton::search::client::slug_to_id(&slug),
        slug: slug.clone(),
        title: format!("Legacy legacykw{id}"),
        access_level: "public".into(),
        is_draft: false,
        service_owner: "team".into(),
        tags: vec![],
        content_preview: "Legacy preview".into(),
        content: String::new(),
        last_updated: 0,
        source_id: None,
        release: None,
    };
    env.search.index_document(&legacy).await.unwrap();
    env.wait_for_search_indexing().await;

    let hits = env
        .search
        .search(&format!("legacykw{id}"), None, false)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].slug, slug);
    assert_eq!(hits[0].content_preview, "Legacy preview");
}
