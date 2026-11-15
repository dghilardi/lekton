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

use crate::db::auth_models::RefreshToken;
use crate::db::user_repository::UserRepository;
use chrono::Utc;

/// In-memory mock for [`UserRepository`].
#[derive(Default)]
pub struct MockUserRepository {
    pub users: Mutex<Vec<crate::db::auth_models::User>>,
    pub tokens: Mutex<Vec<RefreshToken>>,
}

#[async_trait]
impl UserRepository for MockUserRepository {
    async fn set_user_budget_plan(
        &self,
        _user_id: &str,
        _plan: Option<String>,
    ) -> Result<(), AppError> {
        Ok(())
    }

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

    async fn set_user_access_levels(
        &self,
        user_id: &str,
        assigned: Vec<String>,
        effective: Vec<String>,
        can_write: bool,
        can_read_draft: bool,
        can_write_draft: bool,
    ) -> Result<(), AppError> {
        let mut users = self.users.lock().unwrap();
        let user = users
            .iter_mut()
            .find(|u| u.id == user_id)
            .ok_or_else(|| AppError::NotFound(format!("User '{user_id}' not found")))?;
        user.assigned_access_levels = assigned;
        user.effective_access_levels = effective;
        user.can_write = can_write;
        user.can_read_draft = can_read_draft;
        user.can_write_draft = can_write_draft;
        Ok(())
    }

    async fn update_user_effective_levels(
        &self,
        user_id: &str,
        effective: Vec<String>,
    ) -> Result<(), AppError> {
        let mut users = self.users.lock().unwrap();
        if let Some(u) = users.iter_mut().find(|u| u.id == user_id) {
            u.effective_access_levels = effective;
        }
        Ok(())
    }

    async fn list_users_with_assigned_level(
        &self,
        level_name: &str,
    ) -> Result<Vec<crate::db::auth_models::User>, AppError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.assigned_access_levels.contains(&level_name.to_string()))
            .cloned()
            .collect())
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

    async fn revoke_refresh_token_family(&self, family_id: &str) -> Result<(), AppError> {
        if family_id.is_empty() {
            return Ok(());
        }
        let mut tokens = self.tokens.lock().unwrap();
        let now = Utc::now();
        for t in tokens
            .iter_mut()
            .filter(|t| t.family_id == family_id && t.revoked_at.is_none())
        {
            t.revoked_at = Some(now);
        }
        Ok(())
    }
}

// ── RecordingSearch / RecordingRag ─────────────────────────────────────────

/// Records which slugs a [`SearchService`](crate::search::client::SearchService)
/// was asked to index or delete. `fail_index` makes every index call fail.
#[cfg(feature = "ssr")]
#[derive(Default)]
pub struct RecordingSearch {
    pub indexed: Mutex<Vec<String>>,
    pub deleted: Mutex<Vec<String>>,
    pub fail_index: std::sync::atomic::AtomicBool,
}

#[cfg(feature = "ssr")]
impl RecordingSearch {
    pub fn indexed(&self) -> Vec<String> {
        self.indexed.lock().unwrap().clone()
    }
    pub fn deleted(&self) -> Vec<String> {
        self.deleted.lock().unwrap().clone()
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl crate::search::client::SearchService for RecordingSearch {
    async fn index_document(
        &self,
        doc: &crate::search::client::SearchDocument,
    ) -> Result<(), AppError> {
        if self.fail_index.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::Internal("search index rejected".into()));
        }
        self.indexed.lock().unwrap().push(doc.slug.clone());
        Ok(())
    }
    async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
        self.deleted.lock().unwrap().push(slug.to_string());
        Ok(())
    }
    async fn search(
        &self,
        _: &str,
        _: Option<&[String]>,
        _: bool,
    ) -> Result<Vec<crate::search::client::SearchHit>, AppError> {
        Ok(vec![])
    }
    async fn configure_index(&self) -> Result<(), AppError> {
        Ok(())
    }
    async fn health_check(&self) -> Result<(), AppError> {
        Ok(())
    }
}

/// Records which document slugs a [`RagService`](crate::rag::service::RagService)
/// was asked to index or delete. `fail_index` makes every document index call fail.
#[cfg(feature = "ssr")]
#[derive(Default)]
pub struct RecordingRag {
    pub indexed: Mutex<Vec<String>>,
    pub deleted: Mutex<Vec<String>>,
    pub fail_index: std::sync::atomic::AtomicBool,
}

#[cfg(feature = "ssr")]
impl RecordingRag {
    pub fn indexed(&self) -> Vec<String> {
        self.indexed.lock().unwrap().clone()
    }
    pub fn deleted(&self) -> Vec<String> {
        self.deleted.lock().unwrap().clone()
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl crate::rag::service::RagService for RecordingRag {
    async fn index_document(
        &self,
        slug: &str,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
        _: &[String],
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), AppError> {
        if self.fail_index.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::Internal("rag index rejected".into()));
        }
        self.indexed.lock().unwrap().push(slug.to_string());
        Ok(())
    }
    async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
        self.deleted.lock().unwrap().push(slug.to_string());
        Ok(())
    }
    async fn index_attachment(
        &self,
        _: &str,
        _: &str,
        _: &[crate::rag::service::AttachmentPage],
        _: &[String],
        _: &[String],
    ) -> Result<usize, AppError> {
        Ok(0)
    }
    async fn delete_attachment(&self, _: &str) -> Result<(), AppError> {
        Ok(())
    }
    async fn update_attachment_access_levels(&self, _: &str, _: &[String]) -> Result<(), AppError> {
        Ok(())
    }
    async fn health_check(&self) -> Result<(), AppError> {
        Ok(())
    }
}

// ── InMemoryAssetRepo ──────────────────────────────────────────────────────

/// In-memory [`AssetRepository`](crate::db::asset_repository::AssetRepository).
/// `fail_list` makes `list_all` fail.
#[cfg(feature = "ssr")]
#[derive(Default)]
pub struct InMemoryAssetRepo {
    pub assets: Mutex<Vec<crate::db::models::Asset>>,
    pub fail_list: std::sync::atomic::AtomicBool,
    /// Created after `list_all` took its snapshot: lookups by key see them,
    /// the listing does not.
    pub late_assets: Mutex<Vec<crate::db::models::Asset>>,
}

#[cfg(feature = "ssr")]
impl InMemoryAssetRepo {
    pub fn with_assets(assets: Vec<crate::db::models::Asset>) -> Self {
        Self {
            assets: Mutex::new(assets),
            ..Default::default()
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl crate::db::asset_repository::AssetRepository for InMemoryAssetRepo {
    async fn create_or_update(&self, asset: crate::db::models::Asset) -> Result<(), AppError> {
        let mut assets = self.assets.lock().unwrap();
        assets.retain(|a| a.key != asset.key);
        assets.push(asset);
        Ok(())
    }
    async fn find_by_key(&self, key: &str) -> Result<Option<crate::db::models::Asset>, AppError> {
        Ok(self
            .assets
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.key == key)
            .cloned())
    }
    async fn find_by_keys(
        &self,
        keys: &[String],
    ) -> Result<Vec<crate::db::models::Asset>, AppError> {
        let late = self.late_assets.lock().unwrap().clone();
        Ok(self
            .assets
            .lock()
            .unwrap()
            .iter()
            .chain(&late)
            .filter(|a| keys.contains(&a.key))
            .cloned()
            .collect())
    }
    async fn list_all(&self) -> Result<Vec<crate::db::models::Asset>, AppError> {
        if self.fail_list.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::Internal("asset listing failed".into()));
        }
        Ok(self.assets.lock().unwrap().clone())
    }
    async fn list_by_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<crate::db::models::Asset>, AppError> {
        Ok(self
            .assets
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.key.starts_with(prefix))
            .cloned()
            .collect())
    }
    async fn delete(&self, key: &str) -> Result<(), AppError> {
        self.assets.lock().unwrap().retain(|a| a.key != key);
        Ok(())
    }
    async fn update_extraction(
        &self,
        key: &str,
        update: crate::db::asset_repository::ExtractionUpdate,
    ) -> Result<(), AppError> {
        if let Some(asset) = self
            .assets
            .lock()
            .unwrap()
            .iter_mut()
            .find(|a| a.key == key)
        {
            asset.extraction_status = update.status;
            asset.extraction_error = update.error;
            asset.extracted_content_hash = update.extracted_content_hash;
            asset.extracted_at = update.extracted_at;
            asset.indexed_chunks = update.indexed_chunks;
        }
        Ok(())
    }
    async fn set_release_references(
        &self,
        source: &crate::db::models::DocumentReference,
        keys: &[String],
    ) -> Result<Vec<String>, AppError> {
        let mut affected = Vec::new();
        for asset in self.assets.lock().unwrap().iter_mut() {
            let has = asset.referenced_by.contains(source);
            let wants = keys.contains(&asset.key);
            if has && !wants {
                asset.referenced_by.retain(|r| r != source);
                affected.push(asset.key.clone());
            } else if wants && !has {
                asset.referenced_by.push(source.clone());
                affected.push(asset.key.clone());
            }
        }
        Ok(affected)
    }
    async fn list_unfinished_extractions(&self) -> Result<Vec<crate::db::models::Asset>, AppError> {
        Ok(vec![])
    }
}

/// An asset fixture with the given key and referencing slugs (unversioned).
#[cfg(feature = "ssr")]
pub fn make_asset(key: &str, referenced_by: &[&str]) -> crate::db::models::Asset {
    crate::db::models::Asset {
        key: key.to_string(),
        content_type: "application/pdf".to_string(),
        size_bytes: 3,
        s3_key: format!("assets/{key}"),
        uploaded_at: Utc::now(),
        uploaded_by: "test".to_string(),
        referenced_by: referenced_by.iter().map(|s| (*s).into()).collect(),
        content_hash: Some("sha256:same".to_string()),
        extraction_status: crate::db::models::ExtractionStatus::Done,
        extraction_error: None,
        extracted_content_hash: Some("sha256:same".to_string()),
        extracted_at: None,
        indexed_chunks: None,
    }
}
