#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use lekton::app::*;
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    use dotenvy;

    dotenvy::dotenv().ok();
    
    let cwd = std::env::current_dir().unwrap_or_default();
    println!("--- DEBUG: CWD = {:?} ---", cwd);
    
    let mock_env = std::env::var("MOCK_AUTH").unwrap_or_else(|_| "NOT SET".to_string());
    println!("--- DEBUG: MOCK_AUTH = {} ---", mock_env);

    let oidc_id = std::env::var("OIDC_CLIENT_ID").unwrap_or_else(|_| "NOT SET".to_string());
    println!("--- DEBUG: OIDC_CLIENT_ID = {} ---", oidc_id);

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let conf = if std::env::var("LEPTOS_OUTPUT_NAME").is_ok() {
        println!("--- DEBUG: Using get_configuration(None) ---");
        get_configuration(None).unwrap()
    } else {
        println!("--- DEBUG: Using get_configuration(Some(Cargo.toml)) ---");
        get_configuration(Some("Cargo.toml")).unwrap()
    };
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    
    // Initialize AppState
    let state = lekton::state::AppState::new(leptos_options.clone()).await;
    
    // Generate the list of routes in your Leptos App
    let routes = generate_route_list(App);

    let session_store = tower_sessions::MemoryStore::default();
    let session_layer = tower_sessions::SessionManagerLayer::new(session_store)
        .with_secure(false) // For local dev
        .with_expiry(tower_sessions::Expiry::OnInactivity(tower_sessions::cookie::time::Duration::hours(1)));

    let app = Router::new()
        .route("/api/v1/ingest", axum::routing::post(lekton::api::ingest::ingest_handler))
        .route("/api/v1/search", axum::routing::get(lekton::api::search::search_handler))
        .route("/auth/login", axum::routing::get(lekton::auth::login_handler))
        .route("/auth/callback", axum::routing::get(lekton::auth::callback_handler))
        .leptos_routes(&state, routes, {
            let leptos_options = leptos_options.clone();
            let state = state.clone();
            move || {
                provide_context(state.clone());
                shell(leptos_options.clone())
            }
        })
        .fallback(leptos_axum::file_and_error_handler::<lekton::state::AppState, _>(shell))
        .layer(session_layer)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("listening on http://{}", &addr);
    axum::serve(listener, app).await.unwrap();
}

#[cfg(feature = "ssr")]
fn shell(options: leptos::prelude::LeptosOptions) -> impl leptos::prelude::IntoView {
    use leptos::prelude::*;
    use lekton::app::App;

    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options=options />
                <link rel="shortcut icon" type="image/ico" href="/favicon.ico"/>
                <leptos_meta::MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want to use this for hydrate, but normally it's in a different module or handled by wasm-bindgen
}
