//! OAuth2 / OIDC authentication API endpoints.
//!
//! These routes handle the full login/logout lifecycle:
//!
//! | Method | Path                  | Description                               |
//! |--------|-----------------------|-------------------------------------------|
//! | GET    | `/auth/login`         | Redirect browser to the identity provider |
//! | GET    | `/auth/callback`      | Exchange code, set token cookies          |
//! | POST   | `/auth/refresh`       | Rotate refresh token, issue new JWT       |
//! | POST   | `/auth/logout`        | Revoke refresh token, clear cookies       |
//! | GET    | `/auth/me`            | Return current user from JWT              |

use axum::http::StatusCode;
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::auth::extractor::{
    access_token_cookie, auth_state_cookie, clear_access_token_cookie, clear_auth_state_cookie,
    clear_refresh_token_cookie, refresh_token_cookie, OptionalAuthUser, AUTH_STATE_COOKIE,
    REFRESH_TOKEN_COOKIE,
};
use crate::auth::middleware::build_user_from_claims;
use crate::auth::models::AuthenticatedUser;
use crate::auth::provider::AuthFlowState;
use crate::auth::token_service::TokenService;
use crate::db::auth_models::{RefreshToken, UserPermission};
use crate::error::AppError;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user: AuthenticatedUser,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub user: AuthenticatedUser,
}

// ── Callback query params ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    pub code: String,
    pub state: String,
}

// ── Core logic (pure functions, testable without HTTP) ────────────────────────

/// Register or update a user from provider identity after a successful code exchange.
///
/// - If the user doesn't exist yet, create them and assign the default
///   read-only permission on the "public" access level.
/// - If the user exists, touch their `last_login_at`.
///
/// Returns the [`AuthenticatedUser`] to embed in the JWT.
pub async fn upsert_user_after_login(
    user_repo: &dyn crate::db::user_repository::UserRepository,
    sub: &str,
    email: &str,
    name: Option<String>,
    provider_type: &str,
) -> Result<AuthenticatedUser, AppError> {
    if let Some(existing) = user_repo
        .find_user_by_provider_sub(sub, provider_type)
        .await?
    {
        user_repo.touch_last_login(&existing.id).await?;
        return Ok(AuthenticatedUser {
            user_id: existing.id,
            email: existing.email,
            name: existing.name,
            is_admin: existing.is_admin,
        });
    }

    // New user — create record
    let user_id = uuid::Uuid::new_v4().to_string();
    let new_user = build_user_from_claims(
        user_id.clone(),
        email.to_string(),
        name,
        sub.to_string(),
        provider_type,
    );
    user_repo.create_user(new_user.clone()).await?;

    // Grant default read permission on the "public" level
    user_repo
        .upsert_permission(UserPermission {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.clone(),
            access_level_name: "public".to_string(),
            can_read: true,
            can_write: false,
            can_read_draft: false,
            can_write_draft: false,
        })
        .await?;

    Ok(AuthenticatedUser {
        user_id,
        email: new_user.email,
        name: new_user.name,
        is_admin: false,
    })
}

/// Issue a new access token + refresh token pair and store the refresh token hash.
///
/// Returns `(access_token_raw, refresh_token_raw)` for cookie storage.
pub async fn issue_token_pair(
    user_repo: &dyn crate::db::user_repository::UserRepository,
    token_service: &TokenService,
    user: &AuthenticatedUser,
) -> Result<(String, String), AppError> {
    let access_token = token_service.generate_access_token(user)?;
    let (refresh_raw, refresh_hash) = token_service.generate_refresh_token();

    let expires_at =
        chrono::Utc::now() + chrono::Duration::days(token_service.refresh_token_ttl_days());

    user_repo
        .create_refresh_token(RefreshToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user.user_id.clone(),
            token_hash: refresh_hash,
            expires_at,
            revoked_at: None,
            created_at: chrono::Utc::now(),
        })
        .await?;

    Ok((access_token, refresh_raw))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /auth/login` — Redirect the browser to the identity provider.
///
/// Sets a short-lived `lekton_auth_state` cookie with the CSRF token (and
/// OIDC nonce when applicable) for verification in `/auth/callback`.
pub async fn login_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    jar: CookieJar,
) -> Result<(CookieJar, axum::response::Redirect), AppError> {
    let provider = state
        .auth_provider
        .as_ref()
        .ok_or_else(|| AppError::Auth("Auth provider not configured".into()))?;

    let (url, flow_state) = provider.login_url()?;

    let state_json = serde_json::to_string(&flow_state)
        .map_err(|e| AppError::Internal(format!("Failed to serialize auth state: {e}")))?;

    let jar = jar.add(auth_state_cookie(state_json, !state.insecure_cookies));

    Ok((jar, axum::response::Redirect::temporary(&url)))
}

/// `GET /auth/callback?code=...&state=...` — Exchange the authorization code.
///
/// Verifies the CSRF state, exchanges the code for user identity, upserts the
/// user record, issues JWT + refresh token, and redirects to the portal root.
pub async fn callback_handler(
    axum::extract::State(app_state): axum::extract::State<AppState>,
    axum::extract::Query(params): axum::extract::Query<CallbackParams>,
    jar: CookieJar,
) -> Result<(CookieJar, axum::response::Redirect), AppError> {
    let provider = app_state
        .auth_provider
        .as_ref()
        .ok_or_else(|| AppError::Auth("Auth provider not configured".into()))?;

    // Read + clear the CSRF state cookie
    let state_json = jar
        .get(AUTH_STATE_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| AppError::Auth("Missing auth state cookie".into()))?;

    let flow_state: AuthFlowState = serde_json::from_str(&state_json)
        .map_err(|_| AppError::Auth("Invalid auth state cookie".into()))?;

    let jar = jar.remove(clear_auth_state_cookie());

    // Exchange code for user identity
    let user_info = provider
        .exchange_code(&params.code, &params.state, &flow_state)
        .await?;

    // Register / update user in DB
    let auth_user = upsert_user_after_login(
        app_state.user_repo.as_ref(),
        &user_info.sub,
        &user_info.email,
        user_info.name,
        provider.provider_type(),
    )
    .await?;

    // Issue tokens
    let (access_token, refresh_token) = issue_token_pair(
        app_state.user_repo.as_ref(),
        &app_state.token_service,
        &auth_user,
    )
    .await?;

    let ttl_secs = app_state.token_service.access_token_ttl_secs();
    let ttl_days = app_state.token_service.refresh_token_ttl_days();

    let secure = !app_state.insecure_cookies;
    let jar = jar
        .add(access_token_cookie(access_token, ttl_secs, secure))
        .add(refresh_token_cookie(refresh_token, ttl_days, secure));

    Ok((jar, axum::response::Redirect::temporary("/")))
}

/// `POST /auth/refresh` — Rotate the refresh token and issue a new JWT.
///
/// Reads the `lekton_refresh_token` cookie, verifies it in the database,
/// revokes the old token, and issues a fresh pair.
pub async fn refresh_handler(
    axum::extract::State(app_state): axum::extract::State<AppState>,
    jar: CookieJar,
) -> Result<(CookieJar, axum::Json<RefreshResponse>), AppError> {
    let raw_token = jar
        .get(REFRESH_TOKEN_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| AppError::Auth("No refresh token cookie".into()))?;

    let hash = TokenService::hash_token(&raw_token);

    let stored = app_state
        .user_repo
        .find_refresh_token_by_hash(&hash)
        .await?
        .ok_or_else(|| AppError::Auth("Refresh token not found".into()))?;

    if !stored.is_valid() {
        return Err(AppError::Auth("Refresh token is expired or revoked".into()));
    }

    let user_record = app_state
        .user_repo
        .find_user_by_id(&stored.user_id)
        .await?
        .ok_or_else(|| AppError::Auth("User not found".into()))?;

    let auth_user = AuthenticatedUser {
        user_id: user_record.id.clone(),
        email: user_record.email.clone(),
        name: user_record.name.clone(),
        is_admin: user_record.is_admin,
    };

    // Revoke old token and issue new pair
    app_state.user_repo.revoke_refresh_token(&stored.id).await?;
    let (access_token, new_refresh) = issue_token_pair(
        app_state.user_repo.as_ref(),
        &app_state.token_service,
        &auth_user,
    )
    .await?;

    let ttl_secs = app_state.token_service.access_token_ttl_secs();
    let ttl_days = app_state.token_service.refresh_token_ttl_days();

    let secure = !app_state.insecure_cookies;
    let jar = jar
        .add(access_token_cookie(access_token, ttl_secs, secure))
        .add(refresh_token_cookie(new_refresh, ttl_days, secure));

    Ok((jar, axum::Json(RefreshResponse { user: auth_user })))
}

/// `POST /auth/logout` — Revoke the refresh token and clear all auth cookies.
pub async fn logout_handler(
    axum::extract::State(app_state): axum::extract::State<AppState>,
    jar: CookieJar,
) -> (StatusCode, CookieJar) {
    // Best-effort revocation — ignore errors so logout always succeeds
    if let Some(raw) = jar.get(REFRESH_TOKEN_COOKIE).map(|c| c.value().to_string()) {
        let hash = TokenService::hash_token(&raw);
        if let Ok(Some(stored)) = app_state.user_repo.find_refresh_token_by_hash(&hash).await {
            let _ = app_state.user_repo.revoke_refresh_token(&stored.id).await;
        }
    }

    let jar = jar
        .remove(clear_access_token_cookie())
        .remove(clear_refresh_token_cookie());

    (StatusCode::OK, jar)
}

/// `GET /auth/me` — Return the current user from the JWT cookie.
pub async fn me_handler(
    OptionalAuthUser(user): OptionalAuthUser,
) -> Result<axum::Json<MeResponse>, AppError> {
    let user = user.ok_or_else(|| AppError::Auth("Not authenticated".into()))?;
    Ok(axum::Json(MeResponse { user }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::token_service::TokenService;
    use crate::db::auth_models::User;
    use crate::db::user_repository::UserRepository;
    use crate::test_utils::MockUserRepository as MockRepo;
    use chrono::Utc;

    fn make_svc() -> TokenService {
        TokenService::new("test-secret-key-at-least-32-bytes!!", 3600, 30)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_upsert_new_user_creates_user_and_public_permission() {
        let repo = MockRepo::default();

        let user = upsert_user_after_login(&repo, "sub-1", "a@test.com", None, "oidc")
            .await
            .unwrap();

        assert_eq!(user.email, "a@test.com");
        assert!(!user.is_admin);

        let perms = repo.get_permissions(&user.user_id).await.unwrap();
        assert_eq!(perms.len(), 1);
        assert_eq!(perms[0].access_level_name, "public");
        assert!(perms[0].can_read);
        assert!(!perms[0].can_write);
    }

    #[tokio::test]
    async fn test_upsert_existing_user_touches_last_login() {
        let repo = MockRepo::default();

        // First login
        let user = upsert_user_after_login(&repo, "sub-1", "a@test.com", None, "oidc")
            .await
            .unwrap();
        assert!(repo
            .find_user_by_id(&user.user_id)
            .await
            .unwrap()
            .unwrap()
            .last_login_at
            .is_none());

        // Second login — should touch last_login_at
        let user2 = upsert_user_after_login(&repo, "sub-1", "a@test.com", None, "oidc")
            .await
            .unwrap();
        assert_eq!(user.user_id, user2.user_id, "same user returned");
        assert!(repo
            .find_user_by_id(&user.user_id)
            .await
            .unwrap()
            .unwrap()
            .last_login_at
            .is_some());
    }

    #[tokio::test]
    async fn test_issue_token_pair_stores_refresh_hash() {
        let repo = MockRepo::default();
        let svc = make_svc();
        let user = AuthenticatedUser {
            user_id: "u-1".to_string(),
            email: "u@test.com".to_string(),
            name: None,
            is_admin: false,
        };

        let (access, refresh_raw) = issue_token_pair(&repo, &svc, &user).await.unwrap();

        // Access token should be a valid JWT
        assert!(svc.validate_access_token(&access).is_ok());

        // Refresh token hash should be findable in the repo
        let hash = TokenService::hash_token(&refresh_raw);
        let stored = repo.find_refresh_token_by_hash(&hash).await.unwrap();
        assert!(stored.is_some());
        assert!(stored.unwrap().is_valid());
    }

    #[tokio::test]
    async fn test_expired_refresh_token_rejected() {
        let repo = MockRepo::default();
        let svc = make_svc();
        let user = AuthenticatedUser {
            user_id: "u-1".to_string(),
            email: "u@test.com".to_string(),
            name: None,
            is_admin: false,
        };

        let (_, refresh_raw) = issue_token_pair(&repo, &svc, &user).await.unwrap();
        let hash = TokenService::hash_token(&refresh_raw);

        // Manually expire the stored token
        {
            let mut tokens = repo.tokens.lock().unwrap();
            if let Some(t) = tokens.iter_mut().find(|t| t.token_hash == hash) {
                t.expires_at = Utc::now() - chrono::Duration::seconds(1);
            }
        }

        let stored = repo
            .find_refresh_token_by_hash(&hash)
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.is_valid(), "expired token should be invalid");
    }
}
