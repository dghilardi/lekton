use serde::{Deserialize, Serialize};
use mongodb::bson::oid::ObjectId;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Document {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub slug: String,
    pub title: String,
    pub s3_key: String,
    pub access_level: String, // "public", "developer", "admin"
    pub service_owner: String,
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub tags: Vec<String>,
    pub links_out: Vec<String>,
    pub backlinks: Vec<String>,
}
