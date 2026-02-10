use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents the access levels for RBAC in Lekton.
///
/// The order of variants matters: it defines the privilege hierarchy.
/// `Public` is the least privileged, `Admin` is the most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AccessLevel {
    /// Publicly accessible content.
    Public = 0,
    /// Internal developer documentation.
    Developer = 1,
    /// Architecture-level documentation.
    Architect = 2,
    /// Full administrative access.
    Admin = 3,
}

impl fmt::Display for AccessLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessLevel::Public => write!(f, "public"),
            AccessLevel::Developer => write!(f, "developer"),
            AccessLevel::Architect => write!(f, "architect"),
            AccessLevel::Admin => write!(f, "admin"),
        }
    }
}

impl AccessLevel {
    /// Parse an access level from a string (case-insensitive).
    pub fn from_str_ci(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "public" => Some(AccessLevel::Public),
            "developer" => Some(AccessLevel::Developer),
            "architect" => Some(AccessLevel::Architect),
            "admin" => Some(AccessLevel::Admin),
            _ => None,
        }
    }

    /// Returns `true` if `self` has at least the required access level.
    pub fn has_access(&self, required: AccessLevel) -> bool {
        *self >= required
    }
}

/// Represents an authenticated user extracted from OIDC claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    /// Unique user identifier from the OIDC provider.
    pub user_id: String,
    /// User email address.
    pub email: String,
    /// The user's access level, mapped from OIDC groups/claims.
    pub access_level: AccessLevel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_level_ordering() {
        assert!(AccessLevel::Admin > AccessLevel::Architect);
        assert!(AccessLevel::Architect > AccessLevel::Developer);
        assert!(AccessLevel::Developer > AccessLevel::Public);
    }

    #[test]
    fn test_has_access() {
        let admin = AccessLevel::Admin;
        let dev = AccessLevel::Developer;
        let public = AccessLevel::Public;

        assert!(admin.has_access(AccessLevel::Admin));
        assert!(admin.has_access(AccessLevel::Public));
        assert!(dev.has_access(AccessLevel::Developer));
        assert!(dev.has_access(AccessLevel::Public));
        assert!(!dev.has_access(AccessLevel::Admin));
        assert!(!public.has_access(AccessLevel::Developer));
        assert!(public.has_access(AccessLevel::Public));
    }

    #[test]
    fn test_from_str_ci() {
        assert_eq!(AccessLevel::from_str_ci("Public"), Some(AccessLevel::Public));
        assert_eq!(AccessLevel::from_str_ci("ADMIN"), Some(AccessLevel::Admin));
        assert_eq!(AccessLevel::from_str_ci("developer"), Some(AccessLevel::Developer));
        assert_eq!(AccessLevel::from_str_ci("Architect"), Some(AccessLevel::Architect));
        assert_eq!(AccessLevel::from_str_ci("unknown"), None);
    }

    #[test]
    fn test_display() {
        assert_eq!(AccessLevel::Public.to_string(), "public");
        assert_eq!(AccessLevel::Developer.to_string(), "developer");
        assert_eq!(AccessLevel::Architect.to_string(), "architect");
        assert_eq!(AccessLevel::Admin.to_string(), "admin");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let user = AuthenticatedUser {
            user_id: "user-123".to_string(),
            email: "test@example.com".to_string(),
            access_level: AccessLevel::Developer,
        };
        let json = serde_json::to_string(&user).unwrap();
        let deserialized: AuthenticatedUser = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.user_id, "user-123");
        assert_eq!(deserialized.email, "test@example.com");
        assert_eq!(deserialized.access_level, AccessLevel::Developer);
    }
}
