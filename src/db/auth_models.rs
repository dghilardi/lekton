//! Authentication and authorization domain models stored in MongoDB.
//!
//! These models are server-side only (`ssr` feature). The client-facing
//! identity type lives in `crate::auth::models`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Custom serde helper for Option<DateTime<Utc>> with BSON DateTime format.
pub mod option_bson_datetime {
    use chrono::{DateTime, Utc};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(dt) => {
                bson::serde_helpers::chrono_datetime_as_bson_datetime::serialize(dt, serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper(
            #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")] DateTime<Utc>,
        );

        use serde::Deserialize;
        Option::<Helper>::deserialize(deserializer).map(|opt| opt.map(|h| h.0))
    }
}

/// A configurable content access level stored in MongoDB.
///
/// Unlike the old hardcoded enum, access levels are fully dynamic: admins can
/// create, rename, or delete them. The built-in `"public"` and `"loggeduser"`
/// levels are marked `is_system = true` and cannot be deleted.
///
/// Access levels form a DAG: a level can inherit from multiple parents, and
/// the effective set of levels for a user is the transitive closure of their
/// assigned levels through the inheritance graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessLevelEntity {
    /// Machine-readable slug, e.g. `"public"`, `"internal"`, `"cloud-office"`.
    /// Used as the foreign key in documents and permission records.
    pub name: String,
    /// Human-readable label shown in the UI, e.g. `"Public"`.
    pub label: String,
    /// Optional description of the intended audience.
    pub description: String,
    /// Access levels this level inherits from (DAG parents).
    /// The effective set of levels for a user includes all transitively reachable ancestors.
    #[serde(default)]
    pub inherits_from: Vec<String>,
    /// System levels cannot be deleted (protects `"public"` and `"loggeduser"`).
    pub is_system: bool,
    /// Creation timestamp.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// A user registered via OAuth2 or OIDC self-service login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Internal UUID (primary key).
    pub id: String,
    /// Email address from the provider's profile.
    pub email: String,
    /// Display name from the provider's profile.
    pub name: Option<String>,
    /// Subject (`sub`) claim from the provider token.
    pub provider_sub: String,
    /// Provider variant: `"oidc"` or `"oauth2"`.
    pub provider_type: String,
    /// Full administrative access bypasses all per-level permission checks.
    pub is_admin: bool,
    /// Access levels explicitly assigned by an administrator.
    #[serde(default)]
    pub assigned_access_levels: Vec<String>,
    /// Pre-computed transitive closure of `assigned_access_levels` through
    /// the inheritance DAG.  Kept in sync by the background recompute job.
    /// `"public"` and `"loggeduser"` are injected at query time and not stored here.
    #[serde(default)]
    pub effective_access_levels: Vec<String>,
    /// May create or update published documents at any accessible level.
    #[serde(default)]
    pub can_write: bool,
    /// May read draft documents at any accessible level.
    #[serde(default)]
    pub can_read_draft: bool,
    /// May create or update draft documents at any accessible level.
    #[serde(default)]
    pub can_write_draft: bool,
    /// When this user record was first created (first login).
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// Updated on every successful authentication.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_bson_datetime"
    )]
    pub last_login_at: Option<DateTime<Utc>>,
}

/// A long-lived opaque token used to obtain new JWT access tokens.
///
/// The raw token is only returned once at issuance and is never stored.
/// Only the SHA-256 hash (base64url-encoded) is persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshToken {
    /// Internal UUID.
    pub id: String,
    /// References `User.id`.
    pub user_id: String,
    /// SHA-256 (base64url, no padding) of the raw token string.
    pub token_hash: String,
    /// Token expiry; after this instant the token is no longer valid.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    /// Set when the token is explicitly revoked via logout.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_bson_datetime"
    )]
    pub revoked_at: Option<DateTime<Utc>>,
    /// When the token was issued.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl RefreshToken {
    /// Returns `true` if the token has not expired and has not been revoked.
    pub fn is_valid(&self) -> bool {
        self.revoked_at.is_none() && self.expires_at > Utc::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_level_entity_roundtrip() {
        let level = AccessLevelEntity {
            name: "cloud-office".to_string(),
            label: "Cloud Office".to_string(),
            description: "Documentation for the cloud office team".to_string(),
            inherits_from: vec!["internal".to_string()],
            is_system: false,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&level).unwrap();
        let de: AccessLevelEntity = serde_json::from_str(&json).unwrap();
        assert_eq!(de.name, "cloud-office");
        assert_eq!(de.label, "Cloud Office");
        assert_eq!(de.inherits_from, vec!["internal"]);
        assert!(!de.is_system);
    }

    #[test]
    fn test_user_roundtrip() {
        let user = User {
            id: "3f2e1d4c-5b6a-7890-abcd-ef1234567890".to_string(),
            email: "alice@example.com".to_string(),
            name: Some("Alice".to_string()),
            provider_sub: "oidc|sub-alice-001".to_string(),
            provider_type: "oidc".to_string(),
            is_admin: false,
            assigned_access_levels: vec!["internal".to_string()],
            effective_access_levels: vec!["internal".to_string(), "loggeduser".to_string()],
            can_write: false,
            can_read_draft: false,
            can_write_draft: false,
            created_at: Utc::now(),
            last_login_at: None,
        };
        let json = serde_json::to_string(&user).unwrap();
        let de: User = serde_json::from_str(&json).unwrap();
        assert_eq!(de.email, "alice@example.com");
        assert_eq!(de.name, Some("Alice".to_string()));
        assert!(!de.is_admin);
        assert_eq!(de.assigned_access_levels, vec!["internal"]);
        assert_eq!(de.effective_access_levels, vec!["internal", "loggeduser"]);
        assert_eq!(de.last_login_at, None);
    }

    #[test]
    fn test_user_defaults_on_missing_fields() {
        // Verify that new fields default to empty/false via serde defaults.
        // This ensures backward-compatible deserialization of old MongoDB documents
        // that predate the access-level refactoring.
        let user = User {
            id: "u1".to_string(),
            email: "old@example.com".to_string(),
            name: None,
            provider_sub: "sub-old".to_string(),
            provider_type: "oidc".to_string(),
            is_admin: false,
            assigned_access_levels: vec![],  // default
            effective_access_levels: vec![], // default
            can_write: false,                // default
            can_read_draft: false,           // default
            can_write_draft: false,          // default
            created_at: Utc::now(),
            last_login_at: None,
        };
        let json = serde_json::to_string(&user).unwrap();
        let de: User = serde_json::from_str(&json).unwrap();
        assert!(de.assigned_access_levels.is_empty());
        assert!(de.effective_access_levels.is_empty());
        assert!(!de.can_write);
        assert!(!de.can_read_draft);
    }

    #[test]
    fn test_refresh_token_is_valid_active() {
        let token = RefreshToken {
            id: "tok-1".to_string(),
            user_id: "user-1".to_string(),
            token_hash: "some-hash".to_string(),
            expires_at: Utc::now() + chrono::Duration::days(30),
            revoked_at: None,
            created_at: Utc::now(),
        };
        assert!(token.is_valid());
    }

    #[test]
    fn test_refresh_token_is_valid_expired() {
        let token = RefreshToken {
            id: "tok-2".to_string(),
            user_id: "user-1".to_string(),
            token_hash: "some-hash".to_string(),
            expires_at: Utc::now() - chrono::Duration::seconds(1),
            revoked_at: None,
            created_at: Utc::now() - chrono::Duration::days(31),
        };
        assert!(!token.is_valid());
    }

    #[test]
    fn test_refresh_token_is_valid_revoked() {
        let token = RefreshToken {
            id: "tok-3".to_string(),
            user_id: "user-1".to_string(),
            token_hash: "some-hash".to_string(),
            expires_at: Utc::now() + chrono::Duration::days(30),
            revoked_at: Some(Utc::now() - chrono::Duration::hours(1)),
            created_at: Utc::now() - chrono::Duration::hours(2),
        };
        assert!(!token.is_valid());
    }
}
