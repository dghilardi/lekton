use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Represents a documentation entry stored in MongoDB.
///
/// Corresponds to the `documents` collection defined in REQUIREMENTS.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// The URL-safe slug path (e.g., `engineering/deployment-guide`).
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Short human-readable summary used for resource discovery.
    #[serde(default)]
    pub summary: Option<String>,
    /// The S3 key where the markdown content is stored.
    pub s3_key: String,
    /// Access level name (references `AccessLevelEntity.name`, e.g. `"public"`, `"internal"`).
    pub access_level: String,
    /// When `true` the document is a work-in-progress and not shown to regular readers.
    #[serde(default)]
    pub is_draft: bool,
    /// The team/service that owns this document.
    pub service_owner: String,
    /// Timestamp of the last update.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
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
    /// SHA-256 hash of the markdown content (format: `"sha256:<base64url>"`).
    /// Used to decide whether to re-upload content to S3. `None` for documents
    /// ingested before content hashing was introduced.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// SHA-256 hash of the document's front-matter metadata (same format as
    /// `content_hash`).  Used by the sync protocol to detect metadata-only
    /// changes (e.g. `access_level`, `title`, `summary`) that don't alter the body text.
    /// `None` for documents ingested before metadata hashing was introduced.
    #[serde(default)]
    pub metadata_hash: Option<String>,
    /// Whether this document has been archived (removed from source but kept
    /// for historical reference). Archived documents are excluded from
    /// navigation and search.
    #[serde(default)]
    pub is_archived: bool,
    /// The relative path of the source file within the repository (e.g.,
    /// `docs/guides/intro.md`). Used as a stable identity for the document
    /// across slug renames. `None` for documents ingested before this field
    /// was introduced.
    #[serde(default)]
    pub source_path: Option<String>,
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
    /// The team/service that owns this schema family.
    #[serde(default)]
    pub service_owner: String,
    /// Tags used for filtering or grouping in the UI.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Versioned entries.
    pub versions: Vec<SchemaVersion>,
}

/// An API operation extracted from a schema artifact (OpenAPI path/method or AsyncAPI channel/action).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaEndpoint {
    /// HTTP method (GET, POST, …) or AsyncAPI action (publish, subscribe, send, receive).
    pub method: String,
    /// URL path (e.g. `/payments/{id}`) or AsyncAPI channel name.
    pub path: String,
    /// Short human-readable summary of the operation, if present in the spec.
    #[serde(default)]
    pub summary: Option<String>,
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
    /// Access level required to read this schema version.
    #[serde(default = "default_public_access_level")]
    pub access_level: String,
    /// SHA-256 content hash of the raw schema artifact.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// SHA-256 hash of version metadata (status/access level).
    #[serde(default)]
    pub metadata_hash: Option<String>,
    /// Whether this version has been archived by sync.
    #[serde(default)]
    pub is_archived: bool,
    /// API operations extracted from the schema at ingest time.
    #[serde(default)]
    pub endpoints: Vec<SchemaEndpoint>,
}

fn default_public_access_level() -> String {
    "public".to_string()
}

/// Represents a binary asset stored in MongoDB with content in S3.
///
/// Assets are identified by a caller-defined key (e.g., "project-a/configs/nginx.conf").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    /// Caller-defined unique key (e.g., "project-a/configs/nginx.conf").
    pub key: String,
    /// MIME content type (e.g., "image/png", "application/pdf").
    pub content_type: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// S3 key where the asset content is stored.
    pub s3_key: String,
    /// When the asset was last uploaded or replaced.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub uploaded_at: DateTime<Utc>,
    /// Identifier of who uploaded the asset.
    pub uploaded_by: String,
    /// Document slugs that reference this asset (managed during document save).
    #[serde(default)]
    pub referenced_by: Vec<String>,
    /// SHA-256 hash of the asset content (format: `"sha256:<base64url>"`).
    /// Used for deduplication during sync. `None` for assets uploaded before
    /// content hashing was introduced.
    #[serde(default)]
    pub content_hash: Option<String>,
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
    /// Optional short summary for resource discovery.
    #[serde(default)]
    pub summary: Option<String>,
    /// Raw Markdown content.
    pub content: String,
    /// Access level name (e.g. `"public"`, `"internal"`).
    pub access_level: String,
    /// When `true`, the document is stored as a draft.
    #[serde(default)]
    pub is_draft: bool,
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
    /// The relative path of the source file within the repository (e.g.,
    /// `docs/guides/intro.md`). Required for stable slug tracking across
    /// title changes. Used by the server to resolve the canonical slug for
    /// a document when the desired slug would differ from the stored one.
    pub source_path: String,
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
    /// Whether the document content or metadata actually changed.
    /// `false` when the content hash and all metadata fields match the existing document.
    #[serde(default = "default_true")]
    pub changed: bool,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_serialization() {
        let doc = Document {
            slug: "engineering/deployment-guide".to_string(),
            title: "Deployment Guide".to_string(),
            summary: Some("How to deploy services to Kubernetes.".to_string()),
            s3_key: "docs/eng/deploy_v4.md".to_string(),
            access_level: "internal".to_string(),
            is_draft: false,
            service_owner: "devops-team".to_string(),
            last_updated: Utc::now(),
            tags: vec!["k8s".to_string(), "cicd".to_string()],
            links_out: vec!["/docs/setup".to_string()],
            backlinks: vec![],
            parent_slug: Some("engineering".to_string()),
            order: 10,
            is_hidden: false,
            content_hash: Some("sha256:abc123".to_string()),
            metadata_hash: None,
            is_archived: false,
            source_path: Some("engineering/deployment-guide.md".to_string()),
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.slug, "engineering/deployment-guide");
        assert_eq!(
            deserialized.summary.as_deref(),
            Some("How to deploy services to Kubernetes.")
        );
        assert_eq!(deserialized.access_level, "internal");
        assert!(!deserialized.is_draft);
        assert_eq!(deserialized.tags.len(), 2);
        assert_eq!(deserialized.parent_slug, Some("engineering".to_string()));
        assert_eq!(deserialized.order, 10);
        assert!(!deserialized.is_hidden);
        assert_eq!(deserialized.content_hash, Some("sha256:abc123".to_string()));
    }

    #[test]
    fn test_document_defaults() {
        // Verify that new fields have sensible defaults when deserializing older documents.
        // last_updated uses BSON DateTime format (extended JSON) because of the
        // `chrono_datetime_as_bson_datetime` serde helper.
        let json = r###"{
            "slug": "getting-started",
            "title": "Getting Started",
            "s3_key": "docs/getting-started.md",
            "access_level": "public",
            "service_owner": "docs-team",
            "last_updated": { "$date": { "$numberLong": "1704067200000" } },
            "tags": [],
            "links_out": [],
            "backlinks": []
        }"###;

        let doc: Document = serde_json::from_str(json).unwrap();
        assert_eq!(doc.access_level, "public");
        assert!(!doc.is_draft);
        assert_eq!(doc.parent_slug, None);
        assert_eq!(doc.order, 0);
        assert!(!doc.is_hidden);
        assert_eq!(doc.content_hash, None); // backward compat
        assert_eq!(doc.summary, None); // backward compat
        assert_eq!(doc.source_path, None); // backward compat
    }

    #[test]
    fn test_schema_defaults_are_backward_compatible() {
        let json = r###"{
            "name": "payment-service-api",
            "schema_type": "openapi",
            "versions": [
                {
                    "version": "1.0.0",
                    "s3_key": "schemas/payment-service-api/1.0.0.json",
                    "status": "stable"
                }
            ]
        }"###;

        let schema: Schema = serde_json::from_str(json).unwrap();
        assert_eq!(schema.service_owner, "");
        assert!(schema.tags.is_empty());
        assert_eq!(schema.versions.len(), 1);
        assert_eq!(schema.versions[0].access_level, "public");
        assert_eq!(schema.versions[0].content_hash, None);
        assert_eq!(schema.versions[0].metadata_hash, None);
        assert!(!schema.versions[0].is_archived);
    }

    #[test]
    fn test_draft_document() {
        let doc = Document {
            slug: "engineering/wip".to_string(),
            title: "Work in Progress".to_string(),
            summary: None,
            s3_key: "docs/wip.md".to_string(),
            access_level: "internal".to_string(),
            is_draft: true,
            service_owner: "platform-team".to_string(),
            last_updated: Utc::now(),
            tags: vec![],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            content_hash: None,
            metadata_hash: None,
            is_archived: false,
            source_path: None,
        };

        let json = serde_json::to_string(&doc).unwrap();
        let de: Document = serde_json::from_str(&json).unwrap();
        assert!(de.is_draft);
    }

    #[test]
    fn test_ingest_request_deserialization() {
        let json = r###"{
            "service_token": "tok-123",
            "source_path": "docs/hello.md",
            "slug": "docs/hello",
            "title": "Hello World",
            "summary": "Introduces the internal onboarding flow.",
            "content": "# Hello\nWorld",
            "access_level": "internal",
            "service_owner": "my-team",
            "tags": ["intro"]
        }"###;

        let req: IngestRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.slug, "docs/hello");
        assert_eq!(
            req.summary.as_deref(),
            Some("Introduces the internal onboarding flow.")
        );
        assert_eq!(req.source_path, "docs/hello.md");
        assert_eq!(req.access_level, "internal");
        assert!(!req.is_draft);
    }

    #[test]
    fn test_ingest_request_draft_flag() {
        let json = r###"{
            "service_token": "tok",
            "source_path": "docs/wip.md",
            "slug": "docs/wip",
            "title": "WIP",
            "summary": null,
            "content": "draft content",
            "access_level": "internal",
            "service_owner": "team",
            "is_draft": true
        }"###;

        let req: IngestRequest = serde_json::from_str(json).unwrap();
        assert!(req.is_draft);
    }

    #[test]
    fn test_ingest_request_defaults() {
        let json = r###"{
            "service_token": "tok-123",
            "source_path": "docs/hello.md",
            "slug": "docs/hello",
            "title": "Hello",
            "content": "## Hello",
            "access_level": "public",
            "service_owner": "team"
        }"###;

        let req: IngestRequest = serde_json::from_str(json).unwrap();
        assert!(req.tags.is_empty());
        assert_eq!(req.summary, None);
        assert!(!req.is_draft);
        assert_eq!(req.order, 0);
    }

    #[test]
    fn test_ingest_response_serialization() {
        let resp = IngestResponse {
            message: "Document ingested successfully".to_string(),
            slug: "docs/hello".to_string(),
            s3_key: "docs/hello/v1.md".to_string(),
            changed: true,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("docs/hello"));
    }

    #[test]
    fn test_asset_serialization() {
        let asset = Asset {
            key: "project-a/configs/nginx.conf".to_string(),
            content_type: "application/octet-stream".to_string(),
            size_bytes: 2048,
            s3_key: "assets/project-a/configs/nginx.conf".to_string(),
            uploaded_at: Utc::now(),
            uploaded_by: "ci-pipeline".to_string(),
            referenced_by: vec!["deployment-guide".to_string()],
            content_hash: Some("sha256:abc123".to_string()),
        };

        let json = serde_json::to_string(&asset).unwrap();
        let deserialized: Asset = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.key, "project-a/configs/nginx.conf");
        assert_eq!(deserialized.size_bytes, 2048);
        assert_eq!(deserialized.referenced_by.len(), 1);
    }

    #[test]
    fn test_schema_serialization() {
        let schema = Schema {
            name: "payment-api".to_string(),
            schema_type: "openapi".to_string(),
            service_owner: "payments".to_string(),
            tags: vec!["payments".to_string(), "api".to_string()],
            versions: vec![
                SchemaVersion {
                    version: "1.0.0".to_string(),
                    s3_key: "schemas/payment/1.0.0.json".to_string(),
                    status: "deprecated".to_string(),
                    access_level: "public".to_string(),
                    content_hash: Some("sha256:v1".to_string()),
                    metadata_hash: Some("sha256:m1".to_string()),
                    is_archived: false,
                    endpoints: vec![],
                },
                SchemaVersion {
                    version: "2.0.0".to_string(),
                    s3_key: "schemas/payment/2.0.0.json".to_string(),
                    status: "stable".to_string(),
                    access_level: "internal".to_string(),
                    content_hash: Some("sha256:v2".to_string()),
                    metadata_hash: Some("sha256:m2".to_string()),
                    is_archived: false,
                    endpoints: vec![],
                },
            ],
        };
        let json = serde_json::to_string(&schema).unwrap();
        let deserialized: Schema = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.versions.len(), 2);
        assert_eq!(deserialized.versions[1].status, "stable");
    }
}
