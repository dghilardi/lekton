use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::models::AccessLevel;

/// Represents a documentation entry stored in MongoDB.
///
/// Corresponds to the `documents` collection defined in REQUIREMENTS.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// The URL-safe slug path (e.g., `engineering/deployment-guide`).
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// The S3 key where the markdown content is stored.
    pub s3_key: String,
    /// Minimum access level required to view this document.
    pub access_level: AccessLevel,
    /// The team/service that owns this document.
    pub service_owner: String,
    /// Timestamp of the last update.
    pub last_updated: DateTime<Utc>,
    /// Tags for categorization and search.
    pub tags: Vec<String>,
    /// Outgoing links to other documents (for backlink tracking).
    pub links_out: Vec<String>,
    /// Incoming backlinks (populated by ingestion logic).
    pub backlinks: Vec<String>,
    /// Optional parent slug for explicit hierarchical relationships.
    /// If None, the document is considered a top-level document.
    #[serde(default)]
    pub parent_slug: Option<String>,
    /// Sort order within parent (or top-level if no parent).
    /// Lower numbers appear first in navigation.
    #[serde(default)]
    pub order: u32,
    /// Whether this document should be hidden from navigation.
    /// Hidden documents can still be accessed directly via URL.
    #[serde(default)]
    pub is_hidden: bool,
}

/// Represents an API schema entry stored in MongoDB.
///
/// Corresponds to the `schemas` collection defined in REQUIREMENTS.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    /// Schema name (e.g., `payment-service-api`).
    pub name: String,
    /// Schema type: `openapi`, `asyncapi`, or `jsonschema`.
    pub schema_type: String,
    /// Versioned entries.
    pub versions: Vec<SchemaVersion>,
}

/// A single version of a schema artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersion {
    /// Semantic version string.
    pub version: String,
    /// S3 key for the schema file.
    pub s3_key: String,
    /// Status: `stable`, `beta`, `deprecated`.
    pub status: String,
}

/// The request payload for the ingest API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    /// Service authentication token.
    pub service_token: String,
    /// The slug path for the document.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Raw Markdown content.
    pub content: String,
    /// Minimum access level.
    pub access_level: String,
    /// The team/service that owns this document.
    pub service_owner: String,
    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional parent slug for hierarchy.
    #[serde(default)]
    pub parent_slug: Option<String>,
    /// Sort order (defaults to 0 if not specified).
    #[serde(default)]
    pub order: u32,
    /// Whether to hide from navigation (defaults to false).
    #[serde(default)]
    pub is_hidden: bool,
}

/// The response from a successful ingest operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResponse {
    /// A message indicating success.
    pub message: String,
    /// The slug of the created/updated document.
    pub slug: String,
    /// The S3 key where content was stored.
    pub s3_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_serialization() {
        let doc = Document {
            slug: "engineering/deployment-guide".to_string(),
            title: "Deployment Guide".to_string(),
            s3_key: "docs/eng/deploy_v4.md".to_string(),
            access_level: AccessLevel::Developer,
            service_owner: "devops-team".to_string(),
            last_updated: Utc::now(),
            tags: vec!["k8s".to_string(), "cicd".to_string()],
            links_out: vec!["/docs/setup".to_string()],
            backlinks: vec![],
            parent_slug: Some("engineering".to_string()),
            order: 10,
            is_hidden: false,
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.slug, "engineering/deployment-guide");
        assert_eq!(deserialized.access_level, AccessLevel::Developer);
        assert_eq!(deserialized.tags.len(), 2);
        assert_eq!(deserialized.parent_slug, Some("engineering".to_string()));
        assert_eq!(deserialized.order, 10);
        assert_eq!(deserialized.is_hidden, false);
    }

    #[test]
    fn test_document_hierarchy_defaults() {
        // Test that new hierarchy fields have proper defaults when deserializing old documents
        let json = r###"{
            "slug": "getting-started",
            "title": "Getting Started",
            "s3_key": "docs/getting-started.md",
            "access_level": "Public",
            "service_owner": "docs-team",
            "last_updated": "2024-01-01T00:00:00Z",
            "tags": [],
            "links_out": [],
            "backlinks": []
        }"###;

        let doc: Document = serde_json::from_str(json).unwrap();
        assert_eq!(doc.parent_slug, None);
        assert_eq!(doc.order, 0);
        assert_eq!(doc.is_hidden, false);
    }

    #[test]
    fn test_ingest_request_deserialization() {
        let json = r###"{
            "service_token": "tok-123",
            "slug": "docs/hello",
            "title": "Hello World",
            "content": "# Hello\nWorld",
            "access_level": "public",
            "service_owner": "my-team",
            "tags": ["intro"]
        }"###;

        let req: IngestRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.slug, "docs/hello");
        assert_eq!(req.title, "Hello World");
        assert_eq!(req.tags, vec!["intro"]);
    }

    #[test]
    fn test_ingest_request_default_tags() {
        let json = r###"{
            "service_token": "tok-123",
            "slug": "docs/hello",
            "title": "Hello",
            "content": "## Hello",
            "access_level": "public",
            "service_owner": "team"
        }"###;

        let req: IngestRequest = serde_json::from_str(json).unwrap();
        assert!(req.tags.is_empty());
    }

    #[test]
    fn test_ingest_response_serialization() {
        let resp = IngestResponse {
            message: "Document ingested successfully".to_string(),
            slug: "docs/hello".to_string(),
            s3_key: "docs/hello/v1.md".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("docs/hello"));
    }

    #[test]
    fn test_schema_serialization() {
        let schema = Schema {
            name: "payment-api".to_string(),
            schema_type: "openapi".to_string(),
            versions: vec![
                SchemaVersion {
                    version: "1.0.0".to_string(),
                    s3_key: "schemas/payment/1.0.0.json".to_string(),
                    status: "deprecated".to_string(),
                },
                SchemaVersion {
                    version: "2.0.0".to_string(),
                    s3_key: "schemas/payment/2.0.0.json".to_string(),
                    status: "stable".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&schema).unwrap();
        let deserialized: Schema = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.versions.len(), 2);
        assert_eq!(deserialized.versions[1].status, "stable");
    }
}
