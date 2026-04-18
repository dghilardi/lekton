//! Public-facing auth identity types.
//!
//! These types are available in both SSR and hydrate builds (they appear
//! as server-function return values visible to the Leptos client).

use serde::{Deserialize, Serialize};

/// Error sentinel returned by server functions when the caller is not
/// authenticated (expired or missing access token).
///
/// The client uses this exact string to distinguish a "needs refresh"
/// condition from other errors.  Using a constant prevents typo-divergence
/// between the server helpers that emit it and the client code that detects it.
pub const UNAUTHORIZED_SENTINEL: &str = "unauthorized";

/// Minimal user identity carried in the JWT and returned to clients.
///
/// Does **not** include permissions — those are loaded from the database on
/// each request by the auth extractor and held in [`UserContext`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthenticatedUser {
    /// Internal user UUID.
    pub user_id: String,
    /// Email address from the provider profile.
    pub email: String,
    /// Display name from the provider profile.
    pub name: Option<String>,
    /// When `true`, all permission checks are bypassed.
    pub is_admin: bool,
}

/// Full request context: identity + loaded RBAC permissions.
///
/// Constructed by the Axum auth extractor after validating the JWT and
/// fetching the user's permission records from MongoDB.
/// Server-side only (`ssr` feature).
#[cfg(feature = "ssr")]
#[derive(Debug, Clone)]
pub struct UserContext {
    /// Authenticated identity.
    pub user: AuthenticatedUser,
    /// The user's RBAC permissions, one record per access level.
    pub permissions: Vec<crate::db::auth_models::UserPermission>,
}

#[cfg(feature = "ssr")]
impl UserContext {
    /// Returns `true` if the user may read published documents at `level`.
    pub fn can_read(&self, level: &str) -> bool {
        if self.user.is_admin {
            return true;
        }
        self.permissions
            .iter()
            .any(|p| p.access_level_name == level && p.can_read)
    }

    /// Returns `true` if the user may create/update documents at `level`.
    pub fn can_write(&self, level: &str) -> bool {
        if self.user.is_admin {
            return true;
        }
        self.permissions
            .iter()
            .any(|p| p.access_level_name == level && p.can_write)
    }

    /// Returns `true` if the user may read draft documents at `level`.
    pub fn can_read_draft(&self, level: &str) -> bool {
        if self.user.is_admin {
            return true;
        }
        self.permissions
            .iter()
            .any(|p| p.access_level_name == level && p.can_read_draft)
    }

    /// Returns `true` if the user may write draft documents at `level`.
    pub fn can_write_draft(&self, level: &str) -> bool {
        if self.user.is_admin {
            return true;
        }
        self.permissions
            .iter()
            .any(|p| p.access_level_name == level && p.can_write_draft)
    }

    /// Collects the access level names the user can read (published docs).
    ///
    /// Returns `None` for admin users (meaning: no restriction).
    pub fn readable_levels(&self) -> Option<Vec<String>> {
        if self.user.is_admin {
            return None;
        }
        Some(
            self.permissions
                .iter()
                .filter(|p| p.can_read)
                .map(|p| p.access_level_name.clone())
                .collect(),
        )
    }

    /// Returns `(readable_levels, include_draft)` suitable for passing to
    /// `DocumentRepository::list_by_access_levels` or `SearchService::search`.
    ///
    /// Admin → `(None, true)` (see everything).
    pub fn document_visibility(&self) -> (Option<Vec<String>>, bool) {
        if self.user.is_admin {
            return (None, true);
        }
        let levels: Vec<String> = self
            .permissions
            .iter()
            .filter(|p| p.can_read)
            .map(|p| p.access_level_name.clone())
            .collect();
        let include_draft = self.permissions.iter().any(|p| p.can_read_draft);
        (Some(levels), include_draft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unauthorized_sentinel_is_non_empty() {
        assert!(!UNAUTHORIZED_SENTINEL.is_empty());
    }

    #[test]
    fn test_unauthorized_sentinel_is_stable() {
        // The sentinel value is part of the client/server contract.
        // If you change it, update refresh_client.rs too.
        assert_eq!(UNAUTHORIZED_SENTINEL, "unauthorized");
    }

    #[test]
    fn test_authenticated_user_roundtrip() {
        let user = AuthenticatedUser {
            user_id: "u1".to_string(),
            email: "alice@example.com".to_string(),
            name: Some("Alice".to_string()),
            is_admin: false,
        };
        let json = serde_json::to_string(&user).unwrap();
        let de: AuthenticatedUser = serde_json::from_str(&json).unwrap();
        assert_eq!(de.user_id, "u1");
        assert_eq!(de.email, "alice@example.com");
        assert!(!de.is_admin);
    }

    #[test]
    fn test_authenticated_user_admin_roundtrip() {
        let user = AuthenticatedUser {
            user_id: "admin1".to_string(),
            email: "admin@example.com".to_string(),
            name: None,
            is_admin: true,
        };
        let json = serde_json::to_string(&user).unwrap();
        let de: AuthenticatedUser = serde_json::from_str(&json).unwrap();
        assert!(de.is_admin);
    }

    #[cfg(feature = "ssr")]
    mod context_tests {
        use super::*;
        use crate::db::auth_models::UserPermission;

        fn make_perm(level: &str, read: bool, write: bool, read_draft: bool) -> UserPermission {
            UserPermission {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: "u1".to_string(),
                access_level_name: level.to_string(),
                can_read: read,
                can_write: write,
                can_read_draft: read_draft,
                can_write_draft: false,
            }
        }

        fn make_context(is_admin: bool, perms: Vec<UserPermission>) -> UserContext {
            UserContext {
                user: AuthenticatedUser {
                    user_id: "u1".to_string(),
                    email: "u@test.com".to_string(),
                    name: None,
                    is_admin,
                },
                permissions: perms,
            }
        }

        #[test]
        fn test_admin_bypasses_all_checks() {
            let ctx = make_context(true, vec![]);
            assert!(ctx.can_read("anything"));
            assert!(ctx.can_write("anything"));
            assert!(ctx.can_read_draft("anything"));
            assert_eq!(ctx.readable_levels(), None);
        }

        #[test]
        fn test_regular_user_respects_permissions() {
            let ctx = make_context(
                false,
                vec![
                    make_perm("public", true, false, false),
                    make_perm("internal", true, true, true),
                ],
            );
            assert!(ctx.can_read("public"));
            assert!(!ctx.can_write("public"));
            assert!(ctx.can_write("internal"));
            assert!(ctx.can_read_draft("internal"));
            assert!(!ctx.can_read("secret"));
        }

        #[test]
        fn test_readable_levels_excludes_write_only() {
            let ctx = make_context(
                false,
                vec![
                    make_perm("public", true, false, false),
                    make_perm("internal", false, true, false), // write-only, unusual but valid
                ],
            );
            let levels = ctx.readable_levels().unwrap();
            assert!(levels.contains(&"public".to_string()));
            assert!(!levels.contains(&"internal".to_string()));
        }

        #[test]
        fn test_document_visibility_includes_draft() {
            let ctx = make_context(
                false,
                vec![
                    make_perm("public", true, false, false),
                    make_perm("internal", true, false, true),
                ],
            );
            let (levels, include_draft) = ctx.document_visibility();
            let levels = levels.unwrap();
            assert!(levels.contains(&"public".to_string()));
            assert!(levels.contains(&"internal".to_string()));
            assert!(include_draft);
        }

        #[test]
        fn test_document_visibility_no_draft() {
            let ctx = make_context(false, vec![make_perm("public", true, false, false)]);
            let (_, include_draft) = ctx.document_visibility();
            assert!(!include_draft);
        }
    }
}
