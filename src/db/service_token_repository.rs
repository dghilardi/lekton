use async_trait::async_trait;

use crate::db::service_token_models::ServiceToken;
use crate::error::AppError;

/// Repository trait for scoped service token operations.
#[async_trait]
pub trait ServiceTokenRepository: Send + Sync {
    /// Insert a new service token. Rejects duplicates on name or token_hash.
    async fn create(&self, token: ServiceToken) -> Result<(), AppError>;

    /// Look up a token by the SHA-256 hash of its raw value.
    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<ServiceToken>, AppError>;

    /// Look up a token by its human-readable name.
    async fn find_by_name(&self, name: &str) -> Result<Option<ServiceToken>, AppError>;

    /// Look up a token by its internal ID.
    async fn find_by_id(&self, id: &str) -> Result<Option<ServiceToken>, AppError>;

    /// List all tokens (active and inactive).
    async fn list_all(&self) -> Result<Vec<ServiceToken>, AppError>;

    /// Mark a token as inactive (soft-delete).
    async fn deactivate(&self, id: &str) -> Result<(), AppError>;

    /// Update `last_used_at` to the current time.
    async fn touch_last_used(&self, id: &str) -> Result<(), AppError>;

    /// Check whether any existing active token's scopes overlap with `scopes`.
    /// Optionally exclude one token by ID (useful when updating an existing token).
    async fn check_scope_overlap(
        &self,
        scopes: &[String],
        exclude_id: Option<&str>,
    ) -> Result<bool, AppError>;

    /// Set the `is_active` flag on a token. Returns `NotFound` if the token doesn't exist.
    async fn set_active(&self, id: &str, active: bool) -> Result<(), AppError>;

    /// List all PATs (`token_type = "pat"`) belonging to a specific user, newest first.
    async fn list_by_user_id(&self, user_id: &str) -> Result<Vec<ServiceToken>, AppError>;

    /// List all PATs (`token_type = "pat"`) paginated, newest first.
    /// Returns `(tokens, total_count)`.
    async fn list_pats_paginated(
        &self,
        page: u64,
        per_page: u64,
    ) -> Result<(Vec<ServiceToken>, u64), AppError>;

    /// Permanently delete a token owned by `user_id`. Returns `Forbidden` if the
    /// token exists but belongs to a different user.
    async fn delete_pat(&self, id: &str, user_id: &str) -> Result<(), AppError>;
}

/// MongoDB implementation of [`ServiceTokenRepository`].
#[cfg(feature = "ssr")]
pub struct MongoServiceTokenRepository {
    collection: mongodb::Collection<ServiceToken>,
}

#[cfg(feature = "ssr")]
impl MongoServiceTokenRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("service_tokens"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl ServiceTokenRepository for MongoServiceTokenRepository {
    async fn create(&self, token: ServiceToken) -> Result<(), AppError> {
        use mongodb::bson::doc;

        // Check name uniqueness
        let existing = self
            .collection
            .find_one(doc! { "name": &token.name })
            .await?;
        if existing.is_some() {
            return Err(AppError::BadRequest(format!(
                "Service token name '{}' already exists",
                token.name
            )));
        }

        // Check hash uniqueness
        let existing = self
            .collection
            .find_one(doc! { "token_hash": &token.token_hash })
            .await?;
        if existing.is_some() {
            return Err(AppError::BadRequest(
                "A token with this hash already exists".into(),
            ));
        }

        self.collection.insert_one(&token).await?;
        Ok(())
    }

    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<ServiceToken>, AppError> {
        use mongodb::bson::doc;
        Ok(self
            .collection
            .find_one(doc! { "token_hash": token_hash })
            .await?)
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<ServiceToken>, AppError> {
        use mongodb::bson::doc;
        Ok(self.collection.find_one(doc! { "name": name }).await?)
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<ServiceToken>, AppError> {
        use mongodb::bson::doc;
        Ok(self.collection.find_one(doc! { "id": id }).await?)
    }

    async fn list_all(&self) -> Result<Vec<ServiceToken>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build();

        let mut cursor = self.collection.find(doc! {}).with_options(options).await?;
        let mut tokens = Vec::new();
        while let Some(token) = cursor.try_next().await? {
            tokens.push(token);
        }
        Ok(tokens)
    }

    async fn deactivate(&self, id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let result = self
            .collection
            .update_one(doc! { "id": id }, doc! { "$set": { "is_active": false } })
            .await?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "Service token '{id}' not found"
            )));
        }
        Ok(())
    }

    async fn touch_last_used(&self, id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let now = chrono::Utc::now();
        let bson_now = bson::DateTime::from_chrono(now);

        self.collection
            .update_one(
                doc! { "id": id },
                doc! { "$set": { "last_used_at": bson_now } },
            )
            .await?;
        Ok(())
    }

    async fn check_scope_overlap(
        &self,
        scopes: &[String],
        exclude_id: Option<&str>,
    ) -> Result<bool, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        use crate::db::service_token_models::scopes_overlap;

        let mut filter = doc! { "is_active": true };
        if let Some(id) = exclude_id {
            filter.insert("id", doc! { "$ne": id });
        }

        let mut cursor = self.collection.find(filter).await?;
        while let Some(token) = cursor.try_next().await? {
            if scopes_overlap(scopes, &token.allowed_scopes) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn set_active(&self, id: &str, active: bool) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let result = self
            .collection
            .update_one(doc! { "id": id }, doc! { "$set": { "is_active": active } })
            .await?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "Service token '{id}' not found"
            )));
        }
        Ok(())
    }

    async fn list_by_user_id(&self, user_id: &str) -> Result<Vec<ServiceToken>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build();

        let filter = doc! { "token_type": "pat", "user_id": user_id };
        let mut cursor = self.collection.find(filter).with_options(options).await?;
        let mut tokens = Vec::new();
        while let Some(token) = cursor.try_next().await? {
            tokens.push(token);
        }
        Ok(tokens)
    }

    async fn list_pats_paginated(
        &self,
        page: u64,
        per_page: u64,
    ) -> Result<(Vec<ServiceToken>, u64), AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::{CountOptions, FindOptions};

        let filter = doc! { "token_type": "pat" };

        let total = self
            .collection
            .count_documents(filter.clone())
            .with_options(CountOptions::builder().build())
            .await? as u64;

        let skip = page.saturating_sub(1) * per_page;
        let options = FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .skip(skip)
            .limit(per_page as i64)
            .build();

        let mut cursor = self.collection.find(filter).with_options(options).await?;
        let mut tokens = Vec::new();
        while let Some(token) = cursor.try_next().await? {
            tokens.push(token);
        }
        Ok((tokens, total))
    }

    async fn delete_pat(&self, id: &str, user_id: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        // First verify ownership
        let token = self
            .collection
            .find_one(doc! { "id": id, "token_type": "pat" })
            .await?
            .ok_or_else(|| AppError::NotFound(format!("PAT '{id}' not found")))?;

        if token.user_id.as_deref() != Some(user_id) {
            return Err(AppError::Forbidden("You do not own this token".into()));
        }

        self.collection.delete_one(doc! { "id": id }).await?;
        Ok(())
    }
}
