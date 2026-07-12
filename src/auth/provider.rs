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
//! fully verifies the `id_token`: JWKS signature (key selected by `kid`),
//! issuer, audience (client ID), expiry and the nonce bound to the flow.

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

/// Claims deserialised from a cryptographically **verified** OIDC `id_token`.
///
/// `iss`, `aud` and `exp` are enforced by [`jsonwebtoken::Validation`] during
/// [`verify_id_token`]; the fields kept here are the ones we read out afterwards.
#[derive(Debug, serde::Deserialize)]
struct IdTokenClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
}

/// Asymmetric signature algorithms permitted for OIDC `id_token`s.
///
/// Symmetric (`HS*`) and `none` are rejected: accepting them would enable an
/// algorithm-confusion attack where the (public) JWKS key material is replayed
/// as an HMAC secret to forge a token.
const OIDC_ALLOWED_ALGS: &[jsonwebtoken::Algorithm] = &[
    jsonwebtoken::Algorithm::RS256,
    jsonwebtoken::Algorithm::RS384,
    jsonwebtoken::Algorithm::RS512,
    jsonwebtoken::Algorithm::ES256,
    jsonwebtoken::Algorithm::ES384,
    jsonwebtoken::Algorithm::PS256,
    jsonwebtoken::Algorithm::PS384,
    jsonwebtoken::Algorithm::PS512,
];

/// Select the JWK to verify a token with, by its `kid` header.
///
/// A token without a `kid` is only accepted when the JWKS holds exactly one
/// key; ambiguity is rejected rather than guessed.
fn select_jwk<'a>(
    jwks: &'a jsonwebtoken::jwk::JwkSet,
    kid: Option<&str>,
) -> Result<&'a jsonwebtoken::jwk::Jwk, AppError> {
    match kid {
        Some(kid) => jwks
            .find(kid)
            .ok_or_else(|| AppError::Auth(format!("id_token kid '{kid}' not found in JWKS"))),
        None => match jwks.keys.as_slice() {
            [single] => Ok(single),
            [] => Err(AppError::Auth("JWKS contains no keys".into())),
            _ => Err(AppError::Auth(
                "id_token has no kid but JWKS has multiple keys".into(),
            )),
        },
    }
}

/// Verify an OIDC `id_token` and return the normalised identity.
///
/// Enforces, in order: allowed (asymmetric) algorithm, signature against the
/// JWKS key selected by `kid`, `iss`, `aud` (== `client_id`), `exp`, and the
/// `nonce` bound to this login flow.
fn verify_id_token(
    id_token: &str,
    jwks: &jsonwebtoken::jwk::JwkSet,
    issuer: &str,
    client_id: &str,
    expected_nonce: &str,
) -> Result<UserInfo, AppError> {
    use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};

    let header = decode_header(id_token)
        .map_err(|e| AppError::Auth(format!("id_token header decode failed: {e}")))?;

    if !OIDC_ALLOWED_ALGS.contains(&header.alg) {
        return Err(AppError::Auth(format!(
            "id_token signed with disallowed algorithm {:?}",
            header.alg
        )));
    }

    let jwk = select_jwk(jwks, header.kid.as_deref())?;
    let key = DecodingKey::from_jwk(jwk)
        .map_err(|e| AppError::Auth(format!("JWKS key decode failed: {e}")))?;

    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    validation.validate_exp = true;

    let claims = decode::<IdTokenClaims>(id_token, &key, &validation)
        .map_err(|e| AppError::Auth(format!("id_token verification failed: {e}")))?
        .claims;

    match claims.nonce.as_deref() {
        Some(n) if n == expected_nonce => {}
        _ => return Err(AppError::Auth("OIDC nonce mismatch".into())),
    }

    let email = claims
        .email
        .ok_or_else(|| AppError::Auth("No email claim in id_token".into()))?;
    Ok(UserInfo {
        sub: claims.sub,
        email,
        name: claims.name,
    })
}

/// Fetch and parse a JWKS document from the provider's `jwks_uri`.
async fn fetch_jwks(
    http: &reqwest::Client,
    jwks_uri: &str,
) -> Result<jsonwebtoken::jwk::JwkSet, AppError> {
    http.get(jwks_uri)
        .send()
        .await
        .map_err(|e| AppError::Auth(format!("JWKS fetch failed: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Auth(format!("JWKS parse failed: {e}")))
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
/// and the JWKS, then cryptographically verifies the returned `id_token`
/// (signature, issuer, audience, expiry and nonce).
///
/// User identity is extracted from the *verified* `id_token` claims (sub,
/// email, name) so no extra userinfo HTTP request is needed.
#[derive(Debug)]
pub struct OidcAuthProvider {
    client_id: String,
    client_secret: String,
    authorization_endpoint: String,
    token_endpoint: String,
    redirect_uri: String,
    scopes: String,
    /// Expected `iss` claim, taken from the discovery document.
    issuer: String,
    /// JWKS endpoint, used for the initial fetch and for refetch on key rotation.
    jwks_uri: String,
    /// Cached signing keys; refetched when a token presents an unknown `kid`.
    jwks: tokio::sync::RwLock<jsonwebtoken::jwk::JwkSet>,
    http: reqwest::Client,
}

impl OidcAuthProvider {
    /// Initialise the provider by fetching the OIDC discovery document (for the
    /// issuer, JWKS URI and token endpoint) and the initial JWKS.
    ///
    /// Discovery is always performed: the `issuer` and `jwks_uri` are required
    /// to verify the `id_token`, so a preconfigured `token_endpoint` no longer
    /// bypasses it.
    pub async fn from_config(config: &AuthProviderConfig) -> Result<Self, AppError> {
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

        let issuer = metadata["issuer"]
            .as_str()
            .ok_or_else(|| AppError::Auth("OIDC discovery: missing issuer".into()))?
            .to_string();
        let jwks_uri = metadata["jwks_uri"]
            .as_str()
            .ok_or_else(|| AppError::Auth("OIDC discovery: missing jwks_uri".into()))?
            .to_string();
        let token_endpoint = config
            .token_endpoint
            .clone()
            .or_else(|| metadata["token_endpoint"].as_str().map(str::to_string))
            .ok_or_else(|| AppError::Auth("OIDC discovery: missing token_endpoint".into()))?;

        let jwks = fetch_jwks(&http, &jwks_uri).await?;

        Ok(Self {
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            authorization_endpoint: config.authorization_endpoint.clone(),
            token_endpoint,
            redirect_uri: config.redirect_uri.clone(),
            scopes: config.scopes.clone(),
            issuer,
            jwks_uri,
            jwks: tokio::sync::RwLock::new(jwks),
            http,
        })
    }

    /// Verify an `id_token`, refetching the JWKS once if the token's `kid` is
    /// not in the cached key set (handles provider key rotation).
    async fn verify_with_rotation(
        &self,
        id_token: &str,
        expected_nonce: &str,
    ) -> Result<UserInfo, AppError> {
        let kid = jsonwebtoken::decode_header(id_token)
            .map_err(|e| AppError::Auth(format!("id_token header decode failed: {e}")))?
            .kid;

        {
            let jwks = self.jwks.read().await;
            if select_jwk(&jwks, kid.as_deref()).is_ok() {
                return verify_id_token(
                    id_token,
                    &jwks,
                    &self.issuer,
                    &self.client_id,
                    expected_nonce,
                );
            }
        }

        // Unknown kid — the provider may have rotated keys. Refetch once.
        let fresh = fetch_jwks(&self.http, &self.jwks_uri).await?;
        let result = verify_id_token(
            id_token,
            &fresh,
            &self.issuer,
            &self.client_id,
            expected_nonce,
        );
        *self.jwks.write().await = fresh;
        result
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

        // 3. Cryptographically verify the id_token (signature, iss, aud, exp,
        //    nonce) and extract the identity from its claims.
        self.verify_with_rotation(id_token, expected_nonce).await
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
        // Build without discovery (CSRF check runs before any token work).
        let provider = OidcAuthProvider {
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            authorization_endpoint: config.authorization_endpoint.clone(),
            token_endpoint: config.token_endpoint.unwrap(),
            redirect_uri: config.redirect_uri.clone(),
            scopes: config.scopes.clone(),
            issuer: "https://provider".to_string(),
            jwks_uri: "https://provider/jwks".to_string(),
            jwks: tokio::sync::RwLock::new(jsonwebtoken::jwk::JwkSet { keys: vec![] }),
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
            issuer: "https://provider".to_string(),
            jwks_uri: "https://provider/jwks".to_string(),
            jwks: tokio::sync::RwLock::new(jsonwebtoken::jwk::JwkSet { keys: vec![] }),
            http: reqwest::Client::new(),
        };
        let (url, state) = provider.login_url().unwrap();
        assert!(url.contains("nonce="), "URL should contain nonce");
        assert!(state.nonce.is_some(), "state should have nonce");
        assert!(url.contains(&state.csrf_token), "URL should contain state");
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

    // ── OIDC id_token verification (P1-01) ─────────────────────────────────────
    //
    // Test vectors: a 2048-bit RSA keypair generated once for the suite. The
    // public half is published as `TEST_JWKS_JSON`; `TEST_RSA_PRIV_PEM` signs
    // tokens as the trusted IdP would. `OTHER_RSA_PRIV_PEM` is an unrelated key
    // used to forge a signature the JWKS cannot validate.

    const TEST_KID: &str = "test-key-1";
    const TEST_ISSUER: &str = "https://issuer.example.com";
    const TEST_CLIENT_ID: &str = "lekton-client";

    const TEST_RSA_PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCcMYVVctMZ860/\nI99gR8ASEEnjgooUhuXQ5P7pBRLhko+lip5naOfomvKtlctI1d7iTBhULI739PO/\nj13zZ8ej719Cgpf2jKnXpjAOR3OgLeGcnO5gyJJbXG9/LJ27rRZJvRftKAqmWH8A\n67Lo+TIlXKeAbd8TG/Y0cn3jvmImlyLCb9aJfhCR8rSHKMO/QZjf/0F7Fr0G8Q6L\ndSuieb/151plhn8YpW69/0CKqrZb01I2OoyX3wo9s0NMPDE+zs7cUjF6Sb+uxqXK\nHUMyAKbo6vP+dtoLa6AqO00yZ2vH/YrTke95qKrelms65TKsiwib1HtkfU+DmK0t\nr4wMq2S/AgMBAAECggEAE0fHn4XLijNlX4W+6qR2EKTDF4WYeYfcsYhlm6pdK3bL\nKueYEx1vgcEyvfu00BpaZ3vLha8fEiZCD74+SNq22Zp68KZXOiogmGFIIv3DR51c\n8VdWfkrz9Ll4fUMq0K0WiGas9UDmmPm+lQSt8oqO8UQP3HoeOb56Cf0CDjgE50y6\nJ4A2Nvz1hqBCL+CKM9pR+V6hvCq/gzQPvdQtTP9NGK86nZyD4eu8jgU/6ZPEMcSy\naPORFkboq51KjaXBJ4FxJvKyQNXNWd0Ehr6PUu7jAVPTva+NnP7obYmRjBbw6YWp\nQm9k+xByHMvYbH15pGAkYn/NhieICdJhzK1WDXM39QKBgQDbIJ6Gk/Q1qe+HDx4U\ntVQx8iziZ0SzGzrSbZdnnzvZfnWlMKbNtWEGHDnVY7H3OtRcj45P8Vn/Yo33NKEf\nXDRQ73SVP8xbbuX8uqr21BBHnjClFXavYNpNFg+5y3spy7JxNZjIHnQ5kwAapqCV\ncjBLIKRkSlTth392xNvNMZAzQwKBgQC2eeLlCTtWyCnii7/J81/h0KRcrcWyag0B\nl+VbNemHAXJfmNMX4FQS1yQLIcrHh4nicCT/OlT/NyCbx9n5pHSidmXsfoDC2ewy\nx7dCqa3WBWpbGSTavaogNNqe7R4hMpJ56fvvlB6+myYAO7q+qPQ+1FyhHc5ArvOK\nf32vlr1q1QKBgFwsEWKMc6nrDF0kR8PwLjVAKA8n3ybzqj9/Z7NnsHYhEn1kxJU8\n2U4Hq3AOGnrjHRa+L9+Cpxecrhiw46FcWIk+4CwzhNNlB4rSPj3LH/nwGYgnSiAl\nPk40nHLLm8gN7cZfBCARZ1QceGu9cUjLmnLPjTa+aZDscPpVfhuG4KAfAoGBAKiV\nFPUqwUKcrG2bVLX7/fI+8wqYlJQPfDKjpYbN2REcWhFNvIBLhQDe+HK8Zn5Ojym/\nF78gohQjVyH00kHcGNFbdzC1croR4TDM6FdTMcIPwMGnCjB4l2snyW6YfISJF2BA\ntrwRaRIJfmMqy42HxBcj1OwZAEssFt42iOSm7Bp9AoGBAMpVZmRn01M223v5diGY\nS7dtfOWlvSM/s75JNyWN6ZdOhxfXj2sSDQNCKjJzu2kLi7fvU2bScSQufHsZL+D7\nlKWdwwLuML0RJ53NZ2V0dBuQH295BowQD8gPVq3TBA4HWn0jdaoXJCz+LjXQ0Ofg\ngnZD43pdMejJwxO4X7CGOeBW\n-----END PRIVATE KEY-----\n";

    const OTHER_RSA_PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQChn91QbvEXJE2I\nZazY95U5ycmRfdP+haMme8uRETPd4qLpe2I13HlOwns7P6YpTkjN9PpfgxjXfPhK\nbv6VOSMo+aOtiRwFaKti4MiLxCgAt63YhtNE4IkdBdG8Y+Ggh/MN4suSbaeuoO5Z\ngh0OCb7YQtESvrWRZ/kDrkV40/ZUz1rKHNMXn0Jik/CQs9OAZ9hf6ans3ZQxdN8U\n9LMOj8NA6OzNojbYRq5j9YH5SmtMGwFYDRwgOCpi1plPd3wb5UY8DVud/m+gIz/r\nsOLJvILGAOCcbkQHcXWG6ivgZPbdy48M/ndI3mSkeCLP1jVIW/E30t8dlqlaJmx5\nkH8nayhtAgMBAAECggEACGfomkTS3CnCsIvdNAGYbMN+bMc3Dd9Eb31zxm03Hdyq\nLWuB3ZxEYODDiP7T9QUBy1ho5yvJ0HdK8JAaRIeOuRRWu3FMmElr1H4tX/uJOxRS\ngdbtSQFGkFBbqqklNHStovS/MrPqSy5KhzQ5r5+5IcVD2244wkG+lT7slQ6tGpcC\n79itn/8jmvRjG5DGyb8FqiHKKSltTY3SX8uGHaBZOfdmLAFfy55pr0tV3SHBggcn\naJP4Y8N3iFfP9ofY/PyBuCaRHqldd/NiYUZE4GfKp+27Hhrtgb+yrEgOi3ldFdYC\nTwVFQD4rXwtHKCWTIyxadqOd9NsMsFbM156U8OEvDwKBgQDjFO1h5lF6zw+saJVl\nNFH5l6Sy9mELsIUw4HchdNdiCBes7L4GtCQh6Jpp7jtKGLzccBYfSADxNi3hHnsD\nriT9RY2MQPfJR+rGBpcMKyYKvJ/JFHfdx6D7Pz3v6arMoRKsAMRIqxqToGJK42WU\nPErlBTdLGGT07Y9KdFrGrqAMDwKBgQC2NPhakov1wnVtlNgRM4CaYuqz0kdSsP61\nB+wraDC2EYb/Gz7bO+P2XcMt8GzG3SvKRdtZ2KO60oFqd9NixeOIMMw/euHDRj5i\nfMRbIaxbl6Oh9uQj4So8UZtsZ7Gsa0f0IdoZxfPp3DFROdtyT/ajMxcCINU1PWyl\niaDHuKl3wwKBgQC51qyWzCiuerp2/HgXBQR5YRMWqu+s4199KpYUOUzzG3BUwsKZ\nNd/EKFoGi0LWVvxo4woXu5OZ1B8j9w/xaRk4dS1sNNcUUNjgCxCeksllTAzwQOIA\nDJnARHdp2i8/nCptvYrldbIgUsdeRk8hS153oxcnS+WDVM9JvYN0ygCNHQKBgGSz\nv/Nm/T213OrUkdpn6ZLqSqdZ95tnQU7Scx/GdO2boE3MRCAs6KAYUDBKqgP65yHj\nHAl7J3qwr2Alm8oCbu+tcKVBCBfB8ebC6E8pvDXfCEwSxSJjZtFxlQIECXmuzVTL\nhNwrwNQmo4ct//Ac68ZlDTla5huHuE2BVEAt+lO7AoGAJ7g1CzOOs62Ccny/SetG\nVJ3E7g7HL8h7xQQKNB9v3pnlYXhYEBMDiUJrozcnH5nWbwwmNM9TQew7VfdOju4o\naFEEIiypLXHaqdU+aLIQ4nykCnWwiT7PMCCrOmcAn2f8ExDAcaaKGwp0ASS61fK1\nNfbCWGtT2QSds646mP9JICQ=\n-----END PRIVATE KEY-----\n";

    const TEST_JWKS_JSON: &str = r#"{"keys":[{"kty":"RSA","use":"sig","kid":"test-key-1","alg":"RS256","n":"nDGFVXLTGfOtPyPfYEfAEhBJ44KKFIbl0OT-6QUS4ZKPpYqeZ2jn6JryrZXLSNXe4kwYVCyO9_Tzv49d82fHo-9fQoKX9oyp16YwDkdzoC3hnJzuYMiSW1xvfyydu60WSb0X7SgKplh_AOuy6PkyJVyngG3fExv2NHJ9475iJpciwm_WiX4QkfK0hyjDv0GY3_9Bexa9BvEOi3Uronm_9edaZYZ_GKVuvf9Aiqq2W9NSNjqMl98KPbNDTDwxPs7O3FIxekm_rsalyh1DMgCm6Orz_nbaC2ugKjtNMmdrx_2K05Hveaiq3pZrOuUyrIsIm9R7ZH1Pg5itLa-MDKtkvw","e":"AQAB"}]}"#;

    fn test_jwks() -> jsonwebtoken::jwk::JwkSet {
        serde_json::from_str(TEST_JWKS_JSON).expect("valid test JWKS")
    }

    fn future_exp() -> u64 {
        jsonwebtoken::get_current_timestamp() + 3600
    }

    #[derive(serde::Serialize)]
    struct TestIdClaims<'a> {
        iss: &'a str,
        aud: &'a str,
        sub: &'a str,
        email: &'a str,
        name: &'a str,
        nonce: &'a str,
        iat: u64,
        exp: u64,
    }

    /// Sign a test id_token with the given algorithm/key/kid and claims.
    fn sign_id_token(
        alg: jsonwebtoken::Algorithm,
        signing_key: &jsonwebtoken::EncodingKey,
        kid: &str,
        iss: &str,
        aud: &str,
        nonce: &str,
        exp: u64,
    ) -> String {
        let mut header = jsonwebtoken::Header::new(alg);
        header.kid = Some(kid.to_string());
        let claims = TestIdClaims {
            iss,
            aud,
            sub: "user-123",
            email: "user@example.com",
            name: "Test User",
            nonce,
            iat: jsonwebtoken::get_current_timestamp(),
            exp,
        };
        jsonwebtoken::encode(&header, &claims, signing_key).expect("sign test token")
    }

    fn trusted_key() -> jsonwebtoken::EncodingKey {
        jsonwebtoken::EncodingKey::from_rsa_pem(TEST_RSA_PRIV_PEM.as_bytes()).expect("valid key")
    }

    #[test]
    fn test_verify_id_token_accepts_valid() {
        let token = sign_id_token(
            jsonwebtoken::Algorithm::RS256,
            &trusted_key(),
            TEST_KID,
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
            future_exp(),
        );
        let info = verify_id_token(
            &token,
            &test_jwks(),
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
        )
        .expect("valid token should verify");
        assert_eq!(info.sub, "user-123");
        assert_eq!(info.email, "user@example.com");
        assert_eq!(info.name.as_deref(), Some("Test User"));
    }

    #[test]
    fn test_verify_id_token_rejects_forged_signature() {
        // Signed with a key that is NOT in the JWKS but claims the trusted kid.
        let forged =
            jsonwebtoken::EncodingKey::from_rsa_pem(OTHER_RSA_PRIV_PEM.as_bytes()).unwrap();
        let token = sign_id_token(
            jsonwebtoken::Algorithm::RS256,
            &forged,
            TEST_KID,
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
            future_exp(),
        );
        let result = verify_id_token(
            &token,
            &test_jwks(),
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
        );
        assert!(result.is_err(), "forged signature must be rejected");
    }

    #[test]
    fn test_verify_id_token_rejects_wrong_issuer() {
        let token = sign_id_token(
            jsonwebtoken::Algorithm::RS256,
            &trusted_key(),
            TEST_KID,
            "https://evil.example.com",
            TEST_CLIENT_ID,
            "nonce-abc",
            future_exp(),
        );
        let result = verify_id_token(
            &token,
            &test_jwks(),
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
        );
        assert!(result.is_err(), "wrong issuer must be rejected");
    }

    #[test]
    fn test_verify_id_token_rejects_wrong_audience() {
        let token = sign_id_token(
            jsonwebtoken::Algorithm::RS256,
            &trusted_key(),
            TEST_KID,
            TEST_ISSUER,
            "some-other-client",
            "nonce-abc",
            future_exp(),
        );
        let result = verify_id_token(
            &token,
            &test_jwks(),
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
        );
        assert!(result.is_err(), "wrong audience must be rejected");
    }

    #[test]
    fn test_verify_id_token_rejects_expired() {
        // exp far in the past (2001-09-09).
        let token = sign_id_token(
            jsonwebtoken::Algorithm::RS256,
            &trusted_key(),
            TEST_KID,
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
            1_000_000_000,
        );
        let result = verify_id_token(
            &token,
            &test_jwks(),
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
        );
        assert!(result.is_err(), "expired token must be rejected");
    }

    #[test]
    fn test_verify_id_token_rejects_nonce_mismatch() {
        let token = sign_id_token(
            jsonwebtoken::Algorithm::RS256,
            &trusted_key(),
            TEST_KID,
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "attacker-nonce",
            future_exp(),
        );
        let result = verify_id_token(
            &token,
            &test_jwks(),
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "expected-nonce",
        );
        assert!(result.is_err(), "nonce mismatch must be rejected");
    }

    #[test]
    fn test_verify_id_token_rejects_symmetric_alg() {
        // alg-confusion: attacker signs HS256 treating public key bytes as an
        // HMAC secret. Must be rejected before any key lookup.
        let hs = jsonwebtoken::EncodingKey::from_secret(b"public-key-material");
        let token = sign_id_token(
            jsonwebtoken::Algorithm::HS256,
            &hs,
            TEST_KID,
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
            future_exp(),
        );
        let result = verify_id_token(
            &token,
            &test_jwks(),
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
        );
        assert!(result.is_err(), "symmetric algorithm must be rejected");
    }

    #[test]
    fn test_verify_id_token_rejects_unknown_kid() {
        let token = sign_id_token(
            jsonwebtoken::Algorithm::RS256,
            &trusted_key(),
            "rotated-away-kid",
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
            future_exp(),
        );
        let result = verify_id_token(
            &token,
            &test_jwks(),
            TEST_ISSUER,
            TEST_CLIENT_ID,
            "nonce-abc",
        );
        assert!(result.is_err(), "unknown kid must be rejected");
    }
}
