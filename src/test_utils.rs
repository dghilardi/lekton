//! Shared mock implementations for unit tests.
//!
//! This module provides reusable in-memory mocks for the core traits,
//! eliminating duplication across test modules.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::AppError;

// ── MockStorage ────────────────────────────────────────────────────────────

/// In-memory mock for [`StorageClient`](crate::storage::client::StorageClient).
pub struct MockStorage {
    pub objects: Mutex<HashMap<String, Vec<u8>>>,
    /// Number of `put_object` calls (for verifying upload was skipped/performed).
    pub put_count: std::sync::atomic::AtomicU32,
}

impl MockStorage {
    pub fn new() -> Self {
        Self {
            objects: Mutex::new(HashMap::new()),
            put_count: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

impl Default for MockStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl crate::storage::client::StorageClient for MockStorage {
    async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), AppError> {
        self.put_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.objects
            .lock()
            .unwrap()
            .insert(key.to_string(), content);
        Ok(())
    }

    async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, AppError> {
        Ok(self.objects.lock().unwrap().get(key).cloned())
    }

    async fn delete_object(&self, key: &str) -> Result<(), AppError> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }
}

// ── MockUserRepository ─────────────────────────────────────────────────────

use crate::db::auth_models::{RefreshToken, UserPermission};
use crate::db::user_repository::UserRepository;
use chrono::Utc;

/// In-memory mock for [`UserRepository`].
#[derive(Default)]
pub struct MockUserRepository {
    pub users: Mutex<Vec<crate::db::auth_models::User>>,
    pub permissions: Mutex<Vec<UserPermission>>,
    pub tokens: Mutex<Vec<RefreshToken>>,
}

#[async_trait]
impl UserRepository for MockUserRepository {
    async fn create_user(&self, user: crate::db::auth_models::User) -> Result<(), AppError> {
        self.users.lock().unwrap().push(user);
        Ok(())
    }

    async fn find_user_by_id(
        &self,
        id: &str,
    ) -> Result<Option<crate::db::auth_models::User>, AppError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .iter()
            .find(|u| u.id == id)
            .cloned())
    }

    async fn find_user_by_email(
        &self,
        email: &str,
    ) -> Result<Option<crate::db::auth_models::User>, AppError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .iter()
            .find(|u| u.email == email)
            .cloned())
    }

    async fn find_user_by_provider_sub(
        &self,
        sub: &str,
        provider_type: &str,
    ) -> Result<Option<crate::db::auth_models::User>, AppError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .iter()
            .find(|u| u.provider_sub == sub && u.provider_type == provider_type)
            .cloned())
    }

    async fn touch_last_login(&self, user_id: &str) -> Result<(), AppError> {
        let mut users = self.users.lock().unwrap();
        if let Some(u) = users.iter_mut().find(|u| u.id == user_id) {
            u.last_login_at = Some(Utc::now());
        }
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<crate::db::auth_models::User>, AppError> {
        Ok(self.users.lock().unwrap().clone())
    }

    async fn upsert_permission(&self, perm: UserPermission) -> Result<(), AppError> {
        let mut perms = self.permissions.lock().unwrap();
        perms.retain(|p| {
            !(p.user_id == perm.user_id && p.access_level_name == perm.access_level_name)
        });
        perms.push(perm);
        Ok(())
    }

    async fn get_permissions(&self, user_id: &str) -> Result<Vec<UserPermission>, AppError> {
        Ok(self
            .permissions
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn delete_permission(
        &self,
        user_id: &str,
        access_level_name: &str,
    ) -> Result<(), AppError> {
        self.permissions
            .lock()
            .unwrap()
            .retain(|p| !(p.user_id == user_id && p.access_level_name == access_level_name));
        Ok(())
    }

    async fn create_refresh_token(&self, token: RefreshToken) -> Result<(), AppError> {
        self.tokens.lock().unwrap().push(token);
        Ok(())
    }

    async fn find_refresh_token_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<RefreshToken>, AppError> {
        Ok(self
            .tokens
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.token_hash == hash)
            .cloned())
    }

    async fn revoke_refresh_token(&self, token_id: &str) -> Result<(), AppError> {
        let mut tokens = self.tokens.lock().unwrap();
        if let Some(t) = tokens.iter_mut().find(|t| t.id == token_id) {
            t.revoked_at = Some(Utc::now());
        }
        Ok(())
    }

    async fn revoke_all_user_tokens(&self, user_id: &str) -> Result<(), AppError> {
        let mut tokens = self.tokens.lock().unwrap();
        let now = Utc::now();
        for t in tokens
            .iter_mut()
            .filter(|t| t.user_id == user_id && t.revoked_at.is_none())
        {
            t.revoked_at = Some(now);
        }
        Ok(())
    }
}
