use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::app::AppState;

#[cfg(feature = "ssr")]
fn logout_cookies(demo_mode: bool) -> Vec<axum_extra::extract::cookie::Cookie<'static>> {
    use crate::auth::extractor::{
        clear_access_token_cookie, clear_legacy_refresh_token_cookie, clear_logged_in_cookie,
        clear_refresh_token_cookie, DEMO_USER_COOKIE,
    };

    if demo_mode {
        vec![
            axum_extra::extract::cookie::Cookie::build((DEMO_USER_COOKIE, ""))
                .path("/")
                .removal()
                .build(),
            clear_logged_in_cookie(),
        ]
    } else {
        vec![
            clear_access_token_cookie(),
            clear_refresh_token_cookie(),
            clear_legacy_refresh_token_cookie(),
            clear_logged_in_cookie(),
        ]
    }
}

#[server(GetCurrentUser, "/api")]
pub async fn get_current_user(
) -> Result<Option<crate::auth::models::AuthenticatedUser>, ServerFnError> {
    use crate::auth::extractor::{ACCESS_TOKEN_COOKIE, LOGGED_IN_COOKIE};
    use crate::auth::token_service::TokenService;
    use axum_extra::extract::CookieJar;

    let state = expect_context::<AppState>();
    let jar: CookieJar = leptos_axum::extract().await?;

    if let Some(cookie) = jar.get(ACCESS_TOKEN_COOKIE) {
        return match state.token_service.validate_access_token(cookie.value()) {
            Ok(claims) => Ok(Some(TokenService::claims_to_user(&claims))),
            Err(_) => Err(ServerFnError::new(
                crate::auth::models::UNAUTHORIZED_SENTINEL,
            )),
        };
    }

    if state.demo_mode {
        if let Some(cookie) = jar.get("lekton_demo_user") {
            if let Ok(user) =
                serde_json::from_str::<crate::auth::models::AuthenticatedUser>(cookie.value())
            {
                return Ok(Some(user));
            }
        }
    }

    if jar.get(LOGGED_IN_COOKIE).is_some() {
        return Err(ServerFnError::new(
            crate::auth::models::UNAUTHORIZED_SENTINEL,
        ));
    }

    Ok(None)
}

#[server(GetIsDemoMode, "/api")]
pub async fn get_is_demo_mode() -> Result<bool, ServerFnError> {
    let state = expect_context::<AppState>();
    Ok(state.demo_mode)
}

#[server(GetIsRagEnabled, "/api")]
pub async fn get_is_rag_enabled() -> Result<bool, ServerFnError> {
    let state = expect_context::<AppState>();
    Ok(state.rag_service.is_some() && state.chat_service.is_some())
}

/// Lightweight SSR-side cookie presence check — no token validation, no DB call.
/// Used to decide whether to show the entrance splash screen: if the session
/// cookie is absent the user is anonymous and the layout can render immediately.
#[server(HasSessionCookie, "/api")]
pub async fn has_session_cookie() -> Result<bool, ServerFnError> {
    use crate::auth::extractor::LOGGED_IN_COOKIE;
    use axum_extra::extract::CookieJar;
    let jar: CookieJar = leptos_axum::extract().await?;
    Ok(jar.get(LOGGED_IN_COOKIE).is_some())
}

#[server(LogoutUser, "/api")]
pub async fn logout_user() -> Result<(), ServerFnError> {
    use crate::auth::extractor::REFRESH_TOKEN_COOKIE;
    use crate::auth::token_service::TokenService;
    use axum::http::header::SET_COOKIE;
    use axum_extra::extract::CookieJar;
    use leptos_axum::ResponseOptions;

    let state = expect_context::<AppState>();
    let response = expect_context::<ResponseOptions>();
    let jar: CookieJar = leptos_axum::extract().await?;

    if !state.demo_mode {
        if let Some(raw) = jar
            .get(REFRESH_TOKEN_COOKIE)
            .map(|cookie| cookie.value().to_string())
        {
            let hash = TokenService::hash_token(&raw);
            if let Ok(Some(stored)) = state.user_repo.find_refresh_token_by_hash(&hash).await {
                let _ = state.user_repo.revoke_refresh_token(&stored.id).await;
            }
        }
    }

    for cookie in logout_cookies(state.demo_mode) {
        let value = axum::http::HeaderValue::from_str(&cookie.to_string())
            .map_err(|e| ServerFnError::new(format!("Invalid cookie header: {e}")))?;
        response.append_header(SET_COOKIE, value);
    }

    Ok(())
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::logout_cookies;
    use crate::auth::extractor::{
        ACCESS_TOKEN_COOKIE, DEMO_USER_COOKIE, LOGGED_IN_COOKIE, REFRESH_TOKEN_COOKIE,
    };

    #[test]
    fn oidc_logout_clears_access_refresh_and_logged_in_cookies() {
        let names: Vec<_> = logout_cookies(false)
            .into_iter()
            .map(|cookie| cookie.name().to_string())
            .collect();

        assert!(names.contains(&ACCESS_TOKEN_COOKIE.to_string()));
        assert!(names.contains(&REFRESH_TOKEN_COOKIE.to_string()));
        assert!(names.contains(&LOGGED_IN_COOKIE.to_string()));
    }

    #[test]
    fn demo_logout_clears_demo_and_logged_in_cookies() {
        let names: Vec<_> = logout_cookies(true)
            .into_iter()
            .map(|cookie| cookie.name().to_string())
            .collect();

        assert!(names.contains(&DEMO_USER_COOKIE.to_string()));
        assert!(names.contains(&LOGGED_IN_COOKIE.to_string()));
    }
}
