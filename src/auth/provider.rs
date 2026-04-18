//! OAuth2 / OIDC auth provider abstraction.
//!
//! [`AuthProvider`] is the trait both implementations satisfy.  The concrete
//! type is chosen at startup based on `AUTH_PROVIDER_TYPE`:
//!
//! - [`OidcAuthProvider`] — uses the OIDC discovery document for endpoint
//!   discovery and validates the `nonce` embedded in the `id_token`.
//! - [`OAuth2AuthProvider`] — plain OAuth2 with a manual userinfo HTTP call.
//!
//! Both providers perform the CSRF `state` check.  `OidcAuthProvider` also
//! validates the nonce against the stored [`AuthFlowState`].
//!
//! # Note on id_token signature verification
//! `OidcAuthProvider` currently decodes the `id_token` payload without
//! verifying the JWKS signature.  This is acceptable because the token is
//! received over TLS directly from the provider's token endpoint (not from
//! the browser), so the transport itself provides the authenticity guarantee.
//! Full JWKS signature verification can be added as a follow-up.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::config::AuthProviderConfig;
use crate::error::AppError;

// ── Public types ─────────────────────────────────────────────────────────────

/// Normalised identity returned by every provider after code exchange.
#[derive(Debug, Clone)]
pub struct UserInfo {
    /// Subject claim — unique identifier from the provider.
    pub sub: String,
    /// User email address.
    pub email: String,
    /// Display name (may be absent for some providers).
    pub name: Option<String>,
}

/// State stored in a short-lived cookie (`lekton_auth_state`) during the
/// OAuth2 redirect roundtrip to prevent CSRF.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthFlowState {
    /// Random CSRF token sent as the `state` parameter.
    pub csrf_token: String,
    /// OIDC nonce (only set for OIDC providers).
    pub nonce: Option<String>,
}

impl AuthFlowState {
    pub fn new_oauth2(csrf_token: String) -> Self {
        Self {
            csrf_token,
            nonce: None,
        }
    }

    pub fn new_oidc(csrf_token: String, nonce: String) -> Self {
        Self {
            csrf_token,
            nonce: Some(nonce),
        }
    }
}

// ── Trait ────────────────────────────────────────────────────────────────────

/// Abstraction over OAuth2 and OIDC authentication providers.
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// Identifies the provider variant (`"oidc"` or `"oauth2"`).
    fn provider_type(&self) -> &'static str;

    /// Build the URL to redirect the user to for authentication.
    ///
    /// Returns the URL string and the flow state that must be stored in a
    /// short-lived httpOnly cookie and verified during the callback.
    fn login_url(&self) -> Result<(String, AuthFlowState), AppError>;

    /// Exchange an authorization code for user identity.
    ///
    /// `stored_state` is the [`AuthFlowState`] that was serialised into the
    /// cookie during `login_url()`; it is used to verify the `state` parameter
    /// and (for OIDC) the nonce.
    async fn exchange_code(
        &self,
        code: &str,
        returned_state: &str,
        stored_state: &AuthFlowState,
    ) -> Result<UserInfo, AppError>;
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Resolve a dot-notation path (e.g. `"data.loginEmail"`) against a JSON value.
///
/// Returns `None` if any segment along the path is missing or not an object.
fn resolve_json_path<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Extract a string from a JSON value using an optional dot-notation path,
/// falling back to a list of standard top-level field names.
fn extract_field(
    json: &serde_json::Value,
    custom_path: Option<&str>,
    fallback_keys: &[&str],
) -> Option<String> {
    if let Some(path) = custom_path {
        if let Some(v) = resolve_json_path(json, path) {
            return match v {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            };
        }
        return None;
    }
    for key in fallback_keys {
        if let Some(s) = json.get(*key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// Extract a name by resolving one or more comma-separated dot-notation paths
/// and joining the results with a space.  Falls back to standard field names.
fn extract_name_field(
    json: &serde_json::Value,
    custom_paths: Option<&str>,
    fallback_keys: &[&str],
) -> Option<String> {
    if let Some(paths) = custom_paths {
        let parts: Vec<String> = paths
            .split(',')
            .filter_map(|p| {
                resolve_json_path(json, p.trim())
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .filter(|s| !s.is_empty())
            .collect();
        if !parts.is_empty() {
            return Some(parts.join(" "));
        }
        return None;
    }
    for key in fallback_keys {
        if let Some(s) = json.get(*key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// Decode a JWT payload (base64url) into a JSON value without verifying the
/// signature.  Used to extract claims from an OIDC `id_token`.
///
/// Signature verification is intentionally omitted: the id_token arrives via
/// TLS-protected back-channel (token endpoint response), not from the browser,
/// so transport-level authenticity is guaranteed.  See module-level docs.
fn decode_jwt_payload(token: &str) -> Result<serde_json::Value, AppError> {
    use base64::Engine;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AppError::Auth("id_token is not a valid JWT".into()));
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| AppError::Auth(format!("id_token base64 decode failed: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Auth(format!("id_token claims parse failed: {e}")))
}

// ── OAuth2 provider (no OIDC) ─────────────────────────────────────────────────

/// Plain OAuth2 provider — used for providers that lack OIDC support.
///
/// After code exchange, user identity is obtained by calling the
/// `userinfo_endpoint` with the returned access token.
#[derive(Debug)]
pub struct OAuth2AuthProvider {
    client_id: String,
    client_secret: String,
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
    redirect_uri: String,
    scopes: String,
    http: reqwest::Client,
    /// Optional dot-notation path to the subject/ID field.
    sub_field: Option<String>,
    /// Optional dot-notation path to the email field.
    email_field: Option<String>,
    /// Optional comma-separated dot-notation paths to name field(s).
    name_field: Option<String>,
}

impl OAuth2AuthProvider {
    pub fn from_config(config: &AuthProviderConfig) -> Result<Self, AppError> {
        let token_endpoint = config
            .token_endpoint
            .clone()
            .ok_or_else(|| AppError::Auth("AUTH_TOKEN_ENDPOINT required for oauth2".into()))?;
        let userinfo_endpoint = config
            .userinfo_endpoint
            .clone()
            .ok_or_else(|| AppError::Auth("AUTH_USERINFO_ENDPOINT required for oauth2".into()))?;

        Ok(Self {
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            authorization_endpoint: config.authorization_endpoint.clone(),
            token_endpoint,
            userinfo_endpoint,
            redirect_uri: config.redirect_uri.clone(),
            scopes: config.scopes.clone(),
            http: reqwest::Client::new(),
            sub_field: config.userinfo_sub_field.clone(),
            email_field: config.userinfo_email_field.clone(),
            name_field: config.userinfo_name_field.clone(),
        })
    }
}

#[async_trait]
impl AuthProvider for OAuth2AuthProvider {
    fn provider_type(&self) -> &'static str {
        "oauth2"
    }

    fn login_url(&self) -> Result<(String, AuthFlowState), AppError> {
        let csrf_token = uuid::Uuid::new_v4().to_string();

        let url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
            self.authorization_endpoint,
            urlencoding::encode(&self.client_id),
            urlencoding::encode(&self.redirect_uri),
            urlencoding::encode(&self.scopes),
            urlencoding::encode(&csrf_token),
        );

        Ok((url, AuthFlowState::new_oauth2(csrf_token)))
    }

    async fn exchange_code(
        &self,
        code: &str,
        returned_state: &str,
        stored_state: &AuthFlowState,
    ) -> Result<UserInfo, AppError> {
        // 1. Verify CSRF state
        if returned_state != stored_state.csrf_token {
            return Err(AppError::Auth(
                "OAuth2 state mismatch (CSRF check failed)".into(),
            ));
        }

        // 2. Exchange code for access token
        let token_resp = self
            .http
            .post(&self.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri),
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
            ])
            .send()
            .await
            .map_err(|e| AppError::Auth(format!("Token exchange request failed: {e}")))?;

        if !token_resp.status().is_success() {
            let body = token_resp.text().await.unwrap_or_default();
            return Err(AppError::Auth(format!("Token exchange failed: {body}")));
        }

        let token_body: serde_json::Value = token_resp
            .json()
            .await
            .map_err(|e| AppError::Auth(format!("Token response parse failed: {e}")))?;

        let access_token = token_body["access_token"]
            .as_str()
            .ok_or_else(|| AppError::Auth("No access_token in token response".into()))?
            .to_string();

        // 3. Fetch user info
        let userinfo_resp = self
            .http
            .get(&self.userinfo_endpoint)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|e| AppError::Auth(format!("Userinfo request failed: {e}")))?;

        if !userinfo_resp.status().is_success() {
            let body = userinfo_resp.text().await.unwrap_or_default();
            return Err(AppError::Auth(format!("Userinfo request failed: {body}")));
        }

        let profile: serde_json::Value = userinfo_resp
            .json()
            .await
            .map_err(|e| AppError::Auth(format!("Userinfo parse failed: {e}")))?;

        let sub = extract_field(&profile, self.sub_field.as_deref(), &["sub", "id"])
            .ok_or_else(|| AppError::Auth("No subject/ID in userinfo response".into()))?;

        let email = extract_field(&profile, self.email_field.as_deref(), &["email"])
            .ok_or_else(|| AppError::Auth("No email in userinfo response".into()))?;

        let name = extract_name_field(
            &profile,
            self.name_field.as_deref(),
            &["name", "display_name"],
        );

        Ok(UserInfo { sub, email, name })
    }
}

// ── OIDC provider ────────────────────────────────────────────────────────────

/// OpenID Connect provider — uses the discovery document to locate endpoints
/// and validates the `nonce` in the returned `id_token`.
///
/// User identity is extracted from the `id_token` claims (sub, email, name)
/// so no extra userinfo HTTP request is needed.
#[derive(Debug)]
pub struct OidcAuthProvider {
    client_id: String,
    client_secret: String,
    authorization_endpoint: String,
    token_endpoint: String,
    redirect_uri: String,
    scopes: String,
    http: reqwest::Client,
}

impl OidcAuthProvider {
    /// Initialise the provider, performing OIDC discovery if `token_endpoint`
    /// is not already set in config.
    pub async fn from_config(config: &AuthProviderConfig) -> Result<Self, AppError> {
        let token_endpoint = if let Some(ep) = config.token_endpoint.clone() {
            ep
        } else {
            let http = reqwest::Client::new();
            let discovery_url = format!(
                "{}/.well-known/openid-configuration",
                config.authorization_endpoint.trim_end_matches('/')
            );
            let metadata: serde_json::Value = http
                .get(&discovery_url)
                .send()
                .await
                .map_err(|e| AppError::Auth(format!("OIDC discovery request failed: {e}")))?
                .json()
                .await
                .map_err(|e| AppError::Auth(format!("OIDC discovery parse failed: {e}")))?;

            metadata["token_endpoint"]
                .as_str()
                .ok_or_else(|| AppError::Auth("OIDC discovery: missing token_endpoint".into()))?
                .to_string()
        };

        Ok(Self {
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            authorization_endpoint: config.authorization_endpoint.clone(),
            token_endpoint,
            redirect_uri: config.redirect_uri.clone(),
            scopes: config.scopes.clone(),
            http: reqwest::Client::new(),
        })
    }
}

#[async_trait]
impl AuthProvider for OidcAuthProvider {
    fn provider_type(&self) -> &'static str {
        "oidc"
    }

    fn login_url(&self) -> Result<(String, AuthFlowState), AppError> {
        let csrf_token = uuid::Uuid::new_v4().to_string();
        let nonce = uuid::Uuid::new_v4().to_string();

        let url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}",
            self.authorization_endpoint,
            urlencoding::encode(&self.client_id),
            urlencoding::encode(&self.redirect_uri),
            urlencoding::encode(&self.scopes),
            urlencoding::encode(&csrf_token),
            urlencoding::encode(&nonce),
        );

        Ok((url, AuthFlowState::new_oidc(csrf_token, nonce)))
    }

    async fn exchange_code(
        &self,
        code: &str,
        returned_state: &str,
        stored_state: &AuthFlowState,
    ) -> Result<UserInfo, AppError> {
        // 1. CSRF check
        if returned_state != stored_state.csrf_token {
            return Err(AppError::Auth(
                "OIDC state mismatch (CSRF check failed)".into(),
            ));
        }

        let expected_nonce = stored_state
            .nonce
            .as_deref()
            .ok_or_else(|| AppError::Auth("OIDC nonce missing from flow state".into()))?;

        // 2. Exchange code for tokens
        let token_resp = self
            .http
            .post(&self.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri),
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
            ])
            .send()
            .await
            .map_err(|e| AppError::Auth(format!("OIDC token exchange request failed: {e}")))?;

        if !token_resp.status().is_success() {
            let body = token_resp.text().await.unwrap_or_default();
            return Err(AppError::Auth(format!(
                "OIDC token exchange failed: {body}"
            )));
        }

        let token_body: serde_json::Value = token_resp
            .json()
            .await
            .map_err(|e| AppError::Auth(format!("OIDC token response parse failed: {e}")))?;

        let id_token = token_body["id_token"]
            .as_str()
            .ok_or_else(|| AppError::Auth("No id_token in OIDC response".into()))?;

        // 3. Decode id_token claims and verify nonce
        let claims = decode_jwt_payload(id_token)?;

        let token_nonce = claims["nonce"]
            .as_str()
            .ok_or_else(|| AppError::Auth("No nonce claim in id_token".into()))?;
        if token_nonce != expected_nonce {
            return Err(AppError::Auth("OIDC nonce mismatch".into()));
        }

        let sub = claims["sub"]
            .as_str()
            .ok_or_else(|| AppError::Auth("No sub claim in id_token".into()))?
            .to_string();

        let email = claims["email"]
            .as_str()
            .ok_or_else(|| AppError::Auth("No email claim in id_token".into()))?
            .to_string();

        let name = claims["name"].as_str().map(str::to_string);

        Ok(UserInfo { sub, email, name })
    }
}

// ── Factory ───────────────────────────────────────────────────────────────────

/// Build the configured [`AuthProvider`] from the application config.
///
/// Returns `None` when required auth fields are not set
/// (auth is then unavailable but the server starts in degraded mode).
pub async fn build_provider(
    auth: &crate::config::AuthConfig,
) -> Option<std::sync::Arc<dyn AuthProvider>> {
    let config = match AuthProviderConfig::from_app_config(auth) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Auth provider not configured: {e}");
            return None;
        }
    };

    match config.provider_type.as_str() {
        "oidc" => match OidcAuthProvider::from_config(&config).await {
            Ok(p) => {
                tracing::info!("OIDC auth provider initialised");
                Some(std::sync::Arc::new(p))
            }
            Err(e) => {
                tracing::error!("Failed to initialise OIDC provider: {e}");
                None
            }
        },
        _ => match OAuth2AuthProvider::from_config(&config) {
            Ok(p) => {
                tracing::info!("OAuth2 auth provider initialised");
                Some(std::sync::Arc::new(p))
            }
            Err(e) => {
                tracing::error!("Failed to initialise OAuth2 provider: {e}");
                None
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_flow_state_oauth2() {
        let state = AuthFlowState::new_oauth2("csrf-abc".to_string());
        assert_eq!(state.csrf_token, "csrf-abc");
        assert!(state.nonce.is_none());
    }

    #[test]
    fn test_auth_flow_state_oidc() {
        let state = AuthFlowState::new_oidc("csrf-xyz".to_string(), "nonce-123".to_string());
        assert_eq!(state.csrf_token, "csrf-xyz");
        assert_eq!(state.nonce.as_deref(), Some("nonce-123"));
    }

    #[test]
    fn test_auth_flow_state_roundtrip() {
        let state = AuthFlowState::new_oidc("tok".to_string(), "n".to_string());
        let json = serde_json::to_string(&state).unwrap();
        let de: AuthFlowState = serde_json::from_str(&json).unwrap();
        assert_eq!(de.csrf_token, "tok");
        assert_eq!(de.nonce.as_deref(), Some("n"));
    }

    fn make_oauth2_config() -> AuthProviderConfig {
        AuthProviderConfig {
            provider_type: "oauth2".to_string(),
            client_id: "my-client-id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "https://app.example.com/auth/callback".to_string(),
            authorization_endpoint: "https://provider.example.com/oauth/authorize".to_string(),
            token_endpoint: Some("https://provider.example.com/oauth/token".to_string()),
            userinfo_endpoint: Some("https://provider.example.com/userinfo".to_string()),
            scopes: "read:user user:email".to_string(),
            userinfo_sub_field: None,
            userinfo_email_field: None,
            userinfo_name_field: None,
        }
    }

    #[test]
    fn test_oauth2_login_url_contains_client_id() {
        let provider = OAuth2AuthProvider::from_config(&make_oauth2_config()).unwrap();
        let (url, state) = provider.login_url().unwrap();

        assert!(url.contains("my-client-id"), "URL should contain client_id");
        assert!(
            url.contains(&state.csrf_token),
            "URL should contain csrf_token as state"
        );
        assert!(
            url.contains("read%3Auser"),
            "URL should contain encoded scope"
        );
        assert!(state.nonce.is_none());
    }

    #[test]
    fn test_oauth2_requires_token_endpoint() {
        let mut config = make_oauth2_config();
        config.token_endpoint = None;
        let result = OAuth2AuthProvider::from_config(&config);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("TOKEN_ENDPOINT")),
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    #[test]
    fn test_oauth2_requires_userinfo_endpoint() {
        let mut config = make_oauth2_config();
        config.userinfo_endpoint = None;
        let result = OAuth2AuthProvider::from_config(&config);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("USERINFO_ENDPOINT")),
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_oauth2_csrf_state_mismatch_rejected() {
        let provider = OAuth2AuthProvider::from_config(&make_oauth2_config()).unwrap();
        let stored = AuthFlowState::new_oauth2("correct-csrf".to_string());

        let result = provider
            .exchange_code("any-code", "wrong-csrf", &stored)
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("CSRF"), "expected CSRF: {msg}"),
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_oidc_csrf_state_mismatch_rejected() {
        let config = AuthProviderConfig {
            provider_type: "oidc".to_string(),
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "https://app/callback".to_string(),
            authorization_endpoint: "https://provider/auth".to_string(),
            token_endpoint: Some("https://provider/token".to_string()),
            userinfo_endpoint: None,
            scopes: "openid".to_string(),
            userinfo_sub_field: None,
            userinfo_email_field: None,
            userinfo_name_field: None,
        };
        // Build without discovery (token_endpoint is already provided)
        let provider = OidcAuthProvider {
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            authorization_endpoint: config.authorization_endpoint.clone(),
            token_endpoint: config.token_endpoint.unwrap(),
            redirect_uri: config.redirect_uri.clone(),
            scopes: config.scopes.clone(),
            http: reqwest::Client::new(),
        };
        let stored = AuthFlowState::new_oidc("correct-csrf".to_string(), "nonce".to_string());

        let result = provider
            .exchange_code("any-code", "wrong-csrf", &stored)
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("CSRF"), "expected CSRF: {msg}"),
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    #[test]
    fn test_oidc_login_url_contains_nonce() {
        let provider = OidcAuthProvider {
            client_id: "client".to_string(),
            client_secret: "secret".to_string(),
            authorization_endpoint: "https://provider/auth".to_string(),
            token_endpoint: "https://provider/token".to_string(),
            redirect_uri: "https://app/callback".to_string(),
            scopes: "openid email".to_string(),
            http: reqwest::Client::new(),
        };
        let (url, state) = provider.login_url().unwrap();
        assert!(url.contains("nonce="), "URL should contain nonce");
        assert!(state.nonce.is_some(), "state should have nonce");
        assert!(url.contains(&state.csrf_token), "URL should contain state");
    }

    #[test]
    fn test_decode_jwt_payload_invalid() {
        let result = decode_jwt_payload("not-a-jwt");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_json_path_nested() {
        let json: serde_json::Value = serde_json::json!({
            "data": {
                "userShortId": 42,
                "loginEmail": "user@example.com",
                "firstName": "John",
                "lastName": "Doe"
            }
        });
        assert_eq!(
            resolve_json_path(&json, "data.loginEmail").and_then(|v| v.as_str()),
            Some("user@example.com")
        );
        assert_eq!(
            resolve_json_path(&json, "data.userShortId").and_then(|v| v.as_i64()),
            Some(42)
        );
        assert!(resolve_json_path(&json, "data.missing").is_none());
        assert!(resolve_json_path(&json, "nonexistent.path").is_none());
    }

    #[test]
    fn test_extract_field_with_custom_path() {
        let json: serde_json::Value = serde_json::json!({
            "data": { "loginEmail": "custom@example.com" },
            "email": "standard@example.com"
        });
        // Custom path takes priority
        assert_eq!(
            extract_field(&json, Some("data.loginEmail"), &["email"]),
            Some("custom@example.com".to_string())
        );
        // Falls back to standard fields when no custom path
        assert_eq!(
            extract_field(&json, None, &["email"]),
            Some("standard@example.com".to_string())
        );
    }

    #[test]
    fn test_extract_field_numeric_id() {
        let json: serde_json::Value = serde_json::json!({
            "data": { "userShortId": 12345 }
        });
        assert_eq!(
            extract_field(&json, Some("data.userShortId"), &["sub"]),
            Some("12345".to_string())
        );
    }

    #[test]
    fn test_extract_name_field_concatenated() {
        let json: serde_json::Value = serde_json::json!({
            "data": { "firstName": "John", "lastName": "Doe" },
            "name": "Standard Name"
        });
        // Multiple paths joined with comma
        assert_eq!(
            extract_name_field(&json, Some("data.firstName,data.lastName"), &["name"]),
            Some("John Doe".to_string())
        );
        // Single path
        assert_eq!(
            extract_name_field(&json, Some("data.firstName"), &["name"]),
            Some("John".to_string())
        );
        // Fallback
        assert_eq!(
            extract_name_field(&json, None, &["name"]),
            Some("Standard Name".to_string())
        );
    }

    #[test]
    fn test_extract_name_field_partial_missing() {
        let json: serde_json::Value = serde_json::json!({
            "data": { "firstName": "John" }
        });
        // Only one of the paths resolves — should still return the resolved part
        assert_eq!(
            extract_name_field(&json, Some("data.firstName,data.lastName"), &["name"]),
            Some("John".to_string())
        );
    }
}
