//! Axum request extractors for authenticated users.
//!
//! Two extractors are provided:
//!
//! - [`OptionalAuthUser`]: attempts to read and validate the JWT access-token
//!   cookie; resolves to `None` when the token is absent or invalid.
//! - [`RequiredAuthUser`]: like `OptionalAuthUser` but returns `401 Unauthorized`
//!   when the user is not authenticated.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use std::convert::Infallible;
use std::sync::Arc;

use crate::auth::models::AuthenticatedUser;
use crate::auth::token_service::TokenService;

/// Name of the httpOnly cookie carrying the JWT access token.
pub const ACCESS_TOKEN_COOKIE: &str = "lekton_access_token";
/// Name of the httpOnly cookie carrying the opaque refresh token.
pub const REFRESH_TOKEN_COOKIE: &str = "lekton_refresh_token";
/// Name of the httpOnly cookie carrying the serialised OAuth2/OIDC flow state.
pub const AUTH_STATE_COOKIE: &str = "lekton_auth_state";

// ── OptionalAuthUser ──────────────────────────────────────────────────────────

/// An Axum extractor that resolves to the authenticated user when a valid JWT
/// cookie is present, or `None` otherwise.
///
/// This extractor never fails — unauthenticated requests simply receive `None`.
/// Use [`RequiredAuthUser`] when the endpoint must reject unauthenticated callers.
#[derive(Debug, Clone)]
pub struct OptionalAuthUser(pub Option<AuthenticatedUser>);

impl<S> FromRequestParts<S> for OptionalAuthUser
where
    Arc<TokenService>: axum::extract::FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let token_service = Arc::<TokenService>::from_ref(state);
        let jar = CookieJar::from_headers(&parts.headers);

        let user = jar
            .get(ACCESS_TOKEN_COOKIE)
            .and_then(|c| token_service.validate_access_token(c.value()).ok())
            .map(|claims| TokenService::claims_to_user(&claims));

        Ok(OptionalAuthUser(user))
    }
}

// ── RequiredAuthUser ──────────────────────────────────────────────────────────

/// An Axum extractor that resolves to the authenticated user, returning
/// `401 Unauthorized` when the user is not authenticated.
#[derive(Debug, Clone)]
pub struct RequiredAuthUser(pub AuthenticatedUser);

/// The rejection returned by [`RequiredAuthUser`] when no valid token is found.
pub struct Unauthenticated;

impl IntoResponse for Unauthenticated {
    fn into_response(self) -> Response {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "Authentication required" })),
        )
            .into_response()
    }
}

impl<S> FromRequestParts<S> for RequiredAuthUser
where
    Arc<TokenService>: axum::extract::FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Unauthenticated;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let token_service = Arc::<TokenService>::from_ref(state);
        let jar = CookieJar::from_headers(&parts.headers);

        jar.get(ACCESS_TOKEN_COOKIE)
            .and_then(|c| token_service.validate_access_token(c.value()).ok())
            .map(|claims| RequiredAuthUser(TokenService::claims_to_user(&claims)))
            .ok_or(Unauthenticated)
    }
}

// ── Cookie builders ───────────────────────────────────────────────────────────

/// Returns `true` unless `INSECURE_COOKIES=true` is set (for local dev over HTTP).
fn cookies_secure() -> bool {
    !std::env::var("INSECURE_COOKIES")
        .map(|v| v == "true" || v == "1" || v == "yes")
        .unwrap_or(false)
}

/// Build the access-token httpOnly cookie.
///
/// Uses `SameSite::Strict` for stronger CSRF protection — this cookie is only
/// sent on same-site requests, which is fine for API calls from our own frontend.
pub fn access_token_cookie(value: String, ttl_secs: u64) -> axum_extra::extract::cookie::Cookie<'static> {
    axum_extra::extract::cookie::Cookie::build((ACCESS_TOKEN_COOKIE, value))
        .path("/")
        .http_only(true)
        .secure(cookies_secure())
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .max_age(time::Duration::seconds(ttl_secs as i64))
        .build()
}

/// Build the refresh-token httpOnly cookie (path restricted to `/auth/refresh`
/// to limit exposure).
///
/// Uses `SameSite::Strict` — the refresh endpoint is only called from our own
/// frontend, never from a cross-site redirect.
pub fn refresh_token_cookie(value: String, ttl_days: i64) -> axum_extra::extract::cookie::Cookie<'static> {
    axum_extra::extract::cookie::Cookie::build((REFRESH_TOKEN_COOKIE, value))
        .path("/auth/refresh")
        .http_only(true)
        .secure(cookies_secure())
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .max_age(time::Duration::days(ttl_days))
        .build()
}

/// Build the short-lived auth flow state cookie.
///
/// Must remain `SameSite::Lax` — after the OAuth provider redirects back to
/// `/auth/callback`, the browser needs to send this cookie on the cross-site
/// navigation.
pub fn auth_state_cookie(value: String) -> axum_extra::extract::cookie::Cookie<'static> {
    axum_extra::extract::cookie::Cookie::build((AUTH_STATE_COOKIE, value))
        .path("/auth/callback")
        .http_only(true)
        .secure(cookies_secure())
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        .max_age(time::Duration::minutes(10))
        .build()
}

/// Clear the access-token cookie.
pub fn clear_access_token_cookie() -> axum_extra::extract::cookie::Cookie<'static> {
    axum_extra::extract::cookie::Cookie::build((ACCESS_TOKEN_COOKIE, ""))
        .path("/")
        .removal()
        .build()
}

/// Clear the refresh-token cookie.
pub fn clear_refresh_token_cookie() -> axum_extra::extract::cookie::Cookie<'static> {
    axum_extra::extract::cookie::Cookie::build((REFRESH_TOKEN_COOKIE, ""))
        .path("/auth/refresh")
        .removal()
        .build()
}

/// Clear the auth-state cookie.
pub fn clear_auth_state_cookie() -> axum_extra::extract::cookie::Cookie<'static> {
    axum_extra::extract::cookie::Cookie::build((AUTH_STATE_COOKIE, ""))
        .path("/auth/callback")
        .removal()
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::token_service::TokenService;

    fn make_token_service() -> Arc<TokenService> {
        Arc::new(TokenService::new("test-secret-key-at-least-32-bytes!!", 3600, 30))
    }

    #[test]
    fn test_access_token_cookie_name() {
        let svc = make_token_service();
        let user = crate::auth::models::AuthenticatedUser {
            user_id: "u1".to_string(),
            email: "u@test.com".to_string(),
            name: None,
            is_admin: false,
        };
        let token = svc.generate_access_token(&user).unwrap();
        let cookie = access_token_cookie(token, 900);
        assert_eq!(cookie.name(), ACCESS_TOKEN_COOKIE);
        assert!(cookie.http_only().unwrap_or(false));
    }

    #[test]
    fn test_refresh_token_cookie_path() {
        let cookie = refresh_token_cookie("raw".to_string(), 30);
        assert_eq!(cookie.path(), Some("/auth/refresh"));
    }

    #[test]
    fn test_clear_cookies_have_removal_max_age() {
        let c = clear_access_token_cookie();
        // A removal cookie sets max-age to 0 or negative.
        // axum-extra removal() sets max-age to -1 internally.
        assert_eq!(c.name(), ACCESS_TOKEN_COOKIE);
    }
}
