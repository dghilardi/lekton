use std::path::PathBuf;

use crate::config::PromptVariable;

// ── Attachment types ──────────────────────────────────────────────────────────

/// A local file reference found in a markdown document.
#[derive(Debug, Clone)]
pub struct LocalFileRef {
    /// The raw path as it appears in the markdown (e.g., "./images/arch.png")
    pub raw_path: String,
    /// The resolved absolute path on disk
    pub disk_path: PathBuf,
}

/// A resolved attachment ready for upload.
#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    /// The raw relative path as it appears in markdown
    pub raw_path: String,
    /// Resolved absolute path on disk
    pub disk_path: PathBuf,
    /// SHA-256 content hash of the file bytes
    pub content_hash: String,
    /// The asset key on the server: "attachments/{doc-slug}/{filename}"
    pub asset_key: String,
    /// File size in bytes
    pub size_bytes: u64,
    /// MIME content type
    pub content_type: String,
}

// ── Domain models ─────────────────────────────────────────────────────────────

pub struct DocumentInfo {
    /// Relative path of the source file from the repo root (e.g. `docs/guide.md`).
    pub source_path: String,
    /// Desired slug (title-derived or from front matter). Used for ingest if the
    /// server does not resolve a different actual_slug via migration.
    pub slug: String,
    /// Path-derived slug (old behavior). Sent as `legacy_slug` in the sync request
    /// when it differs from `slug`, so the server can locate documents indexed
    /// before `source_path` was introduced.
    pub legacy_slug: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub content: String,
    pub rewritten_content: String,
    pub content_hash: String,
    pub metadata_hash: String,
    pub access_level: String,
    pub service_owner: String,
    pub tags: Vec<String>,
    pub parent_slug: Option<String>,
    pub order: i32,
    pub is_hidden: bool,
    pub attachments: Vec<AttachmentInfo>,
}

#[derive(Debug)]
pub struct PromptInfo {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub prompt_body: String,
    pub content_hash: String,
    pub metadata_hash: String,
    pub access_level: String,
    pub status: String,
    pub owner: String,
    pub tags: Vec<String>,
    pub variables: Vec<PromptVariable>,
    pub publish_to_mcp: bool,
    pub default_primary: bool,
    pub context_cost: String,
}

#[derive(Debug)]
pub struct SchemaInfo {
    pub key: String,
    pub name: String,
    pub schema_type: String,
    pub version: String,
    pub status: String,
    pub access_level: String,
    pub service_owner: String,
    pub tags: Vec<String>,
    pub content: String,
    pub content_hash: String,
    pub metadata_hash: String,
}

/// Intermediate representation used during scanning before order assignment.
pub struct ScannedDoc {
    pub source_path: String,
    pub slug: String,
    pub legacy_slug: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub content: String,
    pub rewritten_content: String,
    pub content_hash: String,
    pub metadata_hash: String,
    pub access_level: String,
    pub service_owner: String,
    pub tags: Vec<String>,
    pub parent_slug: Option<String>,
    pub explicit_order: Option<i32>,
    pub is_hidden: bool,
    pub attachments: Vec<AttachmentInfo>,
}
