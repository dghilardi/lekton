use serde::{Deserialize, Serialize};

use crate::config::PromptVariable;

// ── Document sync ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SyncRequest {
    pub service_token: String,
    pub source_id: String,
    pub documents: Vec<SyncDocEntry>,
    pub archive_missing: bool,
}

#[derive(Serialize)]
pub struct SyncDocEntry {
    pub source_path: String,
    pub slug: String,
    pub content_hash: String,
    pub metadata_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_slug: Option<String>,
}

#[derive(Deserialize)]
pub struct SyncUploadEntry {
    pub source_path: String,
    pub actual_slug: String,
}

#[derive(Deserialize)]
pub struct SyncResponse {
    pub to_upload: Vec<SyncUploadEntry>,
    pub to_archive: Vec<String>,
    pub unchanged: Vec<String>,
}

// ── Prompt sync ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct PromptSyncRequest {
    pub service_token: String,
    pub prompts: Vec<PromptSyncEntry>,
    pub archive_missing: bool,
}

#[derive(Serialize)]
pub struct PromptSyncEntry {
    pub slug: String,
    pub content_hash: String,
    pub metadata_hash: String,
}

#[derive(Deserialize)]
pub struct PromptSyncResponse {
    pub to_upload: Vec<String>,
    pub to_archive: Vec<String>,
    pub unchanged: Vec<String>,
}

// ── Schema sync ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SchemaSyncRequest {
    pub service_token: String,
    pub schemas: Vec<SchemaSyncEntry>,
    pub archive_missing: bool,
}

#[derive(Serialize)]
pub struct SchemaSyncEntry {
    pub name: String,
    pub version: String,
    pub content_hash: String,
    pub metadata_hash: String,
}

#[derive(Deserialize)]
pub struct SchemaSyncResponse {
    pub to_upload: Vec<String>,
    pub to_archive: Vec<String>,
    pub unchanged: Vec<String>,
}

// ── Ingest ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct IngestRequest {
    pub service_token: String,
    pub source_path: String,
    pub source_id: String,
    pub slug: String,
    pub title: String,
    pub summary: Option<String>,
    pub content: String,
    pub access_level: String,
    pub service_owner: String,
    pub tags: Vec<String>,
    pub parent_slug: Option<String>,
    pub order: u32,
    pub is_hidden: bool,
}

#[derive(Deserialize)]
pub struct IngestResponse {
    pub changed: bool,
    /// `false` when the server persisted the document but could not (re)index it
    /// in search/RAG. Defaults to `true` for older servers that omit the field.
    #[serde(default = "default_true")]
    pub indexed: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
pub struct PromptIngestRequest {
    pub service_token: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub prompt_body: String,
    pub access_level: String,
    pub status: String,
    pub owner: String,
    pub tags: Vec<String>,
    pub variables: Vec<PromptVariable>,
    pub publish_to_mcp: bool,
    pub default_primary: bool,
    pub context_cost: String,
}

#[derive(Deserialize)]
pub struct PromptIngestResponse {
    pub changed: bool,
}

#[derive(Serialize)]
pub struct SchemaIngestRequest {
    pub service_token: String,
    pub name: String,
    pub schema_type: String,
    pub version: String,
    pub status: String,
    pub access_level: String,
    pub service_owner: String,
    pub tags: Vec<String>,
    pub content: String,
}

#[derive(Deserialize)]
pub struct SchemaIngestResponse {
    pub changed: bool,
}

// ── Asset check-hashes ────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct CheckHashesRequest {
    pub service_token: String,
    pub entries: Vec<CheckHashEntry>,
}

#[derive(Serialize)]
pub struct CheckHashEntry {
    pub key: String,
    pub content_hash: String,
}

#[derive(Deserialize)]
pub struct CheckHashesResponse {
    pub to_upload: Vec<String>,
}
