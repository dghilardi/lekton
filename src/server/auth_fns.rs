use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::app::AppState;

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

#[server(LogoutUser, "/api")]
pub async fn logout_user() -> Result<(), ServerFnError> {
    use leptos_axum::ResponseOptions;

    let state = expect_context::<AppState>();
    let response = expect_context::<ResponseOptions>();

    let clear_cookie = |name: &str, path: &str| -> String {
        format!("{name}=; Path={path}; HttpOnly; SameSite=Strict; Max-Age=0")
    };

    let set_clear_cookie = |name: &str, path: &str| -> Result<(), ServerFnError> {
        let value = axum::http::HeaderValue::from_str(&clear_cookie(name, path))
            .map_err(|e| ServerFnError::new(format!("Invalid cookie header: {e}")))?;
        response.append_header(axum::http::header::SET_COOKIE, value);
        Ok(())
    };

    if state.demo_mode {
        set_clear_cookie("lekton_demo_user", "/")?;
    } else {
        set_clear_cookie("lekton_access_token", "/")?;
        set_clear_cookie("lekton_refresh_token", "/auth/refresh")?;
    }

    Ok(())
}
