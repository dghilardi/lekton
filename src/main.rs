#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use lekton::api;
    use lekton::app::App;
    use lekton::db::repository::MongoDocumentRepository;
    use lekton::db::schema_repository::MongoSchemaRepository;
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

    // Build application state
    let app_state = lekton::app::AppState {
        document_repo,
        schema_repo,
        storage_client,
        search_service,
        leptos_options: leptos_options.clone(),
        service_token,
        demo_mode,
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
        );

    // Mount demo auth routes when demo mode is enabled
    if demo_mode {
        use lekton::auth::demo_auth;

        app = app
            .route("/api/auth/login", axum::routing::post(demo_auth::login_handler))
            .route("/api/auth/me", axum::routing::get(demo_auth::me_handler))
            .route("/api/auth/logout", axum::routing::post(demo_auth::logout_handler));

        tracing::info!("Demo auth routes mounted: /api/auth/login, /api/auth/me, /api/auth/logout");
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
