/// Middleware that sets `Content-Type: application/javascript` for `.mjs` files.
/// tower-http's ServeDir does not recognise the `.mjs` extension in all versions
/// of mime_guess, causing browsers to block ES module imports.
#[cfg(feature = "ssr")]
async fn mjs_content_type(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let is_mjs = request.uri().path().ends_with(".mjs");
    let mut response = next.run(request).await;
    if is_mjs {
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/javascript"),
        );
    }
    response
}

/// Middleware that sets `Cache-Control` headers for static JS/CSS assets under `/js/`.
/// Assets requested with a `?v=` version query get a 1-year immutable cache.
/// Assets without a version query get a 1-hour cache (safe without fingerprinting).
#[cfg(feature = "ssr")]
async fn static_cache_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path().to_owned();
    let has_version = request
        .uri()
        .query()
        .is_some_and(|q| q.split('&').any(|p| p == "v" || p.starts_with("v=")));

    let is_js_asset = path.starts_with("/js/")
        && (path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".css"));

    let mut response = next.run(request).await;

    if is_js_asset {
        let cache_header = if has_version {
            axum::http::HeaderValue::from_static("public, max-age=31536000, immutable")
        } else {
            axum::http::HeaderValue::from_static("public, max-age=3600")
        };
        response
            .headers_mut()
            .insert(axum::http::header::CACHE_CONTROL, cache_header);
    }

    response
}

/// Build the public + admin REST API route surface (state applied by the caller).
///
/// Upload endpoints get a 50 MB body limit; all other routes use Axum's default
/// 2 MB limit. Keeping the full route surface in one auditable function makes it
/// easy to review which endpoints exist and how they are authenticated.
#[cfg(feature = "ssr")]
fn api_routes(features: &lekton::app::FeatureFlags) -> axum::Router<lekton::app::AppState> {
    use axum::Router;
    use lekton::api;

    // Core + read-only routes that are always available regardless of feature
    // flags. Asset *serving* and listing stay on even in read-only mode; only
    // the write/upload surface is gated by the editor feature below.
    let mut router = Router::new()
        // Health endpoints
        .route("/health", axum::routing::get(api::health::liveness_handler))
        .route(
            "/health/ready",
            axum::routing::get(api::health::readiness_handler),
        )
        // API routes
        .route(
            "/api/v1/ingest",
            axum::routing::post(api::ingest::ingest_handler),
        )
        .route(
            "/api/v1/image/{filename}",
            axum::routing::get(api::upload::serve_image_handler),
        )
        .route("/api/v1/sync", axum::routing::post(api::sync::sync_handler))
        .route(
            "/api/v1/assets",
            axum::routing::get(api::assets::list_assets_handler),
        )
        // Admin API
        .route(
            "/api/v1/admin/access-levels",
            axum::routing::get(api::admin::list_access_levels_handler)
                .post(api::admin::create_access_level_handler),
        )
        .route(
            "/api/v1/admin/access-levels/{name}",
            axum::routing::put(api::admin::update_access_level_handler)
                .delete(api::admin::delete_access_level_handler),
        )
        .route(
            "/api/v1/admin/users",
            axum::routing::get(api::admin::list_users_handler),
        )
        .route(
            "/api/v1/admin/users/{user_id}",
            axum::routing::get(api::admin::get_user_handler),
        )
        .route(
            "/api/v1/admin/users/{user_id}/access-levels",
            axum::routing::put(api::admin::set_user_access_levels_handler),
        )
        .route(
            "/api/v1/admin/service-tokens",
            axum::routing::get(api::admin::list_service_tokens_handler)
                .post(api::admin::create_service_token_handler),
        )
        .route(
            "/api/v1/admin/service-tokens/{id}",
            axum::routing::delete(api::admin::deactivate_service_token_handler),
        )
        // PAT management (user self-service + admin)
        .route(
            "/api/v1/user/pats",
            axum::routing::get(api::pat::list_user_pats_handler)
                .post(api::pat::create_user_pat_handler),
        )
        .route(
            "/api/v1/user/pats/{id}",
            axum::routing::patch(api::pat::toggle_user_pat_handler)
                .delete(api::pat::delete_user_pat_handler),
        )
        .route(
            "/api/v1/admin/pats",
            axum::routing::get(api::pat::admin_list_pats_handler),
        )
        .route(
            "/api/v1/admin/pats/{id}",
            axum::routing::patch(api::pat::admin_toggle_pat_handler),
        );

    // ── Search (Meilisearch) ────────────────────────────────────────────────
    if features.search {
        router = router
            .route(
                "/api/v1/search",
                axum::routing::get(api::search::search_handler),
            )
            .route(
                "/api/v1/admin/search/reindex",
                axum::routing::post(api::search::trigger_reindex_handler),
            )
            .route(
                "/api/v1/admin/search/reindex/status",
                axum::routing::get(api::search::reindex_status_handler),
            );
    }

    // ── Schema registry ─────────────────────────────────────────────────────
    if features.schema_registry {
        router = router
            .route(
                "/api/v1/schemas",
                axum::routing::get(api::schemas::list_schemas_handler)
                    .post(api::schemas::ingest_schema_handler),
            )
            .route(
                "/api/v1/schemas/sync",
                axum::routing::post(api::schemas::schema_sync_handler),
            )
            .route(
                "/api/v1/schemas/{*rest}",
                axum::routing::get(api::schemas::get_schema_route_handler),
            )
            .route(
                "/api/v1/admin/schemas/reindex-endpoints",
                axum::routing::post(api::schemas::trigger_schema_endpoint_reindex_handler),
            )
            .route(
                "/api/v1/admin/schemas/reindex-endpoints/status",
                axum::routing::get(api::schemas::schema_endpoint_reindex_status_handler),
            );
    }

    // ── Prompt library ──────────────────────────────────────────────────────
    if features.prompt_library {
        router = router
            .route(
                "/api/v1/prompts/ingest",
                axum::routing::post(api::prompts::prompt_ingest_handler),
            )
            .route(
                "/api/v1/prompts/sync",
                axum::routing::post(api::prompts::prompt_sync_handler),
            );
    }

    // ── RAG (chat + indexing) ───────────────────────────────────────────────
    if features.rag {
        router = router
            .route(
                "/api/v1/admin/rag/reindex",
                axum::routing::post(api::rag::trigger_reindex_handler),
            )
            .route(
                "/api/v1/admin/rag/reindex/status",
                axum::routing::get(api::rag::reindex_status_handler),
            )
            .route(
                "/api/v1/admin/rag/feedback",
                axum::routing::get(api::rag::admin_list_feedback_handler),
            )
            .route(
                "/api/v1/rag/chat",
                axum::routing::post(api::rag::chat_handler),
            )
            .route(
                "/api/v1/rag/sessions",
                axum::routing::get(api::rag::list_sessions_handler),
            )
            .route(
                "/api/v1/rag/sessions/{id}",
                axum::routing::delete(api::rag::delete_session_handler),
            )
            .route(
                "/api/v1/rag/sessions/{id}/messages",
                axum::routing::get(api::rag::get_session_messages_handler),
            )
            .route(
                "/api/v1/rag/messages/{id}/feedback",
                axum::routing::post(api::rag::submit_feedback_handler)
                    .delete(api::rag::delete_feedback_handler),
            );
    }

    // ── Editor (write surface) ──────────────────────────────────────────────
    // Asset serving (GET /api/v1/assets/{*key}) and listing stay available in
    // read-only mode; only uploads/deletes and image upload are gated here.
    if features.editor {
        let editor_uploads = Router::new()
            .route(
                "/api/v1/upload-image",
                axum::routing::post(api::upload::upload_image_handler),
            )
            .route(
                "/api/v1/editor/upload-asset",
                axum::routing::post(api::assets::editor_upload_asset_handler),
            )
            .route(
                "/api/v1/assets/check-hashes",
                axum::routing::post(api::assets::check_hashes_handler),
            )
            .route(
                "/api/v1/assets/{*key}",
                axum::routing::put(api::assets::upload_asset_handler)
                    .get(api::assets::serve_asset_handler)
                    .delete(api::assets::delete_asset_handler),
            )
            .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)); // 50 MB
        router = router.merge(editor_uploads);
    } else {
        router = router.route(
            "/api/v1/assets/{*key}",
            axum::routing::get(api::assets::serve_asset_handler),
        );
    }

    // ── Document upload (admin form) ─────────────────────────────────────────
    // Admin-only PDF upload backing the guided document-upload form. Gated by
    // its own feature so it works even when the editor is off (read-only portal).
    if features.document_upload {
        router = router.route(
            "/api/v1/document-upload/asset",
            axum::routing::post(api::assets::admin_upload_asset_handler)
                .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)), // 50 MB
        );
    }

    // AI summary streaming for the document-upload form. Requires both
    // document_upload and rag (needs the chat LLM).
    if features.document_upload && features.rag {
        router = router.route(
            "/api/v1/document-upload/summary",
            axum::routing::get(api::document_upload::summary_stream_handler),
        );
    }

    router
}

/// Build the auth route surface: demo auth when `demo_mode`, OAuth2/OIDC otherwise.
#[cfg(feature = "ssr")]
fn auth_routes(demo_mode: bool) -> axum::Router<lekton::app::AppState> {
    use axum::Router;

    if demo_mode {
        use lekton::auth::demo_auth;
        Router::new()
            .route(
                "/api/auth/login",
                axum::routing::post(demo_auth::login_handler),
            )
            .route("/api/auth/me", axum::routing::get(demo_auth::me_handler))
            .route(
                "/api/auth/logout",
                axum::routing::post(demo_auth::logout_handler),
            )
    } else {
        use lekton::api::auth as auth_api;
        Router::new()
            .route("/auth/login", axum::routing::get(auth_api::login_handler))
            .route(
                "/auth/callback",
                axum::routing::get(auth_api::callback_handler),
            )
            .route(
                "/auth/refresh",
                axum::routing::post(auth_api::refresh_handler),
            )
            .route(
                "/auth/logout",
                axum::routing::post(auth_api::logout_handler),
            )
            .route(
                "/auth/refresh/logout",
                axum::routing::post(auth_api::logout_handler),
            )
            .route("/auth/me", axum::routing::get(auth_api::me_handler))
    }
}

/// Build the MCP server route (`/mcp`, Streamable HTTP) with PAT auth applied.
///
/// Kept alongside api_routes()/auth_routes() so the whole route surface is
/// auditable in one place. Mounted whenever the MCP feature is enabled; the
/// embedding/vectorstore are optional and only the RAG `search_documents` tool
/// requires them — the server hides tools whose feature is disabled.
#[cfg(feature = "ssr")]
fn mcp_routes(
    app_state: &lekton::app::AppState,
    config: &lekton::config::AppConfig,
    emb: Option<std::sync::Arc<dyn lekton::rag::embedding::EmbeddingService>>,
    vs: Option<std::sync::Arc<dyn lekton::rag::vectorstore::VectorStore>>,
    features: lekton::app::FeatureFlags,
) -> axum::Router<lekton::app::AppState> {
    use axum::Router;
    use lekton::mcp::auth::{pat_auth_middleware, McpAuthState};
    use lekton::mcp::server::LektonMcpServer;
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, tower::StreamableHttpService,
        StreamableHttpServerConfig,
    };

    let doc_repo = app_state.document_repo.clone();
    let schema_repo = app_state.schema_repo.clone();
    let prompt_repo = app_state.prompt_repo.clone();
    let user_prompt_preference_repo = app_state.user_prompt_preference_repo.clone();
    let documentation_feedback_repo = app_state.documentation_feedback_repo.clone();
    let storage = app_state.storage_client.clone();

    let mcp_config = if config.mcp.allowed_hosts.is_empty() {
        StreamableHttpServerConfig::default().disable_allowed_hosts()
    } else {
        StreamableHttpServerConfig::default().with_allowed_hosts(config.mcp.allowed_hosts.clone())
    }
    .with_stateful_mode(config.mcp.stateful_mode)
    .with_json_response(config.mcp.json_response);

    let mut session_manager = LocalSessionManager::default();
    session_manager.session_config.keep_alive = config
        .mcp
        .session_keep_alive_secs
        .map(std::time::Duration::from_secs);
    session_manager.session_config.completed_cache_ttl =
        std::time::Duration::from_secs(config.mcp.completed_cache_ttl_secs);

    let mcp_service = StreamableHttpService::new(
        move || {
            Ok(LektonMcpServer::new(
                doc_repo.clone(),
                schema_repo.clone(),
                prompt_repo.clone(),
                user_prompt_preference_repo.clone(),
                documentation_feedback_repo.clone(),
                storage.clone(),
                emb.clone(),
                vs.clone(),
                features,
            ))
        },
        session_manager.into(),
        mcp_config,
    );

    let mcp_auth = McpAuthState {
        service_token_repo: app_state.service_token_repo.clone(),
        user_repo: app_state.user_repo.clone(),
    };

    Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(axum::middleware::from_fn_with_state(
            mcp_auth,
            pat_auth_middleware,
        ))
}

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::middleware;
    use lekton::app::App;
    use lekton::auth::provider::build_provider;
    use lekton::auth::token_service::TokenService;
    use lekton::db::access_level_repository::MongoAccessLevelRepository;
    use lekton::db::asset_repository::MongoAssetRepository;
    use lekton::db::document_version_repository::MongoDocumentVersionRepository;
    use lekton::db::documentation_feedback_repository::MongoDocumentationFeedbackRepository;
    use lekton::db::navigation_order_repository::MongoNavigationOrderRepository;
    use lekton::db::prompt_repository::MongoPromptRepository;
    use lekton::db::prompt_version_repository::MongoPromptVersionRepository;
    use lekton::db::repository::MongoDocumentRepository;
    use lekton::db::schema_repository::MongoSchemaRepository;
    use lekton::db::service_token_repository::MongoServiceTokenRepository;
    use lekton::db::settings_repository::MongoSettingsRepository;
    use lekton::db::user_prompt_preference_repository::MongoUserPromptPreferenceRepository;
    use lekton::db::user_repository::MongoUserRepository;
    use lekton::search::attachment_search::AttachmentSearchService as _;
    use lekton::search::client::{MeilisearchService, SearchService as _};
    use lekton::storage::client::S3StorageClient;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tower_http::services::ServeDir;

    // Install a default rustls CryptoProvider before any TLS connections are made.
    // Both `aws-lc-rs` and `ring` end up in the dependency tree (via gcp_auth + other crates),
    // so rustls cannot auto-detect the provider and panics unless we set one explicitly.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls CryptoProvider");

    // Load configuration first — fast-fail on bad config before anything else starts.
    let config =
        lekton::config::AppConfig::load().expect("Failed to load application configuration");

    // Fail-fast: reject features that are enabled without their prerequisites.
    config
        .validate_features()
        .unwrap_or_else(|e| panic!("Invalid feature configuration: {e}"));

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.server.log_filter.as_str().into()),
        )
        .init();

    tracing::info!("Starting Lekton server...");
    tracing::debug!(demo_mode = config.auth.demo_mode, "auth config loaded");

    // Check demo mode
    let demo_mode = config.auth.demo_mode;

    if demo_mode {
        if config.auth.jwt_secret.is_some() && !config.auth.allow_demo_in_production {
            panic!(
                "auth.demo_mode is enabled but auth.jwt_secret is set, which suggests a \
                 production environment. Set auth.allow_demo_in_production = true (or \
                 LKN__AUTH__ALLOW_DEMO_IN_PRODUCTION=true) to override this safety check."
            );
        }

        tracing::warn!(
            "⚠️  DEMO MODE ENABLED — built-in credentials are active. Do NOT use in production!"
        );
    }

    // Load Leptos options from Cargo.toml metadata
    let conf =
        get_configuration(None).expect("Failed to load Leptos configuration from Cargo.toml");
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let site_root = leptos_options.site_root.to_string();

    // Compute asset fingerprints for cache-busting versioned URLs.
    lekton::static_assets::init(&site_root);

    // Connect to MongoDB
    // Inject credentials into the URI if provided separately.
    // Using the `url` crate for correct percent-encoding and authority handling.
    let mongo_uri = match (&config.database.username, &config.database.password) {
        (Some(user), Some(pass)) if !user.is_empty() => {
            match url::Url::parse(&config.database.uri) {
                Ok(mut parsed) => {
                    let _ = parsed.set_username(user);
                    let _ = parsed.set_password(Some(pass));
                    parsed.to_string()
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Could not parse MongoDB URI to inject credentials; using URI as-is");
                    config.database.uri.clone()
                }
            }
        }
        _ => config.database.uri.clone(),
    };

    let mut mongo_options = mongodb::options::ClientOptions::parse(&mongo_uri)
        .await
        .expect("Failed to parse MongoDB URI");
    mongo_options.app_name = Some(config.database.app_name.clone());
    mongo_options.server_selection_timeout = Some(std::time::Duration::from_millis(
        config.database.server_selection_timeout_ms,
    ));
    mongo_options.connect_timeout = Some(std::time::Duration::from_millis(
        config.database.connect_timeout_ms,
    ));
    mongo_options.max_pool_size = Some(config.database.max_pool_size);
    mongo_options.min_pool_size = config.database.min_pool_size;

    let mongo_client =
        mongodb::Client::with_options(mongo_options).expect("Failed to connect to MongoDB");
    let mongo_db = mongo_client.database(&config.database.name);
    let document_repo: Arc<dyn lekton::db::repository::DocumentRepository> =
        Arc::new(MongoDocumentRepository::new(&mongo_db));
    let schema_repo: Arc<dyn lekton::db::schema_repository::SchemaRepository> =
        Arc::new(MongoSchemaRepository::new(&mongo_db));
    let settings_repo: Arc<dyn lekton::db::settings_repository::SettingsRepository> =
        Arc::new(MongoSettingsRepository::new(&mongo_db));
    let asset_repo: Arc<dyn lekton::db::asset_repository::AssetRepository> =
        Arc::new(MongoAssetRepository::new(&mongo_db));
    let user_repo: Arc<dyn lekton::db::user_repository::UserRepository> =
        Arc::new(MongoUserRepository::new(&mongo_db));
    let access_level_repo: Arc<dyn lekton::db::access_level_repository::AccessLevelRepository> =
        Arc::new(MongoAccessLevelRepository::new(&mongo_db));
    let service_token_repo: Arc<dyn lekton::db::service_token_repository::ServiceTokenRepository> =
        Arc::new(MongoServiceTokenRepository::new(&mongo_db));
    let document_version_repo: Arc<
        dyn lekton::db::document_version_repository::DocumentVersionRepository,
    > = Arc::new(MongoDocumentVersionRepository::new(&mongo_db));
    let prompt_repo: Arc<dyn lekton::db::prompt_repository::PromptRepository> =
        Arc::new(MongoPromptRepository::new(&mongo_db));
    let prompt_version_repo: Arc<
        dyn lekton::db::prompt_version_repository::PromptVersionRepository,
    > = Arc::new(MongoPromptVersionRepository::new(&mongo_db));
    let user_prompt_preference_repo: Arc<
        dyn lekton::db::user_prompt_preference_repository::UserPromptPreferenceRepository,
    > = Arc::new(MongoUserPromptPreferenceRepository::new(&mongo_db));
    let navigation_order_repo: Arc<
        dyn lekton::db::navigation_order_repository::NavigationOrderRepository,
    > = Arc::new(MongoNavigationOrderRepository::new(&mongo_db));
    let chat_repo: Option<Arc<dyn lekton::db::chat_repository::ChatRepository>> =
        if config.features.rag {
            Some(Arc::new(
                lekton::db::chat_repository::MongoChatRepository::new(&mongo_db),
            ))
        } else {
            None
        };
    let feedback_repo: Option<Arc<dyn lekton::db::feedback_repository::FeedbackRepository>> =
        if config.features.rag {
            Some(Arc::new(
                lekton::db::feedback_repository::MongoFeedbackRepository::new(&mongo_db),
            ))
        } else {
            None
        };
    let documentation_feedback_repo: Arc<
        dyn lekton::db::documentation_feedback_repository::DocumentationFeedbackRepository,
    > = Arc::new(MongoDocumentationFeedbackRepository::new(&mongo_db));
    let document_source_repo: Arc<dyn lekton::db::source_repository::DocumentSourceRepository> =
        Arc::new(lekton::db::source_repository::MongoDocumentSourceRepository::new(&mongo_db));
    let embedding_cache_repo: Option<
        Arc<dyn lekton::db::embedding_cache_repository::EmbeddingCacheRepository>,
    > = if config.features.rag {
        Some(Arc::new(
            lekton::db::embedding_cache_repository::MongoEmbeddingCacheRepository::new(&mongo_db),
        ))
    } else {
        None
    };

    // Run database migrations before seeding or serving traffic.
    lekton::db::migrations::build_plan()
        .run(mongo_db.clone())
        .await
        .expect("Database migration failed — check __migrations collection and restart");

    // Seed default access levels (no-op if already present).
    if let Err(e) = access_level_repo.seed_defaults().await {
        tracing::warn!("Failed to seed default access levels: {e}");
    }

    tracing::info!("Connected to MongoDB at {}", mongo_uri);

    // Connect to S3
    let storage_client: Arc<dyn lekton::storage::client::StorageClient> = Arc::new(
        S3StorageClient::from_app_config(&config.storage)
            .await
            .expect("Failed to initialize S3 client"),
    );

    tracing::info!("S3 storage client initialized");

    // Initialize Meilisearch — only when the search feature is enabled.
    // validate_features() already guaranteed search.url is set in that case.
    let search_service: Option<Arc<dyn lekton::search::client::SearchService>> =
        if config.features.search {
            match MeilisearchService::from_app_config(&config.search) {
                Ok(service) => {
                    if let Err(e) = service.configure_index().await {
                        tracing::warn!("Failed to configure Meilisearch index: {e}");
                    }
                    tracing::info!("Meilisearch search service initialized");
                    Some(Arc::new(service))
                }
                Err(e) => {
                    tracing::warn!("Meilisearch not available: {e} — search will be disabled");
                    None
                }
            }
        } else {
            tracing::info!("Search not configured — feature disabled");
            None
        };
    // Keyword search over PDF attachment content: a second Meilisearch index,
    // same instance/credentials as `search_service`. Only useful together
    // with attachment extraction, but its own availability only depends on
    // the search feature — it's threaded into AttachmentExtractionService
    // below regardless of whether attachment_indexing ends up enabled.
    let attachment_search_service: Option<
        Arc<dyn lekton::search::attachment_search::AttachmentSearchService>,
    > = if config.features.search {
        match lekton::search::attachment_search::MeilisearchAttachmentService::from_app_config(
            &config.search,
        ) {
            Ok(service) => {
                if let Err(e) = service.configure_index().await {
                    tracing::warn!("Failed to configure Meilisearch attachment index: {e}");
                }
                Some(Arc::new(service))
            }
            Err(e) => {
                tracing::warn!(
                    "Meilisearch not available for attachment search: {e} — attachment keyword search disabled"
                );
                None
            }
        }
    } else {
        None
    };
    let search_reindex_state = if search_service.is_some() {
        Some(Arc::new(
            lekton::search::reindex::SearchReindexState::default(),
        ))
    } else {
        None
    };
    let schema_endpoint_reindex_state =
        Arc::new(lekton::schema::reindex::SchemaEndpointReindexState::default());

    // Service token for API authentication
    let service_token = match config.auth.service_token.as_deref() {
        Some(token) if !token.is_empty() => token.to_string(),
        _ if demo_mode => {
            tracing::warn!("auth.service_token not set — using insecure default (demo mode only)");
            "dev-token".to_string()
        }
        _ => {
            panic!("auth.service_token is required in production (set LKN__AUTH__SERVICE_TOKEN)");
        }
    };

    // JWT token service
    let token_service = Arc::new(match TokenService::from_app_config(&config.auth) {
        Ok(ts) => ts,
        Err(_) if demo_mode => {
            tracing::warn!("auth.jwt_secret not set — using insecure dev key (demo mode only)");
            TokenService::new("dev-insecure-secret-change-in-production!!", 900, 30)
        }
        Err(e) => {
            panic!("auth.jwt_secret is required in production: {e}");
        }
    });

    // OAuth2 / OIDC auth provider (optional — server starts without auth if not configured)
    let auth_provider = build_provider(&config.auth).await;

    // Initialize RAG services (optional — app works without them)
    //
    // The embedding + vectorstore arcs are also kept for the MCP server.
    #[allow(clippy::type_complexity)]
    let (rag_service, chat_service, embedding_service, vector_store): (
        Option<Arc<dyn lekton::rag::service::RagService>>,
        Option<Arc<lekton::rag::chat::ChatService>>,
        Option<Arc<dyn lekton::rag::embedding::EmbeddingService>>,
        Option<Arc<dyn lekton::rag::vectorstore::VectorStore>>,
    ) = if config.features.rag {
        use lekton::rag::cached_embedding::CachedEmbeddingService;
        use lekton::rag::embedding::build_embedding_service;
        use lekton::rag::vectorstore::QdrantVectorStore;

        match (
            build_embedding_service(&config.rag).await,
            QdrantVectorStore::from_rag_config(&config.rag),
        ) {
            (Ok(raw_embedding), Ok(vectorstore)) => {
                let vectorstore: Arc<dyn lekton::rag::vectorstore::VectorStore> =
                    Arc::new(vectorstore);

                // Wrap raw embedding with the cache for chunk indexing.
                let cached_embedding: Arc<dyn lekton::rag::embedding::EmbeddingService> =
                    if let Some(ref cache_repo) = embedding_cache_repo {
                        // Endpoint identity: distinct per backend so repointing the URL
                        // (or Vertex project) busts the cache instead of returning stale
                        // vectors from the old backend.
                        let endpoint = if config.rag.embedding_vertex_project_id.is_empty() {
                            format!("oai:{}", config.rag.embedding_url)
                        } else {
                            format!(
                                "vertex:{}/{}",
                                config.rag.embedding_vertex_project_id,
                                config.rag.embedding_vertex_location
                            )
                        };
                        let namespace = lekton::rag::cached_embedding::cache_namespace(
                            &config.rag.embedding_model,
                            config.rag.embedding_dimensions,
                            &endpoint,
                        );
                        Arc::new(CachedEmbeddingService::new(
                            raw_embedding.clone(),
                            cache_repo.clone(),
                            namespace,
                            config.rag.embedding_cache_store_text,
                        ))
                    } else {
                        raw_embedding.clone()
                    };

                // For chat queries, use the cached embedding only if the config flag is set.
                let query_embedding: Arc<dyn lekton::rag::embedding::EmbeddingService> =
                    if config.rag.embedding_cache_query {
                        cached_embedding.clone()
                    } else {
                        raw_embedding.clone()
                    };

                // Ensure collection exists
                if let Err(e) = vectorstore
                    .ensure_collection(config.rag.embedding_dimensions)
                    .await
                {
                    tracing::warn!("Failed to ensure Qdrant collection: {e} — RAG disabled");
                    (None, None, None, None)
                } else {
                    let rag_svc = Arc::new(lekton::rag::service::DefaultRagService::new(
                        cached_embedding.clone(),
                        vectorstore.clone(),
                        config.rag.chunk_size_tokens as usize,
                        config.rag.chunk_overlap_tokens as usize,
                    ));

                    let chat_svc = if let Some(ref chat_repo) = chat_repo {
                        let reranker: Option<Arc<dyn lekton::rag::reranker::Reranker>> =
                            lekton::rag::reranker::CrossEncoderReranker::from_rag_config(
                                &config.rag,
                            )
                            .map(|r| Arc::new(r) as Arc<dyn lekton::rag::reranker::Reranker>);

                        match lekton::rag::chat::ChatService::from_rag_config(
                            &config.rag,
                            chat_repo.clone(),
                            query_embedding,
                            vectorstore.clone(),
                            search_service.clone(),
                            reranker,
                            config.features.attachment_indexing,
                        )
                        .await
                        {
                            Ok(svc) => {
                                tracing::info!("RAG chat service initialized");
                                Some(Arc::new(svc))
                            }
                            Err(e) => {
                                tracing::warn!("RAG chat not available: {e}");
                                None
                            }
                        }
                    } else {
                        None
                    };

                    tracing::info!(
                        collection = %config.rag.qdrant_collection,
                        cache_query = config.rag.embedding_cache_query,
                        store_text = config.rag.embedding_cache_store_text,
                        "RAG service initialized"
                    );
                    (
                        Some(rag_svc as Arc<dyn lekton::rag::service::RagService>),
                        chat_svc,
                        Some(cached_embedding),
                        Some(vectorstore),
                    )
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!("RAG not available: {e} — RAG will be disabled");
                (None, None, None, None)
            }
        }
    } else {
        tracing::info!("RAG not configured — feature disabled");
        (None, None, None, None)
    };

    // Resolve runtime feature flags from config + actual service availability.
    // External-service features (rag, search) are on only when both the config
    // flag is set and the service initialised. validate_features() already
    // failed fast on enabled-but-misconfigured features, so this reflects what
    // the client UI can safely surface.
    let features = lekton::app::FeatureFlags {
        mcp: config.features.mcp,
        rag: config.features.rag && rag_service.is_some() && chat_service.is_some(),
        editor: config.features.editor,
        schema_registry: config.features.schema_registry,
        search: config.features.search && search_service.is_some(),
        prompt_library: config.features.prompt_library,
        documentation_feedback: config.features.documentation_feedback,
        attachment_indexing: config.features.attachment_indexing && rag_service.is_some(),
        document_upload: config.features.document_upload,
        sources: config.features.sources,
    };

    // Spawn the attachment extraction worker when attachment indexing is enabled.
    // Bounded queue; uploads enqueue keys and a single worker drains them.
    let (attachment_queue, attachment_service) = if let (true, Some(rag)) =
        (features.attachment_indexing, rag_service.clone())
    {
        // Optional VLM transcriber for image-heavy PDF pages, from [rag.vlm].
        let vlm = match &config.rag.vlm {
            Some(step) => {
                let resolved = config.rag.resolve_step(step);
                match lekton::rag::provider::LlmProvider::initialize(&resolved).await {
                    Ok(provider) => Some(Arc::new(lekton::rag::extraction::VlmTranscriber::new(
                        provider,
                        step.model.clone(),
                        step.max_tokens,
                        resolved.headers,
                    ))),
                    Err(e) => {
                        tracing::warn!(
                            "VLM not available: {e} — image-heavy PDF pages will use native text only"
                        );
                        None
                    }
                }
            }
            None => None,
        };

        let extractors = Arc::new(lekton::rag::extraction::AttachmentExtractors::new(
            config.rag.attachment_page_text_threshold,
            vlm,
        ));
        let svc = Arc::new(
            lekton::rag::attachment_extraction::AttachmentExtractionService::new(
                storage_client.clone(),
                asset_repo.clone(),
                document_repo.clone(),
                rag,
                attachment_search_service.clone(),
                extractors,
            ),
        );
        (Some(svc.clone().spawn(256)), Some(svc))
    } else {
        (None, None)
    };

    // Recover attachment extractions left unfinished by a previous run: the
    // queue is in-memory, so Pending/InProgress assets would otherwise be lost
    // across a restart. Sweep in the background so startup is not blocked.
    if let Some(queue) = attachment_queue.clone() {
        let asset_repo = asset_repo.clone();
        tokio::spawn(async move {
            lekton::rag::attachment_extraction::resume_unfinished_extractions(
                asset_repo.as_ref(),
                &queue,
            )
            .await;
        });
    }

    // Build application state
    let app_state = lekton::app::AppState {
        document_repo,
        schema_repo,
        settings_repo,
        asset_repo,
        storage_client,
        search_service,
        leptos_options: leptos_options.clone(),
        service_token,
        service_token_repo,
        document_version_repo,
        prompt_repo,
        prompt_version_repo,
        user_prompt_preference_repo,
        demo_mode,
        user_repo,
        access_level_repo,
        navigation_order_repo,
        token_service,
        auth_provider,
        reindex_state: if rag_service.is_some() {
            Some(Arc::new(lekton::rag::reindex::ReindexState::default()))
        } else {
            None
        },
        rag_service,
        attachment_queue,
        attachment_service,
        attachment_search_service,
        chat_repo,
        chat_service,
        search_reindex_state,
        schema_endpoint_reindex_state,
        feedback_repo,
        documentation_feedback_repo,
        document_source_repo,
        embedding_cache_repo,
        insecure_cookies: config.server.insecure_cookies,
        max_attachment_size_bytes: config.server.max_attachment_size_mb * 1024 * 1024,
        features,
    };

    // Generate the Leptos route list for SSR
    let routes = generate_route_list(App);

    // Build the Axum router. The full route surface lives in api_routes() so it
    // can be audited in one place; auth routes are mounted below.
    let mut app = api_routes(&features);

    // Mount auth routes: demo auth when demo_mode is enabled, OAuth2/OIDC otherwise.
    app = app.merge(auth_routes(demo_mode));
    if demo_mode {
        tracing::info!("Demo auth routes mounted: /api/auth/login, /api/auth/me, /api/auth/logout");
    } else {
        tracing::info!(
            "OAuth2/OIDC auth routes mounted: /auth/login, /auth/callback, /auth/refresh, /auth/logout, /auth/me"
        );
    }

    // MCP server — mounted whenever the MCP feature is enabled. The RAG
    // embedding/vectorstore are passed through optionally; the server hides the
    // search_documents tool (and any other) whose feature is off.
    if features.mcp {
        app = app.merge(mcp_routes(
            &app_state,
            &config,
            embedding_service.clone(),
            vector_store.clone(),
            features,
        ));

        tracing::info!(
            rag_tools = features.rag && embedding_service.is_some() && vector_store.is_some(),
            stateful_mode = config.mcp.stateful_mode,
            json_response = config.mcp.json_response,
            session_keep_alive_secs = ?config.mcp.session_keep_alive_secs,
            completed_cache_ttl_secs = config.mcp.completed_cache_ttl_secs,
            "MCP server mounted at POST /mcp (Streamable HTTP, PAT auth)"
        );
    } else {
        tracing::info!("MCP server disabled (features.mcp = false)");
    }

    // Rate limiting applies to explicit dynamic routes only; static fallback assets
    // are mounted after `route_layer` so page loads do not consume API quota.
    let burst_size = config.server.rate_limit_burst;
    let rate_limit_per_second = config.server.rate_limit_per_second;
    let key_extractor = lekton::rate_limit::TrustedProxyIpKeyExtractor::from_config(
        &config.server.rate_limit_trusted_proxies,
    )
    .expect("Invalid server.rate_limit_trusted_proxies");
    let governor_conf = Arc::new(
        tower_governor::governor::GovernorConfigBuilder::default()
            .per_second(rate_limit_per_second)
            .burst_size(burst_size)
            .key_extractor(key_extractor)
            .finish()
            .expect("Failed to build rate limiter configuration"),
    );
    let governor_limiter = governor_conf.limiter().clone();

    // Background task to clean up expired rate limit entries
    let interval = std::time::Duration::from_secs(60);
    tokio::task::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            governor_limiter.retain_recent();
        }
    });

    // CORS: same-origin by default; set cors_allowed_origins for cross-origin access.
    let cors = match config
        .server
        .cors_allowed_origins
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        Some(origins) => {
            let allowed: Vec<_> = origins
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            tower_http::cors::CorsLayer::new()
                .allow_origin(allowed)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::PUT,
                    axum::http::Method::DELETE,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
                .allow_credentials(true)
        }
        None => {
            // Default: no CORS headers (same-origin only)
            tower_http::cors::CorsLayer::new()
        }
    };

    let app = app
        // Leptos SSR routes
        .leptos_routes(&app_state, routes, {
            let options = app_state.leptos_options.clone();
            move || lekton::app::shell(options.clone(), features)
        })
        .route_layer(tower_governor::GovernorLayer::new(governor_conf))
        // Static files (including custom.css)
        .fallback_service(ServeDir::new(&site_root))
        .layer(middleware::from_fn(static_cache_headers))
        .layer(middleware::from_fn(mjs_content_type))
        .layer(cors)
        .with_state(app_state);

    // Start the server
    tracing::info!("Listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind TCP listener");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("Server exited with error");
}

// When compiled for WASM (client-side), there's no main function.
// The hydrate() function in lib.rs handles client-side initialization.
#[cfg(not(feature = "ssr"))]
fn main() {
    // This is intentionally empty.
    // Client-side hydration is handled by lib.rs::hydrate()
}
