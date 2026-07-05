//! Built-in demo authentication used when `DEMO_MODE=true`.
//!
//! Provides a simple username/password login that issues a session cookie
//! (`lekton_demo_user`) carrying the selected demo account identifier.
//! The cookie never stores role-bearing user JSON so privileges are always
//! resolved server-side from the built-in demo account table.

use serde::{Deserialize, Serialize};

use crate::auth::models::AuthenticatedUser;
use crate::error::AppError;

/// A hard-coded demo user definition.
#[derive(Debug, Clone)]
struct DemoUser {
    username: &'static str,
    password: &'static str,
    is_admin: bool,
    email: &'static str,
    name: &'static str,
}

/// The hard-coded demo users available when `DEMO_MODE=true`.
const DEMO_USERS: &[DemoUser] = &[
    DemoUser {
        username: "public",
        password: "public",
        is_admin: false,
        email: "public@demo.lekton.dev",
        name: "Public User",
    },
    DemoUser {
        username: "demo",
        password: "demo",
        is_admin: false,
        email: "demo@demo.lekton.dev",
        name: "Demo User",
    },
    DemoUser {
        username: "admin",
        password: "admin",
        is_admin: true,
        email: "admin@demo.lekton.dev",
        name: "Demo Admin",
    },
];

/// Login request body.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Login response body.
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub message: String,
    pub user: AuthenticatedUser,
}

fn demo_user_to_authenticated_user(user: &DemoUser) -> AuthenticatedUser {
    AuthenticatedUser {
        user_id: format!("demo-{}", user.username),
        email: user.email.to_string(),
        name: Some(user.name.to_string()),
        is_admin: user.is_admin,
    }
}

fn find_demo_user(username: &str) -> Option<&'static DemoUser> {
    DEMO_USERS.iter().find(|user| user.username == username)
}

/// Resolve a demo session cookie value to the corresponding authenticated user.
///
/// The cookie value is a stable demo username, not a serialized user object.
pub fn resolve_demo_session_user(session_value: &str) -> Option<AuthenticatedUser> {
    find_demo_user(session_value).map(demo_user_to_authenticated_user)
}

/// Validate demo credentials and return the corresponding [`AuthenticatedUser`].
pub fn authenticate_demo_user(
    username: &str,
    password: &str,
) -> Result<AuthenticatedUser, AppError> {
    DEMO_USERS
        .iter()
        .find(|u| u.username == username && u.password == password)
        .map(demo_user_to_authenticated_user)
        .ok_or_else(|| AppError::Auth("Invalid username or password".into()))
}

/// `POST /api/auth/demo/login` — Demo login handler.
///
/// Validates credentials against the built-in user table.
/// On success, sets a `lekton_demo_user` cookie and returns the user info.
#[cfg(feature = "ssr")]
pub async fn login_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    jar: axum_extra::extract::CookieJar,
    axum::Json(req): axum::Json<LoginRequest>,
) -> Result<(axum_extra::extract::CookieJar, axum::Json<LoginResponse>), AppError> {
    let user = authenticate_demo_user(&req.username, &req.password)?;
    let session_value = find_demo_user(&req.username)
        .map(|demo_user| demo_user.username)
        .ok_or_else(|| AppError::Auth("Invalid username or password".into()))?;

    let cookie =
        axum_extra::extract::cookie::Cookie::build(("lekton_demo_user", session_value.to_string()))
            .path("/")
            .http_only(true)
            .secure(!state.insecure_cookies)
            .same_site(axum_extra::extract::cookie::SameSite::Strict)
            .build();

    // Also set the logged-in indicator cookie for consistency with the
    // production OAuth flow.  Use a session cookie (no max-age) to match
    // the demo user cookie lifetime.
    let logged_in =
        axum_extra::extract::cookie::Cookie::build((crate::auth::extractor::LOGGED_IN_COOKIE, "1"))
            .path("/")
            .http_only(false)
            .secure(!state.insecure_cookies)
            .same_site(axum_extra::extract::cookie::SameSite::Strict)
            .build();

    let jar = jar.add(cookie).add(logged_in);

    Ok((
        jar,
        axum::Json(LoginResponse {
            message: "Login successful".to_string(),
            user,
        }),
    ))
}

/// `GET /api/auth/me` — Returns the current demo user from cookie.
#[cfg(feature = "ssr")]
pub async fn me_handler(
    jar: axum_extra::extract::CookieJar,
) -> Result<axum::Json<AuthenticatedUser>, AppError> {
    let cookie = jar
        .get("lekton_demo_user")
        .ok_or_else(|| AppError::Auth("Not logged in".into()))?;

    let user = resolve_demo_session_user(cookie.value())
        .ok_or_else(|| AppError::Auth("Invalid session".into()))?;

    Ok(axum::Json(user))
}

/// `POST /api/auth/logout` — Clears the demo session cookie and logged-in indicator.
#[cfg(feature = "ssr")]
pub async fn logout_handler(jar: axum_extra::extract::CookieJar) -> axum_extra::extract::CookieJar {
    let cookie = axum_extra::extract::cookie::Cookie::build(("lekton_demo_user", ""))
        .path("/")
        .removal()
        .build();

    jar.remove(cookie)
        .remove(crate::auth::extractor::clear_logged_in_cookie())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_authenticate_demo_user_success() {
        let user = authenticate_demo_user("demo", "demo").unwrap();
        assert_eq!(user.user_id, "demo-demo");
        assert_eq!(user.name, Some("Demo User".to_string()));
        assert!(!user.is_admin);
    }

    #[test]
    fn test_authenticate_admin() {
        let user = authenticate_demo_user("admin", "admin").unwrap();
        assert_eq!(user.user_id, "demo-admin");
        assert!(user.is_admin);
    }

    #[test]
    fn test_authenticate_public() {
        let user = authenticate_demo_user("public", "public").unwrap();
        assert_eq!(user.user_id, "demo-public");
        assert!(!user.is_admin);
    }

    #[test]
    fn test_wrong_password() {
        let result = authenticate_demo_user("demo", "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_user() {
        let result = authenticate_demo_user("nobody", "nothing");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_demo_session_user_rejects_forged_json_payload() {
        let forged = serde_json::json!({
            "user_id": "demo-admin",
            "email": "attacker@example.com",
            "name": "Attacker",
            "is_admin": true
        })
        .to_string();

        assert!(resolve_demo_session_user(&forged).is_none());
    }

    #[test]
    fn resolve_demo_session_user_maps_known_username() {
        let user = resolve_demo_session_user("admin").unwrap();

        assert_eq!(user.user_id, "demo-admin");
        assert!(user.is_admin);
    }
}
