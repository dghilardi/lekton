//! Authentication and authorization domain models stored in MongoDB.
//!
//! These models are server-side only (`ssr` feature). The client-facing
//! identity type lives in `crate::auth::models`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A configurable content access level stored in MongoDB.
///
/// Unlike the old hardcoded enum, access levels are fully dynamic: admins can
/// create, rename, or delete them. The built-in `"public"` level is marked
/// `is_system = true` and cannot be deleted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessLevelEntity {
    /// Machine-readable slug, e.g. `"public"`, `"internal"`, `"cloud-office"`.
    /// Used as the foreign key in documents and permission records.
    pub name: String,
    /// Human-readable label shown in the UI, e.g. `"Public"`.
    pub label: String,
    /// Optional description of the intended audience.
    pub description: String,
    /// Controls display order in admin UIs (lower = first).
    pub sort_order: u32,
    /// System levels cannot be deleted (protects `"public"`).
    pub is_system: bool,
    /// Creation timestamp.
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
    /// When this user record was first created (first login).
    pub created_at: DateTime<Utc>,
    /// Updated on every successful authentication.
    pub last_login_at: Option<DateTime<Utc>>,
}

/// RBAC permission grant for a user on a specific access level.
///
/// There is at most one `UserPermission` record per `(user_id, access_level_name)` pair.
/// Admin users bypass these checks entirely.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPermission {
    /// Internal UUID.
    pub id: String,
    /// References `User.id`.
    pub user_id: String,
    /// References `AccessLevelEntity.name`.
    pub access_level_name: String,
    /// May read published (non-draft) documents at this level.
    pub can_read: bool,
    /// May create or update documents at this level (implies `can_read`).
    pub can_write: bool,
    /// May read draft documents at this level.
    pub can_read_draft: bool,
    /// May create or update draft documents at this level.
    pub can_write_draft: bool,
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
    pub expires_at: DateTime<Utc>,
    /// Set when the token is explicitly revoked via logout.
    pub revoked_at: Option<DateTime<Utc>>,
    /// When the token was issued.
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
            sort_order: 20,
            is_system: false,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&level).unwrap();
        let de: AccessLevelEntity = serde_json::from_str(&json).unwrap();
        assert_eq!(de.name, "cloud-office");
        assert_eq!(de.label, "Cloud Office");
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
            created_at: Utc::now(),
            last_login_at: None,
        };
        let json = serde_json::to_string(&user).unwrap();
        let de: User = serde_json::from_str(&json).unwrap();
        assert_eq!(de.email, "alice@example.com");
        assert_eq!(de.name, Some("Alice".to_string()));
        assert!(!de.is_admin);
        assert_eq!(de.last_login_at, None);
    }

    #[test]
    fn test_user_permission_roundtrip() {
        let perm = UserPermission {
            id: "perm-uuid-1".to_string(),
            user_id: "user-uuid-1".to_string(),
            access_level_name: "internal".to_string(),
            can_read: true,
            can_write: false,
            can_read_draft: true,
            can_write_draft: false,
        };
        let json = serde_json::to_string(&perm).unwrap();
        let de: UserPermission = serde_json::from_str(&json).unwrap();
        assert_eq!(de.access_level_name, "internal");
        assert!(de.can_read);
        assert!(!de.can_write);
        assert!(de.can_read_draft);
        assert!(!de.can_write_draft);
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
