use serde::{Deserialize, Serialize};

/// `.lekton.yml` project-level configuration.
#[derive(Deserialize, Default)]
pub struct LektonConfig {
    /// Stable identifier for this repository import, used by Lekton to resolve
    /// relative cross-links between documents in the same source.
    /// Required field — sync fails with a clear error if missing.
    #[serde(default)]
    pub id: Option<String>,
    /// Base URL of the Lekton server (can also be set via LEKTON_URL env var)
    pub url: Option<String>,
    /// Default access level applied when a document has no `access_level` in its front matter
    #[serde(default)]
    pub default_access_level: Option<String>,
    /// Default service_owner applied when a document has no `service_owner` in its front matter
    #[serde(default)]
    pub default_service_owner: Option<String>,
    /// Slug prefix prepended to every document slug (e.g. "protocols/my-service")
    #[serde(default)]
    pub slug_prefix: Option<String>,
    /// Archive documents not found locally (can be overridden by --archive-missing flag)
    #[serde(default)]
    pub archive_missing: Option<bool>,
    /// Maximum attachment file size in MB (default: 10)
    #[serde(default)]
    pub max_attachment_size_mb: Option<u32>,
    /// Directory containing prompt YAML files, relative to the root path.
    #[serde(default)]
    pub prompts_dir: Option<String>,
    /// Slug prefix prepended to prompt slugs (default: "prompts")
    #[serde(default)]
    pub prompt_slug_prefix: Option<String>,
    /// Directory containing schema manifests, relative to the root path.
    #[serde(default)]
    pub schemas_dir: Option<String>,
    /// Prefix prepended to schema names discovered from manifests.
    #[serde(default)]
    pub schema_name_prefix: Option<String>,
    /// Archive schema versions not found locally.
    #[serde(default)]
    pub archive_missing_schemas: Option<bool>,
}

/// YAML front matter parsed from the top of each `.md` file.
#[derive(Deserialize, Default)]
pub struct FrontMatter {
    pub slug: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    #[serde(alias = "access-level", alias = "accessLevel")]
    pub access_level: Option<String>,
    #[serde(alias = "service-owner", alias = "serviceOwner")]
    pub service_owner: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(alias = "parent-slug", alias = "parentSlug")]
    pub parent_slug: Option<String>,
    pub order: Option<i32>,
    #[serde(alias = "is-hidden", alias = "isHidden")]
    pub is_hidden: Option<bool>,
    /// Must be `true` for the file to be synced to Lekton.
    #[serde(
        rename = "lekton-import",
        alias = "lektonImport",
        alias = "lekton_import",
        default
    )]
    pub lekton_import: bool,
}

/// YAML prompt definition parsed from files under the prompt directory.
#[derive(Deserialize, Default, Clone)]
pub struct PromptFile {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub access_level: Option<String>,
    pub status: Option<String>,
    pub owner: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub variables: Option<Vec<PromptVariable>>,
    #[serde(default)]
    pub publish_to_mcp: Option<bool>,
    #[serde(default)]
    pub default_primary: Option<bool>,
    #[serde(default)]
    pub context_cost: Option<String>,
    pub prompt_body: Option<String>,
    /// Optional explicit import flag for parity with document sync.
    #[serde(rename = "lekton-import", default)]
    pub lekton_import: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct SchemaManifestFile {
    pub name: Option<String>,
    pub schema_type: String,
    #[serde(default)]
    pub service_owner: Option<String>,
    #[serde(default)]
    pub default_access_level: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub versions: Vec<SchemaVersionFile>,
}

#[derive(Deserialize, Default)]
pub struct SchemaVersionFile {
    pub file: String,
    pub version: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub access_level: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct PromptVariable {
    pub name: String,
    pub description: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

fn default_status() -> String {
    "stable".to_string()
}
