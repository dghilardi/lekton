#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use lekton::api;
    use lekton::app::App;
    use lekton::auth::provider::build_provider;
    use lekton::auth::token_service::TokenService;
    use std::net::SocketAddr;
    use lekton::db::access_level_repository::MongoAccessLevelRepository;
    use lekton::db::navigation_order_repository::MongoNavigationOrderRepository;
    use lekton::db::asset_repository::MongoAssetRepository;
    use lekton::db::repository::MongoDocumentRepository;
    use lekton::db::schema_repository::MongoSchemaRepository;
    use lekton::db::settings_repository::MongoSettingsRepository;
    use lekton::db::document_version_repository::MongoDocumentVersionRepository;
    use lekton::db::service_token_repository::MongoServiceTokenRepository;
    use lekton::db::user_repository::MongoUserRepository;
    use lekton::search::client::{MeilisearchService, SearchService as _};
    use lekton::storage::client::S3StorageClient;
    use std::sync::Arc;
    use tower_http::services::ServeDir;

    // Load configuration first — fast-fail on bad config before anything else starts.
    let config = lekton::config::AppConfig::load()
        .expect("Failed to load application configuration");

    // Debug config loading
    println!("[DEBUG] LKN__AUTH__DEMO_MODE env: {:?}", std::env::var("LKN__AUTH__DEMO_MODE"));
    println!("[DEBUG] Loaded config auth.demo_mode: {}", config.auth.demo_mode);
    use std::io::Write;
    std::io::stdout().flush().ok();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.server.log_filter.as_str().into()),
        )
        .init();

    tracing::info!("Starting Lekton server...");

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

        tracing::warn!("⚠️  DEMO MODE ENABLED — built-in credentials are active. Do NOT use in production!");
    }

    // Load Leptos options from Cargo.toml metadata
    let conf = get_configuration(None).expect("Failed to load Leptos configuration from Cargo.toml");
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let site_root = leptos_options.site_root.to_string();

    // Connect to MongoDB
    let mongo_uri = config.database.uri.clone();

    // Inject credentials into the URI if provided separately
    let mongo_uri = match (&config.database.username, &config.database.password) {
        (Some(user), Some(pass)) if !user.is_empty() => {
            let encoded_user = urlencoding::encode(user);
            let encoded_pass = urlencoding::encode(pass);
            // Insert credentials after the scheme (mongodb:// or mongodb+srv://)
            if let Some(rest) = mongo_uri
                .strip_prefix("mongodb+srv://")
                .map(|r| ("mongodb+srv://", r))
                .or_else(|| mongo_uri.strip_prefix("mongodb://").map(|r| ("mongodb://", r)))
            {
                // Strip any existing credentials (user:pass@host → host)
                let host_part = rest.1.find('@').map_or(rest.1, |i| &rest.1[i + 1..]);
                format!("{}{}:{}@{}", rest.0, encoded_user, encoded_pass, host_part)
            } else {
                mongo_uri
            }
        }
        _ => mongo_uri,
    };

    let mongo_client = mongodb::Client::with_uri_str(&mongo_uri)
        .await
        .expect("Failed to connect to MongoDB");
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
    let document_version_repo: Arc<dyn lekton::db::document_version_repository::DocumentVersionRepository> =
        Arc::new(MongoDocumentVersionRepository::new(&mongo_db));
    let navigation_order_repo: Arc<dyn lekton::db::navigation_order_repository::NavigationOrderRepository> =
        Arc::new(MongoNavigationOrderRepository::new(&mongo_db));
    let chat_repo: Option<Arc<dyn lekton::db::chat_repository::ChatRepository>> =
        if config.rag.is_enabled() {
            Some(Arc::new(
                lekton::db::chat_repository::MongoChatRepository::new(&mongo_db),
            ))
        } else {
            None
        };

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

    // Initialize Meilisearch (optional — app works without it)
    let search_service: Option<Arc<dyn lekton::search::client::SearchService>> =
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
        };

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
    let (rag_service, chat_service): (
        Option<Arc<dyn lekton::rag::service::RagService>>,
        Option<Arc<lekton::rag::chat::ChatService>>,
    ) = if config.rag.is_enabled() {
        use lekton::rag::embedding::OpenAICompatibleEmbedding;
        use lekton::rag::vectorstore::QdrantVectorStore;

        match (
            OpenAICompatibleEmbedding::from_rag_config(&config.rag),
            QdrantVectorStore::from_rag_config(&config.rag),
        ) {
            (Ok(embedding), Ok(vectorstore)) => {
                let embedding: Arc<dyn lekton::rag::embedding::EmbeddingService> =
                    Arc::new(embedding);
                let vectorstore: Arc<dyn lekton::rag::vectorstore::VectorStore> =
                    Arc::new(vectorstore);

                // Ensure collection exists
                if let Err(e) = vectorstore
                    .ensure_collection(config.rag.embedding_dimensions)
                    .await
                {
                    tracing::warn!("Failed to ensure Qdrant collection: {e} — RAG disabled");
                    (None, None)
                } else {
                    let rag_svc = Arc::new(
                        lekton::rag::service::DefaultRagService::new(
                            embedding.clone(),
                            vectorstore.clone(),
                        ),
                    );

                    let chat_svc = if let Some(ref chat_repo) = chat_repo {
                        match lekton::rag::chat::ChatService::from_rag_config(
                            &config.rag,
                            chat_repo.clone(),
                            embedding,
                            vectorstore,
                        ) {
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
                        "RAG service initialized"
                    );
                    (Some(rag_svc as Arc<dyn lekton::rag::service::RagService>), chat_svc)
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!("RAG not available: {e} — RAG will be disabled");
                (None, None)
            }
        }
    } else {
        tracing::info!("RAG not configured — feature disabled");
        (None, None)
    };

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
        chat_repo,
        chat_service,
        insecure_cookies: config.server.insecure_cookies,
        max_attachment_size_bytes: config.server.max_attachment_size_mb * 1024 * 1024,
    };

    // Generate the Leptos route list for SSR
    let routes = generate_route_list(App);

    // Build the Axum router
    //
    // Upload endpoints get a 50 MB body limit; all other routes use the
    // default 2 MB limit provided by Axum.
    let upload_routes = Router::new()
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

    let mut app = Router::new()
        .merge(upload_routes)
        // API routes
        .route(
            "/api/v1/ingest",
            axum::routing::post(api::ingest::ingest_handler),
        )
        .route(
            "/api/v1/search",
            axum::routing::get(api::search::search_handler),
        )
        .route(
            "/api/v1/image/{filename}",
            axum::routing::get(api::upload::serve_image_handler),
        )
        .route(
            "/api/v1/schemas",
            axum::routing::get(api::schemas::list_schemas_handler)
                .post(api::schemas::ingest_schema_handler),
        )
        .route(
            "/api/v1/schemas/{name}",
            axum::routing::get(api::schemas::get_schema_handler),
        )
        .route(
            "/api/v1/schemas/{name}/{version}",
            axum::routing::get(api::schemas::get_schema_version_handler),
        )
        .route(
            "/api/v1/sync",
            axum::routing::post(api::sync::sync_handler),
        )
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
            "/api/v1/admin/users/{user_id}/permissions",
            axum::routing::get(api::admin::get_user_permissions_handler)
                .put(api::admin::set_user_permissions_handler),
        )
        .route(
            "/api/v1/admin/users/{user_id}/permissions/{level}",
            axum::routing::delete(api::admin::delete_user_permission_handler),
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
        .route(
            "/api/v1/admin/rag/reindex",
            axum::routing::post(api::rag::trigger_reindex_handler),
        )
        .route(
            "/api/v1/admin/rag/reindex/status",
            axum::routing::get(api::rag::reindex_status_handler),
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
        );

    // Mount demo auth routes when demo mode is enabled, OAuth2/OIDC routes otherwise
    if demo_mode {
        use lekton::auth::demo_auth;

        app = app
            .route("/api/auth/login", axum::routing::post(demo_auth::login_handler))
            .route("/api/auth/me", axum::routing::get(demo_auth::me_handler))
            .route("/api/auth/logout", axum::routing::post(demo_auth::logout_handler));

        tracing::info!("Demo auth routes mounted: /api/auth/login, /api/auth/me, /api/auth/logout");
    } else {
        use lekton::api::auth as auth_api;

        app = app
            .route("/auth/login", axum::routing::get(auth_api::login_handler))
            .route("/auth/callback", axum::routing::get(auth_api::callback_handler))
            .route("/auth/refresh", axum::routing::post(auth_api::refresh_handler))
            .route("/auth/logout", axum::routing::post(auth_api::logout_handler))
            .route("/auth/me", axum::routing::get(auth_api::me_handler));

        tracing::info!("OAuth2/OIDC auth routes mounted: /auth/login, /auth/callback, /auth/refresh, /auth/logout, /auth/me");
    }

    // Rate limiting: replenished at 1 per second, burst from config
    let burst_size = config.server.rate_limit_burst;
    let governor_conf = Arc::new(
        tower_governor::governor::GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(burst_size)
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
    let cors = match config.server.cors_allowed_origins.as_deref().filter(|s| !s.is_empty()) {
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
                .allow_headers([axum::http::header::CONTENT_TYPE, axum::http::header::AUTHORIZATION])
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
            move || {
                lekton::app::shell(options.clone())
            }
        })
        // Static files (including custom.css)
        .fallback_service(ServeDir::new(&site_root))
        .layer(cors)
        .layer(tower_governor::GovernorLayer::new(governor_conf))
        .with_state(app_state);

    // Start the server
    tracing::info!("Listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind TCP listener");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
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
