use serde::{Deserialize, Serialize};

use crate::auth::models::{AccessLevel, AuthenticatedUser};
use crate::error::AppError;

/// Built-in demo user definition.
#[derive(Debug, Clone)]
struct DemoUser {
    username: &'static str,
    password: &'static str,
    access_level: AccessLevel,
    email: &'static str,
}

/// The hard-coded demo users available when `DEMO_MODE=true`.
const DEMO_USERS: &[DemoUser] = &[
    DemoUser {
        username: "public",
        password: "public",
        access_level: AccessLevel::Public,
        email: "public@demo.lekton.dev",
    },
    DemoUser {
        username: "demo",
        password: "demo",
        access_level: AccessLevel::Developer,
        email: "demo@demo.lekton.dev",
    },
    DemoUser {
        username: "admin",
        password: "admin",
        access_level: AccessLevel::Admin,
        email: "admin@demo.lekton.dev",
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

/// Validate demo credentials and return the corresponding user.
pub fn authenticate_demo_user(username: &str, password: &str) -> Result<AuthenticatedUser, AppError> {
    DEMO_USERS
        .iter()
        .find(|u| u.username == username && u.password == password)
        .map(|u| AuthenticatedUser {
            user_id: format!("demo-{}", u.username),
            email: u.email.to_string(),
            access_level: u.access_level,
        })
        .ok_or_else(|| AppError::Auth("Invalid username or password".into()))
}

/// `POST /api/auth/login` — Demo login handler.
///
/// Validates credentials against the built-in user table.
/// On success, sets a `lekton_demo_user` cookie and returns the user info.
#[cfg(feature = "ssr")]
pub async fn login_handler(
    jar: axum_extra::extract::CookieJar,
    axum::Json(req): axum::Json<LoginRequest>,
) -> Result<(axum_extra::extract::CookieJar, axum::Json<LoginResponse>), AppError> {
    let user = authenticate_demo_user(&req.username, &req.password)?;

    let user_json = serde_json::to_string(&user)
        .map_err(|e| AppError::Internal(format!("Failed to serialize user: {}", e)))?;

    let cookie = axum_extra::extract::cookie::Cookie::build(("lekton_demo_user", user_json))
        .path("/")
        .http_only(true)
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
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
pub async fn logout_handler(
    jar: axum_extra::extract::CookieJar,
) -> axum_extra::extract::CookieJar {
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
        assert_eq!(user.access_level, AccessLevel::Developer);
    }

    #[test]
    fn test_authenticate_admin() {
        let user = authenticate_demo_user("admin", "admin").unwrap();
        assert_eq!(user.user_id, "demo-admin");
        assert_eq!(user.access_level, AccessLevel::Admin);
    }

    #[test]
    fn test_authenticate_public() {
        let user = authenticate_demo_user("public", "public").unwrap();
        assert_eq!(user.access_level, AccessLevel::Public);
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
