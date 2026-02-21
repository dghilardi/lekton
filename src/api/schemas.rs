use serde::{Deserialize, Serialize};

use crate::db::models::{Schema, SchemaVersion};
use crate::db::schema_repository::SchemaRepository;
use crate::error::AppError;
use crate::storage::client::StorageClient;

/// Request payload for ingesting a schema version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestSchemaRequest {
    /// Service authentication token.
    pub service_token: String,
    /// Schema name (e.g., "payment-service-api").
    pub name: String,
    /// Schema type: "openapi", "asyncapi", or "jsonschema".
    pub schema_type: String,
    /// Semantic version string (e.g., "1.0.0").
    pub version: String,
    /// Version status: "stable", "beta", or "deprecated".
    #[serde(default = "default_status")]
    pub status: String,
    /// Raw schema content (JSON or YAML).
    pub content: String,
}

fn default_status() -> String {
    "stable".to_string()
}

/// Response from a successful schema ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestSchemaResponse {
    pub message: String,
    pub name: String,
    pub version: String,
    pub s3_key: String,
}

/// Response for listing schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaListItem {
    pub name: String,
    pub schema_type: String,
    pub latest_version: Option<String>,
    pub version_count: usize,
}

/// Response for a single schema with all versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDetail {
    pub name: String,
    pub schema_type: String,
    pub versions: Vec<SchemaVersionInfo>,
}

/// Version info returned in API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersionInfo {
    pub version: String,
    pub status: String,
    pub s3_key: String,
}

const VALID_SCHEMA_TYPES: &[&str] = &["openapi", "asyncapi", "jsonschema"];
const VALID_STATUSES: &[&str] = &["stable", "beta", "deprecated"];

/// Core schema ingestion logic — separated from HTTP layer for testability.
pub async fn process_schema_ingest(
    schema_repo: &dyn SchemaRepository,
    storage: &dyn StorageClient,
    request: IngestSchemaRequest,
    expected_token: &str,
) -> Result<IngestSchemaResponse, AppError> {
    // 1. Validate service token
    if request.service_token != expected_token {
        return Err(AppError::Auth("Invalid service token".into()));
    }

    // 2. Validate schema name
    if request.name.is_empty() {
        return Err(AppError::BadRequest("Schema name cannot be empty".into()));
    }

    // 3. Validate schema type
    if !VALID_SCHEMA_TYPES.contains(&request.schema_type.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Invalid schema type '{}'. Expected: {}",
            request.schema_type,
            VALID_SCHEMA_TYPES.join(", ")
        )));
    }

    // 4. Validate version
    if request.version.is_empty() {
        return Err(AppError::BadRequest("Version cannot be empty".into()));
    }

    // 5. Validate status
    if !VALID_STATUSES.contains(&request.status.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Invalid status '{}'. Expected: {}",
            request.status,
            VALID_STATUSES.join(", ")
        )));
    }

    // 6. Build S3 key
    let extension = if request.content.trim_start().starts_with('{') {
        "json"
    } else {
        "yaml"
    };
    let s3_key = format!(
        "schemas/{}/{}.{}",
        request.name, request.version, extension
    );

    // 7. Upload content to S3
    storage
        .put_object(&s3_key, request.content.into_bytes())
        .await?;

    // 8. Upsert schema in MongoDB
    let new_version = SchemaVersion {
        version: request.version.clone(),
        s3_key: s3_key.clone(),
        status: request.status,
    };

    let existing = schema_repo.find_by_name(&request.name).await?;

    match existing {
        Some(mut schema) => {
            // Update existing version or add new one
            if let Some(v) = schema
                .versions
                .iter_mut()
                .find(|v| v.version == request.version)
            {
                v.s3_key = new_version.s3_key;
                v.status = new_version.status;
                schema_repo.create_or_update(schema).await?;
            } else {
                schema_repo
                    .add_version(&request.name, new_version)
                    .await?;
            }
        }
        None => {
            let schema = Schema {
                name: request.name.clone(),
                schema_type: request.schema_type,
                versions: vec![new_version],
            };
            schema_repo.create_or_update(schema).await?;
        }
    }

    Ok(IngestSchemaResponse {
        message: "Schema version ingested successfully".to_string(),
        name: request.name,
        version: request.version,
        s3_key,
    })
}

/// Core logic to list all schemas.
pub async fn process_list_schemas(
    schema_repo: &dyn SchemaRepository,
) -> Result<Vec<SchemaListItem>, AppError> {
    let schemas = schema_repo.list_all().await?;

    Ok(schemas
        .into_iter()
        .map(|s| {
            let latest = s
                .versions
                .iter()
                .filter(|v| v.status != "deprecated")
                .last()
                .or(s.versions.last())
                .map(|v| v.version.clone());

            SchemaListItem {
                name: s.name,
                schema_type: s.schema_type,
                latest_version: latest,
                version_count: s.versions.len(),
            }
        })
        .collect())
}

/// Core logic to get a schema's details.
pub async fn process_get_schema(
    schema_repo: &dyn SchemaRepository,
    name: &str,
) -> Result<SchemaDetail, AppError> {
    let schema = schema_repo.find_by_name(name).await?;

    let schema = schema.ok_or_else(|| {
        AppError::NotFound(format!("Schema '{}' not found", name))
    })?;

    Ok(SchemaDetail {
        name: schema.name,
        schema_type: schema.schema_type,
        versions: schema
            .versions
            .into_iter()
            .map(|v| SchemaVersionInfo {
                version: v.version,
                status: v.status,
                s3_key: v.s3_key,
            })
            .collect(),
    })
}

/// Core logic to get a specific schema version's content from S3.
pub async fn process_get_schema_content(
    schema_repo: &dyn SchemaRepository,
    storage: &dyn StorageClient,
    name: &str,
    version: &str,
) -> Result<String, AppError> {
    let schema = schema_repo.find_by_name(name).await?;

    let schema = schema.ok_or_else(|| {
        AppError::NotFound(format!("Schema '{}' not found", name))
    })?;

    let ver = schema
        .versions
        .iter()
        .find(|v| v.version == version)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Version '{}' not found for schema '{}'",
                version, name
            ))
        })?;

    let content_bytes = storage.get_object(&ver.s3_key).await?;

    let content_bytes = content_bytes.ok_or_else(|| {
        AppError::NotFound(format!(
            "Schema content not found in storage for '{}/{}'",
            name, version
        ))
    })?;

    String::from_utf8(content_bytes)
        .map_err(|e| AppError::Internal(format!("Invalid UTF-8 in schema content: {e}")))
}

/// Axum handler for `POST /api/v1/schemas`.
#[cfg(feature = "ssr")]
pub async fn ingest_schema_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<IngestSchemaRequest>,
) -> Result<axum::Json<IngestSchemaResponse>, AppError> {
    let response = process_schema_ingest(
        state.schema_repo.as_ref(),
        state.storage_client.as_ref(),
        request,
        &state.service_token,
    )
    .await?;

    Ok(axum::Json(response))
}

/// Axum handler for `GET /api/v1/schemas`.
#[cfg(feature = "ssr")]
pub async fn list_schemas_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
) -> Result<axum::Json<Vec<SchemaListItem>>, AppError> {
    let result = process_list_schemas(state.schema_repo.as_ref()).await?;
    Ok(axum::Json(result))
}

/// Axum handler for `GET /api/v1/schemas/:name`.
#[cfg(feature = "ssr")]
pub async fn get_schema_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<axum::Json<SchemaDetail>, AppError> {
    let result = process_get_schema(state.schema_repo.as_ref(), &name).await?;
    Ok(axum::Json(result))
}

/// Axum handler for `GET /api/v1/schemas/:name/:version`.
#[cfg(feature = "ssr")]
pub async fn get_schema_version_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Path((name, version)): axum::extract::Path<(String, String)>,
) -> Result<String, AppError> {
    process_get_schema_content(
        state.schema_repo.as_ref(),
        state.storage_client.as_ref(),
        &name,
        &version,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    // -- Mock implementations --

    struct MockStorage {
        objects: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                objects: Mutex::new(std::collections::HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl StorageClient for MockStorage {
        async fn put_object(&self, key: &str, content: Vec<u8>) -> Result<(), AppError> {
            self.objects
                .lock()
                .unwrap()
                .insert(key.to_string(), content);
            Ok(())
        }

        async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, AppError> {
            Ok(self.objects.lock().unwrap().get(key).cloned())
        }
    }

    struct MockSchemaRepo {
        schemas: Mutex<Vec<Schema>>,
    }

    impl MockSchemaRepo {
        fn new() -> Self {
            Self {
                schemas: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl SchemaRepository for MockSchemaRepo {
        async fn create_or_update(&self, schema: Schema) -> Result<(), AppError> {
            let mut schemas = self.schemas.lock().unwrap();
            schemas.retain(|s| s.name != schema.name);
            schemas.push(schema);
            Ok(())
        }

        async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, AppError> {
            Ok(self
                .schemas
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.name == name)
                .cloned())
        }

        async fn list_all(&self) -> Result<Vec<Schema>, AppError> {
            Ok(self.schemas.lock().unwrap().clone())
        }

        async fn add_version(
            &self,
            schema_name: &str,
            version: SchemaVersion,
        ) -> Result<(), AppError> {
            let mut schemas = self.schemas.lock().unwrap();
            let schema = schemas
                .iter_mut()
                .find(|s| s.name == schema_name)
                .ok_or_else(|| {
                    AppError::NotFound(format!("Schema '{}' not found", schema_name))
                })?;

            if schema.versions.iter().any(|v| v.version == version.version) {
                return Err(AppError::BadRequest(format!(
                    "Version '{}' already exists",
                    version.version
                )));
            }

            schema.versions.push(version);
            Ok(())
        }

        async fn delete(&self, name: &str) -> Result<(), AppError> {
            let mut schemas = self.schemas.lock().unwrap();
            let len_before = schemas.len();
            schemas.retain(|s| s.name != name);
            if schemas.len() == len_before {
                return Err(AppError::NotFound(format!(
                    "Schema '{}' not found",
                    name
                )));
            }
            Ok(())
        }
    }

    fn make_schema_request(token: &str, name: &str, version: &str) -> IngestSchemaRequest {
        IngestSchemaRequest {
            service_token: token.to_string(),
            name: name.to_string(),
            schema_type: "openapi".to_string(),
            version: version.to_string(),
            status: "stable".to_string(),
            content: r#"{"openapi": "3.0.0", "info": {"title": "Test", "version": "1.0.0"}}"#
                .to_string(),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_success() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let request = make_schema_request("valid-token", "test-api", "1.0.0");

        let result =
            process_schema_ingest(&repo, &storage, request, "valid-token").await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.name, "test-api");
        assert_eq!(response.version, "1.0.0");
        assert!(response.s3_key.contains("schemas/test-api/1.0.0"));

        // Verify schema was stored in repo
        let schema = repo.find_by_name("test-api").await.unwrap().unwrap();
        assert_eq!(schema.schema_type, "openapi");
        assert_eq!(schema.versions.len(), 1);
        assert_eq!(schema.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn test_ingest_schema_invalid_token() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let request = make_schema_request("wrong-token", "test-api", "1.0.0");

        let result =
            process_schema_ingest(&repo, &storage, request, "valid-token").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("Invalid service token")),
            other => panic!("Expected Auth error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_empty_name() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let request = make_schema_request("valid-token", "", "1.0.0");

        let result =
            process_schema_ingest(&repo, &storage, request, "valid-token").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("name cannot be empty")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_invalid_type() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let mut request = make_schema_request("valid-token", "test-api", "1.0.0");
        request.schema_type = "graphql".to_string();

        let result =
            process_schema_ingest(&repo, &storage, request, "valid-token").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("Invalid schema type")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_invalid_status() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let mut request = make_schema_request("valid-token", "test-api", "1.0.0");
        request.status = "released".to_string();

        let result =
            process_schema_ingest(&repo, &storage, request, "valid-token").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("Invalid status")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_add_second_version() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();

        // Ingest v1
        let req1 = make_schema_request("valid-token", "test-api", "1.0.0");
        process_schema_ingest(&repo, &storage, req1, "valid-token")
            .await
            .unwrap();

        // Ingest v2
        let req2 = make_schema_request("valid-token", "test-api", "2.0.0");
        process_schema_ingest(&repo, &storage, req2, "valid-token")
            .await
            .unwrap();

        let schema = repo.find_by_name("test-api").await.unwrap().unwrap();
        assert_eq!(schema.versions.len(), 2);
    }

    #[tokio::test]
    async fn test_ingest_schema_update_existing_version() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();

        // Ingest v1
        let req1 = make_schema_request("valid-token", "test-api", "1.0.0");
        process_schema_ingest(&repo, &storage, req1, "valid-token")
            .await
            .unwrap();

        // Re-ingest v1 with different status
        let mut req2 = make_schema_request("valid-token", "test-api", "1.0.0");
        req2.status = "deprecated".to_string();
        process_schema_ingest(&repo, &storage, req2, "valid-token")
            .await
            .unwrap();

        let schema = repo.find_by_name("test-api").await.unwrap().unwrap();
        assert_eq!(schema.versions.len(), 1);
        assert_eq!(schema.versions[0].status, "deprecated");
    }

    #[tokio::test]
    async fn test_ingest_schema_yaml_detection() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let mut request = make_schema_request("valid-token", "test-api", "1.0.0");
        request.content = "openapi: '3.0.0'\ninfo:\n  title: Test".to_string();

        let result =
            process_schema_ingest(&repo, &storage, request, "valid-token")
                .await
                .unwrap();

        assert!(result.s3_key.ends_with(".yaml"));
    }

    #[tokio::test]
    async fn test_list_schemas() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();

        // Ingest two schemas
        let req1 = make_schema_request("valid-token", "api-a", "1.0.0");
        process_schema_ingest(&repo, &storage, req1, "valid-token")
            .await
            .unwrap();

        let mut req2 = make_schema_request("valid-token", "api-b", "1.0.0");
        req2.schema_type = "asyncapi".to_string();
        process_schema_ingest(&repo, &storage, req2, "valid-token")
            .await
            .unwrap();

        let list = process_list_schemas(&repo).await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_get_schema_detail() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();

        let req = make_schema_request("valid-token", "test-api", "1.0.0");
        process_schema_ingest(&repo, &storage, req, "valid-token")
            .await
            .unwrap();

        let detail = process_get_schema(&repo, "test-api").await.unwrap();
        assert_eq!(detail.name, "test-api");
        assert_eq!(detail.schema_type, "openapi");
        assert_eq!(detail.versions.len(), 1);
    }

    #[tokio::test]
    async fn test_get_schema_not_found() {
        let repo = MockSchemaRepo::new();
        let result = process_get_schema(&repo, "nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::NotFound(msg) => assert!(msg.contains("nonexistent")),
            other => panic!("Expected NotFound error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_get_schema_content() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();

        let req = make_schema_request("valid-token", "test-api", "1.0.0");
        process_schema_ingest(&repo, &storage, req, "valid-token")
            .await
            .unwrap();

        let content =
            process_get_schema_content(&repo, &storage, "test-api", "1.0.0")
                .await
                .unwrap();

        assert!(content.contains("openapi"));
    }

    #[tokio::test]
    async fn test_get_schema_content_version_not_found() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();

        let req = make_schema_request("valid-token", "test-api", "1.0.0");
        process_schema_ingest(&repo, &storage, req, "valid-token")
            .await
            .unwrap();

        let result =
            process_get_schema_content(&repo, &storage, "test-api", "9.9.9").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::NotFound(msg) => assert!(msg.contains("9.9.9")),
            other => panic!("Expected NotFound error, got: {:?}", other),
        }
    }
}
