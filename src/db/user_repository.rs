//! Repository for `User`, `UserPermission`, and `RefreshToken` entities.
//!
//! All three entity types are managed through a single repository to keep
//! the `AppState` concise and to allow transactions in the future.

use async_trait::async_trait;
use chrono::Utc;

use crate::db::auth_models::{RefreshToken, User, UserPermission};
use crate::error::AppError;

/// Operations for users, their RBAC permissions, and refresh tokens.
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

    // ── Permissions ──────────────────────────────────────────────────────────

    /// Insert or replace the permission record for `(user_id, access_level_name)`.
    async fn upsert_permission(&self, perm: UserPermission) -> Result<(), AppError>;

    /// List all permission records for a user.
    async fn get_permissions(&self, user_id: &str) -> Result<Vec<UserPermission>, AppError>;

    /// Remove a single permission record.
    async fn delete_permission(
        &self,
        user_id: &str,
        access_level_name: &str,
    ) -> Result<(), AppError>;

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
    permissions: mongodb::Collection<UserPermission>,
    refresh_tokens: mongodb::Collection<RefreshToken>,
}

#[cfg(feature = "ssr")]
impl MongoUserRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            users: db.collection("users"),
            permissions: db.collection("user_permissions"),
            refresh_tokens: db.collection("refresh_tokens"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl UserRepository for MongoUserRepository {
    // ── Users ────────────────────────────────────────────────────────────────

    async fn create_user(&self, user: User) -> Result<(), AppError> {
        self.users
            .insert_one(&user)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    async fn find_user_by_id(&self, id: &str) -> Result<Option<User>, AppError> {
        use mongodb::bson::doc;
        self.users
            .find_one(doc! { "id": id })
            .await
            .map_err(|e| AppError::Database(e.to_string()))
    }

    async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        use mongodb::bson::doc;
        self.users
            .find_one(doc! { "email": email })
            .await
            .map_err(|e| AppError::Database(e.to_string()))
    }

    async fn find_user_by_provider_sub(
        &self,
        sub: &str,
        provider_type: &str,
    ) -> Result<Option<User>, AppError> {
        use mongodb::bson::doc;
        self.users
            .find_one(doc! { "provider_sub": sub, "provider_type": provider_type })
            .await
            .map_err(|e| AppError::Database(e.to_string()))
    }

    async fn touch_last_login(&self, user_id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let now = bson::DateTime::from_millis(Utc::now().timestamp_millis());
        self.users
            .update_one(
                doc! { "id": user_id },
                doc! { "$set": { "last_login_at": now } },
            )
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>, AppError> {
        use futures::TryStreamExt;

        let mut cursor = self
            .users
            .find(mongodb::bson::doc! {})
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut users = Vec::new();
        while let Some(u) = cursor
            .try_next()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        {
            users.push(u);
        }
        Ok(users)
    }

    // ── Permissions ──────────────────────────────────────────────────────────

    async fn upsert_permission(&self, perm: UserPermission) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::ReplaceOptions;

        let filter = doc! {
            "user_id": &perm.user_id,
            "access_level_name": &perm.access_level_name,
        };
        let options = ReplaceOptions::builder().upsert(true).build();

        self.permissions
            .replace_one(filter, &perm)
            .with_options(options)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_permissions(&self, user_id: &str) -> Result<Vec<UserPermission>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        let mut cursor = self
            .permissions
            .find(doc! { "user_id": user_id })
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut perms = Vec::new();
        while let Some(p) = cursor
            .try_next()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        {
            perms.push(p);
        }
        Ok(perms)
    }

    async fn delete_permission(
        &self,
        user_id: &str,
        access_level_name: &str,
    ) -> Result<(), AppError> {
        use mongodb::bson::doc;

        self.permissions
            .delete_one(doc! { "user_id": user_id, "access_level_name": access_level_name })
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    // ── Refresh tokens ───────────────────────────────────────────────────────

    async fn create_refresh_token(&self, token: RefreshToken) -> Result<(), AppError> {
        self.refresh_tokens
            .insert_one(&token)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    async fn find_refresh_token_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<RefreshToken>, AppError> {
        use mongodb::bson::doc;

        self.refresh_tokens
            .find_one(doc! { "token_hash": hash })
            .await
            .map_err(|e| AppError::Database(e.to_string()))
    }

    async fn revoke_refresh_token(&self, token_id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let now = bson::DateTime::from_millis(Utc::now().timestamp_millis());
        self.refresh_tokens
            .update_one(
                doc! { "id": token_id },
                doc! { "$set": { "revoked_at": now } },
            )
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
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
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
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
            created_at: Utc::now(),
            last_login_at: None,
        }
    }

    fn make_perm(user_id: &str, level: &str, can_read: bool, can_write: bool) -> UserPermission {
        UserPermission {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            access_level_name: level.to_string(),
            can_read,
            can_write,
            can_read_draft: false,
            can_write_draft: false,
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
        repo.create_user(make_user("u1", "a@test.com")).await.unwrap();

        let found = repo.find_user_by_provider_sub("sub-u1", "oidc").await.unwrap();
        assert!(found.is_some());

        let not_found = repo.find_user_by_provider_sub("sub-u1", "oauth2").await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_touch_last_login() {
        let repo = MockUserRepository::default();
        repo.create_user(make_user("u1", "a@test.com")).await.unwrap();
        assert!(repo.find_user_by_id("u1").await.unwrap().unwrap().last_login_at.is_none());

        repo.touch_last_login("u1").await.unwrap();
        assert!(repo.find_user_by_id("u1").await.unwrap().unwrap().last_login_at.is_some());
    }

    // ── Permission tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_upsert_permission_creates_new() {
        let repo = MockUserRepository::default();
        repo.upsert_permission(make_perm("u1", "public", true, false)).await.unwrap();

        let perms = repo.get_permissions("u1").await.unwrap();
        assert_eq!(perms.len(), 1);
        assert!(perms[0].can_read);
    }

    #[tokio::test]
    async fn test_upsert_permission_replaces_existing() {
        let repo = MockUserRepository::default();
        repo.upsert_permission(make_perm("u1", "public", true, false)).await.unwrap();
        repo.upsert_permission(make_perm("u1", "public", true, true)).await.unwrap();

        let perms = repo.get_permissions("u1").await.unwrap();
        assert_eq!(perms.len(), 1, "upsert must not duplicate");
        assert!(perms[0].can_write);
    }

    #[tokio::test]
    async fn test_delete_permission() {
        let repo = MockUserRepository::default();
        repo.upsert_permission(make_perm("u1", "public", true, false)).await.unwrap();
        repo.upsert_permission(make_perm("u1", "internal", true, false)).await.unwrap();

        repo.delete_permission("u1", "internal").await.unwrap();
        let perms = repo.get_permissions("u1").await.unwrap();
        assert_eq!(perms.len(), 1);
        assert_eq!(perms[0].access_level_name, "public");
    }

    // ── Refresh token tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_find_refresh_token_by_hash() {
        let repo = MockUserRepository::default();
        repo.create_refresh_token(make_token("u1", "hash-abc", true)).await.unwrap();

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
        let found = repo.find_refresh_token_by_hash("hash-xyz").await.unwrap().unwrap();
        assert!(!found.is_valid());
    }

    #[tokio::test]
    async fn test_revoke_all_user_tokens() {
        let repo = MockUserRepository::default();
        repo.create_refresh_token(make_token("u1", "hash-1", true)).await.unwrap();
        repo.create_refresh_token(make_token("u1", "hash-2", true)).await.unwrap();
        repo.create_refresh_token(make_token("u2", "hash-3", true)).await.unwrap();

        repo.revoke_all_user_tokens("u1").await.unwrap();

        assert!(!repo.find_refresh_token_by_hash("hash-1").await.unwrap().unwrap().is_valid());
        assert!(!repo.find_refresh_token_by_hash("hash-2").await.unwrap().unwrap().is_valid());
        // u2's token should be unaffected
        assert!(repo.find_refresh_token_by_hash("hash-3").await.unwrap().unwrap().is_valid());
    }
}
