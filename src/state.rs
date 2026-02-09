use std::sync::Arc;
use mongodb::Client as MongoClient;
use aws_sdk_s3::Client as S3Client;
use crate::models::document::Document;

#[derive(Clone)]
pub struct AppState {
    pub db: MongoClient,
    pub s3: S3Client,
    pub meili: Option<meilisearch_sdk::client::Client>,
    pub config: AppConfig,
    pub leptos_options: leptos::prelude::LeptosOptions,
    pub auth_client: Arc<crate::auth::AuthClient>,
}

impl axum::extract::FromRef<AppState> for leptos::prelude::LeptosOptions {
    fn from_ref(state: &AppState) -> Self {
        state.leptos_options.clone()
    }
}

#[derive(Clone)]
pub struct AppConfig {
    pub mongodb_uri: String,
    pub s3_bucket: String,
    pub s3_region: String,
    pub s3_endpoint: Option<String>,
    pub meili_url: String,
    pub meili_api_key: String,
}

impl AppState {
    pub async fn new(leptos_options: leptos::prelude::LeptosOptions) -> Self {
        let mongodb_uri = std::env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
        let s3_bucket = std::env::var("S3_BUCKET").unwrap_or_else(|_| "lekton-docs".to_string());
        let s3_region = std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let s3_endpoint = std::env::var("S3_ENDPOINT").ok();
        let meili_url = std::env::var("MEILI_URL").unwrap_or_else(|_| "http://localhost:7700".to_string());
        let meili_api_key = std::env::var("MEILI_API_KEY").unwrap_or_else(|_| "masterKey".to_string());

        let db_client = MongoClient::with_uri_str(&mongodb_uri).await.expect("Failed to connect to MongoDB");
        
        let mut s3_config_builder = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new(s3_region.clone()));
            
        if let Some(ref endpoint) = s3_endpoint {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint);
        }

        let s3_client = S3Client::new(&s3_config_builder.load().await);
        
        // Initialize Meilisearch
        let meili = Some(meilisearch_sdk::client::Client::new(&meili_url, Some(&meili_api_key)).expect("Failed to connect to Meilisearch"));

        // Initialize OIDC Client (requires env vars)
        let auth_client = Arc::new(crate::auth::AuthClient::new().await);

        Self {
            db: db_client,
            s3: s3_client,
            meili,
            config: AppConfig {
                mongodb_uri,
                s3_bucket,
                s3_region,
                s3_endpoint,
                meili_url,
                meili_api_key,
            },
            leptos_options,
            auth_client,
        }
    }

    pub fn documents_collection(&self) -> mongodb::Collection<Document> {
        self.db.database("lekton").collection::<Document>("documents")
    }
}
