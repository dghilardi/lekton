#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use lekton::api;
    use lekton::app::App;
    use lekton::db::repository::MongoDocumentRepository;
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

    tracing::info!("Connected to MongoDB at {}", mongo_uri);

    // Connect to S3
    let storage_client: Arc<dyn lekton::storage::client::StorageClient> = Arc::new(
        S3StorageClient::from_env()
            .await
            .expect("Failed to initialize S3 client"),
    );

    tracing::info!("S3 storage client initialized");

    // Service token for API authentication
    let service_token =
        std::env::var("SERVICE_TOKEN").unwrap_or_else(|_| "dev-token".to_string());

    // Build application state
    let app_state = lekton::app::AppState {
        document_repo,
        storage_client,
        leptos_options: leptos_options.clone(),
        service_token,
    };

    // Generate the Leptos route list for SSR
    let routes = generate_route_list(App);

    // Build the Axum router
    let app = Router::new()
        // API routes
        .route(
            "/api/v1/ingest",
            axum::routing::post(api::ingest::ingest_handler),
        )
        // Leptos SSR routes
        .leptos_routes(&app_state, routes, {
            move || {
                lekton::app::App()
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
