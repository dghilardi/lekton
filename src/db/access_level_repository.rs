//! Repository for `AccessLevelEntity` — the configurable content categories.

use async_trait::async_trait;
use chrono::Utc;

use crate::db::auth_models::AccessLevelEntity;
use crate::error::AppError;

/// Default access levels seeded on first startup.
///
/// The `"public"` level is a system level and cannot be deleted.
/// The others are pre-populated for convenience but can be modified.
pub const DEFAULT_ACCESS_LEVELS: &[(&str, &str, &str, u32, bool)] = &[
    ("public",    "Public",    "Publicly accessible content",           0,  true),
    ("internal",  "Internal",  "Internal company documentation",        10, false),
    ("developer", "Developer", "Developer-focused documentation",       20, false),
    ("architect", "Architect", "Architecture-level documentation",      30, false),
];

/// CRUD operations for `AccessLevelEntity`.
#[async_trait]
pub trait AccessLevelRepository: Send + Sync {
    /// Insert a new access level. Fails if `name` already exists.
    async fn create(&self, level: AccessLevelEntity) -> Result<(), AppError>;

    /// Find a level by its slug name.
    async fn find_by_name(&self, name: &str) -> Result<Option<AccessLevelEntity>, AppError>;

    /// List all levels ordered by `sort_order` ascending.
    async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError>;

    /// Replace an existing level (matched by `name`).
    async fn update(&self, level: AccessLevelEntity) -> Result<(), AppError>;

    /// Delete a level by name. Returns `Forbidden` if `is_system = true`.
    async fn delete(&self, name: &str) -> Result<(), AppError>;

    /// Return `true` if a level with the given name exists.
    async fn exists(&self, name: &str) -> Result<bool, AppError>;

    /// Seed the default access levels if the collection is empty.
    async fn seed_defaults(&self) -> Result<(), AppError>;
}

/// MongoDB implementation of `AccessLevelRepository`.
#[cfg(feature = "ssr")]
pub struct MongoAccessLevelRepository {
    collection: mongodb::Collection<AccessLevelEntity>,
}

#[cfg(feature = "ssr")]
impl MongoAccessLevelRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("access_levels"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl AccessLevelRepository for MongoAccessLevelRepository {
    async fn create(&self, level: AccessLevelEntity) -> Result<(), AppError> {
        use mongodb::bson::doc;

        // Enforce uniqueness on name (MongoDB will also enforce via index if set).
        if self.exists(&level.name).await? {
            return Err(AppError::BadRequest(format!(
                "Access level '{}' already exists",
                level.name
            )));
        }

        self.collection
            .insert_one(&level)
            .await?;

        Ok(())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<AccessLevelEntity>, AppError> {
        use mongodb::bson::doc;

        Ok(self.collection
            .find_one(doc! { "name": name })
            .await?)
    }

    async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError> {
        use futures::TryStreamExt;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder()
            .sort(mongodb::bson::doc! { "sort_order": 1 })
            .build();

        let mut cursor = self
            .collection
            .find(mongodb::bson::doc! {})
            .with_options(options)
            .await?;

        let mut levels = Vec::new();
        while let Some(level) = cursor
            .try_next()
            .await?
        {
            levels.push(level);
        }
        Ok(levels)
    }

    async fn update(&self, level: AccessLevelEntity) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::ReplaceOptions;

        let filter = doc! { "name": &level.name };
        let options = ReplaceOptions::builder().upsert(false).build();

        let result = self
            .collection
            .replace_one(filter, &level)
            .with_options(options)
            .await?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "Access level '{}' not found",
                level.name
            )));
        }

        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let level = self
            .find_by_name(name)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Access level '{}' not found", name)))?;

        if level.is_system {
            return Err(AppError::Forbidden(format!(
                "Cannot delete system access level '{}'",
                name
            )));
        }

        self.collection
            .delete_one(doc! { "name": name })
            .await?;

        Ok(())
    }

    async fn exists(&self, name: &str) -> Result<bool, AppError> {
        use mongodb::bson::doc;

        let count = self
            .collection
            .count_documents(doc! { "name": name })
            .await?;

        Ok(count > 0)
    }

    async fn seed_defaults(&self) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let count = self
            .collection
            .count_documents(doc! {})
            .await?;

        if count > 0 {
            return Ok(());
        }

        for (name, label, description, sort_order, is_system) in DEFAULT_ACCESS_LEVELS {
            let level = AccessLevelEntity {
                name: name.to_string(),
                label: label.to_string(),
                description: description.to_string(),
                sort_order: *sort_order,
                is_system: *is_system,
                created_at: Utc::now(),
            };
            self.collection
                .insert_one(&level)
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_levels_have_public_system() {
        let public = DEFAULT_ACCESS_LEVELS
            .iter()
            .find(|(name, ..)| *name == "public");
        assert!(public.is_some(), "public level must be in defaults");
        let (.., is_system) = public.unwrap();
        assert!(*is_system, "public must be a system level");
    }

    #[test]
    fn test_default_levels_sorted_by_sort_order() {
        let orders: Vec<u32> = DEFAULT_ACCESS_LEVELS.iter().map(|(.., order, _)| *order).collect();
        let mut sorted = orders.clone();
        sorted.sort();
        assert_eq!(orders, sorted, "DEFAULT_ACCESS_LEVELS must be ordered by sort_order");
    }
}
