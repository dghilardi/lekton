//! Centralised application configuration.
//!
//! Configuration is loaded in priority order (highest wins):
//!
//! 1. Environment variables with prefix `LKN__` and `__` as the nesting separator.
//!    Examples: `LKN__DATABASE__URI`, `LKN__AUTH__JWT_SECRET`.
//! 2. `config/lekton.toml` — optional local override file (git-ignored).
//! 3. `config/default.toml` — embedded defaults shipped with the binary.
//!
//! AWS credentials (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`)
//! are **not** part of this config: they are read directly by the `aws-config`
//! crate using its standard credential chain.
//!
//! `LEPTOS_SITE_ADDR` is also excluded: it is managed by `cargo-leptos`.

use std::collections::HashMap;

use serde::Deserialize;

// ── Top-level config ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub search: SearchConfig,
    pub auth: AuthConfig,
    pub mcp: McpConfig,
    pub rag: RagConfig,
}

// ── Server ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Maximum burst size for the rate limiter (requests per second replenishment = 1).
    pub rate_limit_burst: u32,
    /// Comma-separated allowed CORS origins. Empty/unset means same-origin only.
    pub cors_allowed_origins: Option<String>,
    /// Allow non-HTTPS cookies (local dev over HTTP).
    pub insecure_cookies: bool,
    /// `tracing-subscriber` filter string. Falls back when `RUST_LOG` is unset.
    pub log_filter: String,
    /// Maximum attachment size in megabytes.
    pub max_attachment_size_mb: u64,
}

// ── Database ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    /// Full MongoDB connection URI.
    pub uri: String,
    /// MongoDB database name.
    pub name: String,
    /// Optional username — injected into the URI when set.
    pub username: Option<String>,
    /// Optional password — injected into the URI when set.
    pub password: Option<String>,
}

// ── Storage ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    /// S3 bucket name. Required.
    pub bucket: String,
    /// Custom S3 endpoint for MinIO / Garage / LocalStack.
    pub endpoint: Option<String>,
}

// ── Search ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchConfig {
    /// Meilisearch base URL. Empty or unset disables search.
    pub url: String,
    /// Meilisearch API key. Optional.
    pub api_key: String,
}

// ── Auth ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AuthConfig {
    /// Enable built-in demo authentication (not for production).
    pub demo_mode: bool,
    /// Allow demo mode even when a JWT secret is present.
    pub allow_demo_in_production: bool,
    /// API service token for ingestion endpoints. Required in production.
    pub service_token: Option<String>,
    /// HMAC secret for JWT signing. Required when `demo_mode = false`.
    pub jwt_secret: Option<String>,
    /// JWT access token lifetime in seconds.
    pub jwt_access_ttl_secs: u64,
    /// Refresh token lifetime in days.
    pub jwt_refresh_ttl_days: i64,
    /// Expected JWT issuer for access tokens created by Lekton.
    pub jwt_issuer: String,
    /// Expected JWT audience for access tokens created by Lekton.
    pub jwt_audience: String,
    /// OAuth2 provider type: `"oidc"` (default) or `"oauth2"`.
    pub provider_type: String,
    /// OAuth2 client ID.
    pub client_id: Option<String>,
    /// OAuth2 client secret.
    pub client_secret: Option<String>,
    /// OAuth2 redirect URI registered with the provider.
    pub redirect_uri: Option<String>,
    /// OIDC issuer/discovery URL or OAuth2 authorization endpoint.
    pub authorization_endpoint: Option<String>,
    /// Token endpoint (required for `oauth2`; optional for `oidc`).
    pub token_endpoint: Option<String>,
    /// Userinfo endpoint (required for `oauth2`; optional for `oidc`).
    pub userinfo_endpoint: Option<String>,
    /// OAuth2 scopes to request (space-separated).
    pub scopes: String,
    /// Dot-notation path to the subject field in the userinfo response.
    pub userinfo_sub_field: Option<String>,
    /// Dot-notation path to the email field in the userinfo response.
    pub userinfo_email_field: Option<String>,
    /// Comma-separated dot-notation paths to name field(s) in the userinfo response.
    pub userinfo_name_field: Option<String>,
}

// ── MCP ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct McpConfig {
    /// Allowed `Host` header values for the MCP Streamable HTTP endpoint.
    /// Used by rmcp's DNS rebinding protection.
    /// When empty, the host check is disabled (any host is accepted).
    /// Default: `["localhost", "127.0.0.1", "::1"]`.
    #[serde(default = "default_mcp_allowed_hosts")]
    pub allowed_hosts: Vec<String>,
    /// Keep MCP Streamable HTTP sessions across requests.
    ///
    /// Stateful mode enables `Mcp-Session-Id` and SSE resume support, but
    /// sessions are local to the current server process. Set to `false` for
    /// stateless request/response behaviour that is more robust across server
    /// restarts and non-sticky load balancers.
    pub stateful_mode: bool,
    /// Return direct JSON responses in stateless mode instead of SSE-framed
    /// responses. Ignored when `stateful_mode = true`.
    pub json_response: bool,
    /// Inactivity timeout for stateful MCP sessions, in seconds.
    ///
    /// `null` disables the timeout. The rmcp default is 300 seconds.
    pub session_keep_alive_secs: Option<u64>,
    /// How long completed request streams can be resumed, in seconds.
    ///
    /// Only applies in stateful mode.
    pub completed_cache_ttl_secs: u64,
}

fn default_mcp_allowed_hosts() -> Vec<String> {
    vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]
}

// ── RAG ──────────────────────────────────────────────────────────────────────

/// Base LLM configuration shared across all RAG pipeline steps.
///
/// Each individual step (`chat`, `analyzer`, `hyde`, `rewriter`) may override
/// any of these fields. Fields left unset in a step config fall back to the
/// values defined here.
///
/// Via env: `LKN__RAG__LLM__URL`, `LKN__RAG__LLM__HEADERS__X_FOO`, …
#[derive(Debug, Deserialize)]
pub struct LlmConfig {
    /// OpenAI-compatible base URL (e.g. `https://api.openai.com/v1`).
    pub url: String,
    /// API key for the LLM endpoint.
    pub api_key: String,
    /// Default model name. Can be overridden per step.
    #[serde(default)]
    pub model: String,
    /// Extra HTTP headers added to every LLM request.
    ///
    /// Keys are normalised before use: underscores (`_`) → hyphens (`-`).
    /// Via env: `LKN__RAG__LLM__HEADERS__X_PRODUCER=LEKTON` → `x-producer: LEKTON`
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Google Cloud project ID. When set, Vertex AI is used instead of the
    /// OpenAI-compatible endpoint.
    #[serde(default)]
    pub vertex_project_id: String,
    /// Vertex AI region. Defaults to `us-central1` when empty.
    #[serde(default)]
    pub vertex_location: String,
}

/// Per-step LLM configuration. All fields except `model` are optional and fall
/// back to the corresponding field in [`LlmConfig`] when absent.
///
/// A step is **disabled** when its `[rag.<step>]` TOML section is entirely absent
/// (deserialises as `None`). When the section is present, `model` is required.
#[derive(Debug, Deserialize, Clone)]
pub struct LlmStepConfig {
    /// Model name for this step (required).
    pub model: String,
    /// Override the LLM endpoint URL. Falls back to `rag.llm.url`.
    pub url: Option<String>,
    /// Override the API key. Falls back to `rag.llm.api_key`.
    pub api_key: Option<String>,
    /// Override HTTP headers. `None` inherits from `rag.llm.headers`;
    /// `Some({})` sends no extra headers.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Override the Vertex AI project ID. Falls back to `rag.llm.vertex_project_id`.
    pub vertex_project_id: Option<String>,
    /// Override the Vertex AI location. Falls back to `rag.llm.vertex_location`.
    pub vertex_location: Option<String>,
    /// Maximum tokens for this step's LLM call.
    /// Each service has its own built-in default when this is absent.
    pub max_tokens: Option<u32>,
}

/// Configuration for the main chat step. Always active when RAG chat is enabled.
/// All LLM fields are optional and fall back to [`LlmConfig`].
#[derive(Debug, Deserialize)]
pub struct ChatStepConfig {
    /// Override the model. Falls back to `rag.llm.model`.
    pub model: Option<String>,
    /// Override the LLM endpoint URL. Falls back to `rag.llm.url`.
    pub url: Option<String>,
    /// Override the API key. Falls back to `rag.llm.api_key`.
    pub api_key: Option<String>,
    /// Override HTTP headers. `None` inherits from `rag.llm.headers`.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Override the Vertex AI project ID. Falls back to `rag.llm.vertex_project_id`.
    pub vertex_project_id: Option<String>,
    /// Override the Vertex AI location. Falls back to `rag.llm.vertex_location`.
    pub vertex_location: Option<String>,
    /// Tera template for the system prompt. Variables: `{{ context }}`, `{{ question }}`.
    pub system_prompt_template: String,
}

#[derive(Debug, Deserialize)]
pub struct RagConfig {
    /// Qdrant gRPC endpoint (e.g. `http://localhost:6334`). Empty disables RAG.
    pub qdrant_url: String,
    /// Qdrant collection name.
    pub qdrant_collection: String,
    /// OpenAI-compatible embedding endpoint.
    pub embedding_url: String,
    /// Embedding model name (e.g. `nomic-embed-text`).
    pub embedding_model: String,
    /// Vector dimensions produced by the embedding model.
    pub embedding_dimensions: u32,
    /// API key for the embedding endpoint. Optional.
    pub embedding_api_key: String,
    /// Extra HTTP headers for embedding requests. Same normalisation rules as LLM headers.
    #[serde(default)]
    pub embedding_headers: HashMap<String, String>,
    /// When `true`, the original chunk text is stored alongside its embedding in the cache.
    #[serde(default)]
    pub embedding_cache_store_text: bool,
    /// When `true`, embeddings generated for chat queries are also cached.
    #[serde(default)]
    pub embedding_cache_query: bool,
    /// Target chunk size in tokens (cl100k_base). Default: 256.
    pub chunk_size_tokens: u32,
    /// Overlap between consecutive chunks in tokens.
    pub chunk_overlap_tokens: u32,
    /// When `true`, reranked chunks are expanded to their full parent section.
    #[serde(default)]
    pub expand_to_parent: bool,
    /// When `true`, combines Qdrant vector search with Meilisearch full-text search via RRF.
    #[serde(default)]
    pub hybrid_search_enabled: bool,
    /// Cross-encoder reranker endpoint (Jina/Infinity/Cohere-compatible `/rerank` API).
    /// Empty string disables reranking.
    pub reranker_url: String,
    /// Model name passed to the reranker endpoint. Optional for self-hosted servers.
    pub reranker_model: String,
    /// API key for the reranker endpoint. Optional.
    pub reranker_api_key: String,
    /// Extra HTTP headers for reranker requests. Same normalisation rules as LLM headers.
    #[serde(default)]
    pub reranker_headers: HashMap<String, String>,

    /// Default LLM configuration shared by all pipeline steps.
    pub llm: LlmConfig,

    /// Main chat step configuration. Required for RAG chat to function.
    pub chat: ChatStepConfig,

    /// Query analyzer: classifies complexity and decomposes into sub-queries.
    /// Absent = disabled (all queries treated as simple).
    #[serde(default)]
    pub analyzer: Option<LlmStepConfig>,

    /// HyDE: generates a hypothetical answer document for embedding instead of the raw query.
    /// Absent = disabled.
    #[serde(default)]
    pub hyde: Option<LlmStepConfig>,

    /// Query rewriter: rewrites follow-up questions into standalone queries for multi-turn chat.
    /// Absent = disabled (first-turn pass-through only).
    #[serde(default)]
    pub rewriter: Option<LlmStepConfig>,
}

impl RagConfig {
    /// Returns `true` when RAG is fully configured (both Qdrant and embedding URLs set).
    pub fn is_enabled(&self) -> bool {
        !self.qdrant_url.is_empty() && !self.embedding_url.is_empty()
    }

    /// Validates numeric and cross-field constraints that `is_enabled` does not cover.
    /// Call this after loading config when RAG is enabled.
    pub fn validate(&self) -> Result<(), String> {
        if self.embedding_dimensions == 0 {
            return Err("rag.embedding_dimensions must be > 0".into());
        }
        if self.chunk_size_tokens == 0 {
            return Err("rag.chunk_size_tokens must be > 0".into());
        }
        if self.chunk_overlap_tokens >= self.chunk_size_tokens {
            return Err(format!(
                "rag.chunk_overlap_tokens ({}) must be < chunk_size_tokens ({})",
                self.chunk_overlap_tokens, self.chunk_size_tokens
            ));
        }
        Ok(())
    }
}

/// Fully resolved LLM configuration for a single pipeline step, after merging
/// step-level overrides with the base [`LlmConfig`].
#[cfg(feature = "ssr")]
pub struct ResolvedLlmConfig {
    pub url: String,
    pub api_key: String,
    pub model: String,
    pub headers: HashMap<String, String>,
    /// `None` when Vertex AI is not configured for this step.
    pub vertex_project_id: Option<String>,
    /// `None` when using the Vertex AI default location (`us-central1`).
    pub vertex_location: Option<String>,
}

#[cfg(feature = "ssr")]
fn non_empty(s: &str) -> Option<String> {
    let t = s.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

#[cfg(feature = "ssr")]
impl RagConfig {
    /// Resolve the chat step config by merging `rag.chat` overrides onto `rag.llm`.
    pub fn resolve_chat(&self) -> ResolvedLlmConfig {
        ResolvedLlmConfig {
            url: self
                .chat
                .url
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.url))
                .unwrap_or_default(),
            api_key: self
                .chat
                .api_key
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.api_key))
                .unwrap_or_default(),
            model: self
                .chat
                .model
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.model))
                .unwrap_or_default(),
            headers: self
                .chat
                .headers
                .clone()
                .unwrap_or_else(|| self.llm.headers.clone()),
            vertex_project_id: self
                .chat
                .vertex_project_id
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.vertex_project_id)),
            vertex_location: self
                .chat
                .vertex_location
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.vertex_location)),
        }
    }

    /// Resolve a pipeline step config by merging its overrides onto `rag.llm`.
    pub fn resolve_step(&self, step: &LlmStepConfig) -> ResolvedLlmConfig {
        ResolvedLlmConfig {
            url: step
                .url
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.url))
                .unwrap_or_default(),
            api_key: step
                .api_key
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.api_key))
                .unwrap_or_default(),
            model: step.model.clone(),
            headers: step
                .headers
                .clone()
                .unwrap_or_else(|| self.llm.headers.clone()),
            vertex_project_id: step
                .vertex_project_id
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.vertex_project_id)),
            vertex_location: step
                .vertex_location
                .as_deref()
                .and_then(non_empty)
                .or_else(|| non_empty(&self.llm.vertex_location)),
        }
    }
}

// ── Loader ────────────────────────────────────────────────────────────────────

#[cfg(feature = "ssr")]
impl AppConfig {
    /// Load and merge configuration from all sources.
    ///
    /// Panics with a clear message on deserialization failure — this is
    /// intentional: a misconfigured binary should fail fast at startup.
    pub fn load() -> Result<Self, config::ConfigError> {
        config::Config::builder()
            // Embedded defaults — always present
            .add_source(config::File::from_str(
                include_str!("../config/default.toml"),
                config::FileFormat::Toml,
            ))
            // Optional local override (e.g. developer's config/lekton.toml)
            .add_source(config::File::with_name("config/lekton").required(false))
            // Environment variables with prefix LKN__ and __ as separator.
            // try_parsing(true) allows parsing "true", "false", and numbers from env vars.
            .add_source(
                config::Environment::with_prefix("LKN")
                    .prefix_separator("__")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?
            .try_deserialize()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "ssr")]
    fn test_config_env() {
        std::env::set_var("LKN__STORAGE__BUCKET", "testing-bucket");
        std::env::set_var("LKN__AUTH__DEMO_MODE", "true");
        std::env::set_var("LKN__SERVER__RATE_LIMIT_BURST", "123");

        let config = super::AppConfig::load().expect("Failed to load config with env vars");

        assert_eq!(config.storage.bucket, "testing-bucket");
        assert!(config.auth.demo_mode);
        assert_eq!(config.server.rate_limit_burst, 123);
    }

    #[test]
    #[cfg(feature = "ssr")]
    fn test_mcp_session_config_defaults() {
        let config = super::AppConfig::load().expect("Failed to load config");

        assert!(config.mcp.stateful_mode);
        assert!(!config.mcp.json_response);
        assert_eq!(config.mcp.session_keep_alive_secs, Some(300));
        assert_eq!(config.mcp.completed_cache_ttl_secs, 60);
    }

    #[test]
    #[cfg(feature = "ssr")]
    fn test_rag_headers_from_env() {
        // Underscores in the env-var key segment are normalised to hyphens at request time,
        // but config-rs stores them as-is.  Verify that the raw key is loaded correctly.
        std::env::set_var("LKN__RAG__LLM__HEADERS__X_PRODUCER", "LEKTON");
        std::env::set_var(
            "LKN__RAG__EMBEDDING_HEADERS__AUTHORIZATION_EXTRA",
            "Bearer tok",
        );

        let config =
            super::AppConfig::load().expect("Failed to load config with rag header env vars");

        assert_eq!(
            config.rag.llm.headers.get("x_producer").map(String::as_str),
            Some("LEKTON")
        );
        assert_eq!(
            config
                .rag
                .embedding_headers
                .get("authorization_extra")
                .map(String::as_str),
            Some("Bearer tok")
        );
    }

    #[test]
    #[cfg(feature = "ssr")]
    fn test_rag_vertex_provider_from_env() {
        std::env::set_var("LKN__RAG__LLM__VERTEX_PROJECT_ID", "test-project");
        std::env::set_var("LKN__RAG__LLM__VERTEX_LOCATION", "europe-west1");

        let config =
            super::AppConfig::load().expect("Failed to load config with Vertex AI env vars");

        assert_eq!(config.rag.llm.vertex_project_id, "test-project");
        assert_eq!(config.rag.llm.vertex_location, "europe-west1");
    }

    #[test]
    #[cfg(feature = "ssr")]
    fn test_rag_step_config_resolve() {
        use std::collections::HashMap;

        let config = super::AppConfig::load().unwrap();

        // Analyzer absent in defaults → None
        assert!(config.rag.analyzer.is_none());
        assert!(config.rag.hyde.is_none());
        assert!(config.rag.rewriter.is_none());

        // resolve_chat falls back to llm.model when chat.model is None
        let resolved = config.rag.resolve_chat();
        assert_eq!(resolved.model, config.rag.llm.model);
        let _ = HashMap::<String, String>::new(); // suppress unused import
    }
}
