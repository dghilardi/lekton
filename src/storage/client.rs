use async_trait::async_trait;

use crate::error::AppError;

/// Trait for blob storage operations (S3-compatible).
///
/// Abstracted as a trait so tests can use a mock without a real S3 instance.
#[async_trait]
pub trait StorageClient: Send + Sync {
    /// Upload content to the given key.
    async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), AppError>;

    /// Retrieve content by key. Returns `None` if the object doesn't exist.
    async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, AppError>;
}

/// S3 implementation of StorageClient.
///
/// Only available when the `ssr` feature is enabled.
#[cfg(feature = "ssr")]
pub struct S3StorageClient {
    client: aws_sdk_s3::Client,
    bucket: String,
}

#[cfg(feature = "ssr")]
impl S3StorageClient {
    /// Create a new S3 storage client.
    ///
    /// Configuration is read from environment variables:
    /// - `S3_BUCKET` — the bucket name
    /// - `S3_ENDPOINT` (optional) — custom endpoint for MinIO / LocalStack
    /// - `AWS_REGION` or `S3_REGION` — the AWS region
    pub async fn from_env() -> Result<Self, AppError> {
        let bucket = std::env::var("S3_BUCKET")
            .map_err(|_| AppError::Storage("S3_BUCKET not set".into()))?;

        let mut config_loader =
            aws_config::defaults(aws_config::BehaviorVersion::latest());

        // Support custom S3 endpoint (for MinIO, LocalStack, etc.)
        if let Ok(endpoint) = std::env::var("S3_ENDPOINT") {
            config_loader = config_loader.endpoint_url(&endpoint);
        }

        let sdk_config = config_loader.load().await;
        let client = aws_sdk_s3::Client::new(&sdk_config);

        Ok(Self { client, bucket })
    }

    /// Create with explicit values (useful for testing / DI).
    pub fn new(client: aws_sdk_s3::Client, bucket: String) -> Self {
        Self { client, bucket }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl StorageClient for S3StorageClient {
    async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), AppError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(content.into())
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("Failed to put object '{}': {}", key, e)))?;

        Ok(())
    }

    async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, AppError> {
        match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => {
                let bytes = output
                    .body
                    .collect()
                    .await
                    .map_err(|e| AppError::Storage(format!("Failed to read body: {}", e)))?;
                Ok(Some(bytes.into_bytes().to_vec()))
            }
            Err(e) => {
                let service_err = e.into_service_error();
                if service_err.is_no_such_key() {
                    Ok(None)
                } else {
                    Err(AppError::Storage(format!(
                        "Failed to get object '{}': {}",
                        key, service_err
                    )))
                }
            }
        }
    }
}
