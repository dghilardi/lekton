mod common;

use serde_json::json;

// ── Access Level Management ─────────────────────────────────────────────────

#[tokio::test]
async fn access_level_list_returns_seeded_defaults() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    // Seed default levels
    env.access_level_repo.seed_defaults().await.unwrap();

    // Create admin user and get auth cookie
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    let response = server
        .get("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .await;

    response.assert_status_ok();
    let body: Vec<serde_json::Value> = response.json();
    assert!(body.len() >= 4, "should have at least 4 default levels");

    let names: Vec<&str> = body.iter().filter_map(|l| l["name"].as_str()).collect();
    assert!(names.contains(&"public"));
    assert!(names.contains(&"internal"));
    assert!(names.contains(&"developer"));
    assert!(names.contains(&"architect"));
}

#[tokio::test]
async fn access_level_create_success() {
    let env = common::TestEnv::start().await;
    let server = env.server();
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    let response = server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "custom-level",
            "label": "Custom Level",
            "description": "A custom access level for testing",
            "sort_order": 50
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = response.json();
    assert_eq!(body["name"].as_str(), Some("custom-level"));
    assert_eq!(body["label"].as_str(), Some("Custom Level"));
    assert!(!body["is_system"].as_bool().unwrap_or(true));
}

#[tokio::test]
async fn access_level_create_duplicate_fails() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    let payload = json!({
        "name": "duplicate-level",
        "label": "Duplicate",
        "description": "First creation",
        "sort_order": 10
    });

    server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&payload)
        .expect_success()
        .await;

    let response = server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&payload)
        .await;

    response.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn access_level_create_empty_name_fails() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    let response = server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "   ",
            "label": "Empty",
            "description": "Should fail",
            "sort_order": 10
        }))
        .await;

    response.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn access_level_update_success() {
    let env = common::TestEnv::start().await;
    let server = env.server();
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    // Create first
    server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "updatable",
            "label": "Original",
            "description": "Original description",
            "sort_order": 10
        }))
        .await;

    // Update
    let response = server
        .put("/api/v1/admin/access-levels/updatable")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "label": "Updated Label",
            "description": "Updated description",
            "sort_order": 20
        }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["label"].as_str(), Some("Updated Label"));
}

#[tokio::test]
async fn access_level_update_not_found() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    let response = server
        .put("/api/v1/admin/access-levels/nonexistent")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "label": "Does not matter",
            "description": "N/A",
            "sort_order": 10
        }))
        .await;

    response.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn access_level_delete_success() {
    let env = common::TestEnv::start().await;
    let server = env.server();
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    // Create a custom level
    server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "deletable",
            "label": "Deletable",
            "description": "Will be deleted",
            "sort_order": 99
        }))
        .await;

    // Delete it
    let response = server
        .delete("/api/v1/admin/access-levels/deletable")
        .add_cookie(env.auth_cookie(&admin))
        .await;

    response.assert_status(axum::http::StatusCode::NO_CONTENT);

    // Verify it's gone
    let found = env
        .access_level_repo
        .find_by_name("deletable")
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn access_level_delete_system_level_fails() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    // Seed defaults so "public" (is_system=true) exists
    env.access_level_repo.seed_defaults().await.unwrap();

    let response = server
        .delete("/api/v1/admin/access-levels/public")
        .add_cookie(env.auth_cookie(&admin))
        .await;

    response.assert_status(axum::http::StatusCode::FORBIDDEN);
}

// ── User Management ─────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_list_users() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;
    env.create_test_user("user-1", "user1@test.com", false)
        .await;
    env.create_test_user("user-2", "user2@test.com", false)
        .await;

    let response = server
        .get("/api/v1/admin/users")
        .add_cookie(env.auth_cookie(&admin))
        .await;

    response.assert_status_ok();
    let users: Vec<serde_json::Value> = response.json();
    assert_eq!(users.len(), 3);
}

#[tokio::test]
async fn admin_get_user() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;
    env.create_test_user("user-1", "user1@test.com", false)
        .await;

    let response = server
        .get("/api/v1/admin/users/user-1")
        .add_cookie(env.auth_cookie(&admin))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["id"].as_str(), Some("user-1"));
    assert_eq!(body["email"].as_str(), Some("user1@test.com"));
}

#[tokio::test]
async fn admin_set_user_access_levels() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;
    env.create_test_user("user-1", "user1@test.com", false)
        .await;

    env.access_level_repo.seed_defaults().await.unwrap();

    let response = server
        .put("/api/v1/admin/users/user-1/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "assigned_access_levels": ["internal"],
            "can_write": true,
            "can_read_draft": false,
            "can_write_draft": false
        }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["id"].as_str(), Some("user-1"));
    assert!(body["assigned_access_levels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str() == Some("internal")));
}

// ── Auth Guards ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_endpoints_reject_unauthenticated() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    // All admin endpoints should return 401 without auth cookie
    server
        .get("/api/v1/admin/access-levels")
        .await
        .assert_status(axum::http::StatusCode::UNAUTHORIZED);

    server
        .post("/api/v1/admin/access-levels")
        .json(&json!({"name": "x", "label": "X", "description": "", "sort_order": 0}))
        .await
        .assert_status(axum::http::StatusCode::UNAUTHORIZED);

    server
        .get("/api/v1/admin/users")
        .await
        .assert_status(axum::http::StatusCode::UNAUTHORIZED);

    server
        .get("/api/v1/admin/users/any-id")
        .await
        .assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_endpoints_reject_non_admin() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let regular = env.create_test_user("user-1", "user@test.com", false).await;

    server
        .get("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&regular))
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);

    server
        .post("/api/v1/admin/access-levels")
        .add_cookie(env.auth_cookie(&regular))
        .json(&json!({"name": "x", "label": "X", "description": "", "sort_order": 0}))
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);

    server
        .get("/api/v1/admin/users")
        .add_cookie(env.auth_cookie(&regular))
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_set_access_levels_nonexistent_user() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;

    let response = server
        .put("/api/v1/admin/users/nonexistent-user/access-levels")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "assigned_access_levels": [],
            "can_write": false,
            "can_read_draft": false,
            "can_write_draft": false
        }))
        .await;

    response.assert_status(axum::http::StatusCode::NOT_FOUND);
}

// ── Service Token Management ─────────────────────────────────────────────────

#[tokio::test]
async fn admin_create_service_token_returns_raw_token() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let admin = env
        .create_test_user("st-admin-1", "st-admin@test.com", true)
        .await;

    let response = server
        .post("/api/v1/admin/service-tokens")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "ci-pipeline-token",
            "allowed_scopes": ["docs/*"],
            "can_write": true
        }))
        .await;

    let body: serde_json::Value = response.json();
    assert!(body["raw_token"].is_string());
    assert!(!body["raw_token"].as_str().unwrap().is_empty());
    assert_eq!(body["name"], "ci-pipeline-token");
    assert_eq!(body["allowed_scopes"], json!(["docs/*"]));
}

#[tokio::test]
async fn admin_list_service_tokens_hides_hash() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let admin = env
        .create_test_user("st-admin-2", "st-admin2@test.com", true)
        .await;

    // Create a token first
    server
        .post("/api/v1/admin/service-tokens")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "list-test-token",
            "allowed_scopes": ["list-test/*"],
            "can_write": true
        }))
        .await;

    // List tokens
    let response = server
        .get("/api/v1/admin/service-tokens")
        .add_cookie(env.auth_cookie(&admin))
        .await;

    let body: serde_json::Value = response.json();
    let tokens = body.as_array().unwrap();
    assert!(!tokens.is_empty());

    // No token_hash or raw_token should be present
    let token = &tokens[0];
    assert!(token.get("token_hash").is_none());
    assert!(token.get("raw_token").is_none());
    assert!(token["name"].is_string());
    assert!(token["is_active"].is_boolean());
}

#[tokio::test]
async fn admin_deactivate_service_token() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let admin = env
        .create_test_user("st-admin-3", "st-admin3@test.com", true)
        .await;

    // Create
    let create_resp = server
        .post("/api/v1/admin/service-tokens")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "deact-test-token",
            "allowed_scopes": ["deact/*"],
            "can_write": true
        }))
        .await;

    let create_body: serde_json::Value = create_resp.json();
    let id = create_body["id"].as_str().unwrap();

    // Deactivate
    server
        .delete(&format!("/api/v1/admin/service-tokens/{id}"))
        .add_cookie(env.auth_cookie(&admin))
        .await;

    // Verify via list
    let list_resp = server
        .get("/api/v1/admin/service-tokens")
        .add_cookie(env.auth_cookie(&admin))
        .await;

    let tokens: Vec<serde_json::Value> = list_resp.json();
    let deactivated = tokens.iter().find(|t| t["name"] == "deact-test-token");
    assert!(deactivated.is_some());
    assert_eq!(deactivated.unwrap()["is_active"], false);
}

#[tokio::test]
async fn admin_create_token_rejects_overlapping_scopes() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let admin = env
        .create_test_user("st-admin-4", "st-admin4@test.com", true)
        .await;

    // Create first token
    server
        .post("/api/v1/admin/service-tokens")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "first-overlap",
            "allowed_scopes": ["overlap/*"],
            "can_write": true
        }))
        .await;

    // Try to create second with overlapping scope
    let response = server
        .post("/api/v1/admin/service-tokens")
        .add_cookie(env.auth_cookie(&admin))
        .json(&json!({
            "name": "second-overlap",
            "allowed_scopes": ["overlap/sub/*"],
            "can_write": true
        }))
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn admin_non_admin_cannot_manage_tokens() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let user = env
        .create_test_user("st-nonadmin", "nonadmin@test.com", false)
        .await;

    let response = server
        .get("/api/v1/admin/service-tokens")
        .add_cookie(env.auth_cookie(&user))
        .await;

    response.assert_status_forbidden();
}
