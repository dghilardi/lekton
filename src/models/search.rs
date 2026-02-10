use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchDocument {
    pub id: String, // Use slug for search ID
    pub slug: String,
    pub title: String,
    pub content: String,
    pub access_level: String,
    pub tags: Vec<String>,
}
