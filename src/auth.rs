use openidconnect::core::{CoreClient, CoreProviderMetadata, CoreResponseType};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl,
    Nonce, RedirectUrl, Scope,
};
use std::env;

pub struct AuthClient {
    pub client: CoreClient,
}

impl AuthClient {
    pub async fn new() -> Self {
        let client_id = env::var("OIDC_CLIENT_ID").expect("OIDC_CLIENT_ID not set");
        let client_secret = env::var("OIDC_CLIENT_SECRET").expect("OIDC_CLIENT_SECRET not set");
        let issuer_url = env::var("OIDC_ISSUER_URL").expect("OIDC_ISSUER_URL not set");
        let redirect_url = env::var("OIDC_REDIRECT_URL").expect("OIDC_REDIRECT_URL not set");

        let provider_metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(issuer_url).expect("Invalid issuer URL"),
            openidconnect::reqwest::async_http_client,
        )
        .await
        .expect("Failed to discover OIDC provider");

        let client = CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(client_id),
            Some(ClientSecret::new(client_secret)),
        )
        .set_redirect_uri(RedirectUrl::new(redirect_url).expect("Invalid redirect URL"));

        Self { client }
    }

    pub fn auth_url(&self) -> (reqwest::Url, CsrfToken, Nonce) {
        self.client
            .authorize_url(AuthenticationFlow::<CoreResponseType>::AuthorizationCode, CsrfToken::new_random, Nonce::new_random)
            .add_scope(Scope::new("openid".to_string()))
            .add_scope(Scope::new("profile".to_string()))
            .add_scope(Scope::new("email".to_string()))
            .url()
    }
}

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect};
use crate::state::AppState;
use serde::Deserialize;
use tower_sessions::Session;

#[derive(Debug, Deserialize)]
pub struct AuthCallbackQuery {
    pub code: String,
    pub state: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UserSession {
    pub email: String,
    pub roles: Vec<String>,
}

pub async fn login_handler(
    State(state): State<AppState>,
    session: Session,
) -> impl IntoResponse {
    let (auth_url, csrf_token, _nonce) = state.auth_client.auth_url();
    
    // Store CSRF token in session for validation
    session.insert("csrf_token", csrf_token.secret().to_string()).await.expect("Failed to insert session");
    
    Redirect::to(auth_url.as_str())
}

pub async fn callback_handler(
    State(state): State<AppState>,
    session: Session,
    Query(query): Query<AuthCallbackQuery>,
) -> impl IntoResponse {
    let stored_csrf: Option<String> = session.get("csrf_token").await.expect("Failed to get session");
    
    if stored_csrf.as_ref() != Some(&query.state) {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let _token_response = state.auth_client.client
        .exchange_code(AuthorizationCode::new(query.code))
        .request_async(openidconnect::reqwest::async_http_client)
        .await
        .expect("Failed to exchange code");

    // For now, just a placeholder for role mapping
    let user_session = UserSession {
        email: "placeholder@example.com".to_string(),
        roles: vec!["developer".to_string()],
    };

    session.insert("user", user_session).await.expect("Failed to insert session");
    Redirect::to("/").into_response()
}

use http::StatusCode;
use serde::Serialize;
