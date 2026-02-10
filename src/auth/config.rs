use crate::error::AppError;

/// OIDC configuration read from environment variables.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// The OIDC issuer URL (e.g., `https://accounts.google.com`).
    pub issuer_url: String,
    /// The OAuth2 client ID.
    pub client_id: String,
    /// The OAuth2 client secret.
    pub client_secret: String,
    /// The redirect URI after OIDC login completes.
    pub redirect_uri: String,
}

impl OidcConfig {
    /// Build the OIDC config from environment variables.
    ///
    /// Required env vars:
    /// - `OIDC_ISSUER_URL`
    /// - `OIDC_CLIENT_ID`
    /// - `OIDC_CLIENT_SECRET`
    /// - `OIDC_REDIRECT_URI`
    pub fn from_env() -> Result<Self, AppError> {
        Ok(Self {
            issuer_url: std::env::var("OIDC_ISSUER_URL")
                .map_err(|_| AppError::Auth("OIDC_ISSUER_URL not set".into()))?,
            client_id: std::env::var("OIDC_CLIENT_ID")
                .map_err(|_| AppError::Auth("OIDC_CLIENT_ID not set".into()))?,
            client_secret: std::env::var("OIDC_CLIENT_SECRET")
                .map_err(|_| AppError::Auth("OIDC_CLIENT_SECRET not set".into()))?,
            redirect_uri: std::env::var("OIDC_REDIRECT_URI")
                .map_err(|_| AppError::Auth("OIDC_REDIRECT_URI not set".into()))?,
        })
    }

    /// Build with explicit values (useful for testing).
    pub fn new(
        issuer_url: String,
        client_id: String,
        client_secret: String,
        redirect_uri: String,
    ) -> Self {
        Self {
            issuer_url,
            client_id,
            client_secret,
            redirect_uri,
        }
    }
}
