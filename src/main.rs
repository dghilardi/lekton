#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use lekton::api;
    use lekton::app::App;
    use lekton::auth::provider::build_provider_from_env;
    use lekton::auth::token_service::TokenService;
    use lekton::db::access_level_repository::MongoAccessLevelRepository;
    use lekton::db::asset_repository::MongoAssetRepository;
    use lekton::db::repository::MongoDocumentRepository;
    use lekton::db::schema_repository::MongoSchemaRepository;
    use lekton::db::settings_repository::MongoSettingsRepository;
    use lekton::db::user_repository::MongoUserRepository;
    use lekton::search::client::SearchService;
    use lekton::storage::client::S3StorageClient;
    use std::sync::Arc;
    use tower_http::services::ServeDir;

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lekton=info,tower_http=info".into()),
        )
        .init();

    tracing::info!("Starting Lekton server...");

    // Check demo mode
    let demo_mode = std::env::var("DEMO_MODE")
        .map(|v| v == "true" || v == "1" || v == "yes")
        .unwrap_or(false);

    if demo_mode {
        tracing::warn!("⚠️  DEMO MODE ENABLED — built-in credentials are active. Do NOT use in production!");
    }

    // Load Leptos options from Cargo.toml metadata
    let conf = get_configuration(None).unwrap();
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let site_root = leptos_options.site_root.to_string();

    // Connect to MongoDB
    let mongo_uri =
        std::env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
    let mongo_db_name =
        std::env::var("MONGODB_DATABASE").unwrap_or_else(|_| "lekton".to_string());

    let mongo_client = mongodb::Client::with_uri_str(&mongo_uri)
        .await
        .expect("Failed to connect to MongoDB");
    let mongo_db = mongo_client.database(&mongo_db_name);
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

    // Seed default access levels (no-op if already present).
    if let Err(e) = access_level_repo.seed_defaults().await {
        tracing::warn!("Failed to seed default access levels: {e}");
    }

    tracing::info!("Connected to MongoDB at {}", mongo_uri);

    // Connect to S3
    let storage_client: Arc<dyn lekton::storage::client::StorageClient> = Arc::new(
        S3StorageClient::from_env()
            .await
            .expect("Failed to initialize S3 client"),
    );

    tracing::info!("S3 storage client initialized");

    // Initialize Meilisearch (optional — app works without it)
    let search_service: Option<Arc<dyn lekton::search::client::SearchService>> =
        match lekton::search::client::MeilisearchService::from_env() {
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
    let service_token =
        std::env::var("SERVICE_TOKEN").unwrap_or_else(|_| "dev-token".to_string());

    // JWT token service
    let token_service = Arc::new(
        TokenService::from_env()
            .unwrap_or_else(|_| {
                tracing::warn!("JWT_SECRET not set — using insecure dev key. Set JWT_SECRET in production!");
                TokenService::new("dev-insecure-secret-change-in-production!!", 900, 30)
            })
    );

    // OAuth2 / OIDC auth provider (optional — server starts without auth if not configured)
    let auth_provider = build_provider_from_env().await;

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
        demo_mode,
        user_repo,
        access_level_repo,
        token_service,
        auth_provider,
    };

    // Generate the Leptos route list for SSR
    let routes = generate_route_list(App);

    // Build the Axum router
    let mut app = Router::new()
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
            "/api/v1/upload-image",
            axum::routing::post(api::upload::upload_image_handler),
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
            "/api/v1/editor/upload-asset",
            axum::routing::post(api::assets::editor_upload_asset_handler),
        )
        .route(
            "/api/v1/assets",
            axum::routing::get(api::assets::list_assets_handler),
        )
        .route(
            "/api/v1/assets/{*key}",
            axum::routing::put(api::assets::upload_asset_handler)
                .get(api::assets::serve_asset_handler)
                .delete(api::assets::delete_asset_handler),
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
        .with_state(app_state);

    // Start the server
    tracing::info!("Listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

// When compiled for WASM (client-side), there's no main function.
// The hydrate() function in lib.rs handles client-side initialization.
#[cfg(not(feature = "ssr"))]
fn main() {
    // This is intentionally empty.
    // Client-side hydration is handled by lib.rs::hydrate()
}
