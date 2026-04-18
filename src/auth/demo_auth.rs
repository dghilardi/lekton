//! Built-in demo authentication used when `DEMO_MODE=true`.
//!
//! Provides a simple username/password login that issues a session cookie
//! (`lekton_demo_user`) carrying a serialized [`AuthenticatedUser`].
//! This mechanism is intentionally simple and is **not** for production use.

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

/// Validate demo credentials and return the corresponding [`AuthenticatedUser`].
pub fn authenticate_demo_user(
    username: &str,
    password: &str,
) -> Result<AuthenticatedUser, AppError> {
    DEMO_USERS
        .iter()
        .find(|u| u.username == username && u.password == password)
        .map(|u| AuthenticatedUser {
            user_id: format!("demo-{}", u.username),
            email: u.email.to_string(),
            name: Some(u.name.to_string()),
            is_admin: u.is_admin,
        })
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

    let user_json = serde_json::to_string(&user)
        .map_err(|e| AppError::Internal(format!("Failed to serialize user: {}", e)))?;

    let cookie = axum_extra::extract::cookie::Cookie::build(("lekton_demo_user", user_json))
        .path("/")
        .http_only(true)
        .secure(!state.insecure_cookies)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .build();

    let jar = jar.add(cookie);

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

    let user: AuthenticatedUser = serde_json::from_str(cookie.value())
        .map_err(|e| AppError::Auth(format!("Invalid session: {}", e)))?;

    Ok(axum::Json(user))
}

/// `POST /api/auth/logout` — Clears the demo session cookie.
#[cfg(feature = "ssr")]
pub async fn logout_handler(jar: axum_extra::extract::CookieJar) -> axum_extra::extract::CookieJar {
    let cookie = axum_extra::extract::cookie::Cookie::build(("lekton_demo_user", ""))
        .path("/")
        .removal()
        .build();

    jar.remove(cookie)
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
}
