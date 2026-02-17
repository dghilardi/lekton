use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;

use crate::auth::models::AccessLevel;
use crate::error::AppError;

/// Claims embedded in a Meilisearch tenant token.
///
/// See: <https://www.meilisearch.com/docs/learn/security/multitenancy_tenant_tokens>
#[derive(Debug, Serialize)]
struct TenantTokenClaims {
    /// Search rules that restrict which documents the token holder can query.
    #[serde(rename = "searchRules")]
    search_rules: serde_json::Value,
    /// The UID of the API key used to sign this token.
    #[serde(rename = "apiKeyUid")]
    api_key_uid: String,
    /// Optional expiration timestamp (Unix seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    exp: Option<i64>,
}

/// Generate a Meilisearch tenant token that restricts search results
/// to documents at or below the given access level.
///
/// # Arguments
/// * `api_key` — The Meilisearch API key (used as the HMAC secret).
/// * `api_key_uid` — The UID of the API key (embedded in claims).
/// * `user_access_level` — The user's access level; the token will filter to `access_level <= N`.
/// * `expires_at` — Optional expiration time for the token.
pub fn generate_tenant_token(
    api_key: &str,
    api_key_uid: &str,
    user_access_level: AccessLevel,
    expires_at: Option<DateTime<Utc>>,
) -> Result<String, AppError> {
    let search_rules = serde_json::json!({
        "documents": {
            "filter": format!("access_level <= {}", user_access_level as i32)
        }
    });

    let claims = TenantTokenClaims {
        search_rules,
        api_key_uid: api_key_uid.to_string(),
        exp: expires_at.map(|t| t.timestamp()),
    };

    let header = Header::default(); // HS256
    let key = EncodingKey::from_secret(api_key.as_bytes());

    encode(&header, &claims, &key)
        .map_err(|e| AppError::Internal(format!("Failed to generate tenant token: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_generate_tenant_token_structure() {
        let token = generate_tenant_token(
            "my-secret-api-key",
            "key-uid-123",
            AccessLevel::Developer,
            None,
        )
        .unwrap();

        // JWT has 3 parts separated by dots
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);

        // Decode the payload (middle part)
        use base64::Engine;
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();

        assert_eq!(payload["apiKeyUid"], "key-uid-123");
        assert_eq!(
            payload["searchRules"]["documents"]["filter"],
            "access_level <= 1"
        );
        // No expiry set
        assert!(payload.get("exp").is_none());
    }

    #[test]
    fn test_generate_tenant_token_with_expiry() {
        let expires = Utc::now() + Duration::hours(1);
        let token = generate_tenant_token(
            "my-secret-api-key",
            "key-uid-123",
            AccessLevel::Admin,
            Some(expires),
        )
        .unwrap();

        let parts: Vec<&str> = token.split('.').collect();
        use base64::Engine;
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();

        assert_eq!(
            payload["searchRules"]["documents"]["filter"],
            "access_level <= 3"
        );
        assert!(payload["exp"].is_i64());
    }

    #[test]
    fn test_generate_tenant_token_public_access() {
        let token = generate_tenant_token(
            "my-secret-api-key",
            "key-uid-123",
            AccessLevel::Public,
            None,
        )
        .unwrap();

        let parts: Vec<&str> = token.split('.').collect();
        use base64::Engine;
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();

        assert_eq!(
            payload["searchRules"]["documents"]["filter"],
            "access_level <= 0"
        );
    }
}
