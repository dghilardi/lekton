//! JWT access token generation/validation and refresh token management.
//!
//! `TokenService` is responsible for:
//! - Signing and verifying short-lived JWT access tokens (HS256).
//! - Generating cryptographically random refresh tokens and storing only
//!   their SHA-256 hash in the database.

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::models::AuthenticatedUser;
use crate::error::AppError;

/// Default access token lifetime.
const DEFAULT_ACCESS_TTL_SECS: u64 = 15 * 60; // 15 minutes
/// Default refresh token lifetime.
const DEFAULT_REFRESH_TTL_DAYS: i64 = 30;

/// Claims embedded in the JWT access token.
#[derive(Debug, Serialize, Deserialize)]
pub struct JwtClaims {
    /// User's internal ID (`user_id`).
    pub sub: String,
    /// User's email address.
    pub email: String,
    /// Whether the user has admin privileges.
    pub is_admin: bool,
    /// Issued-at timestamp (Unix seconds).
    pub iat: u64,
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
}

#[cfg(feature = "ssr")]
impl TokenService {
    /// Create a `TokenService` from the `JWT_SECRET` environment variable.
    ///
    /// Optionally reads `JWT_ACCESS_TTL_SECS` and `JWT_REFRESH_TTL_DAYS`.
    pub fn from_env() -> Result<Self, AppError> {
        let secret = std::env::var("JWT_SECRET")
            .map_err(|_| AppError::Auth("JWT_SECRET not set".into()))?;
        let access_ttl = std::env::var("JWT_ACCESS_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_ACCESS_TTL_SECS);
        let refresh_ttl = std::env::var("JWT_REFRESH_TTL_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_REFRESH_TTL_DAYS);
        Ok(Self::new(&secret, access_ttl, refresh_ttl))
    }

    /// Create a `TokenService` with explicit parameters (useful for testing).
    pub fn new(secret: &str, access_token_ttl_secs: u64, refresh_token_ttl_days: i64) -> Self {
        Self {
            encoding_key: jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
            access_token_ttl_secs,
            refresh_token_ttl_days,
        }
    }

    /// Sign a JWT access token for the given user.
    pub fn generate_access_token(&self, user: &AuthenticatedUser) -> Result<String, AppError> {
        let now = Utc::now().timestamp() as u64;
        let claims = JwtClaims {
            sub: user.user_id.clone(),
            email: user.email.clone(),
            is_admin: user.is_admin,
            iat: now,
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
        // No leeway: treat tokens as expired the instant their `exp` passes.
        validation.leeway = 0;

        jsonwebtoken::decode::<JwtClaims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    AppError::Auth("Access token has expired".into())
                }
                _ => AppError::Auth(format!("Invalid access token: {e}")),
            })
    }

    /// Generate a fresh refresh token pair: `(raw_token, hash)`.
    ///
    /// The raw token is a UUID v4 (122 bits of randomness) and is returned
    /// once to be sent to the client.  Only the SHA-256 hash is stored in the
    /// database.
    pub fn generate_refresh_token(&self) -> (String, String) {
        let raw = uuid::Uuid::new_v4().to_string();
        let hash = Self::hash_token(&raw);
        (raw, hash)
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
        assert_eq!(claims.email, "user@test.com");
        assert!(!claims.is_admin);
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
    fn test_generate_refresh_token_is_unique() {
        let svc = make_service();
        let (raw1, hash1) = svc.generate_refresh_token();
        let (raw2, hash2) = svc.generate_refresh_token();
        assert_ne!(raw1, raw2);
        assert_ne!(hash1, hash2);
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
