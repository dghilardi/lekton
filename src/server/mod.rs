pub mod access_levels;
pub mod auth_fns;
pub mod custom_css;
pub mod docs;
pub mod document_upload;
pub mod feedback;
pub mod learn;
pub mod nav;
pub mod pats;
pub mod prompts;
pub mod reindex;
pub mod search;
pub mod service_tokens;
pub mod sources;
pub mod users;

#[cfg(feature = "ssr")]
pub(crate) use helpers::{
    request_document_visibility, require_admin_user, require_any_user, require_user_context,
};

#[cfg(feature = "ssr")]
mod helpers {
    use crate::app::AppState;
    use crate::auth::demo_auth::resolve_demo_session_user;
    use leptos::prelude::*;

    pub(crate) async fn request_document_visibility(
        state: &AppState,
    ) -> Result<(Option<Vec<String>>, bool), ServerFnError> {
        use crate::auth::extractor::{ACCESS_TOKEN_COOKIE, LOGGED_IN_COOKIE};
        use crate::auth::models::UserContext;
        use crate::auth::token_service::TokenService;
        use axum_extra::extract::CookieJar;

        let jar: CookieJar = leptos_axum::extract().await?;

        let maybe_user = jar
            .get(ACCESS_TOKEN_COOKIE)
            .and_then(|c| state.token_service.validate_access_token(c.value()).ok())
            .map(|claims| TokenService::claims_to_user(&claims));

        if let Some(auth_user) = maybe_user {
            if auth_user.is_admin {
                return Ok((None, true));
            }
            let user_doc = state
                .user_repo
                .find_user_by_id(&auth_user.user_id)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            let ctx = match user_doc {
                Some(u) => UserContext::from_user_doc(auth_user, &u),
                None => UserContext {
                    user: auth_user,
                    effective_access_levels: vec![],
                    can_write: false,
                    can_read_draft: false,
                    can_write_draft: false,
                },
            };
            return Ok(ctx.document_visibility());
        }

        if state.demo_mode {
            if let Some(cookie) = jar.get("lekton_demo_user") {
                if let Some(demo_user) = resolve_demo_session_user(cookie.value()) {
                    if demo_user.is_admin {
                        return Ok((None, true));
                    } else {
                        return Ok((Some(vec!["public".to_string()]), false));
                    }
                }
            }
        }

        if jar.get(LOGGED_IN_COOKIE).is_some() {
            return Err(ServerFnError::new(
                crate::auth::models::UNAUTHORIZED_SENTINEL,
            ));
        }

        Ok((Some(vec!["public".to_string()]), false))
    }

    pub(crate) async fn require_admin_user(
        state: &AppState,
    ) -> Result<crate::auth::models::AuthenticatedUser, ServerFnError> {
        use crate::auth::extractor::ACCESS_TOKEN_COOKIE;
        use crate::auth::token_service::TokenService;
        use axum_extra::extract::CookieJar;

        let jar: CookieJar = leptos_axum::extract().await?;

        if let Some(user) = jar
            .get(ACCESS_TOKEN_COOKIE)
            .and_then(|c| state.token_service.validate_access_token(c.value()).ok())
            .map(|claims| TokenService::claims_to_user(&claims))
        {
            if user.is_admin {
                return Ok(user);
            }
            return Err(ServerFnError::new("Admin privileges required"));
        }

        if state.demo_mode {
            if let Some(cookie) = jar.get("lekton_demo_user") {
                if let Some(user) = resolve_demo_session_user(cookie.value()) {
                    if user.is_admin {
                        return Ok(user);
                    }
                    return Err(ServerFnError::new("Admin privileges required"));
                }
            }
        }

        Err(ServerFnError::new(
            crate::auth::models::UNAUTHORIZED_SENTINEL,
        ))
    }

    pub(crate) async fn require_any_user(
        state: &AppState,
    ) -> Result<crate::auth::models::AuthenticatedUser, ServerFnError> {
        use crate::auth::extractor::ACCESS_TOKEN_COOKIE;
        use crate::auth::token_service::TokenService;
        use axum_extra::extract::CookieJar;

        let jar: CookieJar = leptos_axum::extract().await?;

        if let Some(user) = jar
            .get(ACCESS_TOKEN_COOKIE)
            .and_then(|c| state.token_service.validate_access_token(c.value()).ok())
            .map(|claims| TokenService::claims_to_user(&claims))
        {
            return Ok(user);
        }

        if state.demo_mode {
            if let Some(cookie) = jar.get("lekton_demo_user") {
                if let Some(user) = resolve_demo_session_user(cookie.value()) {
                    return Ok(user);
                }
            }
        }

        Err(ServerFnError::new(
            crate::auth::models::UNAUTHORIZED_SENTINEL,
        ))
    }

    /// Resolve the full [`UserContext`](crate::auth::models::UserContext) of the
    /// caller, needed for access-filtered retrieval and per-level checks.
    /// Errors with the unauthorized sentinel when no user can be resolved.
    pub(crate) async fn require_user_context(
        state: &AppState,
    ) -> Result<crate::auth::models::UserContext, ServerFnError> {
        use crate::auth::extractor::{ACCESS_TOKEN_COOKIE, LOGGED_IN_COOKIE};
        use crate::auth::models::UserContext;
        use crate::auth::token_service::TokenService;
        use axum_extra::extract::CookieJar;

        let jar: CookieJar = leptos_axum::extract().await?;

        if let Some(auth_user) = jar
            .get(ACCESS_TOKEN_COOKIE)
            .and_then(|c| state.token_service.validate_access_token(c.value()).ok())
            .map(|claims| TokenService::claims_to_user(&claims))
        {
            // Admins short-circuit every access check inside UserContext, so a
            // minimal context is sufficient without loading the user document.
            if auth_user.is_admin {
                return Ok(UserContext {
                    user: auth_user,
                    effective_access_levels: vec![],
                    can_write: true,
                    can_read_draft: true,
                    can_write_draft: true,
                });
            }
            let user_doc = state
                .user_repo
                .find_user_by_id(&auth_user.user_id)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            return Ok(match user_doc {
                Some(u) => UserContext::from_user_doc(auth_user, &u),
                None => UserContext {
                    user: auth_user,
                    effective_access_levels: vec![],
                    can_write: false,
                    can_read_draft: false,
                    can_write_draft: false,
                },
            });
        }

        if state.demo_mode {
            if let Some(cookie) = jar.get("lekton_demo_user") {
                if let Some(demo_user) = resolve_demo_session_user(cookie.value()) {
                    return Ok(UserContext {
                        user: demo_user,
                        effective_access_levels: vec![],
                        can_write: false,
                        can_read_draft: false,
                        can_write_draft: false,
                    });
                }
            }
        }

        if jar.get(LOGGED_IN_COOKIE).is_some() {
            return Err(ServerFnError::new(
                crate::auth::models::UNAUTHORIZED_SENTINEL,
            ));
        }

        Err(ServerFnError::new(
            crate::auth::models::UNAUTHORIZED_SENTINEL,
        ))
    }
}
