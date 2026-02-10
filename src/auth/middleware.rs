use crate::auth::models::{AccessLevel, AuthenticatedUser};

/// Extract the access level from OIDC claims.
///
/// This function maps OIDC groups/roles to Lekton's access levels.
/// The mapping convention is:
/// - Claims containing "admin" → Admin
/// - Claims containing "architect" → Architect
/// - Claims containing "developer" → Developer  
/// - Anything else → Public
pub fn map_claims_to_access_level(groups: &[String]) -> AccessLevel {
    // Return the highest access level found in the claims
    groups
        .iter()
        .filter_map(|g| AccessLevel::from_str_ci(g))
        .max()
        .unwrap_or(AccessLevel::Public)
}

/// Build an `AuthenticatedUser` from OIDC token claims.
pub fn build_authenticated_user(
    user_id: String,
    email: String,
    groups: &[String],
) -> AuthenticatedUser {
    let access_level = map_claims_to_access_level(groups);
    AuthenticatedUser {
        user_id,
        email,
        access_level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_claims_admin() {
        let groups = vec!["developer".to_string(), "admin".to_string()];
        assert_eq!(map_claims_to_access_level(&groups), AccessLevel::Admin);
    }

    #[test]
    fn test_map_claims_developer() {
        let groups = vec!["developer".to_string()];
        assert_eq!(map_claims_to_access_level(&groups), AccessLevel::Developer);
    }

    #[test]
    fn test_map_claims_empty() {
        let groups: Vec<String> = vec![];
        assert_eq!(map_claims_to_access_level(&groups), AccessLevel::Public);
    }

    #[test]
    fn test_map_claims_unknown() {
        let groups = vec!["some-random-group".to_string()];
        assert_eq!(map_claims_to_access_level(&groups), AccessLevel::Public);
    }

    #[test]
    fn test_build_authenticated_user() {
        let user = build_authenticated_user(
            "uid-1".to_string(),
            "dev@company.com".to_string(),
            &["developer".to_string(), "architect".to_string()],
        );
        assert_eq!(user.user_id, "uid-1");
        assert_eq!(user.email, "dev@company.com");
        assert_eq!(user.access_level, AccessLevel::Architect);
    }
}
