use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A scoped service token for programmatic API access.
///
/// Each token grants access to a specific set of document slugs or prefixes,
/// enabling CI pipelines from different repositories to write only to their
/// own namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceToken {
    /// Internal UUID (primary key).
    pub id: String,
    /// Human-readable name (unique). E.g. "iot-protocols-ci".
    pub name: String,
    /// SHA-256 hash of the raw token (base64url, no padding).
    pub token_hash: String,
    /// Scopes this token can access. Each entry is either:
    /// - An exact slug (e.g. `"guidelines/protocols"`)
    /// - A prefix pattern ending with `/*` (e.g. `"protocols/*"`)
    pub allowed_scopes: Vec<String>,
    /// Token type: `"service"` (scope-based CI/CD) or `"pat"` (inherits user permissions).
    #[serde(default = "default_service")]
    pub token_type: String,
    /// For PATs: the user whose permissions this token inherits. Empty for service tokens.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Whether this token can write (create/update) documents.
    /// Read access is implicit for all active tokens.
    #[serde(default)]
    pub can_write: bool,
    /// The admin user ID who created this token.
    pub created_by: String,
    /// When this token was created.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// When this token was last used for an API call.
    #[serde(default, with = "crate::db::auth_models::option_bson_datetime")]
    pub last_used_at: Option<DateTime<Utc>>,
    /// Whether this token is active. Deactivated tokens are rejected.
    #[serde(default = "default_true")]
    pub is_active: bool,
}

fn default_true() -> bool {
    true
}

fn default_service() -> String {
    "service".to_string()
}

impl ServiceToken {
    /// Returns `true` if this is a personal access token.
    pub fn is_pat(&self) -> bool {
        self.token_type == "pat"
    }
}

impl ServiceToken {
    /// Returns `true` if `slug` falls within any of this token's `allowed_scopes`.
    ///
    /// - Exact scope `"a/b"` matches only slug `"a/b"`.
    /// - Prefix scope `"a/*"` matches `"a"`, `"a/b"`, `"a/b/c"`, etc.
    pub fn matches_slug(&self, slug: &str) -> bool {
        self.allowed_scopes
            .iter()
            .any(|scope| scope_matches(scope, slug))
    }
}

/// Check whether a single scope entry matches a slug.
fn scope_matches(scope: &str, slug: &str) -> bool {
    if let Some(prefix) = scope.strip_suffix("/*") {
        // Prefix scope: "protocols/*" matches "protocols", "protocols/x", "protocols/x/y"
        slug == prefix || slug.starts_with(&format!("{prefix}/"))
    } else {
        // Exact scope
        scope == slug
    }
}

/// Returns `true` if any scope in `a` overlaps with any scope in `b`.
///
/// Two scopes overlap when a document slug could match both. Specifically:
/// - Two exact scopes overlap only if they are equal.
/// - An exact scope overlaps with a prefix scope if the prefix covers the exact slug.
/// - Two prefix scopes overlap if one is a prefix of the other (or they are equal).
pub fn scopes_overlap(a: &[String], b: &[String]) -> bool {
    a.iter().any(|sa| b.iter().any(|sb| pair_overlaps(sa, sb)))
}

/// Check whether two individual scope entries overlap.
fn pair_overlaps(a: &str, b: &str) -> bool {
    let a_prefix = a.strip_suffix("/*");
    let b_prefix = b.strip_suffix("/*");

    match (a_prefix, b_prefix) {
        // Both are prefix scopes: overlap if one is a prefix of the other
        (Some(pa), Some(pb)) => {
            pa == pb || pa.starts_with(&format!("{pb}/")) || pb.starts_with(&format!("{pa}/"))
        }
        // a is prefix, b is exact: overlap if b falls under a's prefix
        (Some(pa), None) => b == pa || b.starts_with(&format!("{pa}/")),
        // a is exact, b is prefix: symmetric
        (None, Some(pb)) => a == pb || a.starts_with(&format!("{pb}/")),
        // Both exact: overlap only if equal
        (None, None) => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token(scopes: Vec<&str>) -> ServiceToken {
        ServiceToken {
            id: "t1".to_string(),
            name: "test".to_string(),
            token_hash: "hash".to_string(),
            allowed_scopes: scopes.into_iter().map(String::from).collect(),
            token_type: "service".to_string(),
            user_id: None,
            can_write: true,
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            last_used_at: None,
            is_active: true,
        }
    }

    // ── scope_matches ────────────────────────────────────────────────────

    #[test]
    fn test_matches_slug_exact() {
        let token = make_token(vec!["guidelines/protocols"]);
        assert!(token.matches_slug("guidelines/protocols"));
        assert!(!token.matches_slug("guidelines/protocols/sub"));
        assert!(!token.matches_slug("guidelines"));
    }

    #[test]
    fn test_matches_slug_prefix() {
        let token = make_token(vec!["protocols/*"]);
        assert!(token.matches_slug("protocols/iot"));
        assert!(token.matches_slug("protocols/iot/sub"));
        assert!(token.matches_slug("protocols")); // the root itself
        assert!(!token.matches_slug("other/thing"));
        assert!(!token.matches_slug("protocolsX")); // no false prefix match
    }

    #[test]
    fn test_matches_slug_multiple_scopes() {
        let token = make_token(vec!["protocols/*", "guides/intro"]);
        assert!(token.matches_slug("protocols/x"));
        assert!(token.matches_slug("guides/intro"));
        assert!(!token.matches_slug("guides/other"));
    }

    #[test]
    fn test_matches_slug_no_match() {
        let token = make_token(vec!["protocols/*"]);
        assert!(!token.matches_slug("other/thing"));
        assert!(!token.matches_slug(""));
    }

    // ── scopes_overlap ───────────────────────────────────────────────────

    #[test]
    fn test_overlap_prefix_contains_prefix() {
        let a = vec!["protocols/*".to_string()];
        let b = vec!["protocols/iot/*".to_string()];
        assert!(scopes_overlap(&a, &b));
        assert!(scopes_overlap(&b, &a)); // symmetric
    }

    #[test]
    fn test_overlap_same_prefix() {
        let a = vec!["protocols/*".to_string()];
        let b = vec!["protocols/*".to_string()];
        assert!(scopes_overlap(&a, &b));
    }

    #[test]
    fn test_overlap_prefix_contains_exact() {
        let a = vec!["protocols/*".to_string()];
        let b = vec!["protocols/iot".to_string()];
        assert!(scopes_overlap(&a, &b));
        assert!(scopes_overlap(&b, &a));
    }

    #[test]
    fn test_overlap_exact_same() {
        let a = vec!["protocols/iot".to_string()];
        let b = vec!["protocols/iot".to_string()];
        assert!(scopes_overlap(&a, &b));
    }

    #[test]
    fn test_no_overlap_disjoint_prefixes() {
        let a = vec!["protocols/*".to_string()];
        let b = vec!["guides/*".to_string()];
        assert!(!scopes_overlap(&a, &b));
    }

    #[test]
    fn test_no_overlap_disjoint_exact() {
        let a = vec!["protocols/iot".to_string()];
        let b = vec!["guides/intro".to_string()];
        assert!(!scopes_overlap(&a, &b));
    }

    #[test]
    fn test_no_overlap_similar_names() {
        // "protocols" prefix should NOT overlap with "protocolsX"
        let a = vec!["protocols/*".to_string()];
        let b = vec!["protocolsX/*".to_string()];
        assert!(!scopes_overlap(&a, &b));
    }

    #[test]
    fn test_overlap_multiple_scopes_one_match() {
        let a = vec!["guides/*".to_string(), "protocols/*".to_string()];
        let b = vec!["other/*".to_string(), "protocols/iot".to_string()];
        assert!(scopes_overlap(&a, &b));
    }
}
