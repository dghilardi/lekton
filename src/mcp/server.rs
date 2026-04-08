//! MCP server exposing Lekton documentation tools.
//!
//! Three tools are exposed to MCP clients (IDE agents):
//!
//! - **`get_index`** — returns the document tree with slugs and titles.
//! - **`search_docs`** — semantic search via Qdrant vector store.
//! - **`read_document`** — retrieves the full Markdown content of a document.

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{
        router::tool::ToolRouter,
        wrapper::Parameters,
    },
    model::*,
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router,
};

use crate::auth::models::UserContext;
use crate::db::repository::DocumentRepository;
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
pub struct ReadDocumentParams {
    /// The document slug (e.g. "protocols/mqtt-v5").
    pub doc_slug: String,
}

// ── MCP Server ──────────────────────────────────────────────────────────────

/// The MCP server instance, created once per session.
#[derive(Clone)]
pub struct LektonMcpServer {
    document_repo: Arc<dyn DocumentRepository>,
    storage_client: Arc<dyn StorageClient>,
    embedding_service: Arc<dyn EmbeddingService>,
    vector_store: Arc<dyn VectorStore>,
    tool_router: ToolRouter<LektonMcpServer>,
}

impl LektonMcpServer {
    pub fn new(
        document_repo: Arc<dyn DocumentRepository>,
        storage_client: Arc<dyn StorageClient>,
        embedding_service: Arc<dyn EmbeddingService>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self {
            document_repo,
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

#[tool_router]
impl LektonMcpServer {
    /// Returns the document tree visible to the authenticated user.
    #[tool(
        name = "get_index",
        description = "Returns the tree of available documents with their slugs, titles, and hierarchy. Use this first to discover what documentation exists."
    )]
    async fn get_index(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
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
        name = "search_docs",
        description = "Searches documentation using semantic similarity. Returns relevant text fragments with their source document slugs. Use this for specific questions."
    )]
    async fn search_docs(
        &self,
        Parameters(params): Parameters<SearchDocsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;
        let (levels, include_draft) = user_ctx.document_visibility();

        let limit = params.limit.min(20).max(1);

        // Embed the query
        let vectors = self
            .embedding_service
            .embed(&[params.query.clone()])
            .await
            .map_err(app_err)?;

        let query_vector = vectors.into_iter().next().ok_or_else(|| {
            McpError::internal_error("Embedding returned no vectors", None)
        })?;

        // Search Qdrant with access-level filtering
        let results = self
            .vector_store
            .search(query_vector, limit, levels.as_deref(), include_draft)
            .await
            .map_err(app_err)?;

        let hits: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "doc_slug": r.document_slug,
                    "doc_title": r.document_title,
                    "score": r.score,
                    "text": r.chunk_text,
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&hits)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Retrieves the full Markdown content of a document by slug.
    #[tool(
        name = "read_document",
        description = "Retrieves the full text of a document by its slug. Use this when a search fragment doesn't contain enough context, or when you need the complete document."
    )]
    async fn read_document(
        &self,
        Parameters(params): Parameters<ReadDocumentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user_ctx = user_context(&ctx)?;

        // Fetch the document metadata to check access and get S3 key
        let doc = self
            .document_repo
            .find_by_slug(&params.doc_slug)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("Document '{}' not found", params.doc_slug),
                    None,
                )
            })?;

        // Verify the user can read this document's access level
        if !user_ctx.user.is_admin {
            let can_read = if doc.is_draft {
                user_ctx.can_read_draft(&doc.access_level)
            } else {
                user_ctx.can_read(&doc.access_level)
            };
            if !can_read {
                return Err(McpError::invalid_params(
                    format!("Document '{}' not found", params.doc_slug),
                    None,
                ));
            }
        }

        // Fetch the Markdown content from S3
        let content_bytes = self
            .storage_client
            .get_object(&doc.s3_key)
            .await
            .map_err(app_err)?
            .ok_or_else(|| {
                McpError::internal_error(
                    format!("Content not found for '{}'", params.doc_slug),
                    None,
                )
            })?;

        let markdown = String::from_utf8(content_bytes).map_err(|e| {
            McpError::internal_error(format!("Invalid UTF-8 content: {e}"), None)
        })?;

        // Return with metadata header
        let output = format!(
            "# {}\n\n**Slug:** {}\n**Access level:** {}\n**Tags:** {}\n\n---\n\n{}",
            doc.title,
            doc.slug,
            doc.access_level,
            doc.tags.join(", "),
            markdown,
        );

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

#[tool_handler]
impl ServerHandler for LektonMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::new("lekton-mcp", env!("CARGO_PKG_VERSION")))
        .with_protocol_version(ProtocolVersion::V_2025_03_26)
        .with_instructions(
            "Lekton documentation server. Available tools:\n\
             - get_index: Browse the document tree\n\
             - search_docs: Semantic search across documentation\n\
             - read_document: Read the full text of a specific document"
                .to_string(),
        )
    }
}
