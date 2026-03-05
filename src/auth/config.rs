//! Auth provider configuration.
//!
//! Lekton supports two flavours:
//! - `"oidc"`: full OpenID Connect with discovery (most modern IdPs).
//! - `"oauth2"`: plain OAuth2 without OIDC (e.g. the Comelit Cloud provider
//!   that has no discovery endpoint or `id_token`).
//!
//! Both share the same environment variables; the `AUTH_PROVIDER_TYPE` env var
//! selects which implementation to build.

use crate::error::AppError;

/// Unified provider configuration read from environment variables.
#[derive(Debug, Clone)]
pub struct AuthProviderConfig {
    /// `"oidc"` or `"oauth2"`.
    pub provider_type: String,
    /// OAuth2 client ID.
    pub client_id: String,
    /// OAuth2 client secret.
    pub client_secret: String,
    /// Redirect URI registered with the provider.
    pub redirect_uri: String,
    /// OIDC: discovery/issuer URL.  OAuth2: authorization endpoint.
    pub authorization_endpoint: String,
    /// Token exchange endpoint (required for OAuth2; optional for OIDC which
    /// obtains it from the discovery document).
    pub token_endpoint: Option<String>,
    /// UserInfo endpoint (required for OAuth2; optional for OIDC).
    pub userinfo_endpoint: Option<String>,
    /// OAuth2 scopes to request, space-separated (default: `"openid profile email"`).
    pub scopes: String,
}

impl AuthProviderConfig {
    /// Load provider config from environment variables.
    ///
    /// Required:
    /// - `AUTH_PROVIDER_TYPE` — `"oidc"` or `"oauth2"`
    /// - `AUTH_CLIENT_ID`
    /// - `AUTH_CLIENT_SECRET`
    /// - `AUTH_REDIRECT_URI`
    /// - `AUTH_AUTHORIZATION_ENDPOINT`
    ///
    /// Optional:
    /// - `AUTH_TOKEN_ENDPOINT`       (required for oauth2)
    /// - `AUTH_USERINFO_ENDPOINT`    (required for oauth2)
    /// - `AUTH_SCOPES`               (default: `"openid profile email"`)
    pub fn from_env() -> Result<Self, AppError> {
        let provider_type = std::env::var("AUTH_PROVIDER_TYPE")
            .unwrap_or_else(|_| "oidc".to_string());

        if provider_type != "oidc" && provider_type != "oauth2" {
            return Err(AppError::Auth(format!(
                "AUTH_PROVIDER_TYPE must be 'oidc' or 'oauth2', got '{provider_type}'"
            )));
        }

        Ok(Self {
            provider_type,
            client_id: std::env::var("AUTH_CLIENT_ID")
                .map_err(|_| AppError::Auth("AUTH_CLIENT_ID not set".into()))?,
            client_secret: std::env::var("AUTH_CLIENT_SECRET")
                .map_err(|_| AppError::Auth("AUTH_CLIENT_SECRET not set".into()))?,
            redirect_uri: std::env::var("AUTH_REDIRECT_URI")
                .map_err(|_| AppError::Auth("AUTH_REDIRECT_URI not set".into()))?,
            authorization_endpoint: std::env::var("AUTH_AUTHORIZATION_ENDPOINT")
                .map_err(|_| AppError::Auth("AUTH_AUTHORIZATION_ENDPOINT not set".into()))?,
            token_endpoint: std::env::var("AUTH_TOKEN_ENDPOINT").ok(),
            userinfo_endpoint: std::env::var("AUTH_USERINFO_ENDPOINT").ok(),
            scopes: std::env::var("AUTH_SCOPES")
                .unwrap_or_else(|_| "openid profile email".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_missing_required_vars() {
        // Without any env vars set, from_env should fail.
        // (AUTH_CLIENT_ID is the first required var after provider type.)
        unsafe {
            std::env::remove_var("AUTH_CLIENT_ID");
            std::env::remove_var("AUTH_CLIENT_SECRET");
            std::env::remove_var("AUTH_REDIRECT_URI");
            std::env::remove_var("AUTH_AUTHORIZATION_ENDPOINT");
        }
        let result = AuthProviderConfig::from_env();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_provider_type() {
        unsafe {
            std::env::set_var("AUTH_PROVIDER_TYPE", "saml");
        }
        let result = AuthProviderConfig::from_env();
        // Clean up before asserting to avoid polluting other tests.
        unsafe {
            std::env::remove_var("AUTH_PROVIDER_TYPE");
        }
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("saml")),
            other => panic!("expected Auth error, got {other:?}"),
        }
    }
}
