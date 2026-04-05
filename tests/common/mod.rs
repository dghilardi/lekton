use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::meilisearch::Meilisearch;
use testcontainers_modules::minio::MinIO;
use testcontainers_modules::mongo::Mongo;

use lekton::app::AppState;
use lekton::auth::models::AuthenticatedUser;
use lekton::auth::token_service::TokenService;
use lekton::db::access_level_repository::{AccessLevelRepository, MongoAccessLevelRepository};
use lekton::db::asset_repository::{AssetRepository, MongoAssetRepository};
use lekton::db::auth_models::User;
use lekton::db::document_version_repository::{DocumentVersionRepository, MongoDocumentVersionRepository};
use lekton::db::repository::{DocumentRepository, MongoDocumentRepository};
use lekton::db::schema_repository::{MongoSchemaRepository, SchemaRepository};
use lekton::db::service_token_repository::{MongoServiceTokenRepository, ServiceTokenRepository};
use lekton::db::navigation_order_repository::{MongoNavigationOrderRepository, NavigationOrderRepository};
use lekton::db::settings_repository::{MongoSettingsRepository, SettingsRepository};
use lekton::db::user_repository::{MongoUserRepository, UserRepository};
use lekton::search::client::{MeilisearchService, SearchService};
use lekton::storage::client::{S3StorageClient, StorageClient};

/// Holds running containers and provides the Axum router for integration tests.
///
/// Containers are kept alive for as long as this struct lives. When dropped,
/// containers are stopped and cleaned up automatically.
pub struct TestEnv {
    _mongo: ContainerAsync<Mongo>,
    _minio: ContainerAsync<MinIO>,
    _meili: ContainerAsync<Meilisearch>,
    pub router: Router,
    pub repo: Arc<dyn DocumentRepository>,
    pub schema_repo: Arc<dyn SchemaRepository>,
    pub settings_repo: Arc<dyn SettingsRepository>,
    pub asset_repo: Arc<dyn AssetRepository>,
    pub user_repo: Arc<dyn UserRepository>,
    pub access_level_repo: Arc<dyn AccessLevelRepository>,
    pub service_token_repo: Arc<dyn ServiceTokenRepository>,
    pub document_version_repo: Arc<dyn DocumentVersionRepository>,
    pub navigation_order_repo: Arc<dyn NavigationOrderRepository>,
    pub storage: Arc<dyn StorageClient>,
    pub search: Arc<dyn SearchService>,
    pub token_service: Arc<TokenService>,
}

impl TestEnv {
    /// Spin up all containers and build an Axum router wired to real services.
    pub async fn start() -> Self {
        // Start containers concurrently
        let mongo_fut = Mongo::default().start();
        let minio_fut = MinIO::default().start();
        let meili_fut = Meilisearch::default().start();
        let (mongo_container, minio_container, meili_container) =
            tokio::join!(mongo_fut, minio_fut, meili_fut);
        let mongo_container = mongo_container.expect("Failed to start MongoDB container");
        let minio_container = minio_container.expect("Failed to start MinIO container");
        let meili_container = meili_container.expect("Failed to start Meilisearch container");

        // --- MongoDB ---
        let mongo_port = mongo_container
            .get_host_port_ipv4(27017)
            .await
            .expect("Failed to get MongoDB port");
        let mongo_uri = format!("mongodb://127.0.0.1:{}", mongo_port);
        let mongo_client = mongodb::Client::with_uri_str(&mongo_uri)
            .await
            .expect("Failed to connect to MongoDB");
        let mongo_db = mongo_client.database("lekton_test");
        let repo: Arc<dyn DocumentRepository> =
            Arc::new(MongoDocumentRepository::new(&mongo_db));
        let schema_repo: Arc<dyn SchemaRepository> =
            Arc::new(MongoSchemaRepository::new(&mongo_db));
        let settings_repo: Arc<dyn SettingsRepository> =
            Arc::new(MongoSettingsRepository::new(&mongo_db));
        let asset_repo: Arc<dyn AssetRepository> =
            Arc::new(MongoAssetRepository::new(&mongo_db));
        let user_repo: Arc<dyn UserRepository> =
            Arc::new(MongoUserRepository::new(&mongo_db));
        let access_level_repo: Arc<dyn AccessLevelRepository> =
            Arc::new(MongoAccessLevelRepository::new(&mongo_db));
        let service_token_repo: Arc<dyn ServiceTokenRepository> =
            Arc::new(MongoServiceTokenRepository::new(&mongo_db));
        let document_version_repo: Arc<dyn DocumentVersionRepository> =
            Arc::new(MongoDocumentVersionRepository::new(&mongo_db));
        let navigation_order_repo: Arc<dyn NavigationOrderRepository> =
            Arc::new(MongoNavigationOrderRepository::new(&mongo_db));
        access_level_repo
            .seed_defaults()
            .await
            .expect("Failed to seed access level defaults");

        // --- MinIO (S3) ---
        let minio_port = minio_container
            .get_host_port_ipv4(9000)
            .await
            .expect("Failed to get MinIO port");
        let minio_endpoint = format!("http://127.0.0.1:{}", minio_port);

        // Set env vars for AWS SDK to pick up MinIO credentials
        unsafe {
            std::env::set_var("AWS_ACCESS_KEY_ID", "minioadmin");
            std::env::set_var("AWS_SECRET_ACCESS_KEY", "minioadmin");
            std::env::set_var("AWS_REGION", "us-east-1");
        }

        let s3_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .endpoint_url(&minio_endpoint)
            .region(aws_config::Region::new("us-east-1"))
            .load()
            .await;

        let s3_client = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::config::Builder::from(&s3_config)
                .force_path_style(true)
                .build(),
        );

        // Create test bucket
        let bucket_name = "lekton-test";
        let _ = s3_client
            .create_bucket()
            .bucket(bucket_name)
            .send()
            .await;

        let storage: Arc<dyn StorageClient> =
            Arc::new(S3StorageClient::new(s3_client, bucket_name.to_string()));

        // --- Meilisearch ---
        let meili_port = meili_container
            .get_host_port_ipv4(7700)
            .await
            .expect("Failed to get Meilisearch port");
        let meili_url = format!("http://127.0.0.1:{}", meili_port);

        let meili_service = MeilisearchService::new(&meili_url, None::<String>)
            .expect("Failed to create MeilisearchService");
        meili_service
            .configure_index()
            .await
            .expect("Failed to configure Meilisearch index");
        let search: Arc<dyn SearchService> = Arc::new(meili_service);

        // --- Build AppState ---
        let leptos_options = leptos::prelude::LeptosOptions::builder()
            .output_name("lekton")
            .build();

        let token_service = Arc::new(TokenService::new(
            "test-secret-key-at-least-32-bytes!!",
            3600,
            30,
        ));

        let app_state = AppState {
            document_repo: repo.clone(),
            schema_repo: schema_repo.clone(),
            settings_repo: settings_repo.clone(),
            asset_repo: asset_repo.clone(),
            storage_client: storage.clone(),
            search_service: Some(search.clone()),
            service_token: "test-token".to_string(),
            service_token_repo: service_token_repo.clone(),
            document_version_repo: document_version_repo.clone(),
            demo_mode: true,
            leptos_options,
            user_repo: user_repo.clone(),
            access_level_repo: access_level_repo.clone(),
            navigation_order_repo: navigation_order_repo.clone(),
            token_service: token_service.clone(),
            auth_provider: None,
            insecure_cookies: true,
            max_attachment_size_bytes: 5242880,
        };

        // --- Build Router (API routes only, no Leptos SSR) ---
        let router = Router::new()
            .route(
                "/api/v1/ingest",
                post(lekton::api::ingest::ingest_handler),
            )
            .route(
                "/api/v1/search",
                get(lekton::api::search::search_handler),
            )
            .route(
                "/api/v1/upload-image",
                post(lekton::api::upload::upload_image_handler),
            )
            .route(
                "/api/v1/image/{filename}",
                get(lekton::api::upload::serve_image_handler),
            )
            .route(
                "/api/v1/schemas",
                get(lekton::api::schemas::list_schemas_handler)
                    .post(lekton::api::schemas::ingest_schema_handler),
            )
            .route(
                "/api/v1/schemas/{name}",
                get(lekton::api::schemas::get_schema_handler),
            )
            .route(
                "/api/v1/schemas/{name}/{version}",
                get(lekton::api::schemas::get_schema_version_handler),
            )
            .route(
                "/api/v1/editor/upload-asset",
                post(lekton::api::assets::editor_upload_asset_handler),
            )
            .route(
                "/api/v1/sync",
                post(lekton::api::sync::sync_handler),
            )
            .route(
                "/api/v1/assets",
                get(lekton::api::assets::list_assets_handler),
            )
            .route(
                "/api/v1/assets/{*key}",
                axum::routing::put(lekton::api::assets::upload_asset_handler)
                    .get(lekton::api::assets::serve_asset_handler)
                    .delete(lekton::api::assets::delete_asset_handler),
            )
            // Admin API
            .route(
                "/api/v1/admin/access-levels",
                get(lekton::api::admin::list_access_levels_handler)
                    .post(lekton::api::admin::create_access_level_handler),
            )
            .route(
                "/api/v1/admin/access-levels/{name}",
                axum::routing::put(lekton::api::admin::update_access_level_handler)
                    .delete(lekton::api::admin::delete_access_level_handler),
            )
            .route(
                "/api/v1/admin/users",
                get(lekton::api::admin::list_users_handler),
            )
            .route(
                "/api/v1/admin/users/{user_id}/permissions",
                get(lekton::api::admin::get_user_permissions_handler)
                    .put(lekton::api::admin::set_user_permissions_handler),
            )
            .route(
                "/api/v1/admin/users/{user_id}/permissions/{level}",
                axum::routing::delete(lekton::api::admin::delete_user_permission_handler),
            )
            .route(
                "/api/v1/admin/service-tokens",
                get(lekton::api::admin::list_service_tokens_handler)
                    .post(lekton::api::admin::create_service_token_handler),
            )
            .route(
                "/api/v1/admin/service-tokens/{id}",
                axum::routing::delete(lekton::api::admin::deactivate_service_token_handler),
            )
            // Auth OIDC routes (refresh, me, logout — work without auth_provider)
            .route(
                "/auth/refresh",
                post(lekton::api::auth::refresh_handler),
            )
            .route(
                "/auth/logout",
                post(lekton::api::auth::logout_handler),
            )
            .route(
                "/auth/me",
                get(lekton::api::auth::me_handler),
            )
            // Demo auth routes
            .route(
                "/api/auth/login",
                post(lekton::auth::demo_auth::login_handler),
            )
            .route(
                "/api/auth/me",
                get(lekton::auth::demo_auth::me_handler),
            )
            .route(
                "/api/auth/logout",
                post(lekton::auth::demo_auth::logout_handler),
            )
            .with_state(app_state);

        Self {
            _mongo: mongo_container,
            _minio: minio_container,
            _meili: meili_container,
            router,
            repo,
            schema_repo,
            settings_repo,
            asset_repo,
            user_repo,
            access_level_repo,
            service_token_repo,
            document_version_repo,
            navigation_order_repo,
            storage,
            search,
            token_service,
        }
    }

    /// Build an `axum_test::TestServer` from this environment's router.
    pub fn server(&self) -> axum_test::TestServer {
        axum_test::TestServer::builder()
            .save_cookies()
            .expect_success_by_default()
            .build(self.router.clone())
            .expect("Failed to build TestServer")
    }

    /// Build a `TestServer` that does NOT expect success by default (for error tests).
    pub fn server_permissive(&self) -> axum_test::TestServer {
        axum_test::TestServer::builder()
            .save_cookies()
            .build(self.router.clone())
            .expect("Failed to build TestServer")
    }

    /// Create a test user in the database and return the AuthenticatedUser identity.
    pub async fn create_test_user(
        &self,
        user_id: &str,
        email: &str,
        is_admin: bool,
    ) -> AuthenticatedUser {
        let user = User {
            id: user_id.to_string(),
            email: email.to_string(),
            name: Some(format!("Test User {}", user_id)),
            provider_sub: format!("sub-{}", user_id),
            provider_type: "oidc".to_string(),
            is_admin,
            created_at: chrono::Utc::now(),
            last_login_at: None,
        };
        self.user_repo
            .create_user(user)
            .await
            .expect("Failed to create test user");

        AuthenticatedUser {
            user_id: user_id.to_string(),
            email: email.to_string(),
            name: Some(format!("Test User {}", user_id)),
            is_admin,
        }
    }

    /// Generate a JWT access token cookie value for an authenticated user.
    ///
    /// Use with `server.add_cookie(...)` or by adding the cookie header directly.
    pub fn access_token_for(&self, user: &AuthenticatedUser) -> String {
        self.token_service
            .generate_access_token(user)
            .expect("Failed to generate access token")
    }

    /// Build a `cookie::Cookie` with the access token for an authenticated user.
    ///
    /// Add it to a request with `.add_cookie(env.auth_cookie(&user))`.
    pub fn auth_cookie(&self, user: &AuthenticatedUser) -> cookie::Cookie<'static> {
        let token = self.access_token_for(user);
        cookie::Cookie::build(("lekton_access_token", token))
            .path("/")
            .build()
    }

    /// Create a scoped service token and return the raw token string.
    pub async fn create_service_token(
        &self,
        name: &str,
        scopes: Vec<String>,
        can_write: bool,
    ) -> String {
        let raw_token = uuid::Uuid::new_v4().to_string();
        let token = lekton::db::service_token_models::ServiceToken {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            token_hash: TokenService::hash_token(&raw_token),
            allowed_scopes: scopes,
            can_write,
            created_by: "test-admin".to_string(),
            created_at: chrono::Utc::now(),
            last_used_at: None,
            is_active: true,
        };
        self.service_token_repo
            .create(token)
            .await
            .expect("Failed to create service token");
        raw_token
    }

    /// Helper: ingest a document via the API.
    pub async fn ingest(
        &self,
        server: &axum_test::TestServer,
        slug: &str,
        title: &str,
        content: &str,
        access_level: &str,
    ) -> axum_test::TestResponse {
        server
            .post("/api/v1/ingest")
            .json(&serde_json::json!({
                "service_token": "test-token",
                "slug": slug,
                "title": title,
                "content": content,
                "access_level": access_level,
                "service_owner": "test-team",
                "tags": ["test"],
                "order": 0,
                "is_hidden": false
            }))
            .await
    }

    /// Helper: wait for Meilisearch to process pending tasks (async indexing).
    pub async fn wait_for_search_indexing(&self) {
        // Meilisearch processes tasks asynchronously. A short delay is the
        // simplest reliable approach for integration tests.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Build a `TestServer` with search disabled (for testing graceful degradation).
pub fn server_without_search(env: &TestEnv) -> axum_test::TestServer {
    let leptos_options = leptos::prelude::LeptosOptions::builder()
        .output_name("lekton")
        .build();

    let token_service = Arc::new(TokenService::new(
        "test-secret-key-at-least-32-bytes!!",
        3600,
        30,
    ));

    let app_state = AppState {
        document_repo: env.repo.clone(),
        schema_repo: env.schema_repo.clone(),
        settings_repo: env.settings_repo.clone(),
        asset_repo: env.asset_repo.clone(),
        storage_client: env.storage.clone(),
        search_service: None,
        service_token: "test-token".to_string(),
        service_token_repo: env.service_token_repo.clone(),
        document_version_repo: env.document_version_repo.clone(),
        demo_mode: true,
        leptos_options,
        user_repo: env.user_repo.clone(),
        access_level_repo: env.access_level_repo.clone(),
        navigation_order_repo: env.navigation_order_repo.clone(),
        token_service,
        auth_provider: None,
        insecure_cookies: true,
        max_attachment_size_bytes: 5242880,
    };

    let router = Router::new()
        .route(
            "/api/v1/search",
            get(lekton::api::search::search_handler),
        )
        .with_state(app_state);

    axum_test::TestServer::builder()
        .build(router)
        .expect("Failed to build TestServer")
}
