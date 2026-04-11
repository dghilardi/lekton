use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Per-user overrides for published prompt visibility and inclusion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPromptPreference {
    pub id: String,
    pub user_id: String,
    pub prompt_slug: String,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub is_hidden: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[async_trait]
pub trait UserPromptPreferenceRepository: Send + Sync {
    async fn upsert(&self, preference: UserPromptPreference) -> Result<(), AppError>;
    async fn find_by_user_and_slug(
        &self,
        user_id: &str,
        prompt_slug: &str,
    ) -> Result<Option<UserPromptPreference>, AppError>;
    async fn list_by_user_id(&self, user_id: &str) -> Result<Vec<UserPromptPreference>, AppError>;
    async fn delete(&self, user_id: &str, prompt_slug: &str) -> Result<(), AppError>;
}

#[cfg(feature = "ssr")]
pub struct MongoUserPromptPreferenceRepository {
    collection: mongodb::Collection<UserPromptPreference>,
}

#[cfg(feature = "ssr")]
impl MongoUserPromptPreferenceRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("user_prompt_preferences"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl UserPromptPreferenceRepository for MongoUserPromptPreferenceRepository {
    async fn upsert(&self, preference: UserPromptPreference) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::ReplaceOptions;

        let filter = doc! {
            "user_id": &preference.user_id,
            "prompt_slug": &preference.prompt_slug,
        };
        let options = ReplaceOptions::builder().upsert(true).build();

        self.collection
            .replace_one(filter, &preference)
            .with_options(options)
            .await?;

        Ok(())
    }

    async fn find_by_user_and_slug(
        &self,
        user_id: &str,
        prompt_slug: &str,
    ) -> Result<Option<UserPromptPreference>, AppError> {
        use mongodb::bson::doc;

        Ok(self
            .collection
            .find_one(doc! {
                "user_id": user_id,
                "prompt_slug": prompt_slug,
            })
            .await?)
    }

    async fn list_by_user_id(&self, user_id: &str) -> Result<Vec<UserPromptPreference>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder()
            .sort(doc! { "prompt_slug": 1 })
            .build();

        let mut cursor = self
            .collection
            .find(doc! { "user_id": user_id })
            .with_options(options)
            .await?;

        let mut preferences = Vec::new();
        while let Some(preference) = cursor.try_next().await? {
            preferences.push(preference);
        }
        Ok(preferences)
    }

    async fn delete(&self, user_id: &str, prompt_slug: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        self.collection
            .delete_one(doc! {
                "user_id": user_id,
                "prompt_slug": prompt_slug,
            })
            .await?;
        Ok(())
    }
}
