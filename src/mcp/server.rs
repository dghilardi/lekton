//! MCP server exposing Lekton documentation and prompt tools.
//!
//! Documentation tools exposed to MCP clients:
//!
//! - **`get_index`** — legacy helper that returns the document tree with slugs, titles, and resource URIs.
//! - **`search_documents`** — semantic search via Qdrant vector store.
//!
//! Documentation is exposed primarily as native MCP resources under the
//! `docs://` URI scheme, so clients can enumerate and read full documents
//! directly without going through a read tool.
//!
//! Prompt tools exposed to MCP clients:
//!
//! - **`list_prompts`** — returns the visible prompt catalog.
//! - **`get_prompt`** — retrieves one prompt by slug.
//! - **`search_prompts`** — searches prompt metadata.
//! - **`get_context_prompts`** — returns the prompt set that should be included in the caller context.

use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};

use crate::auth::models::UserContext;
use crate::db::documentation_feedback_models::{
    DocumentationFeedback, DocumentationFeedbackKind, DocumentationFeedbackStatus,
};
use crate::db::documentation_feedback_repository::DocumentationFeedbackRepository;
use crate::db::models::{Document, Schema, SchemaVersion};
use crate::db::prompt_models::{ContextCost, Prompt, PromptStatus, PromptVariable};
use crate::db::prompt_repository::PromptRepository;
use crate::db::repository::DocumentRepository;
use crate::db::schema_repository::SchemaRepository;
use crate::db::user_prompt_preference_repository::UserPromptPreferenceRepository;
use crate::error::AppError;
use crate::rag::embedding::EmbeddingService;
use crate::rag::vectorstore::VectorStore;
use crate::storage::client::StorageClient;

// ── Tool parameter schemas ──────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchDocsParams {
    /// The natural-language query to search for.
    pub query: String,
    /// Maximum number of results to return (default: 5, max: 20).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    5
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetPromptParams {
    /// The prompt slug (e.g. "prompts/code-review").
    pub prompt_slug: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchPromptsParams {
    /// Query string for prompt metadata search.
    pub query: String,
    /// Maximum number of results to return (default: 10, max: 50).
    #[serde(default = "default_prompt_limit")]
    pub limit: usize,
}

fn default_prompt_limit() -> usize {
    10
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchDocumentationFeedbackParams {
    /// Query string used to find similar open feedback items before creating a new one.
    pub query: String,
    /// Optional feedback type filter: "missing_info" or "improvement".
    #[serde(default)]
    pub kind: Option<String>,
    /// Optional status filter. Defaults to "open".
    #[serde(default)]
    pub status: Option<String>,
    /// Maximum number of results to return (default: 5, max: 20).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReportMissingDocumentationParams {
    pub title: String,
    pub summary: String,
    pub user_goal: String,
    pub searched_resources: Vec<String>,
    pub search_queries_used: Vec<String>,
    pub missing_information: String,
    pub impact: String,
    #[serde(default)]
    pub suggested_target_resource: Option<String>,
    #[serde(default)]
    pub related_feedback_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProposeDocumentationImprovementParams {
    pub title: String,
    pub summary: String,
    pub target_resource_uri: String,
    pub problem_summary: String,
    pub proposal: String,
    pub supporting_resources: Vec<String>,
    pub expected_benefit: String,
    #[serde(default)]
    pub search_queries_used: Vec<String>,
    #[serde(default)]
    pub related_feedback_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListSchemasParams {
    /// Filter by schema type: "openapi", "asyncapi", or "jsonschema".
    #[serde(default)]
    pub schema_type: Option<String>,
    /// Filter by tag (exact match).
    #[serde(default)]
    pub tag: Option<String>,
    /// Filter by service owner (case-insensitive substring).
    #[serde(default)]
    pub service_owner: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchSchemasParams {
    /// Query matched (case-insensitive) against schema name, service owner, and tags.
    pub query: String,
    /// Maximum number of results to return (default: 10, max: 50).
    #[serde(default = "default_prompt_limit")]
    pub limit: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetSchemaDetailParams {
    /// The schema name (e.g. "payment-service-api").
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetSchemaContentParams {
    /// The schema name (e.g. "payment-service-api").
    pub name: String,
    /// The version string (e.g. "1.0.0").
    pub version: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchSchemaOperationsParams {
    /// Query matched (case-insensitive) against endpoint path and summary.
    pub query: String,
    /// Optional: restrict search to a specific schema name.
    #[serde(default)]
    pub schema_name: Option<String>,
    /// Maximum number of results to return (default: 10, max: 50).
    #[serde(default = "default_prompt_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct StoredPromptBlob {
    slug: String,
    name: String,
    description: String,
    access_level: String,
    status: PromptStatus,
    owner: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    variables: Vec<PromptVariable>,
    #[serde(default)]
    publish_to_mcp: bool,
    #[serde(default)]
    default_primary: bool,
    #[serde(default)]
    context_cost: ContextCost,
    prompt_body: String,
}

const DOCS_URI_SCHEME: &str = "docs://";
const DOCS_RESOURCE_TEMPLATE: &str = "docs://{id}";

// ── MCP Server ──────────────────────────────────────────────────────────────

/// The MCP server instance, created once per session.
#[derive(Clone)]
pub struct LektonMcpServer {
    document_repo: Arc<dyn DocumentRepository>,
    schema_repo: Arc<dyn SchemaRepository>,
    prompt_repo: Arc<dyn PromptRepository>,
    user_prompt_preference_repo: Arc<dyn UserPromptPreferenceRepository>,
    documentation_feedback_repo: Arc<dyn DocumentationFeedbackRepository>,
    storage_client: Arc<dyn StorageClient>,
    embedding_service: Arc<dyn EmbeddingService>,
    vector_store: Arc<dyn VectorStore>,
    #[allow(dead_code)]
    tool_router: ToolRouter<LektonMcpServer>,
}

impl LektonMcpServer {
    pub fn new(
        document_repo: Arc<dyn DocumentRepository>,
        schema_repo: Arc<dyn SchemaRepository>,
        prompt_repo: Arc<dyn PromptRepository>,
        user_prompt_preference_repo: Arc<dyn UserPromptPreferenceRepository>,
        documentation_feedback_repo: Arc<dyn DocumentationFeedbackRepository>,
        storage_client: Arc<dyn StorageClient>,
        embedding_service: Arc<dyn EmbeddingService>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self {
            document_repo,
            schema_repo,
            prompt_repo,
            user_prompt_preference_repo,
            documentation_feedback_repo,
            storage_client,
            embedding_service,
            vector_store,
            tool_router: Self::tool_router(),
        }
    }
}

/// Extract the [`UserContext`] from the MCP request context.
///
/// The PAT auth middleware inserts it into the HTTP request extensions,
/// which rmcp forwards as `axum::http::request::Parts`.
fn user_context(ctx: &RequestContext<RoleServer>) -> Result<UserContext, McpError> {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<UserContext>())
        .cloned()
        .ok_or_else(|| {
            McpError::internal_error("Missing user context — is PAT auth configured?", None)
        })
}

fn app_err(e: AppError) -> McpError {
    McpError::internal_error(format!("Internal error: {e}"), None)
}

fn document_resource_uri(slug: &str) -> String {
    format!("{DOCS_URI_SCHEME}{slug}")
}

fn slug_from_docs_uri(uri: &str) -> Result<&str, McpError> {
    let slug = uri.strip_prefix(DOCS_URI_SCHEME).ok_or_else(|| {
        McpError::invalid_params(format!("Unsupported resource URI '{uri}'"), None)
    })?;

    if slug.trim().is_empty() {
        return Err(McpError::invalid_params(
            format!("Resource URI '{uri}' does not contain a document id"),
            None,
        ));
    }

    Ok(slug)
}

fn can_read_document(user_ctx: &UserContext, doc: &Document) -> bool {
    if user_ctx.user.is_admin {
        return true;
    }

    if doc.is_draft {
        user_ctx.can_read_draft(&doc.access_level)
    } else {
        user_ctx.can_read(&doc.access_level)
    }
}

fn can_read_prompt(user_ctx: &UserContext, prompt: &Prompt) -> bool {
    if user_ctx.user.is_admin {
        return true;
    }

    match prompt.status {
        PromptStatus::Draft => user_ctx.can_read_draft(&prompt.access_level),
        PromptStatus::Active | PromptStatus::Deprecated => user_ctx.can_read(&prompt.access_level),
    }
}

fn parse_feedback_kind(value: Option<&str>) -> Result<Option<DocumentationFeedbackKind>, McpError> {
    value
        .map(|raw| raw.parse::<DocumentationFeedbackKind>())
        .transpose()
        .map_err(|err| McpError::invalid_params(err, None))
}

fn parse_feedback_status(
    value: Option<&str>,
    default_open: bool,
) -> Result<Option<DocumentationFeedbackStatus>, McpError> {
    match value {
        Some(raw) => raw
            .parse::<DocumentationFeedbackStatus>()
            .map(Some)
            .map_err(|err| McpError::invalid_params(err, None)),
        None if default_open => Ok(Some(DocumentationFeedbackStatus::Open)),
        None => Ok(None),
    }
}

fn validate_docs_resource_uri(uri: &str) -> Result<(), McpError> {
    slug_from_docs_uri(uri).map(|_| ())
}

fn validate_docs_resource_uris(uris: &[String]) -> Result<(), McpError> {
    for uri in uris {
        validate_docs_resource_uri(uri)?;
    }
    Ok(())
}

fn can_read_schema_version(user_ctx: &UserContext, version: &SchemaVersion) -> bool {
    if user_ctx.user.is_admin {
        return true;
    }
    user_ctx.can_read(&version.access_level)
}

fn schema_version_summary(v: &SchemaVersion) -> serde_json::Value {
    serde_json::json!({
        "version": v.version,
        "status": v.status,
        "access_level": v.access_level,
        "endpoints": v.endpoints.iter().map(|e| serde_json::json!({
            "method": e.method,
            "path": e.path,
            "summary": e.summary,
        })).collect::<Vec<_>>(),
    })
}

fn endpoint_matches_query(endpoint: &crate::db::models::SchemaEndpoint, query: &str) -> bool {
    let q = query.to_lowercase();
    endpoint.path.to_lowercase().contains(&q)
        || endpoint
            .summary
            .as_deref()
            .map(|s| s.to_lowercase().contains(&q))
            .unwrap_or(false)
}

fn schema_list_entry(schema: &Schema, visible_versions: &[&SchemaVersion]) -> serde_json::Value {
    let latest = visible_versions
        .iter()
        .find(|v| v.status != "deprecated")
        .or_else(|| visible_versions.first())
        .map(|v| v.version.clone());

    serde_json::json!({
        "name": schema.name,
        "schema_type": schema.schema_type,
        "service_owner": schema.service_owner,
        "tags": schema.tags,
        "latest_version": latest,
        "version_count": visible_versions.len(),
    })
}

fn schema_matches_query(schema: &Schema, query: &str) -> bool {
    let q = query.to_lowercase();
    schema.name.to_lowercase().contains(&q)
        || schema.service_owner.to_lowercase().contains(&q)
        || schema.tags.iter().any(|t| t.to_lowercase().contains(&q))
}

fn feedback_summary_entry(feedback: &DocumentationFeedback) -> serde_json::Value {
    serde_json::json!({
        "id": feedback.id,
        "kind": feedback.kind,
        "status": feedback.status,
        "title": feedback.title,
        "summary": feedback.summary,
        "related_resources": feedback.related_resources,
        "duplicate_of": feedback.duplicate_of,
        "related_feedback_ids": feedback.related_feedback_ids,
        "created_by": feedback.created_by,
        "created_at": feedback.created_at,
    })
}

fn estimate_context_cost(prompts: &[&Prompt]) -> (&'static str, Vec<String>) {
    let total: u32 = prompts
        .iter()
        .map(|prompt| prompt.context_cost.weight() as u32)
        .sum();

    let estimated = if total >= 12 {
        "high"
    } else if total >= 6 {
        "medium"
    } else {
        "low"
    };

    let mut warnings = Vec::new();
    if total >= 12 {
        warnings.push(
            "Selected prompts add heavy context overhead; reduce favorites or hide some primary prompts".to_string(),
        );
    } else if total >= 8 {
        warnings.push("Selected prompts may add significant context overhead".to_string());
    }

    (estimated, warnings)
}

fn select_context_prompts(
    user_ctx: &UserContext,
    prompts: &[Prompt],
    preferences: &[crate::db::user_prompt_preference_repository::UserPromptPreference],
) -> Vec<(Prompt, &'static str)> {
    use std::collections::{BTreeMap, HashSet};

    let hidden: HashSet<String> = preferences
        .iter()
        .filter(|pref| pref.is_hidden)
        .map(|pref| pref.prompt_slug.clone())
        .collect();
    let favorites: HashSet<String> = preferences
        .iter()
        .filter(|pref| pref.is_favorite)
        .map(|pref| pref.prompt_slug.clone())
        .collect();

    let visible_prompts: Vec<&Prompt> = prompts
        .iter()
        .filter(|prompt| !prompt.is_archived)
        .filter(|prompt| prompt.publish_to_mcp)
        .filter(|prompt| can_read_prompt(user_ctx, prompt))
        .collect();

    let mut selected: BTreeMap<&str, (Prompt, &'static str)> = BTreeMap::new();

    for prompt in &visible_prompts {
        if prompt.default_primary && !hidden.contains(&prompt.slug) {
            selected.insert(prompt.slug.as_str(), ((*prompt).clone(), "default_primary"));
        }
    }

    for prompt in &visible_prompts {
        if favorites.contains(&prompt.slug) {
            selected.insert(prompt.slug.as_str(), ((*prompt).clone(), "favorite"));
        }
    }

    selected.into_values().collect()
}

#[tool_router]
impl LektonMcpServer {
    /// Lists all schemas visible to the authenticated user.
    #[tool(
        name = "list_schemas",
        description = "Returns all schema registry entries visible to the authenticated user. Each entry includes name, type, service owner, tags, available version count, and latest version. Supports optional filtering by schema_type, tag, or service_owner."
    )]
    async fn list_schemas(
        &self,
        Parameters(params): Parameters<ListSchemasParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;

        let schemas = self.schema_repo.list_all().await.map_err(app_err)?;

        let entries: Vec<serde_json::Value> = schemas
            .iter()
            .filter_map(|schema| {
                if let Some(ref t) = params.schema_type {
                    if &schema.schema_type != t {
                        return None;
                    }
                }
                if let Some(ref owner) = params.service_owner {
                    if !schema
                        .service_owner
                        .to_lowercase()
                        .contains(&owner.to_lowercase())
                    {
                        return None;
                    }
                }
                if let Some(ref tag) = params.tag {
                    if !schema.tags.iter().any(|t| t == tag) {
                        return None;
                    }
                }

                let visible: Vec<&SchemaVersion> = schema
                    .versions
                    .iter()
                    .filter(|v| !v.is_archived && can_read_schema_version(&user_ctx, v))
                    .collect();

                if visible.is_empty() {
                    return None;
                }

                Some(schema_list_entry(schema, &visible))
            })
            .collect();

        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Searches schemas by name, service owner, or tag.
    #[tool(
        name = "search_schemas",
        description = "Searches the schema registry by matching the query (case-insensitive) against schema name, service owner, and tags. Returns a ranked list of matching schemas visible to the authenticated user."
    )]
    async fn search_schemas(
        &self,
        Parameters(params): Parameters<SearchSchemasParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let query = params.query.trim();
        if query.is_empty() {
            return Err(McpError::invalid_params("query cannot be empty", None));
        }
        let limit = params.limit.clamp(1, 50);

        let schemas = self.schema_repo.list_all().await.map_err(app_err)?;

        let entries: Vec<serde_json::Value> = schemas
            .iter()
            .filter(|schema| schema_matches_query(schema, query))
            .filter_map(|schema| {
                let visible: Vec<&SchemaVersion> = schema
                    .versions
                    .iter()
                    .filter(|v| !v.is_archived && can_read_schema_version(&user_ctx, v))
                    .collect();
                if visible.is_empty() {
                    return None;
                }
                Some(schema_list_entry(schema, &visible))
            })
            .take(limit)
            .collect();

        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Returns detailed metadata for a single schema including all visible versions.
    #[tool(
        name = "get_schema_detail",
        description = "Returns full metadata for a schema by name, including all versions visible to the authenticated user with their status and access level. Use get_schema_content to fetch the actual schema file."
    )]
    async fn get_schema_detail(
        &self,
        Parameters(params): Parameters<GetSchemaDetailParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;

        let schema = self
            .schema_repo
            .find_by_name(&params.name)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Schema '{}' not found", params.name), None)
            })?;

        let visible_versions: Vec<serde_json::Value> = schema
            .versions
            .iter()
            .filter(|v| !v.is_archived && can_read_schema_version(&user_ctx, v))
            .map(schema_version_summary)
            .collect();

        if visible_versions.is_empty() {
            return Err(McpError::invalid_params(
                format!("Schema '{}' not found", params.name),
                None,
            ));
        }

        let output = serde_json::json!({
            "name": schema.name,
            "schema_type": schema.schema_type,
            "service_owner": schema.service_owner,
            "tags": schema.tags,
            "versions": visible_versions,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Returns the raw content of a specific schema version.
    #[tool(
        name = "get_schema_content",
        description = "Fetches the raw OpenAPI/AsyncAPI/JSON Schema content for a specific schema version. Use get_schema_detail first to discover available versions."
    )]
    async fn get_schema_content(
        &self,
        Parameters(params): Parameters<GetSchemaContentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;

        let schema = self
            .schema_repo
            .find_by_name(&params.name)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Schema '{}' not found", params.name), None)
            })?;

        let version = schema
            .versions
            .iter()
            .find(|v| v.version == params.version)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "Version '{}' not found for schema '{}'",
                        params.version, params.name
                    ),
                    None,
                )
            })?;

        if version.is_archived || !can_read_schema_version(&user_ctx, version) {
            return Err(McpError::invalid_params(
                format!(
                    "Version '{}' not found for schema '{}'",
                    params.version, params.name
                ),
                None,
            ));
        }

        let bytes = self
            .storage_client
            .get_object(&version.s3_key)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::internal_error(
                    format!(
                        "Content not found for '{}'@'{}'",
                        params.name, params.version
                    ),
                    None,
                )
            })?;

        let content = String::from_utf8(bytes)
            .map_err(|e| McpError::internal_error(format!("Invalid UTF-8 content: {e}"), None))?;

        let mime = if version.s3_key.ends_with(".yaml") || version.s3_key.ends_with(".yml") {
            "application/yaml"
        } else {
            "application/json"
        };

        Ok(CallToolResult::success(vec![Content::text(format!(
            "# {name} @ {version}\nContent-Type: {mime}\n\n{content}",
            name = params.name,
            version = params.version,
        ))]))
    }

    /// Searches API operations across all schemas by path or summary.
    #[tool(
        name = "search_schema_operations",
        description = "Searches API operations (OpenAPI paths or AsyncAPI channels) across the schema registry. Matches the query against endpoint path and summary. Optionally restrict search to a specific schema. Returns schema name, version, method, path, and summary for each match."
    )]
    async fn search_schema_operations(
        &self,
        Parameters(params): Parameters<SearchSchemaOperationsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let query = params.query.trim();
        if query.is_empty() {
            return Err(McpError::invalid_params("query cannot be empty", None));
        }
        let limit = params.limit.clamp(1, 50);

        let schemas = self.schema_repo.list_all().await.map_err(app_err)?;

        let mut results: Vec<serde_json::Value> = vec![];

        'outer: for schema in &schemas {
            if let Some(ref name_filter) = params.schema_name {
                if &schema.name != name_filter {
                    continue;
                }
            }

            for version in schema
                .versions
                .iter()
                .filter(|v| !v.is_archived && can_read_schema_version(&user_ctx, v))
            {
                for endpoint in version
                    .endpoints
                    .iter()
                    .filter(|e| endpoint_matches_query(e, query))
                {
                    results.push(serde_json::json!({
                        "schema_name": schema.name,
                        "schema_type": schema.schema_type,
                        "service_owner": schema.service_owner,
                        "version": version.version,
                        "method": endpoint.method,
                        "path": endpoint.path,
                        "summary": endpoint.summary,
                    }));
                    if results.len() >= limit {
                        break 'outer;
                    }
                }
            }
        }

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Returns the document tree visible to the authenticated user.
    #[tool(
        name = "get_index",
        description = "Legacy helper that returns the tree of available documents with their slugs, titles, hierarchy, and docs:// resource URIs. Prefer list_resources for native MCP resource discovery."
    )]
    async fn get_index(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let (levels, include_draft) = user_ctx.document_visibility();

        let docs = self
            .document_repo
            .list_by_access_levels(levels.as_deref(), include_draft)
            .await
            .map_err(app_err)?;

        // Build a lightweight index: slug, title, parent, access_level, tags
        let entries: Vec<serde_json::Value> = docs
            .iter()
            .filter(|d| !d.is_archived)
            .map(|d| {
                serde_json::json!({
                    "slug": d.slug,
                    "title": d.title,
                    "resource_uri": document_resource_uri(&d.slug),
                    "parent_slug": d.parent_slug,
                    "access_level": d.access_level,
                    "tags": d.tags,
                    "is_draft": d.is_draft,
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Performs semantic search across documentation.
    #[tool(
        name = "search_documents",
        description = "Searches documentation using semantic similarity. Returns relevant text fragments together with the corresponding docs:// resource URI to read the full document."
    )]
    async fn search_documents(
        &self,
        Parameters(params): Parameters<SearchDocsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let (levels, include_draft) = user_ctx.document_visibility();

        let limit = params.limit.clamp(1, 20);

        // Embed the query
        let vectors = self
            .embedding_service
            .embed(std::slice::from_ref(&params.query))
            .await
            .map_err(app_err)?;

        let query_vector = vectors
            .into_iter()
            .next()
            .ok_or_else(|| McpError::internal_error("Embedding returned no vectors", None))?;

        // Search Qdrant with access-level filtering
        let results = self
            .vector_store
            .search(query_vector, limit, levels.as_deref(), include_draft)
            .await
            .map_err(app_err)?;

        let hits: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let resource_uri = document_resource_uri(&r.document_slug);
                serde_json::json!({
                    "doc_slug": r.document_slug,
                    "resource_uri": resource_uri,
                    "doc_title": r.document_title,
                    "score": r.score,
                    "text": r.chunk_text,
                    "resource_hint": format!("Resource available at {}", resource_uri),
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&hits)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "search_documentation_feedback",
        description = "Searches the documentation feedback registry for similar open missing-info reports or improvement proposals before creating a new one."
    )]
    async fn search_documentation_feedback(
        &self,
        Parameters(params): Parameters<SearchDocumentationFeedbackParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = params.query.trim();
        if query.is_empty() {
            return Err(McpError::invalid_params(
                "query cannot be empty for search_documentation_feedback",
                None,
            ));
        }

        let kind = parse_feedback_kind(params.kind.as_deref())?;
        let status = parse_feedback_status(params.status.as_deref(), true)?;
        let limit = params.limit.clamp(1, 20);

        let results = self
            .documentation_feedback_repo
            .search(query, kind, status, limit)
            .await
            .map_err(app_err)?;

        let json = serde_json::to_string_pretty(
            &results
                .iter()
                .map(feedback_summary_entry)
                .collect::<Vec<_>>(),
        )
        .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "report_missing_documentation",
        description = "Creates a structured documentation-gap report when required guidance is missing after checking docs:// resources and search results."
    )]
    async fn report_missing_documentation(
        &self,
        Parameters(params): Parameters<ReportMissingDocumentationParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        validate_docs_resource_uris(&params.searched_resources)?;
        if let Some(target) = params.suggested_target_resource.as_deref() {
            validate_docs_resource_uri(target)?;
        }

        let feedback = DocumentationFeedback {
            id: uuid::Uuid::new_v4().to_string(),
            kind: DocumentationFeedbackKind::MissingInfo,
            status: DocumentationFeedbackStatus::Open,
            title: params.title.trim().to_string(),
            summary: params.summary.trim().to_string(),
            related_resources: params.searched_resources,
            search_queries: params.search_queries_used,
            created_by: user_ctx.user.email,
            created_at: chrono::Utc::now(),
            duplicate_of: None,
            resolution_note: None,
            related_feedback_ids: params.related_feedback_ids,
            user_goal: Some(params.user_goal.trim().to_string()),
            missing_information: Some(params.missing_information.trim().to_string()),
            impact: Some(params.impact.trim().to_string()),
            suggested_target_resource: params
                .suggested_target_resource
                .map(|value| value.trim().to_string()),
            target_resource_uri: None,
            problem_summary: None,
            proposal: None,
            supporting_resources: vec![],
            expected_benefit: None,
        };

        if feedback.title.is_empty()
            || feedback.summary.is_empty()
            || feedback.user_goal.as_deref().unwrap_or("").is_empty()
            || feedback
                .missing_information
                .as_deref()
                .unwrap_or("")
                .is_empty()
            || feedback.impact.as_deref().unwrap_or("").is_empty()
        {
            return Err(McpError::invalid_params(
                "title, summary, user_goal, missing_information, and impact are required",
                None,
            ));
        }

        self.documentation_feedback_repo
            .create(feedback.clone())
            .await
            .map_err(app_err)?;

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "id": feedback.id,
            "kind": feedback.kind,
            "status": feedback.status,
            "created_at": feedback.created_at,
        }))
        .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "propose_documentation_improvement",
        description = "Creates a structured proposal to improve documentation discoverability or consolidate fragmented guidance."
    )]
    async fn propose_documentation_improvement(
        &self,
        Parameters(params): Parameters<ProposeDocumentationImprovementParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        validate_docs_resource_uri(&params.target_resource_uri)?;
        validate_docs_resource_uris(&params.supporting_resources)?;

        let feedback = DocumentationFeedback {
            id: uuid::Uuid::new_v4().to_string(),
            kind: DocumentationFeedbackKind::Improvement,
            status: DocumentationFeedbackStatus::Open,
            title: params.title.trim().to_string(),
            summary: params.summary.trim().to_string(),
            related_resources: vec![params.target_resource_uri.clone()],
            search_queries: params.search_queries_used,
            created_by: user_ctx.user.email,
            created_at: chrono::Utc::now(),
            duplicate_of: None,
            resolution_note: None,
            related_feedback_ids: params.related_feedback_ids,
            user_goal: None,
            missing_information: None,
            impact: None,
            suggested_target_resource: None,
            target_resource_uri: Some(params.target_resource_uri),
            problem_summary: Some(params.problem_summary.trim().to_string()),
            proposal: Some(params.proposal.trim().to_string()),
            supporting_resources: params.supporting_resources,
            expected_benefit: Some(params.expected_benefit.trim().to_string()),
        };

        if feedback.title.is_empty()
            || feedback.summary.is_empty()
            || feedback.problem_summary.as_deref().unwrap_or("").is_empty()
            || feedback.proposal.as_deref().unwrap_or("").is_empty()
            || feedback
                .expected_benefit
                .as_deref()
                .unwrap_or("")
                .is_empty()
        {
            return Err(McpError::invalid_params(
                "title, summary, problem_summary, proposal, and expected_benefit are required",
                None,
            ));
        }

        self.documentation_feedback_repo
            .create(feedback.clone())
            .await
            .map_err(app_err)?;

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "id": feedback.id,
            "kind": feedback.kind,
            "status": feedback.status,
            "created_at": feedback.created_at,
        }))
        .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "list_prompts",
        description = "Returns the visible prompt catalog with prompt slugs, names, descriptions, owners, publication flags, and context metadata."
    )]
    async fn list_prompts(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let (levels, include_draft) = user_ctx.document_visibility();

        let prompts = self
            .prompt_repo
            .list_by_access_levels(levels.as_deref(), include_draft)
            .await
            .map_err(app_err)?;

        let entries: Vec<_> = prompts
            .iter()
            .filter(|prompt| can_read_prompt(&user_ctx, prompt))
            .map(prompt_catalog_entry)
            .collect();

        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "get_prompt",
        description = "Retrieves a prompt by slug, including its body, variables, publication flags, and context metadata."
    )]
    async fn get_prompt(
        &self,
        Parameters(params): Parameters<GetPromptParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let prompt = self
            .prompt_repo
            .find_by_slug(&params.prompt_slug)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Prompt '{}' not found", params.prompt_slug), None)
            })?;

        if prompt.is_archived || !can_read_prompt(&user_ctx, &prompt) {
            return Err(McpError::invalid_params(
                format!("Prompt '{}' not found", params.prompt_slug),
                None,
            ));
        }

        let blob = self.load_prompt_blob(&prompt).await?;
        let output = serde_json::json!({
            "slug": prompt.slug,
            "name": prompt.name,
            "description": prompt.description,
            "access_level": prompt.access_level,
            "status": prompt.status,
            "owner": prompt.owner,
            "tags": prompt.tags,
            "variables": blob.variables,
            "publish_to_mcp": prompt.publish_to_mcp,
            "default_primary": prompt.default_primary,
            "context_cost": prompt.context_cost,
            "prompt_body": blob.prompt_body,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "search_prompts",
        description = "Searches prompt metadata by slug, name, description, tags, or owner. Returns only prompts visible to the authenticated user."
    )]
    async fn search_prompts(
        &self,
        Parameters(params): Parameters<SearchPromptsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let (levels, include_draft) = user_ctx.document_visibility();
        let limit = params.limit.clamp(1, 50);

        let prompts = self
            .prompt_repo
            .search_metadata(&params.query, levels.as_deref(), include_draft, limit)
            .await
            .map_err(app_err)?;

        let entries: Vec<_> = prompts
            .iter()
            .filter(|prompt| can_read_prompt(&user_ctx, prompt))
            .map(prompt_catalog_entry)
            .collect();

        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "get_context_prompts",
        description = "Returns the published prompt set that should be included in the authenticated user's context, combining default primary prompts and user favorites."
    )]
    async fn get_context_prompts(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let prompt_entries = self
            .build_context_prompt_entries(&user_ctx)
            .await
            .map_err(app_err)?;

        let prompt_refs: Vec<&Prompt> = prompt_entries.iter().map(|(prompt, _)| prompt).collect();
        let (estimated_cost, warnings) = estimate_context_cost(&prompt_refs);

        let output = serde_json::json!({
            "prompts": prompt_entries
                .into_iter()
                .map(|(prompt, reason)| serde_json::json!({
                    "slug": prompt.slug,
                    "name": prompt.name,
                    "reason": reason,
                    "context_cost": prompt.context_cost,
                }))
                .collect::<Vec<_>>(),
            "estimated_context_cost": estimated_cost,
            "warnings": warnings,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

impl LektonMcpServer {
    async fn load_document_markdown(&self, doc: &Document) -> Result<String, McpError> {
        let content_bytes = self
            .storage_client
            .get_object(&doc.s3_key)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::internal_error(format!("Content not found for '{}'", doc.slug), None)
            })?;

        String::from_utf8(content_bytes)
            .map_err(|e| McpError::internal_error(format!("Invalid UTF-8 content: {e}"), None))
    }

    async fn load_prompt_blob(&self, prompt: &Prompt) -> Result<StoredPromptBlob, McpError> {
        let content_bytes = self
            .storage_client
            .get_object(&prompt.s3_key)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::internal_error(
                    format!("Prompt content not found for '{}'", prompt.slug),
                    None,
                )
            })?;

        serde_yaml::from_slice(&content_bytes)
            .map_err(|e| McpError::internal_error(format!("Invalid prompt YAML: {e}"), None))
    }

    async fn build_context_prompt_entries(
        &self,
        user_ctx: &UserContext,
    ) -> Result<Vec<(Prompt, &'static str)>, AppError> {
        let (levels, include_draft) = user_ctx.document_visibility();
        let prompts = self
            .prompt_repo
            .list_by_access_levels(levels.as_deref(), include_draft)
            .await?;
        let preferences = self
            .user_prompt_preference_repo
            .list_by_user_id(&user_ctx.user.user_id)
            .await?;

        Ok(select_context_prompts(user_ctx, &prompts, &preferences))
    }
}

fn prompt_catalog_entry(prompt: &Prompt) -> serde_json::Value {
    serde_json::json!({
        "slug": prompt.slug,
        "name": prompt.name,
        "description": prompt.description,
        "access_level": prompt.access_level,
        "status": prompt.status,
        "owner": prompt.owner,
        "tags": prompt.tags,
        "publish_to_mcp": prompt.publish_to_mcp,
        "default_primary": prompt.default_primary,
        "context_cost": prompt.context_cost,
    })
}

fn prompt_mcp_name(prompt: &Prompt) -> String {
    prompt.slug.clone()
}

fn prompt_mcp_arguments(variables: &[PromptVariable]) -> Option<Vec<PromptArgument>> {
    if variables.is_empty() {
        return None;
    }

    Some(
        variables
            .iter()
            .map(|variable| {
                PromptArgument::new(variable.name.clone())
                    .with_description(variable.description.clone())
                    .with_required(variable.required)
            })
            .collect(),
    )
}

fn prompt_mcp_entry(prompt: &Prompt) -> rmcp::model::Prompt {
    rmcp::model::Prompt::new(
        prompt_mcp_name(prompt),
        Some(prompt.description.clone()),
        prompt_mcp_arguments(&prompt.variables),
    )
    .with_title(prompt.name.clone())
}

fn render_prompt_body(
    prompt_body: &str,
    arguments: Option<&serde_json::Map<String, serde_json::Value>>,
) -> String {
    let Some(arguments) = arguments else {
        return prompt_body.to_string();
    };

    let mut rendered = prompt_body.to_string();
    for (name, value) in arguments {
        let replacement = match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        rendered = rendered.replace(&format!("{{{{{name}}}}}"), &replacement);
        rendered = rendered.replace(&format!("{{{{ {name} }}}}"), &replacement);
    }

    rendered
}

fn prompt_mcp_result(
    prompt: &Prompt,
    prompt_body: &str,
    arguments: Option<&serde_json::Map<String, serde_json::Value>>,
    reason: &str,
) -> GetPromptResult {
    let rendered_body = render_prompt_body(prompt_body, arguments);
    GetPromptResult::new(vec![PromptMessage::new_text(
        PromptMessageRole::User,
        rendered_body,
    )])
    .with_description(format!(
        "{} [reason: {reason}, context_cost: {:?}]",
        prompt.description, prompt.context_cost
    ))
}

#[tool_handler]
impl ServerHandler for LektonMcpServer {
    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let user_ctx = user_context(&context)?;
        let prompt_entries = self
            .build_context_prompt_entries(&user_ctx)
            .await
            .map_err(app_err)?;

        let (prompt, reason) = prompt_entries
            .into_iter()
            .find(|(prompt, _)| prompt_mcp_name(prompt) == request.name)
            .ok_or_else(|| {
                McpError::invalid_params(format!("Prompt '{}' not found", request.name), None)
            })?;

        let blob = self.load_prompt_blob(&prompt).await?;
        Ok(prompt_mcp_result(
            &prompt,
            &blob.prompt_body,
            request.arguments.as_ref(),
            reason,
        ))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        let user_ctx = user_context(&context)?;
        let prompt_entries = self
            .build_context_prompt_entries(&user_ctx)
            .await
            .map_err(app_err)?;

        Ok(ListPromptsResult {
            prompts: prompt_entries
                .into_iter()
                .map(|(prompt, _)| prompt_mcp_entry(&prompt))
                .collect(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let user_ctx = user_context(&context)?;
        let (levels, include_draft) = user_ctx.document_visibility();

        let docs = self
            .document_repo
            .list_by_access_levels(levels.as_deref(), include_draft)
            .await
            .map_err(app_err)?;

        let resources = docs
            .into_iter()
            .filter(|doc| !doc.is_archived)
            .map(|doc| {
                RawResource::new(document_resource_uri(&doc.slug), doc.slug.clone())
                    .with_title(doc.title.clone())
                    .with_description(format!(
                        "Markdown documentation for '{}' (access: {}, tags: {}).",
                        doc.slug,
                        doc.access_level,
                        if doc.tags.is_empty() {
                            "none".to_string()
                        } else {
                            doc.tags.join(", ")
                        }
                    ))
                    .with_mime_type("text/markdown")
                    .no_annotation()
            })
            .collect();

        Ok(ListResourcesResult {
            resources,
            meta: None,
            next_cursor: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![RawResourceTemplate::new(
                DOCS_RESOURCE_TEMPLATE,
                "documentation-document",
            )
            .with_title("Documentation Resource by Slug")
            .with_description(
                "Use this template to read a specific documentation page once you know its slug/id from list_resources or search_documents. Example: docs://hr/ferie",
            )
            .with_mime_type("text/markdown")
            .no_annotation()],
            meta: None,
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let user_ctx = user_context(&context)?;
        let slug = slug_from_docs_uri(&request.uri)?;

        let doc = self
            .document_repo
            .find_by_slug(slug)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("Document resource '{}' not found", request.uri),
                    None,
                )
            })?;

        if doc.is_archived || !can_read_document(&user_ctx, &doc) {
            return Err(McpError::invalid_params(
                format!("Document resource '{}' not found", request.uri),
                None,
            ));
        }

        let markdown = self.load_document_markdown(&doc).await?;

        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            markdown,
            request.uri,
        )
        .with_mime_type("text/markdown")]))
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("lekton-mcp", env!("CARGO_PKG_VERSION")))
        .with_protocol_version(ProtocolVersion::V_2025_03_26)
        .with_instructions(
            "Lekton documentation server. Available tools:\n\
             - get_index: Legacy document-tree helper with docs:// URIs\n\
             - search_documents: Semantic search across documentation fragments\n\
             - search_documentation_feedback: Search existing feedback before opening a new report\n\
             - report_missing_documentation: Report a documentation gap after verifying docs:// resources and search results\n\
             - propose_documentation_improvement: Suggest a concrete improvement to existing documentation\n\
             - list_prompts: Browse the prompt catalog\n\
             - get_prompt: Read a specific prompt\n\
             - search_prompts: Search prompt metadata\n\
             - get_context_prompts: Return the prompt set selected for the current user context\n\
             Full documentation is exposed as read-only MCP resources under docs://...\n\
             Prefer list_resources to discover available documents, read_resource to load the raw markdown, and search_documents when you need vector search to find the right docs:// URI.\n\
             Before creating documentation feedback, first use search_documentation_feedback to reduce duplicate reports.\n\
             Native MCP prompts are also exposed for the effective user context prompt set.\n\
             Schema registry tools:\n\
             - list_schemas: Browse the schema registry with optional filters (type, tag, service_owner)\n\
             - search_schemas: Search schemas by name, service owner, or tag\n\
             - get_schema_detail: Get full metadata, versions, and indexed endpoints for a schema\n\
             - get_schema_content: Fetch raw OpenAPI/AsyncAPI/JSON Schema content for a specific version\n\
             - search_schema_operations: Search API operations by path or summary across all schemas"
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    use crate::auth::models::AuthenticatedUser;
    use crate::db::user_prompt_preference_repository::UserPromptPreference;

    fn prompt(
        slug: &str,
        default_primary: bool,
        publish_to_mcp: bool,
        cost: ContextCost,
    ) -> Prompt {
        Prompt {
            slug: slug.to_string(),
            name: slug.to_string(),
            description: "Prompt".to_string(),
            s3_key: format!("prompts/{}.yaml", slug.replace('/', "_")),
            access_level: "internal".to_string(),
            status: PromptStatus::Active,
            owner: "platform".to_string(),
            last_updated: Utc::now(),
            tags: vec![],
            variables: vec![],
            publish_to_mcp,
            default_primary,
            context_cost: cost,
            content_hash: Some("sha256:x".to_string()),
            metadata_hash: Some("sha256:y".to_string()),
            is_archived: false,
        }
    }

    fn user_ctx() -> UserContext {
        UserContext {
            user: AuthenticatedUser {
                user_id: "u1".to_string(),
                email: "u1@example.com".to_string(),
                name: None,
                is_admin: false,
            },
            effective_access_levels: vec!["internal".to_string()],
            can_write: false,
            can_read_draft: false,
            can_write_draft: false,
        }
    }

    #[test]
    fn select_context_prompts_uses_primary_plus_favorites_and_honors_hidden() {
        let prompts = vec![
            prompt("prompts/code-review", true, true, ContextCost::Low),
            prompt(
                "prompts/architecture-analysis",
                true,
                true,
                ContextCost::Medium,
            ),
            prompt(
                "prompts/git-history-sanitizer",
                false,
                true,
                ContextCost::High,
            ),
        ];

        let preferences = vec![
            UserPromptPreference {
                id: "p1".into(),
                user_id: "u1".into(),
                prompt_slug: "prompts/architecture-analysis".into(),
                is_favorite: false,
                is_hidden: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            UserPromptPreference {
                id: "p2".into(),
                user_id: "u1".into(),
                prompt_slug: "prompts/git-history-sanitizer".into(),
                is_favorite: true,
                is_hidden: false,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];

        let entries = select_context_prompts(&user_ctx(), &prompts, &preferences);
        let pairs: Vec<(&str, &str)> = entries
            .iter()
            .map(|(prompt, reason)| (prompt.slug.as_str(), *reason))
            .collect();

        assert_eq!(
            pairs,
            vec![
                ("prompts/code-review", "default_primary"),
                ("prompts/git-history-sanitizer", "favorite"),
            ]
        );
    }

    #[test]
    fn estimate_context_cost_returns_warning_over_threshold() {
        let prompts = [
            prompt("prompts/a", true, true, ContextCost::High),
            prompt("prompts/b", true, true, ContextCost::High),
            prompt("prompts/c", true, true, ContextCost::Medium),
        ];
        let refs: Vec<&Prompt> = prompts.iter().collect();
        let (cost, warnings) = estimate_context_cost(&refs);
        assert_eq!(cost, "medium");
        assert!(!warnings.is_empty());
    }

    #[test]
    fn prompt_mcp_entry_uses_slug_and_declared_variables() {
        let mut prompt = prompt("prompts/code-review", true, true, ContextCost::Low);
        prompt.name = "Code Review".into();
        prompt.description = "Review a patch before merge".into();
        prompt.variables = vec![PromptVariable {
            name: "diff".into(),
            description: "Unified diff to inspect".into(),
            required: true,
        }];

        let entry = prompt_mcp_entry(&prompt);
        assert_eq!(entry.name, "prompts/code-review");
        assert_eq!(entry.title.as_deref(), Some("Code Review"));
        assert_eq!(
            entry.description.as_deref(),
            Some("Review a patch before merge")
        );
        let arguments = entry.arguments.expect("prompt arguments");
        assert_eq!(arguments.len(), 1);
        assert_eq!(arguments[0].name, "diff");
        assert_eq!(arguments[0].required, Some(true));
    }

    #[test]
    fn render_prompt_body_replaces_declared_arguments() {
        let body = "Review the following diff:\n{{diff}}\nSummary: {{ summary }}";
        let arguments = serde_json::Map::from_iter([
            ("diff".into(), serde_json::Value::String("+new line".into())),
            (
                "summary".into(),
                serde_json::Value::String("fast path".into()),
            ),
        ]);

        let rendered = render_prompt_body(body, Some(&arguments));
        assert_eq!(
            rendered,
            "Review the following diff:\n+new line\nSummary: fast path"
        );
    }

    #[test]
    fn docs_resource_uri_round_trip_uses_slug() {
        let uri = document_resource_uri("hr/ferie");
        assert_eq!(uri, "docs://hr/ferie");
        assert_eq!(slug_from_docs_uri(&uri).unwrap(), "hr/ferie");
    }

    #[test]
    fn docs_resource_uri_requires_docs_scheme_and_non_empty_slug() {
        assert!(slug_from_docs_uri("file://notes").is_err());
        assert!(slug_from_docs_uri("docs://").is_err());
    }
}
