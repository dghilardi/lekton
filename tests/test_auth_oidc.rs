mod common;

use lekton::auth::token_service::TokenService;
use lekton::db::auth_models::RefreshToken;

/// Helper: issue a token pair for a user and return (access_token, refresh_raw, refresh_hash).
async fn issue_tokens(
    env: &common::TestEnv,
    user: &lekton::auth::models::AuthenticatedUser,
) -> (String, String, String) {
    let access_token = env.token_service.generate_access_token(user).unwrap();
    let (refresh_raw, refresh_hash) = env.token_service.generate_refresh_token();

    let token_record = RefreshToken {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user.user_id.clone(),
        token_hash: refresh_hash.clone(),
        expires_at: chrono::Utc::now() + chrono::Duration::days(30),
        revoked_at: None,
        created_at: chrono::Utc::now(),
    };
    env.user_repo
        .create_refresh_token(token_record)
        .await
        .unwrap();

    (access_token, refresh_raw, refresh_hash)
}

#[tokio::test]
async fn refresh_token_rotates_tokens() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let user = env.create_test_user("user-1", "user@test.com", false).await;
    let (_access, refresh_raw, _hash) = issue_tokens(&env, &user).await;

    // The refresh endpoint reads from the lekton_refresh_token cookie at path /auth/refresh
    let refresh_cookie = cookie::Cookie::build(("lekton_refresh_token", refresh_raw))
        .path("/auth/refresh")
        .build();

    let response = server
        .post("/auth/refresh")
        .add_cookie(refresh_cookie)
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["user"]["user_id"].as_str(), Some("user-1"));
    assert_eq!(body["user"]["email"].as_str(), Some("user@test.com"));
}

#[tokio::test]
async fn refresh_token_revokes_old() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let user = env.create_test_user("user-1", "user@test.com", false).await;
    let (_access, refresh_raw, refresh_hash) = issue_tokens(&env, &user).await;

    let refresh_cookie = cookie::Cookie::build(("lekton_refresh_token", refresh_raw))
        .path("/auth/refresh")
        .build();

    server
        .post("/auth/refresh")
        .add_cookie(refresh_cookie)
        .await;

    // The old token should be revoked
    let old_token = env
        .user_repo
        .find_refresh_token_by_hash(&refresh_hash)
        .await
        .unwrap()
        .unwrap();
    assert!(
        old_token.revoked_at.is_some(),
        "old refresh token should be revoked after rotation"
    );
}

#[tokio::test]
async fn refresh_with_revoked_token_fails() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let user = env.create_test_user("user-1", "user@test.com", false).await;
    let (_access, refresh_raw, refresh_hash) = issue_tokens(&env, &user).await;

    // Manually revoke the token
    let stored = env
        .user_repo
        .find_refresh_token_by_hash(&refresh_hash)
        .await
        .unwrap()
        .unwrap();
    env.user_repo
        .revoke_refresh_token(&stored.id)
        .await
        .unwrap();

    let refresh_cookie = cookie::Cookie::build(("lekton_refresh_token", refresh_raw))
        .path("/auth/refresh")
        .build();

    let response = server
        .post("/auth/refresh")
        .add_cookie(refresh_cookie)
        .await;

    response.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_without_cookie_fails() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server.post("/auth/refresh").await;

    response.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_with_valid_jwt() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let user = env.create_test_user("user-1", "user@test.com", false).await;

    let response = server
        .get("/auth/me")
        .add_cookie(env.auth_cookie(&user))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["user"]["user_id"].as_str(), Some("user-1"));
    assert_eq!(body["user"]["email"].as_str(), Some("user@test.com"));
    assert_eq!(body["user"]["is_admin"].as_bool(), Some(false));
}

#[tokio::test]
async fn me_with_expired_jwt_fails() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let user = env.create_test_user("user-1", "user@test.com", false).await;

    // Create a token service with 0-second TTL (already expired)
    let expired_service = TokenService::new("test-secret-key-at-least-32-bytes!!", 0, 30);
    let expired_token = expired_service.generate_access_token(&user).unwrap();

    // Wait a moment to ensure expiry
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let cookie = cookie::Cookie::build(("lekton_access_token", expired_token))
        .path("/")
        .build();

    let response = server.get("/auth/me").add_cookie(cookie).await;

    response.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_without_jwt_fails() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server.get("/auth/me").await;
    response.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_revokes_and_clears() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let user = env.create_test_user("user-1", "user@test.com", false).await;
    let (_access, refresh_raw, refresh_hash) = issue_tokens(&env, &user).await;

    let refresh_cookie = cookie::Cookie::build(("lekton_refresh_token", refresh_raw))
        .path("/auth/refresh")
        .build();

    let response = server
        .post("/auth/logout")
        .add_cookie(env.auth_cookie(&user))
        .add_cookie(refresh_cookie)
        .await;

    response.assert_status_ok();

    // Verify refresh token was revoked
    let stored = env
        .user_repo
        .find_refresh_token_by_hash(&refresh_hash)
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored.revoked_at.is_some(),
        "refresh token should be revoked after logout"
    );
}
