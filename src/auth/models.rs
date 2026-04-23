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

/// Full request context: identity + pre-computed access level set.
///
/// Constructed by the Axum auth extractor after validating the JWT and
/// loading the user document from MongoDB.
/// Server-side only (`ssr` feature).
#[cfg(feature = "ssr")]
#[derive(Debug, Clone)]
pub struct UserContext {
    /// Authenticated identity.
    pub user: AuthenticatedUser,
    /// Pre-computed transitive closure of the user's assigned access levels.
    /// Does not include the implicitly injected `"public"` and `"loggeduser"` levels.
    pub effective_access_levels: Vec<String>,
    /// May create or update published documents at any accessible level.
    pub can_write: bool,
    /// May read draft documents at any accessible level.
    pub can_read_draft: bool,
    /// May create or update draft documents at any accessible level.
    pub can_write_draft: bool,
}

#[cfg(feature = "ssr")]
impl UserContext {
    /// Build a `UserContext` from a loaded [`User`](crate::db::auth_models::User) document.
    pub fn from_user_doc(
        auth_user: AuthenticatedUser,
        user: &crate::db::auth_models::User,
    ) -> Self {
        Self {
            user: auth_user,
            effective_access_levels: user.effective_access_levels.clone(),
            can_write: user.can_write,
            can_read_draft: user.can_read_draft,
            can_write_draft: user.can_write_draft,
        }
    }

    /// Returns the full set of levels the user can read: effective levels plus
    /// the implicitly granted `"public"` and `"loggeduser"` levels.
    fn accessible_levels(&self) -> std::collections::HashSet<&str> {
        let mut set: std::collections::HashSet<&str> = self
            .effective_access_levels
            .iter()
            .map(|s| s.as_str())
            .collect();
        set.insert("public");
        set.insert("loggeduser");
        set
    }

    /// Returns `true` if the user may read published documents at `level`.
    pub fn can_read(&self, level: &str) -> bool {
        if self.user.is_admin {
            return true;
        }
        self.accessible_levels().contains(level)
    }

    /// Returns `true` if the user may create/update documents at `level`.
    pub fn can_write(&self, level: &str) -> bool {
        if self.user.is_admin {
            return true;
        }
        self.can_write && self.accessible_levels().contains(level)
    }

    /// Returns `true` if the user may read draft documents at `level`.
    pub fn can_read_draft(&self, level: &str) -> bool {
        if self.user.is_admin {
            return true;
        }
        self.can_read_draft && self.accessible_levels().contains(level)
    }

    /// Returns `true` if the user may write draft documents at `level`.
    pub fn can_write_draft(&self, level: &str) -> bool {
        if self.user.is_admin {
            return true;
        }
        self.can_write_draft && self.accessible_levels().contains(level)
    }

    /// Collects the access level names the user can read (published docs).
    ///
    /// Returns `None` for admin users (meaning: no restriction).
    /// For authenticated users, includes `"public"` and `"loggeduser"` in addition
    /// to the pre-computed effective levels.
    pub fn readable_levels(&self) -> Option<Vec<String>> {
        if self.user.is_admin {
            return None;
        }
        let mut levels: Vec<String> = self.effective_access_levels.clone();
        if !levels.contains(&"public".to_string()) {
            levels.push("public".to_string());
        }
        if !levels.contains(&"loggeduser".to_string()) {
            levels.push("loggeduser".to_string());
        }
        Some(levels)
    }

    /// Returns `(readable_levels, include_draft)` suitable for passing to
    /// `DocumentRepository::list_by_access_levels` or `SearchService::search`.
    ///
    /// Admin → `(None, true)` (see everything).
    /// Authenticated user → effective levels + `"public"` + `"loggeduser"`.
    pub fn document_visibility(&self) -> (Option<Vec<String>>, bool) {
        if self.user.is_admin {
            return (None, true);
        }
        (self.readable_levels(), self.can_read_draft)
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

        fn make_context(
            is_admin: bool,
            effective_levels: Vec<String>,
            can_write: bool,
            can_read_draft: bool,
        ) -> UserContext {
            UserContext {
                user: AuthenticatedUser {
                    user_id: "u1".to_string(),
                    email: "u@test.com".to_string(),
                    name: None,
                    is_admin,
                },
                effective_access_levels: effective_levels,
                can_write,
                can_read_draft,
                can_write_draft: false,
            }
        }

        #[test]
        fn test_admin_bypasses_all_checks() {
            let ctx = make_context(true, vec![], false, false);
            assert!(ctx.can_read("anything"));
            assert!(ctx.can_write("anything"));
            assert!(ctx.can_read_draft("anything"));
            assert_eq!(ctx.readable_levels(), None);
        }

        #[test]
        fn test_regular_user_respects_effective_levels() {
            let ctx = make_context(false, vec!["internal".to_string()], true, true);
            assert!(ctx.can_read("public")); // implicit
            assert!(ctx.can_read("loggeduser")); // implicit
            assert!(ctx.can_read("internal")); // effective
            assert!(!ctx.can_read("secret")); // not accessible
            assert!(ctx.can_write("internal")); // can_write=true + accessible
            assert!(!ctx.can_write("secret")); // not accessible
        }

        #[test]
        fn test_readable_levels_always_includes_implicit() {
            let ctx = make_context(false, vec!["internal".to_string()], false, false);
            let levels = ctx.readable_levels().unwrap();
            assert!(levels.contains(&"public".to_string()));
            assert!(levels.contains(&"loggeduser".to_string()));
            assert!(levels.contains(&"internal".to_string()));
        }

        #[test]
        fn test_document_visibility_no_draft() {
            let ctx = make_context(false, vec!["internal".to_string()], false, false);
            let (_, include_draft) = ctx.document_visibility();
            assert!(!include_draft);
        }

        #[test]
        fn test_document_visibility_with_draft() {
            let ctx = make_context(false, vec!["internal".to_string()], false, true);
            let (levels, include_draft) = ctx.document_visibility();
            let levels = levels.unwrap();
            assert!(levels.contains(&"internal".to_string()));
            assert!(include_draft);
        }
    }
}
