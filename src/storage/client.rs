use async_trait::async_trait;

use crate::error::AppError;

/// One page of a [`StorageClient::list`] listing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListPage {
    /// Object keys in lexicographic order.
    pub keys: Vec<String>,
    /// Pass back to [`StorageClient::list`] for the next page; `None` on the
    /// last one.
    pub next: Option<String>,
}

/// Trait for blob storage operations (S3-compatible).
///
/// Abstracted as a trait so tests can use a mock without a real S3 instance.
#[async_trait]
pub trait StorageClient: Send + Sync {
    /// Upload content to the given key.
    async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), AppError>;

    /// Retrieve content by key. Returns `None` if the object doesn't exist.
    async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, AppError>;

    /// Delete an object by key.
    async fn delete_object(&self, key: &str) -> Result<(), AppError>;

    /// One page (at most 1,000 keys) of the objects whose key starts with
    /// `prefix`, in key order. Start with `continuation: None`, then pass the
    /// previous page's `next` until it is `None`.
    async fn list(&self, prefix: &str, continuation: Option<&str>) -> Result<ListPage, AppError>;
}

/// Page size of an in-memory [`StorageClient::list`], matching S3's maximum.
pub const LIST_PAGE_SIZE: usize = 1_000;

/// [`StorageClient::list`] over an in-memory set of keys, for test doubles:
/// the continuation token is the last key of the previous page.
pub fn list_keys_page<'a>(
    keys: impl Iterator<Item = &'a String>,
    prefix: &str,
    continuation: Option<&str>,
) -> ListPage {
    let mut matching: Vec<&String> = keys
        .filter(|key| key.starts_with(prefix))
        .filter(|key| continuation.is_none_or(|after| key.as_str() > after))
        .collect();
    matching.sort();
    let next = (matching.len() > LIST_PAGE_SIZE).then(|| matching[LIST_PAGE_SIZE - 1].clone());
    ListPage {
        keys: matching.into_iter().take(LIST_PAGE_SIZE).cloned().collect(),
        next,
    }
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
    /// Create a new S3 storage client from the application's centralised config.
    ///
    /// AWS credentials (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`)
    /// are still read by the `aws-config` crate from the environment / credential chain.
    pub async fn from_app_config(storage: &crate::config::StorageConfig) -> Result<Self, AppError> {
        let bucket = storage.bucket.clone();
        if bucket.is_empty() {
            return Err(AppError::Storage("storage.bucket is not configured".into()));
        }

        let has_custom_endpoint = storage.endpoint.as_ref().is_some_and(|e| !e.is_empty());

        let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

        // Support custom S3 endpoint (for MinIO, Garage, LocalStack, etc.)
        if let Some(endpoint) = &storage.endpoint {
            if !endpoint.is_empty() {
                config_loader = config_loader.endpoint_url(endpoint.as_str());
            }
        }

        let sdk_config = config_loader.load().await;

        // Build S3 client with path-style addressing for Garage/MinIO compatibility
        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(&sdk_config);

        // Force path-style requests when using a custom endpoint (required for Garage)
        if has_custom_endpoint {
            s3_config_builder = s3_config_builder.force_path_style(true);
        }

        let client = aws_sdk_s3::Client::from_conf(s3_config_builder.build());

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

    async fn delete_object(&self, key: &str) -> Result<(), AppError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("Failed to delete object '{}': {}", key, e)))?;

        Ok(())
    }

    async fn list(&self, prefix: &str, continuation: Option<&str>) -> Result<ListPage, AppError> {
        let output = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .max_keys(LIST_PAGE_SIZE as i32)
            .set_continuation_token(continuation.map(str::to_string))
            .send()
            .await
            .map_err(|e| {
                AppError::Storage(format!(
                    "Failed to list objects under '{prefix}': {}",
                    e.into_service_error()
                ))
            })?;
        let keys = output
            .contents()
            .iter()
            .filter_map(|object| object.key().map(str::to_string))
            .collect();
        let next = if output.is_truncated().unwrap_or(false) {
            output.next_continuation_token().map(str::to_string)
        } else {
            None
        };
        Ok(ListPage { keys, next })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_listing_pages_by_prefix_in_key_order() {
        let mut keys: Vec<String> = (0..LIST_PAGE_SIZE + 5)
            .map(|i| format!("docs/{i:05}.md"))
            .collect();
        keys.push("assets/a.png".into());

        let first = list_keys_page(keys.iter(), "docs/", None);
        assert_eq!(first.keys.len(), LIST_PAGE_SIZE);
        assert_eq!(first.keys[0], "docs/00000.md");
        let second = list_keys_page(keys.iter(), "docs/", first.next.as_deref());
        assert_eq!(second.keys.len(), 5);
        assert_eq!(second.next, None);
        assert!(!second.keys.iter().any(|k| first.keys.contains(k)));

        let assets = list_keys_page(keys.iter(), "assets/", None);
        assert_eq!(assets.keys, vec!["assets/a.png".to_string()]);
    }
}
