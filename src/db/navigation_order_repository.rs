//! Repository for navigation ordering — custom weights for sections and categories.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// A navigation ordering entry.
///
/// Each entry associates a navigation slug (section or category) with a
/// numeric weight.  Lower weights appear first.  Items without an explicit
/// entry fall back to alphabetical ordering (treated as weight `i32::MAX`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationOrderEntry {
    /// The slug of the navigation item (e.g. `"engineering"`, `"engineering/guides"`).
    pub slug: String,
    /// Sort weight — lower values appear first.
    pub weight: i32,
}

/// CRUD operations for navigation ordering.
#[async_trait]
pub trait NavigationOrderRepository: Send + Sync {
    /// List all ordering entries, sorted by weight ascending.
    async fn list_all(&self) -> Result<Vec<NavigationOrderEntry>, AppError>;

    /// Bulk-replace all ordering entries atomically.
    ///
    /// This is the primary write path — the admin UI sends the full ordered
    /// list after a drag-and-drop reorder.
    async fn replace_all(&self, entries: Vec<NavigationOrderEntry>) -> Result<(), AppError>;
}

/// MongoDB implementation.
#[cfg(feature = "ssr")]
pub struct MongoNavigationOrderRepository {
    collection: mongodb::Collection<NavigationOrderEntry>,
}

#[cfg(feature = "ssr")]
impl MongoNavigationOrderRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("navigation_order"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl NavigationOrderRepository for MongoNavigationOrderRepository {
    async fn list_all(&self) -> Result<Vec<NavigationOrderEntry>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::FindOptions;

        let options = FindOptions::builder().sort(doc! { "weight": 1 }).build();

        let mut cursor = self.collection.find(doc! {}).with_options(options).await?;

        let mut entries = Vec::new();
        while let Some(entry) = cursor.try_next().await? {
            entries.push(entry);
        }
        Ok(entries)
    }

    async fn replace_all(&self, entries: Vec<NavigationOrderEntry>) -> Result<(), AppError> {
        use mongodb::bson::doc;

        // Delete all existing entries, then insert the new ones.
        self.collection.delete_many(doc! {}).await?;

        if !entries.is_empty() {
            self.collection.insert_many(&entries).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_serialization() {
        let entry = NavigationOrderEntry {
            slug: "engineering".to_string(),
            weight: 10,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: NavigationOrderEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.slug, "engineering");
        assert_eq!(deserialized.weight, 10);
    }
}
