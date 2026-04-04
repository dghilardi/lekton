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
    /// Dot-notation path to the subject/ID field in the userinfo JSON response.
    /// Falls back to standard `sub` then `id` fields when unset.
    /// Example: `"data.userShortId"`
    pub userinfo_sub_field: Option<String>,
    /// Dot-notation path to the email field in the userinfo JSON response.
    /// Falls back to standard `email` field when unset.
    /// Example: `"data.loginEmail"`
    pub userinfo_email_field: Option<String>,
    /// Dot-notation path to the name field in the userinfo JSON response.
    /// Falls back to standard `name` then `display_name` fields when unset.
    /// Multiple paths can be joined with `,` to concatenate values (e.g.
    /// `"data.firstName,data.lastName"`).
    pub userinfo_name_field: Option<String>,
}

impl AuthProviderConfig {
    /// Build from the application's centralised [`crate::config::AuthConfig`].
    #[cfg(feature = "ssr")]
    pub fn from_app_config(auth: &crate::config::AuthConfig) -> Result<Self, AppError> {
        let provider_type = auth.provider_type.clone();
        if provider_type != "oidc" && provider_type != "oauth2" {
            return Err(AppError::Auth(format!(
                "auth.provider_type must be 'oidc' or 'oauth2', got '{provider_type}'"
            )));
        }

        Ok(Self {
            provider_type,
            client_id: auth
                .client_id
                .clone()
                .ok_or_else(|| AppError::Auth("auth.client_id not set".into()))?,
            client_secret: auth
                .client_secret
                .clone()
                .ok_or_else(|| AppError::Auth("auth.client_secret not set".into()))?,
            redirect_uri: auth
                .redirect_uri
                .clone()
                .ok_or_else(|| AppError::Auth("auth.redirect_uri not set".into()))?,
            authorization_endpoint: auth
                .authorization_endpoint
                .clone()
                .ok_or_else(|| AppError::Auth("auth.authorization_endpoint not set".into()))?,
            token_endpoint: auth.token_endpoint.clone(),
            userinfo_endpoint: auth.userinfo_endpoint.clone(),
            scopes: auth.scopes.clone(),
            userinfo_sub_field: auth.userinfo_sub_field.clone(),
            userinfo_email_field: auth.userinfo_email_field.clone(),
            userinfo_name_field: auth.userinfo_name_field.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_auth_config_missing_client() -> crate::config::AuthConfig {
        crate::config::AuthConfig {
            demo_mode: false,
            allow_demo_in_production: false,
            service_token: None,
            jwt_secret: None,
            jwt_access_ttl_secs: 900,
            jwt_refresh_ttl_days: 30,
            provider_type: "oidc".to_string(),
            client_id: None, // missing
            client_secret: None,
            redirect_uri: None,
            authorization_endpoint: None,
            token_endpoint: None,
            userinfo_endpoint: None,
            scopes: "openid profile email".to_string(),
            userinfo_sub_field: None,
            userinfo_email_field: None,
            userinfo_name_field: None,
        }
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_from_app_config_missing_required_fields() {
        let auth = make_auth_config_missing_client();
        let result = AuthProviderConfig::from_app_config(&auth);
        assert!(result.is_err());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_invalid_provider_type() {
        let mut auth = make_auth_config_missing_client();
        auth.provider_type = "saml".to_string();
        let result = AuthProviderConfig::from_app_config(&auth);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("saml")),
            other => panic!("expected Auth error, got {other:?}"),
        }
    }
}
