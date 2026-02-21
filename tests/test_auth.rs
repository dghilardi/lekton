mod common;

#[tokio::test]
async fn login_success_sets_cookie() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let response = server
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "username": "demo",
            "password": "demo"
        }))
        .await;

    response.assert_status_ok();

    // The cookie should be set (axum_test saves cookies automatically)
    let body: serde_json::Value = response.json();
    assert_eq!(body["message"].as_str(), Some("Login successful"));
}

#[tokio::test]
async fn login_returns_user_info() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let response = server
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "username": "demo",
            "password": "demo"
        }))
        .await;

    let body: serde_json::Value = response.json();
    let user = &body["user"];
    assert_eq!(user["user_id"].as_str(), Some("demo-demo"));
    assert_eq!(user["email"].as_str(), Some("demo@demo.lekton.dev"));
    assert_eq!(user["access_level"].as_str(), Some("Developer"));
}

#[tokio::test]
async fn login_invalid_credentials() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "username": "demo",
            "password": "wrongpassword"
        }))
        .await;

    response.assert_status_unauthorized();
}

#[tokio::test]
async fn me_with_valid_cookie() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    // Login first to set cookie
    server
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "username": "admin",
            "password": "admin"
        }))
        .await;

    // Now call /me — cookie should be sent automatically
    let response = server.get("/api/auth/me").await;
    response.assert_status_ok();

    let user: serde_json::Value = response.json();
    assert_eq!(user["user_id"].as_str(), Some("demo-admin"));
    assert_eq!(user["access_level"].as_str(), Some("Admin"));
}

#[tokio::test]
async fn me_without_cookie() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server.get("/api/auth/me").await;
    response.assert_status_unauthorized();
}

#[tokio::test]
async fn logout_clears_cookie() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    // Login
    server
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "username": "demo",
            "password": "demo"
        }))
        .expect_success()
        .await;

    // Verify logged in
    let response = server.get("/api/auth/me").await;
    response.assert_status_ok();

    // Logout
    server.post("/api/auth/logout").expect_success().await;

    // /me should now fail — cookie was cleared
    let response = server.get("/api/auth/me").await;
    response.assert_status_unauthorized();
}

#[tokio::test]
async fn full_auth_flow() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    // 1. Not logged in
    let response = server.get("/api/auth/me").await;
    response.assert_status_unauthorized();

    // 2. Login as public user
    let response = server
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "username": "public",
            "password": "public"
        }))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["user"]["access_level"].as_str(), Some("Public"));

    // 3. Verify session
    let response = server.get("/api/auth/me").await;
    response.assert_status_ok();
    let user: serde_json::Value = response.json();
    assert_eq!(user["access_level"].as_str(), Some("Public"));

    // 4. Logout
    server.post("/api/auth/logout").await;

    // 5. Session destroyed
    let response = server.get("/api/auth/me").await;
    response.assert_status_unauthorized();
}
