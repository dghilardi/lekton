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

use serde::Deserialize;

// ── Top-level config ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub search: SearchConfig,
    pub auth: AuthConfig,
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

// ── RAG ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RagConfig {
    /// Qdrant gRPC endpoint (e.g. `http://localhost:6334`). Empty disables RAG.
    pub qdrant_url: String,
    /// Qdrant collection name. Switchable for model fine-tuning.
    pub qdrant_collection: String,
    /// OpenAI-compatible embedding endpoint (e.g. Ollama at `http://localhost:11434/v1`).
    pub embedding_url: String,
    /// Embedding model name (e.g. `nomic-embed-text`).
    pub embedding_model: String,
    /// Vector dimensions produced by the embedding model.
    pub embedding_dimensions: u32,
    /// API key for the embedding endpoint. Optional.
    pub embedding_api_key: String,
    /// OpenAI-compatible chat/completion endpoint (e.g. OpenRouter, Ollama).
    pub chat_url: String,
    /// Chat model name (e.g. `meta-llama/llama-3-70b`).
    pub chat_model: String,
    /// API key for the chat endpoint. Optional.
    pub chat_api_key: String,
    /// Tera template for the system prompt. Available variables: `{{context}}`, `{{question}}`.
    pub system_prompt_template: String,
}

impl RagConfig {
    /// Returns `true` when RAG is fully configured (both Qdrant and embedding URLs set).
    pub fn is_enabled(&self) -> bool {
        !self.qdrant_url.is_empty() && !self.embedding_url.is_empty()
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
}
