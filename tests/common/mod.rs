use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::meilisearch::Meilisearch;
use testcontainers_modules::minio::MinIO;
use testcontainers_modules::mongo::Mongo;

use lekton::app::AppState;
use lekton::db::repository::{DocumentRepository, MongoDocumentRepository};
use lekton::db::schema_repository::{MongoSchemaRepository, SchemaRepository};
use lekton::db::settings_repository::{MongoSettingsRepository, SettingsRepository};
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
    pub storage: Arc<dyn StorageClient>,
    pub search: Arc<dyn SearchService>,
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

        let app_state = AppState {
            document_repo: repo.clone(),
            schema_repo: schema_repo.clone(),
            settings_repo: settings_repo.clone(),
            storage_client: storage.clone(),
            search_service: Some(search.clone()),
            service_token: "test-token".to_string(),
            demo_mode: true,
            leptos_options,
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
            storage,
            search,
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

    let app_state = AppState {
        document_repo: env.repo.clone(),
        schema_repo: env.schema_repo.clone(),
        settings_repo: env.settings_repo.clone(),
        storage_client: env.storage.clone(),
        search_service: None,
        service_token: "test-token".to_string(),
        demo_mode: true,
        leptos_options,
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
