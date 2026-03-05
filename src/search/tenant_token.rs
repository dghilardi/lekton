use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;

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

/// Generate a Meilisearch tenant token that restricts search results to
/// documents belonging to the specified access levels.
///
/// # Arguments
/// * `api_key`       — The Meilisearch API key (used as the HMAC secret).
/// * `api_key_uid`   — The UID of the API key (embedded in claims).
/// * `allowed_levels` — The access level names the token holder may read
///                     (e.g. `["public", "internal"]`).
///                     Pass `None` to generate an unrestricted admin token.
/// * `include_draft` — Whether draft documents should be visible.
/// * `expires_at`    — Optional expiration time for the token.
pub fn generate_tenant_token(
    api_key: &str,
    api_key_uid: &str,
    allowed_levels: Option<&[String]>,
    include_draft: bool,
    expires_at: Option<DateTime<Utc>>,
) -> Result<String, AppError> {
    // Build a Meilisearch filter string for the search rules.
    let filter = build_filter(allowed_levels, include_draft);

    let search_rules = if filter.is_empty() {
        // No restrictions — allow everything.
        serde_json::json!({ "documents": {} })
    } else {
        serde_json::json!({ "documents": { "filter": filter } })
    };

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

/// Build the Meilisearch filter expression.
pub fn build_filter(allowed_levels: Option<&[String]>, include_draft: bool) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(levels) = allowed_levels {
        if levels.is_empty() {
            // No readable levels — force a filter that matches nothing.
            return "access_level IN []".to_string();
        }
        let quoted: Vec<String> = levels.iter().map(|l| format!("\"{}\"", l)).collect();
        parts.push(format!("access_level IN [{}]", quoted.join(", ")));
    }

    if !include_draft {
        parts.push("is_draft = false".to_string());
    }

    parts.join(" AND ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn decode_payload(token: &str) -> serde_json::Value {
        let parts: Vec<&str> = token.split('.').collect();
        use base64::Engine;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn test_build_filter_with_levels_no_draft() {
        let levels = vec!["public".to_string(), "internal".to_string()];
        let f = build_filter(Some(&levels), false);
        assert!(f.contains("access_level IN"));
        assert!(f.contains("\"public\""));
        assert!(f.contains("\"internal\""));
        assert!(f.contains("is_draft = false"));
    }

    #[test]
    fn test_build_filter_with_draft() {
        let levels = vec!["internal".to_string()];
        let f = build_filter(Some(&levels), true);
        assert!(f.contains("access_level IN"));
        assert!(!f.contains("is_draft"));
    }

    #[test]
    fn test_build_filter_admin_no_restrictions() {
        let f = build_filter(None, true);
        assert!(f.is_empty());
    }

    #[test]
    fn test_build_filter_empty_levels() {
        let f = build_filter(Some(&[]), false);
        assert_eq!(f, "access_level IN []");
    }

    #[test]
    fn test_generate_tenant_token_structure() {
        let levels = vec!["public".to_string(), "internal".to_string()];
        let token = generate_tenant_token(
            "my-secret-api-key",
            "key-uid-123",
            Some(&levels),
            false,
            None,
        )
        .unwrap();

        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT must have 3 parts");

        let payload = decode_payload(&token);
        assert_eq!(payload["apiKeyUid"], "key-uid-123");
        assert!(payload["searchRules"]["documents"]["filter"]
            .as_str()
            .unwrap()
            .contains("access_level IN"));
        assert!(payload.get("exp").is_none());
    }

    #[test]
    fn test_generate_tenant_token_with_expiry() {
        let expires = Utc::now() + Duration::hours(1);
        let token = generate_tenant_token(
            "my-secret-api-key",
            "key-uid-123",
            None,
            true,
            Some(expires),
        )
        .unwrap();

        let payload = decode_payload(&token);
        // Admin token: no filter in search rules
        assert!(payload["searchRules"]["documents"].get("filter").is_none());
        assert!(payload["exp"].is_i64());
    }

    #[test]
    fn test_generate_tenant_token_public_only() {
        let levels = vec!["public".to_string()];
        let token = generate_tenant_token(
            "my-secret-api-key",
            "key-uid-123",
            Some(&levels),
            false,
            None,
        )
        .unwrap();

        let payload = decode_payload(&token);
        let filter = payload["searchRules"]["documents"]["filter"].as_str().unwrap();
        assert!(filter.contains("\"public\""));
        assert!(filter.contains("is_draft = false"));
    }
}
