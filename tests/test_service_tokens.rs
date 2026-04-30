mod common;

use lekton::auth::token_service::TokenService;
use lekton::db::service_token_models::{scopes_overlap, ServiceToken};
use lekton::db::service_token_repository::ServiceTokenRepository;

// ── Repository CRUD ──────────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_find_by_hash() {
    let env = common::TestEnv::start().await;

    let raw = "test-raw-token";
    let hash = TokenService::hash_token(raw);

    let token = ServiceToken {
        id: uuid::Uuid::new_v4().to_string(),
        name: "test-token".to_string(),
        token_hash: hash.clone(),
        allowed_scopes: vec!["docs/*".to_string()],
        token_type: "service".to_string(),
        user_id: None,
        can_write: true,
        created_by: "admin".to_string(),
        created_at: chrono::Utc::now(),
        last_used_at: None,
        is_active: true,
    };

    env.service_token_repo.create(token).await.unwrap();

    let found = env.service_token_repo.find_by_hash(&hash).await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.name, "test-token");
    assert_eq!(found.allowed_scopes, vec!["docs/*"]);
    assert!(found.is_active);
}

#[tokio::test]
async fn find_by_name() {
    let env = common::TestEnv::start().await;

    let raw = env
        .create_service_token("find-by-name-token", vec!["a/*".to_string()], true)
        .await;
    let _ = raw; // we don't need the raw token

    let found = env
        .service_token_repo
        .find_by_name("find-by-name-token")
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "find-by-name-token");

    // Non-existent
    let not_found = env
        .service_token_repo
        .find_by_name("nonexistent")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn create_rejects_duplicate_name() {
    let env = common::TestEnv::start().await;

    env.create_service_token("dup-name", vec!["x/*".to_string()], true)
        .await;

    // Try to create another with the same name
    let token = ServiceToken {
        id: uuid::Uuid::new_v4().to_string(),
        name: "dup-name".to_string(),
        token_hash: TokenService::hash_token("different-raw"),
        allowed_scopes: vec!["y/*".to_string()],
        token_type: "service".to_string(),
        user_id: None,
        can_write: true,
        created_by: "admin".to_string(),
        created_at: chrono::Utc::now(),
        last_used_at: None,
        is_active: true,
    };

    let result = env.service_token_repo.create(token).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_all_returns_active_and_inactive() {
    let env = common::TestEnv::start().await;

    let raw = env
        .create_service_token("list-active", vec!["active/*".to_string()], true)
        .await;
    let _ = raw;

    let raw2 = env
        .create_service_token("list-inactive", vec!["inactive/*".to_string()], true)
        .await;
    let _ = raw2;

    // Deactivate second
    let tok = env
        .service_token_repo
        .find_by_name("list-inactive")
        .await
        .unwrap()
        .unwrap();
    env.service_token_repo.deactivate(&tok.id).await.unwrap();

    let all = env.service_token_repo.list_all().await.unwrap();
    let names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"list-active"));
    assert!(names.contains(&"list-inactive"));

    let inactive = all.iter().find(|t| t.name == "list-inactive").unwrap();
    assert!(!inactive.is_active);
}

#[tokio::test]
async fn deactivate_sets_inactive() {
    let env = common::TestEnv::start().await;

    let _raw = env
        .create_service_token("deact-tok", vec!["deact/*".to_string()], true)
        .await;

    let tok = env
        .service_token_repo
        .find_by_name("deact-tok")
        .await
        .unwrap()
        .unwrap();
    assert!(tok.is_active);

    env.service_token_repo.deactivate(&tok.id).await.unwrap();

    let tok = env
        .service_token_repo
        .find_by_name("deact-tok")
        .await
        .unwrap()
        .unwrap();
    assert!(!tok.is_active);
}

#[tokio::test]
async fn deactivate_nonexistent_returns_error() {
    let env = common::TestEnv::start().await;

    let result = env.service_token_repo.deactivate("nonexistent-id").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn touch_last_used() {
    let env = common::TestEnv::start().await;

    let _raw = env
        .create_service_token("touch-tok", vec!["touch/*".to_string()], true)
        .await;

    let tok = env
        .service_token_repo
        .find_by_name("touch-tok")
        .await
        .unwrap()
        .unwrap();
    assert!(tok.last_used_at.is_none());

    env.service_token_repo
        .touch_last_used(&tok.id)
        .await
        .unwrap();

    let tok = env
        .service_token_repo
        .find_by_name("touch-tok")
        .await
        .unwrap()
        .unwrap();
    assert!(tok.last_used_at.is_some());
}

#[tokio::test]
async fn check_scope_overlap_detects_conflict() {
    let env = common::TestEnv::start().await;

    let _raw = env
        .create_service_token("overlap-tok", vec!["protocols/*".to_string()], true)
        .await;

    // Same prefix should overlap
    let has_overlap = env
        .service_token_repo
        .check_scope_overlap(&["protocols/iot/*".to_string()], None)
        .await
        .unwrap();
    assert!(has_overlap);

    // Disjoint should not
    let no_overlap = env
        .service_token_repo
        .check_scope_overlap(&["guides/*".to_string()], None)
        .await
        .unwrap();
    assert!(!no_overlap);
}

#[tokio::test]
async fn check_scope_overlap_excludes_self() {
    let env = common::TestEnv::start().await;

    let _raw = env
        .create_service_token("self-overlap", vec!["self/*".to_string()], true)
        .await;

    let tok = env
        .service_token_repo
        .find_by_name("self-overlap")
        .await
        .unwrap()
        .unwrap();

    // Should not overlap when excluding self
    let has_overlap = env
        .service_token_repo
        .check_scope_overlap(&["self/*".to_string()], Some(&tok.id))
        .await
        .unwrap();
    assert!(!has_overlap);
}

// ── Ingest with scoped tokens ────────────────────────────────────────────────

#[tokio::test]
async fn ingest_with_scoped_token_succeeds() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let raw = env
        .create_service_token("ingest-scoped", vec!["ci-docs/*".to_string()], true)
        .await;

    let slug = format!("ci-docs/doc-{}", uuid::Uuid::new_v4());
    server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": raw,
            "slug": slug,
            "source_path": format!("docs/{}.md", slug),
            "title": "Scoped Doc",
            "content": "# Scoped",
            "access_level": "public",
            "service_owner": "ci-team",
            "tags": [],
            "order": 0,
            "is_hidden": false
        }))
        .await;

    let doc = env.repo.find_by_slug(&slug).await.unwrap();
    assert!(doc.is_some());
    assert_eq!(doc.unwrap().title, "Scoped Doc");
}

#[tokio::test]
async fn ingest_with_scoped_token_rejects_out_of_scope() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let raw = env
        .create_service_token("ingest-scope-reject", vec!["allowed/*".to_string()], true)
        .await;

    let response = server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": raw,
            "slug": "forbidden/doc",
            "source_path": "docs/forbidden/doc.md",
            "title": "Out of scope",
            "content": "# Nope",
            "access_level": "public",
            "service_owner": "ci-team",
            "tags": [],
            "order": 0,
            "is_hidden": false
        }))
        .await;

    response.assert_status_forbidden();
}

#[tokio::test]
async fn ingest_with_inactive_token_rejected() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let raw = env
        .create_service_token("ingest-inactive", vec!["any/*".to_string()], true)
        .await;

    // Deactivate
    let tok = env
        .service_token_repo
        .find_by_name("ingest-inactive")
        .await
        .unwrap()
        .unwrap();
    env.service_token_repo.deactivate(&tok.id).await.unwrap();

    let response = server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": raw,
            "slug": "any/doc",
            "source_path": "docs/any/doc.md",
            "title": "Test",
            "content": "# Test",
            "access_level": "public",
            "service_owner": "team",
            "tags": [],
            "order": 0,
            "is_hidden": false
        }))
        .await;

    response.assert_status_unauthorized();
}
