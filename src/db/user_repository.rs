//! Repository for `User` and `RefreshToken` entities.

use async_trait::async_trait;
#[cfg(feature = "ssr")]
use chrono::Utc;

use crate::db::auth_models::{RefreshToken, User};
use crate::error::AppError;

/// Operations for users and refresh tokens.
#[async_trait]
pub trait UserRepository: Send + Sync {
    // ── Users ────────────────────────────────────────────────────────────────

    /// Persist a new user record.
    async fn create_user(&self, user: User) -> Result<(), AppError>;

    /// Find a user by internal ID.
    async fn find_user_by_id(&self, id: &str) -> Result<Option<User>, AppError>;

    /// Find a user by email address.
    async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, AppError>;

    /// Find a user by their provider `sub` + provider type combination.
    async fn find_user_by_provider_sub(
        &self,
        sub: &str,
        provider_type: &str,
    ) -> Result<Option<User>, AppError>;

    /// Update `last_login_at` to now.
    async fn touch_last_login(&self, user_id: &str) -> Result<(), AppError>;

    /// List all users (admin endpoint).
    async fn list_users(&self) -> Result<Vec<User>, AppError>;

    /// Set the access levels for a user and update the pre-computed effective set.
    ///
    /// `assigned` is the set of levels explicitly assigned by an admin.
    /// `effective` is the transitive closure through the inheritance DAG,
    /// computed by [`AccessLevelRepository::compute_effective_levels`].
    async fn set_user_access_levels(
        &self,
        user_id: &str,
        assigned: Vec<String>,
        effective: Vec<String>,
        can_write: bool,
        can_read_draft: bool,
        can_write_draft: bool,
    ) -> Result<(), AppError>;

    /// Update only the pre-computed `effective_access_levels` for a user.
    /// Used by the background cascade-recompute job.
    async fn update_user_effective_levels(
        &self,
        user_id: &str,
        effective: Vec<String>,
    ) -> Result<(), AppError>;

    /// List all users that have `level_name` in their `assigned_access_levels`.
    /// Used by the background cascade-recompute job to find affected users.
    async fn list_users_with_assigned_level(&self, level_name: &str)
        -> Result<Vec<User>, AppError>;

    // ── Refresh tokens ───────────────────────────────────────────────────────

    /// Store a new refresh token.
    async fn create_refresh_token(&self, token: RefreshToken) -> Result<(), AppError>;

    /// Look up a token record by its hash.
    async fn find_refresh_token_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<RefreshToken>, AppError>;

    /// Mark a single token as revoked (soft-delete).
    async fn revoke_refresh_token(&self, token_id: &str) -> Result<(), AppError>;

    /// Revoke all active tokens for a user (used on admin disable or forced logout).
    async fn revoke_all_user_tokens(&self, user_id: &str) -> Result<(), AppError>;
}

// ── MongoDB implementation ────────────────────────────────────────────────────

#[cfg(feature = "ssr")]
pub struct MongoUserRepository {
    users: mongodb::Collection<User>,
    refresh_tokens: mongodb::Collection<RefreshToken>,
}

#[cfg(feature = "ssr")]
impl MongoUserRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            users: db.collection("users"),
            refresh_tokens: db.collection("refresh_tokens"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl UserRepository for MongoUserRepository {
    // ── Users ────────────────────────────────────────────────────────────────

    async fn create_user(&self, user: User) -> Result<(), AppError> {
        self.users.insert_one(&user).await?;
        Ok(())
    }

    async fn find_user_by_id(&self, id: &str) -> Result<Option<User>, AppError> {
        use mongodb::bson::doc;
        Ok(self.users.find_one(doc! { "id": id }).await?)
    }

    async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        use mongodb::bson::doc;
        Ok(self.users.find_one(doc! { "email": email }).await?)
    }

    async fn find_user_by_provider_sub(
        &self,
        sub: &str,
        provider_type: &str,
    ) -> Result<Option<User>, AppError> {
        use mongodb::bson::doc;
        Ok(self
            .users
            .find_one(doc! { "provider_sub": sub, "provider_type": provider_type })
            .await?)
    }

    async fn touch_last_login(&self, user_id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let now = bson::DateTime::from_millis(Utc::now().timestamp_millis());
        self.users
            .update_one(
                doc! { "id": user_id },
                doc! { "$set": { "last_login_at": now } },
            )
            .await?;
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>, AppError> {
        use futures::TryStreamExt;

        let mut cursor = self.users.find(mongodb::bson::doc! {}).await?;
        let mut users = Vec::new();
        while let Some(u) = cursor.try_next().await? {
            users.push(u);
        }
        Ok(users)
    }

    async fn set_user_access_levels(
        &self,
        user_id: &str,
        assigned: Vec<String>,
        effective: Vec<String>,
        can_write: bool,
        can_read_draft: bool,
        can_write_draft: bool,
    ) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let assigned_bson: Vec<bson::Bson> = assigned.into_iter().map(bson::Bson::String).collect();
        let effective_bson: Vec<bson::Bson> =
            effective.into_iter().map(bson::Bson::String).collect();

        let result = self
            .users
            .update_one(
                doc! { "id": user_id },
                doc! { "$set": {
                    "assigned_access_levels": assigned_bson,
                    "effective_access_levels": effective_bson,
                    "can_write": can_write,
                    "can_read_draft": can_read_draft,
                    "can_write_draft": can_write_draft,
                }},
            )
            .await?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!("User '{user_id}' not found")));
        }
        Ok(())
    }

    async fn update_user_effective_levels(
        &self,
        user_id: &str,
        effective: Vec<String>,
    ) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let effective_bson: Vec<bson::Bson> =
            effective.into_iter().map(bson::Bson::String).collect();

        self.users
            .update_one(
                doc! { "id": user_id },
                doc! { "$set": { "effective_access_levels": effective_bson } },
            )
            .await?;
        Ok(())
    }

    async fn list_users_with_assigned_level(
        &self,
        level_name: &str,
    ) -> Result<Vec<User>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        let mut cursor = self
            .users
            .find(doc! { "assigned_access_levels": level_name })
            .await?;
        let mut users = Vec::new();
        while let Some(u) = cursor.try_next().await? {
            users.push(u);
        }
        Ok(users)
    }

    // ── Refresh tokens ───────────────────────────────────────────────────────

    async fn create_refresh_token(&self, token: RefreshToken) -> Result<(), AppError> {
        self.refresh_tokens.insert_one(&token).await?;
        Ok(())
    }

    async fn find_refresh_token_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<RefreshToken>, AppError> {
        use mongodb::bson::doc;
        Ok(self
            .refresh_tokens
            .find_one(doc! { "token_hash": hash })
            .await?)
    }

    async fn revoke_refresh_token(&self, token_id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let now = bson::DateTime::from_millis(Utc::now().timestamp_millis());
        self.refresh_tokens
            .update_one(
                doc! { "id": token_id },
                doc! { "$set": { "revoked_at": now } },
            )
            .await?;
        Ok(())
    }

    async fn revoke_all_user_tokens(&self, user_id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let now = bson::DateTime::from_millis(Utc::now().timestamp_millis());
        self.refresh_tokens
            .update_many(
                doc! { "user_id": user_id, "revoked_at": { "$exists": false } },
                doc! { "$set": { "revoked_at": now } },
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::MockUserRepository;

    fn make_user(id: &str, email: &str) -> User {
        User {
            id: id.to_string(),
            email: email.to_string(),
            name: None,
            provider_sub: format!("sub-{}", id),
            provider_type: "oidc".to_string(),
            is_admin: false,
            assigned_access_levels: vec![],
            effective_access_levels: vec![],
            can_write: false,
            can_read_draft: false,
            can_write_draft: false,
            created_at: Utc::now(),
            last_login_at: None,
        }
    }

    fn make_token(user_id: &str, hash: &str, valid: bool) -> RefreshToken {
        RefreshToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            token_hash: hash.to_string(),
            expires_at: if valid {
                Utc::now() + chrono::Duration::days(30)
            } else {
                Utc::now() - chrono::Duration::seconds(1)
            },
            revoked_at: None,
            created_at: Utc::now(),
        }
    }

    // ── User tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_and_find_user_by_id() {
        let repo = MockUserRepository::default();
        let user = make_user("u1", "a@test.com");
        repo.create_user(user.clone()).await.unwrap();

        let found = repo.find_user_by_id("u1").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().email, "a@test.com");
    }

    #[tokio::test]
    async fn test_find_user_not_found() {
        let repo = MockUserRepository::default();
        let found = repo.find_user_by_id("missing").await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_find_user_by_provider_sub() {
        let repo = MockUserRepository::default();
        repo.create_user(make_user("u1", "a@test.com"))
            .await
            .unwrap();

        let found = repo
            .find_user_by_provider_sub("sub-u1", "oidc")
            .await
            .unwrap();
        assert!(found.is_some());

        let not_found = repo
            .find_user_by_provider_sub("sub-u1", "oauth2")
            .await
            .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_touch_last_login() {
        let repo = MockUserRepository::default();
        repo.create_user(make_user("u1", "a@test.com"))
            .await
            .unwrap();
        assert!(repo
            .find_user_by_id("u1")
            .await
            .unwrap()
            .unwrap()
            .last_login_at
            .is_none());

        repo.touch_last_login("u1").await.unwrap();
        assert!(repo
            .find_user_by_id("u1")
            .await
            .unwrap()
            .unwrap()
            .last_login_at
            .is_some());
    }

    #[tokio::test]
    async fn test_set_and_retrieve_access_levels() {
        let repo = MockUserRepository::default();
        repo.create_user(make_user("u1", "a@test.com"))
            .await
            .unwrap();

        repo.set_user_access_levels(
            "u1",
            vec!["developer".to_string()],
            vec!["developer".to_string(), "internal".to_string()],
            false,
            false,
            false,
        )
        .await
        .unwrap();

        let user = repo.find_user_by_id("u1").await.unwrap().unwrap();
        assert_eq!(user.assigned_access_levels, vec!["developer"]);
        assert_eq!(user.effective_access_levels, vec!["developer", "internal"]);
    }

    // ── Refresh token tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_find_refresh_token_by_hash() {
        let repo = MockUserRepository::default();
        repo.create_refresh_token(make_token("u1", "hash-abc", true))
            .await
            .unwrap();

        let found = repo.find_refresh_token_by_hash("hash-abc").await.unwrap();
        assert!(found.is_some());
        assert!(found.unwrap().is_valid());
    }

    #[tokio::test]
    async fn test_revoke_refresh_token() {
        let repo = MockUserRepository::default();
        let token = make_token("u1", "hash-xyz", true);
        let token_id = token.id.clone();
        repo.create_refresh_token(token).await.unwrap();

        repo.revoke_refresh_token(&token_id).await.unwrap();
        let found = repo
            .find_refresh_token_by_hash("hash-xyz")
            .await
            .unwrap()
            .unwrap();
        assert!(!found.is_valid());
    }

    #[tokio::test]
    async fn test_revoke_all_user_tokens() {
        let repo = MockUserRepository::default();
        repo.create_refresh_token(make_token("u1", "hash-1", true))
            .await
            .unwrap();
        repo.create_refresh_token(make_token("u1", "hash-2", true))
            .await
            .unwrap();
        repo.create_refresh_token(make_token("u2", "hash-3", true))
            .await
            .unwrap();

        repo.revoke_all_user_tokens("u1").await.unwrap();

        assert!(!repo
            .find_refresh_token_by_hash("hash-1")
            .await
            .unwrap()
            .unwrap()
            .is_valid());
        assert!(!repo
            .find_refresh_token_by_hash("hash-2")
            .await
            .unwrap()
            .unwrap()
            .is_valid());
        // u2's token should be unaffected
        assert!(repo
            .find_refresh_token_by_hash("hash-3")
            .await
            .unwrap()
            .unwrap()
            .is_valid());
    }
}
