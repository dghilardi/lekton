mod common;

use lekton::api::schemas::{IngestSchemaResponse, SchemaDetail, SchemaListItem};
use lekton::db::schema_repository::SchemaRepository;
use lekton::storage::client::StorageClient;

fn openapi_spec() -> String {
    r#"{"openapi": "3.0.0", "info": {"title": "Test API", "version": "1.0.0"}, "paths": {}}"#
        .to_string()
}

fn asyncapi_spec() -> String {
    r#"{"asyncapi": "2.6.0", "info": {"title": "Event Bus", "version": "1.0.0"}, "channels": {}}"#
        .to_string()
}

/// Helper: ingest a schema version via the API.
async fn ingest_schema(
    server: &axum_test::TestServer,
    name: &str,
    schema_type: &str,
    version: &str,
    status: &str,
    content: &str,
) -> axum_test::TestResponse {
    server
        .post("/api/v1/schemas")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "name": name,
            "schema_type": schema_type,
            "version": version,
            "status": status,
            "content": content,
        }))
        .await
}

#[tokio::test]
async fn schema_ingest_creates_new_schema() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name = format!("api-{}", uuid::Uuid::new_v4());

    let response = ingest_schema(
        &server,
        &name,
        "openapi",
        "1.0.0",
        "stable",
        &openapi_spec(),
    )
    .await;

    let body: IngestSchemaResponse = response.json();
    assert_eq!(body.name, name);
    assert_eq!(body.version, "1.0.0");
    assert!(body.s3_key.contains(&name));

    // Verify in MongoDB
    let schema = env.schema_repo.find_by_name(&name).await.unwrap();
    assert!(schema.is_some());
    let schema = schema.unwrap();
    assert_eq!(schema.schema_type, "openapi");
    assert_eq!(schema.versions.len(), 1);
    assert_eq!(schema.versions[0].version, "1.0.0");
    assert_eq!(schema.versions[0].status, "stable");

    // Verify content in S3
    let content = env
        .storage
        .get_object(&schema.versions[0].s3_key)
        .await
        .unwrap();
    assert!(content.is_some());
    let content_str = String::from_utf8(content.unwrap()).unwrap();
    assert!(content_str.contains("openapi"));
}

#[tokio::test]
async fn schema_ingest_adds_version_to_existing() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name = format!("api-{}", uuid::Uuid::new_v4());

    // Ingest v1
    ingest_schema(&server, &name, "openapi", "1.0.0", "stable", &openapi_spec()).await;

    // Ingest v2
    ingest_schema(&server, &name, "openapi", "2.0.0", "beta", &openapi_spec()).await;

    let schema = env.schema_repo.find_by_name(&name).await.unwrap().unwrap();
    assert_eq!(schema.versions.len(), 2);
    assert_eq!(schema.versions[0].version, "1.0.0");
    assert_eq!(schema.versions[1].version, "2.0.0");
    assert_eq!(schema.versions[1].status, "beta");
}

#[tokio::test]
async fn schema_ingest_updates_existing_version() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name = format!("api-{}", uuid::Uuid::new_v4());

    // Ingest v1 as stable
    ingest_schema(&server, &name, "openapi", "1.0.0", "stable", &openapi_spec()).await;

    // Re-ingest v1 as deprecated
    ingest_schema(
        &server,
        &name,
        "openapi",
        "1.0.0",
        "deprecated",
        &openapi_spec(),
    )
    .await;

    let schema = env.schema_repo.find_by_name(&name).await.unwrap().unwrap();
    assert_eq!(schema.versions.len(), 1);
    assert_eq!(schema.versions[0].status, "deprecated");
}

#[tokio::test]
async fn schema_ingest_rejects_invalid_token() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .post("/api/v1/schemas")
        .json(&serde_json::json!({
            "service_token": "wrong-token",
            "name": "test-api",
            "schema_type": "openapi",
            "version": "1.0.0",
            "status": "stable",
            "content": openapi_spec(),
        }))
        .await;

    response.assert_status_unauthorized();
}

#[tokio::test]
async fn schema_ingest_rejects_invalid_type() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .post("/api/v1/schemas")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "name": "test-api",
            "schema_type": "graphql",
            "version": "1.0.0",
            "status": "stable",
            "content": "type Query { hello: String }",
        }))
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn schema_list_returns_all_schemas() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name_a = format!("api-a-{}", uuid::Uuid::new_v4());
    let name_b = format!("api-b-{}", uuid::Uuid::new_v4());

    ingest_schema(&server, &name_a, "openapi", "1.0.0", "stable", &openapi_spec()).await;
    ingest_schema(
        &server,
        &name_b,
        "asyncapi",
        "1.0.0",
        "stable",
        &asyncapi_spec(),
    )
    .await;

    let response = server.get("/api/v1/schemas").await;
    let list: Vec<SchemaListItem> = response.json();

    assert!(list.iter().any(|s| s.name == name_a));
    assert!(list.iter().any(|s| s.name == name_b));

    let schema_b = list.iter().find(|s| s.name == name_b).unwrap();
    assert_eq!(schema_b.schema_type, "asyncapi");
    assert_eq!(schema_b.version_count, 1);
}

#[tokio::test]
async fn schema_get_returns_detail_with_versions() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name = format!("api-{}", uuid::Uuid::new_v4());

    ingest_schema(&server, &name, "openapi", "1.0.0", "deprecated", &openapi_spec()).await;
    ingest_schema(&server, &name, "openapi", "2.0.0", "stable", &openapi_spec()).await;

    let response = server.get(&format!("/api/v1/schemas/{}", name)).await;
    let detail: SchemaDetail = response.json();

    assert_eq!(detail.name, name);
    assert_eq!(detail.schema_type, "openapi");
    assert_eq!(detail.versions.len(), 2);
    assert_eq!(detail.versions[0].version, "1.0.0");
    assert_eq!(detail.versions[0].status, "deprecated");
    assert_eq!(detail.versions[1].version, "2.0.0");
    assert_eq!(detail.versions[1].status, "stable");
}

#[tokio::test]
async fn schema_get_not_found() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server.get("/api/v1/schemas/nonexistent-api").await;
    response.assert_status_not_found();
}

#[tokio::test]
async fn schema_get_version_content() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name = format!("api-{}", uuid::Uuid::new_v4());
    let spec = openapi_spec();

    ingest_schema(&server, &name, "openapi", "1.0.0", "stable", &spec).await;

    let response = server
        .get(&format!("/api/v1/schemas/{}/1.0.0", name))
        .await;

    let content = response.text();
    assert!(content.contains("openapi"));
    assert!(content.contains("3.0.0"));
}

#[tokio::test]
async fn schema_get_version_not_found() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name = format!("api-{}", uuid::Uuid::new_v4());

    ingest_schema(&server, &name, "openapi", "1.0.0", "stable", &openapi_spec()).await;

    let server = env.server_permissive();
    let response = server
        .get(&format!("/api/v1/schemas/{}/9.9.9", name))
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn schema_yaml_content_stored_with_yaml_extension() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name = format!("api-{}", uuid::Uuid::new_v4());
    let yaml_spec = "openapi: '3.0.0'\ninfo:\n  title: Test\n  version: '1.0.0'\npaths: {}";

    ingest_schema(&server, &name, "openapi", "1.0.0", "stable", yaml_spec).await;

    let schema = env.schema_repo.find_by_name(&name).await.unwrap().unwrap();
    assert!(
        schema.versions[0].s3_key.ends_with(".yaml"),
        "YAML content should be stored with .yaml extension, got: {}",
        schema.versions[0].s3_key
    );
}

#[tokio::test]
async fn schema_full_lifecycle() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let name = format!("lifecycle-api-{}", uuid::Uuid::new_v4());

    // 1. Ingest v1
    ingest_schema(&server, &name, "openapi", "1.0.0", "stable", &openapi_spec()).await;

    // 2. Ingest v2 beta
    ingest_schema(&server, &name, "openapi", "2.0.0", "beta", &openapi_spec()).await;

    // 3. List should show schema
    let list: Vec<SchemaListItem> = server.get("/api/v1/schemas").await.json();
    let item = list.iter().find(|s| s.name == name).unwrap();
    assert_eq!(item.version_count, 2);
    // Latest non-deprecated should be "2.0.0" (beta is not deprecated)
    assert_eq!(item.latest_version.as_deref(), Some("2.0.0"));

    // 4. Get detail
    let detail: SchemaDetail = server
        .get(&format!("/api/v1/schemas/{}", name))
        .await
        .json();
    assert_eq!(detail.versions.len(), 2);

    // 5. Fetch version content
    let content = server
        .get(&format!("/api/v1/schemas/{}/1.0.0", name))
        .await
        .text();
    assert!(content.contains("openapi"));

    // 6. Deprecate v1
    ingest_schema(
        &server,
        &name,
        "openapi",
        "1.0.0",
        "deprecated",
        &openapi_spec(),
    )
    .await;

    let detail: SchemaDetail = server
        .get(&format!("/api/v1/schemas/{}", name))
        .await
        .json();
    assert_eq!(detail.versions[0].status, "deprecated");
    assert_eq!(detail.versions[1].status, "beta");
}
