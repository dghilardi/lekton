//! PAT (Personal Access Token) authentication middleware for the MCP endpoint.
//!
//! Validates `Authorization: Bearer <token>` headers against the service-token
//! repository. Only tokens with `token_type = "pat"` and `is_active = true` are
//! accepted. On success, the middleware resolves the linked user's permissions
//! and injects a [`UserContext`] into the request extensions so that downstream
//! handlers can enforce access-level filtering.
//!
//! ## Admin PAT (no user_id)
//!
//! If the PAT has `user_id = None`, it is treated as an admin token with full
//! access to all documents. This is useful for machine-to-machine integrations
//! (e.g. demo mode, CI pipelines) where tying the token to a specific user
//! account is not practical.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::auth::models::{AuthenticatedUser, UserContext};
use crate::auth::token_service::TokenService;
use crate::db::service_token_repository::ServiceTokenRepository;
use crate::db::user_repository::UserRepository;

/// Shared state needed by the PAT auth middleware.
#[derive(Clone)]
pub struct McpAuthState {
    pub service_token_repo: Arc<dyn ServiceTokenRepository>,
    pub user_repo: Arc<dyn UserRepository>,
}

/// Axum middleware that validates a PAT bearer token and injects [`UserContext`].
pub async fn pat_auth_middleware(
    State(auth): State<McpAuthState>,
    headers: HeaderMap,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let raw_token = extract_bearer(&headers).ok_or(StatusCode::UNAUTHORIZED)?;

    let token_hash = TokenService::hash_token(raw_token);

    let token = auth
        .service_token_repo
        .find_by_hash(&token_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !token.is_active || !token.is_pat() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Update last_used_at (fire-and-forget — don't block the request)
    let repo = auth.service_token_repo.clone();
    let token_id = token.id.clone();
    tokio::spawn(async move {
        let _ = repo.touch_last_used(&token_id).await;
    });

    let user_ctx = match token.user_id.as_deref() {
        // PAT linked to a real user — resolve permissions from DB
        Some(user_id) => {
            let user = auth
                .user_repo
                .find_user_by_id(user_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .ok_or(StatusCode::UNAUTHORIZED)?;

            let permissions = auth
                .user_repo
                .get_permissions(user_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            UserContext {
                user: AuthenticatedUser {
                    user_id: user.id,
                    email: user.email,
                    name: user.name,
                    is_admin: user.is_admin,
                },
                permissions,
            }
        }
        // Admin PAT — no user_id, full access to all documents
        None => UserContext {
            user: AuthenticatedUser {
                user_id: token.id.clone(),
                email: format!("pat:{}@lekton", token.name),
                name: Some(token.name.clone()),
                is_admin: true,
            },
            permissions: vec![],
        },
    };

    request.extensions_mut().insert(user_ctx);
    Ok(next.run(request).await)
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}
