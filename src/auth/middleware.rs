//! Auth middleware utilities.
//!
//! The heavy lifting (JWT validation, user + permission loading) lives in
//! `crate::auth::extractor`. This module provides helpers used during the
//! OAuth2/OIDC callback to build or update the user record.

use crate::db::auth_models::User;

/// Build a new [`User`] from provider identity claims received during login.
///
/// The returned user has `is_admin = false` and no permissions; those are
/// assigned separately by an administrator.
pub fn build_user_from_claims(
    id: String,
    email: String,
    name: Option<String>,
    provider_sub: String,
    provider_type: &str,
) -> User {
    use chrono::Utc;
    User {
        id,
        email,
        name,
        provider_sub,
        provider_type: provider_type.to_string(),
        is_admin: false,
        created_at: Utc::now(),
        last_login_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_user_from_claims() {
        let user = build_user_from_claims(
            "u-uuid".to_string(),
            "dev@example.com".to_string(),
            Some("Dev User".to_string()),
            "sub-12345".to_string(),
            "oidc",
        );
        assert_eq!(user.email, "dev@example.com");
        assert_eq!(user.provider_type, "oidc");
        assert!(!user.is_admin);
        assert!(user.last_login_at.is_none());
    }

    #[test]
    fn test_build_user_without_name() {
        let user = build_user_from_claims(
            "u-uuid2".to_string(),
            "a@b.com".to_string(),
            None,
            "sub-oauth2".to_string(),
            "oauth2",
        );
        assert!(user.name.is_none());
        assert_eq!(user.provider_type, "oauth2");
    }
}
