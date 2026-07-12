//! JWT access token generation/validation and refresh token management.
//!
//! `TokenService` is responsible for:
//! - Signing and verifying short-lived JWT access tokens (HS256).
//! - Generating cryptographically random refresh tokens and storing only
//!   their SHA-256 hash in the database.

use chrono::Utc;
use rand::{distr::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};

use crate::auth::models::AuthenticatedUser;
use crate::error::AppError;

/// Default issuer embedded in access tokens.
const DEFAULT_ACCESS_ISSUER: &str = "lekton";
/// Default audience embedded in access tokens.
const DEFAULT_ACCESS_AUDIENCE: &str = "lekton";
/// Alphanumeric token length required to reach at least 256 bits of entropy.
const OPAQUE_TOKEN_LENGTH: usize = 43;
/// Minimum accepted length for the HS256 signing secret. A short secret is
/// brute-forceable, so startup rejects anything below this (recommend 32+
/// random bytes).
#[cfg(feature = "ssr")]
const MIN_JWT_SECRET_BYTES: usize = 32;
/// Issuer/audience marker for the signed OAuth/OIDC flow-state token, distinct
/// from access tokens so the two cannot be confused despite sharing a secret.
#[cfg(feature = "ssr")]
const FLOW_STATE_MARKER: &str = "lekton:auth-flow";
/// Lifetime of a signed flow-state token — long enough for a login roundtrip.
#[cfg(feature = "ssr")]
const FLOW_STATE_TTL_SECS: u64 = 600;

/// Claims for the signed OAuth/OIDC flow-state token (CSRF token + optional
/// nonce), so the cookie carrying them is tamper-proof and expires.
#[cfg(feature = "ssr")]
#[derive(Debug, Serialize, Deserialize)]
struct FlowStateClaims {
    iss: String,
    aud: String,
    csrf_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    exp: u64,
}

/// Claims embedded in the JWT access token.
#[derive(Debug, Serialize, Deserialize)]
pub struct JwtClaims {
    /// User's internal ID (`user_id`).
    pub sub: String,
    /// JWT issuer.
    pub iss: String,
    /// JWT audience.
    pub aud: String,
    /// User's email address.
    pub email: String,
    /// Whether the user has admin privileges.
    pub is_admin: bool,
    /// Issued-at timestamp (Unix seconds).
    pub iat: u64,
    /// Not-before timestamp (Unix seconds).
    pub nbf: u64,
    /// Expiry timestamp (Unix seconds).
    pub exp: u64,
}

/// Service for JWT access tokens and refresh token lifecycle.
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct TokenService {
    encoding_key: jsonwebtoken::EncodingKey,
    decoding_key: jsonwebtoken::DecodingKey,
    access_token_ttl_secs: u64,
    refresh_token_ttl_days: i64,
    access_token_issuer: String,
    access_token_audience: String,
}

#[cfg(feature = "ssr")]
impl TokenService {
    /// Create a `TokenService` from the application's centralised config.
    pub fn from_app_config(auth: &crate::config::AuthConfig) -> Result<Self, AppError> {
        let secret = auth
            .jwt_secret
            .clone()
            .ok_or_else(|| AppError::Auth("auth.jwt_secret not set".into()))?;
        if secret.len() < MIN_JWT_SECRET_BYTES {
            return Err(AppError::Auth(format!(
                "auth.jwt_secret must be at least {MIN_JWT_SECRET_BYTES} bytes \
                 (recommend 32+ random bytes); got {}",
                secret.len()
            )));
        }
        Ok(Self::new_with_claims(
            &secret,
            auth.jwt_access_ttl_secs,
            auth.jwt_refresh_ttl_days,
            auth.jwt_issuer.clone(),
            auth.jwt_audience.clone(),
        ))
    }

    /// Create a `TokenService` with explicit parameters (useful for testing).
    pub fn new(secret: &str, access_token_ttl_secs: u64, refresh_token_ttl_days: i64) -> Self {
        Self::new_with_claims(
            secret,
            access_token_ttl_secs,
            refresh_token_ttl_days,
            DEFAULT_ACCESS_ISSUER.to_string(),
            DEFAULT_ACCESS_AUDIENCE.to_string(),
        )
    }

    /// Create a `TokenService` with explicit token claim settings.
    pub fn new_with_claims(
        secret: &str,
        access_token_ttl_secs: u64,
        refresh_token_ttl_days: i64,
        access_token_issuer: String,
        access_token_audience: String,
    ) -> Self {
        Self {
            encoding_key: jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
            access_token_ttl_secs,
            refresh_token_ttl_days,
            access_token_issuer,
            access_token_audience,
        }
    }

    /// Sign a JWT access token for the given user.
    pub fn generate_access_token(&self, user: &AuthenticatedUser) -> Result<String, AppError> {
        let now = Utc::now().timestamp() as u64;
        let claims = JwtClaims {
            sub: user.user_id.clone(),
            iss: self.access_token_issuer.clone(),
            aud: self.access_token_audience.clone(),
            email: user.email.clone(),
            is_admin: user.is_admin,
            iat: now,
            nbf: now,
            exp: now + self.access_token_ttl_secs,
        };

        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &self.encoding_key,
        )
        .map_err(|e| AppError::Internal(format!("Failed to sign JWT: {e}")))
    }

    /// Validate a JWT and return its claims.
    ///
    /// Returns `AppError::Auth` if the token is expired or has an invalid signature.
    pub fn validate_access_token(&self, token: &str) -> Result<JwtClaims, AppError> {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        // No leeway: treat tokens as expired the instant their `exp` passes.
        validation.leeway = 0;
        validation.set_issuer(&[self.access_token_issuer.as_str()]);
        validation.set_audience(&[self.access_token_audience.as_str()]);

        jsonwebtoken::decode::<JwtClaims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    AppError::Auth("Access token has expired".into())
                }
                _ => AppError::Auth(format!("Invalid access token: {e}")),
            })
    }

    /// Sign the OAuth/OIDC flow state (CSRF token + optional nonce) as a
    /// short-lived HS256 JWT, so the cookie carrying it across the login
    /// redirect cannot be forged or tampered with by the browser.
    pub fn sign_flow_state(
        &self,
        state: &crate::auth::provider::AuthFlowState,
    ) -> Result<String, AppError> {
        let now = Utc::now().timestamp() as u64;
        let claims = FlowStateClaims {
            iss: FLOW_STATE_MARKER.to_string(),
            aud: FLOW_STATE_MARKER.to_string(),
            csrf_token: state.csrf_token.clone(),
            nonce: state.nonce.clone(),
            exp: now + FLOW_STATE_TTL_SECS,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &self.encoding_key,
        )
        .map_err(|e| AppError::Internal(format!("Failed to sign auth flow state: {e}")))
    }

    /// Verify and decode a signed flow-state token, rejecting tampered, expired,
    /// or wrong-purpose tokens.
    pub fn verify_flow_state(
        &self,
        token: &str,
    ) -> Result<crate::auth::provider::AuthFlowState, AppError> {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        validation.leeway = 0;
        validation.set_issuer(&[FLOW_STATE_MARKER]);
        validation.set_audience(&[FLOW_STATE_MARKER]);

        let claims =
            jsonwebtoken::decode::<FlowStateClaims>(token, &self.decoding_key, &validation)
                .map(|d| d.claims)
                .map_err(|e| AppError::Auth(format!("Invalid auth flow state: {e}")))?;

        Ok(crate::auth::provider::AuthFlowState {
            csrf_token: claims.csrf_token,
            nonce: claims.nonce,
        })
    }

    /// Generate a fresh refresh token pair: `(raw_token, hash)`.
    ///
    /// The raw token is a 43-character alphanumeric secret (~256 bits of
    /// entropy) and is returned once to be sent to the client. Only the
    /// SHA-256 hash is stored in the database.
    pub fn generate_refresh_token(&self) -> (String, String) {
        let raw = Self::generate_opaque_token();
        let hash = Self::hash_token(&raw);
        (raw, hash)
    }

    /// Generate an opaque bearer token using the OS CSPRNG.
    pub fn generate_opaque_token() -> String {
        rand::rng()
            .sample_iter(Alphanumeric)
            .take(OPAQUE_TOKEN_LENGTH)
            .map(char::from)
            .collect()
    }

    /// Return the access token lifetime in seconds (used for cookie max-age).
    pub fn access_token_ttl_secs(&self) -> u64 {
        self.access_token_ttl_secs
    }

    /// Return the refresh token lifetime in days (used when creating the DB record).
    pub fn refresh_token_ttl_days(&self) -> i64 {
        self.refresh_token_ttl_days
    }

    /// Compute the SHA-256 (base64url, no padding) hash of a token string.
    pub fn hash_token(token: &str) -> String {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        let result = hasher.finalize();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result)
    }

    /// Build an [`AuthenticatedUser`] from validated JWT claims.
    ///
    /// The `name` field cannot be recovered from the JWT; callers that need it
    /// should fetch the full user from the database.
    pub fn claims_to_user(claims: &JwtClaims) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: claims.sub.clone(),
            email: claims.email.clone(),
            name: None,
            is_admin: claims.is_admin,
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ssr")]
    use super::*;

    #[cfg(feature = "ssr")]
    fn make_service() -> TokenService {
        TokenService::new("test-secret-key-at-least-32-bytes!!", 3600, 30)
    }

    #[cfg(feature = "ssr")]
    fn make_auth_config(secret: Option<&str>) -> crate::config::AuthConfig {
        crate::config::AuthConfig {
            demo_mode: false,
            allow_demo_in_production: false,
            service_token: None,
            jwt_secret: secret.map(str::to_string),
            jwt_access_ttl_secs: 900,
            jwt_refresh_ttl_days: 30,
            jwt_issuer: "lekton".to_string(),
            jwt_audience: "lekton".to_string(),
            provider_type: "oidc".to_string(),
            client_id: None,
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
    fn from_app_config_rejects_short_secret() {
        let auth = make_auth_config(Some("too-short"));
        match TokenService::from_app_config(&auth) {
            Err(AppError::Auth(msg)) => assert!(
                msg.contains("32"),
                "error should mention the minimum length, got: {msg}"
            ),
            Err(other) => panic!("expected Auth error, got {other:?}"),
            Ok(_) => panic!("a short jwt_secret must be rejected"),
        }
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn from_app_config_accepts_sufficiently_long_secret() {
        let auth = make_auth_config(Some("this-secret-is-exactly-32-bytes!"));
        assert_eq!(auth.jwt_secret.as_deref().unwrap().len(), 32);
        assert!(TokenService::from_app_config(&auth).is_ok());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn from_app_config_rejects_missing_secret() {
        let auth = make_auth_config(None);
        assert!(TokenService::from_app_config(&auth).is_err());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn flow_state_signed_roundtrip() {
        use crate::auth::provider::AuthFlowState;
        let svc = make_service();
        let token = svc
            .sign_flow_state(&AuthFlowState::new_oidc("csrf-1".into(), "nonce-1".into()))
            .unwrap();
        let out = svc.verify_flow_state(&token).unwrap();
        assert_eq!(out.csrf_token, "csrf-1");
        assert_eq!(out.nonce.as_deref(), Some("nonce-1"));
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn flow_state_rejects_tampered_token() {
        use crate::auth::provider::AuthFlowState;
        let svc = make_service();
        let token = svc
            .sign_flow_state(&AuthFlowState::new_oauth2("csrf-1".into()))
            .unwrap();
        // Flip the first character of the signature segment.
        let parts: Vec<&str> = token.split('.').collect();
        let sig = parts[2];
        let flipped = if sig.starts_with('A') { 'B' } else { 'A' };
        let tampered = format!("{}.{}.{}{}", parts[0], parts[1], flipped, &sig[1..]);
        assert!(
            svc.verify_flow_state(&tampered).is_err(),
            "a tampered flow-state token must be rejected"
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn flow_state_rejects_token_from_other_secret() {
        use crate::auth::provider::AuthFlowState;
        let signer = TokenService::new("another-secret-key-32-bytes-long!", 3600, 30);
        let verifier = make_service();
        let token = signer
            .sign_flow_state(&AuthFlowState::new_oauth2("csrf-1".into()))
            .unwrap();
        assert!(
            verifier.verify_flow_state(&token).is_err(),
            "a flow-state token signed with a different secret must be rejected"
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn flow_state_verify_rejects_access_token() {
        // An access token shares the secret but has a different audience; it
        // must not be accepted as flow state.
        let svc = make_service();
        let access = svc.generate_access_token(&make_user(false)).unwrap();
        assert!(svc.verify_flow_state(&access).is_err());
    }

    #[cfg(feature = "ssr")]
    fn make_user(is_admin: bool) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: "u-test-1".to_string(),
            email: "user@test.com".to_string(),
            name: Some("Test User".to_string()),
            is_admin,
        }
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_generate_and_validate_access_token() {
        let svc = make_service();
        let user = make_user(false);

        let token = svc.generate_access_token(&user).unwrap();
        assert!(!token.is_empty());

        let claims = svc.validate_access_token(&token).unwrap();
        assert_eq!(claims.sub, "u-test-1");
        assert_eq!(claims.iss, "lekton");
        assert_eq!(claims.aud, "lekton");
        assert_eq!(claims.email, "user@test.com");
        assert!(!claims.is_admin);
        assert_eq!(claims.iat, claims.nbf);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_admin_flag_preserved_in_jwt() {
        let svc = make_service();
        let admin = make_user(true);

        let token = svc.generate_access_token(&admin).unwrap();
        let claims = svc.validate_access_token(&token).unwrap();
        assert!(claims.is_admin);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_expired_token_rejected() {
        // TTL = 0 seconds → already expired on issue
        let svc = TokenService::new("test-secret-key-at-least-32-bytes!!", 0, 30);
        let user = make_user(false);

        let token = svc.generate_access_token(&user).unwrap();
        // Sleep to guarantee exp < now
        std::thread::sleep(std::time::Duration::from_secs(1));

        let result = svc.validate_access_token(&token);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("expired"), "expected 'expired' in: {msg}"),
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_tampered_token_rejected() {
        let svc = make_service();
        let user = make_user(false);
        let mut token = svc.generate_access_token(&user).unwrap();
        // Flip the last character to corrupt the signature
        let last = token.pop().unwrap();
        token.push(if last == 'A' { 'B' } else { 'A' });

        let result = svc.validate_access_token(&token);
        assert!(result.is_err());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_wrong_issuer_rejected() {
        let issuer_a = TokenService::new_with_claims(
            "test-secret-key-at-least-32-bytes!!",
            3600,
            30,
            "lekton-a".to_string(),
            "lekton".to_string(),
        );
        let issuer_b = TokenService::new_with_claims(
            "test-secret-key-at-least-32-bytes!!",
            3600,
            30,
            "lekton-b".to_string(),
            "lekton".to_string(),
        );
        let token = issuer_a.generate_access_token(&make_user(false)).unwrap();

        let result = issuer_b.validate_access_token(&token);
        assert!(result.is_err());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_wrong_audience_rejected() {
        let audience_a = TokenService::new_with_claims(
            "test-secret-key-at-least-32-bytes!!",
            3600,
            30,
            "lekton".to_string(),
            "lekton-web".to_string(),
        );
        let audience_b = TokenService::new_with_claims(
            "test-secret-key-at-least-32-bytes!!",
            3600,
            30,
            "lekton".to_string(),
            "lekton-api".to_string(),
        );
        let token = audience_a.generate_access_token(&make_user(false)).unwrap();

        let result = audience_b.validate_access_token(&token);
        assert!(result.is_err());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_generate_refresh_token_is_unique() {
        let svc = make_service();
        let (raw1, hash1) = svc.generate_refresh_token();
        let (raw2, hash2) = svc.generate_refresh_token();
        assert_ne!(raw1, raw2);
        assert_ne!(hash1, hash2);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_generate_opaque_token_is_alphanumeric_and_43_chars() {
        let token = TokenService::generate_opaque_token();
        assert_eq!(token.len(), 43);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_hash_token_deterministic() {
        let hash1 = TokenService::hash_token("my-refresh-token");
        let hash2 = TokenService::hash_token("my-refresh-token");
        assert_eq!(hash1, hash2);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_hash_token_different_inputs_different_hashes() {
        let h1 = TokenService::hash_token("token-a");
        let h2 = TokenService::hash_token("token-b");
        assert_ne!(h1, h2);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn test_claims_to_user() {
        let svc = make_service();
        let user = make_user(true);
        let token = svc.generate_access_token(&user).unwrap();
        let claims = svc.validate_access_token(&token).unwrap();
        let recovered = TokenService::claims_to_user(&claims);
        assert_eq!(recovered.user_id, user.user_id);
        assert_eq!(recovered.email, user.email);
        assert!(recovered.is_admin);
        assert!(recovered.name.is_none()); // name not in JWT
    }
}
