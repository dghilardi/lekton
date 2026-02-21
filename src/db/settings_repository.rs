use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Application-level settings stored in MongoDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Settings document key (always "global").
    pub key: String,
    /// Custom CSS to inject at runtime.
    #[serde(default)]
    pub custom_css: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            key: "global".to_string(),
            custom_css: String::new(),
        }
    }
}

/// Repository trait for application settings.
#[async_trait]
pub trait SettingsRepository: Send + Sync {
    /// Get the global application settings.
    async fn get_settings(&self) -> Result<AppSettings, AppError>;

    /// Update the custom CSS.
    async fn set_custom_css(&self, css: &str) -> Result<(), AppError>;
}

/// MongoDB implementation of the SettingsRepository.
#[cfg(feature = "ssr")]
pub struct MongoSettingsRepository {
    collection: mongodb::Collection<AppSettings>,
}

#[cfg(feature = "ssr")]
impl MongoSettingsRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("settings"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl SettingsRepository for MongoSettingsRepository {
    async fn get_settings(&self) -> Result<AppSettings, AppError> {
        use mongodb::bson::doc;

        let result = self
            .collection
            .find_one(doc! { "key": "global" })
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(result.unwrap_or_default())
    }

    async fn set_custom_css(&self, css: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::UpdateOptions;

        let options = UpdateOptions::builder().upsert(true).build();

        self.collection
            .update_one(
                doc! { "key": "global" },
                doc! { "$set": { "key": "global", "custom_css": css } },
            )
            .with_options(options)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = AppSettings::default();
        assert_eq!(settings.key, "global");
        assert!(settings.custom_css.is_empty());
    }

    #[test]
    fn test_settings_serialization() {
        let settings = AppSettings {
            key: "global".to_string(),
            custom_css: ":root { --lekton-font-family: monospace; }".to_string(),
        };
        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.custom_css, settings.custom_css);
    }
}
