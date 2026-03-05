mod common;

use axum_test::multipart::{MultipartForm, Part};

/// Helper: upload an asset via multipart PUT.
async fn upload_asset(
    server: &axum_test::TestServer,
    key: &str,
    content: &[u8],
    content_type: &str,
    token: &str,
) -> axum_test::TestResponse {
    let form = MultipartForm::new()
        .add_text("service_token", token)
        .add_part("file", Part::bytes(content.to_vec()).file_name("file").mime_type(content_type));

    server
        .put(&format!("/api/v1/assets/{}", key))
        .multipart(form)
        .await
}

#[tokio::test]
async fn upload_asset_creates_new_asset() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let content = b"hello world";
    let response = upload_asset(&server, "project-a/readme.txt", content, "text/plain", "test-token").await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["key"], "project-a/readme.txt");
    assert_eq!(body["s3_key"], "assets/project-a/readme.txt");
    assert_eq!(body["content_type"], "text/plain");
    assert_eq!(body["size_bytes"], 11);

    // Verify in repo
    let asset = env.asset_repo.find_by_key("project-a/readme.txt").await.unwrap().unwrap();
    assert_eq!(asset.content_type, "text/plain");
    assert_eq!(asset.size_bytes, 11);

    // Verify in S3
    let stored = env.storage.get_object("assets/project-a/readme.txt").await.unwrap().unwrap();
    assert_eq!(stored, content);
}

#[tokio::test]
async fn upload_asset_replaces_existing() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    // Upload v1
    upload_asset(&server, "logo.png", b"v1-data", "image/png", "test-token").await;

    // Upload v2
    upload_asset(&server, "logo.png", b"v2-data-longer", "image/png", "test-token").await;

    // Should still be one asset
    let assets = env.asset_repo.list_all().await.unwrap();
    assert_eq!(assets.len(), 1);

    let asset = env.asset_repo.find_by_key("logo.png").await.unwrap().unwrap();
    assert_eq!(asset.size_bytes, b"v2-data-longer".len() as u64);

    // S3 should have new content
    let stored = env.storage.get_object("assets/logo.png").await.unwrap().unwrap();
    assert_eq!(stored, b"v2-data-longer");
}

#[tokio::test]
async fn upload_asset_rejects_invalid_token() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = upload_asset(&server, "file.txt", b"data", "text/plain", "wrong-token").await;
    response.assert_status_unauthorized();
}

#[tokio::test]
async fn upload_asset_with_nested_key() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let response = upload_asset(
        &server,
        "org/team/project/configs/nginx.conf",
        b"server { }",
        "application/octet-stream",
        "test-token",
    )
    .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["key"], "org/team/project/configs/nginx.conf");

    let asset = env.asset_repo.find_by_key("org/team/project/configs/nginx.conf").await.unwrap();
    assert!(asset.is_some());
}

#[tokio::test]
async fn serve_asset_returns_content() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let content = b"PDF content here";
    upload_asset(&server, "docs/manual.pdf", content, "application/pdf", "test-token").await;

    let response = server.get("/api/v1/assets/docs/manual.pdf").await;
    response.assert_status_ok();

    let headers = response.headers();
    assert_eq!(headers.get("content-type").unwrap(), "application/pdf");

    let body = response.into_bytes();
    assert_eq!(body.as_ref(), content);
}

#[tokio::test]
async fn serve_asset_not_found() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server.get("/api/v1/assets/nonexistent.txt").await;
    response.assert_status_not_found();
}

#[tokio::test]
async fn list_assets_returns_all() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    for name in &["a/file1.txt", "b/file2.txt", "c/file3.txt"] {
        upload_asset(&server, name, b"data", "text/plain", "test-token").await;
    }

    let response = server.get("/api/v1/assets").await;
    response.assert_status_ok();
    let list: Vec<serde_json::Value> = response.json();
    assert_eq!(list.len(), 3);
}

#[tokio::test]
async fn list_assets_with_prefix_filter() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    for name in &["project-a/config.yaml", "project-a/logo.png", "project-b/readme.md"] {
        upload_asset(&server, name, b"data", "text/plain", "test-token").await;
    }

    let response = server
        .get("/api/v1/assets")
        .add_query_param("prefix", "project-a/")
        .await;
    response.assert_status_ok();
    let list: Vec<serde_json::Value> = response.json();
    assert_eq!(list.len(), 2);
}

#[tokio::test]
async fn delete_asset_removes_from_storage_and_db() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    upload_asset(&server, "temp/file.txt", b"temporary", "text/plain", "test-token").await;

    let response = server
        .delete("/api/v1/assets/temp/file.txt")
        .json(&serde_json::json!({"service_token": "test-token"}))
        .await;
    response.assert_status_ok();

    // Verify removed from repo
    assert!(env.asset_repo.find_by_key("temp/file.txt").await.unwrap().is_none());

    // Verify removed from S3
    assert!(env.storage.get_object("assets/temp/file.txt").await.unwrap().is_none());
}

#[tokio::test]
async fn delete_asset_not_found() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .delete("/api/v1/assets/nonexistent.txt")
        .json(&serde_json::json!({"service_token": "test-token"}))
        .await;
    response.assert_status_not_found();
}

#[tokio::test]
async fn delete_asset_rejects_invalid_token() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    upload_asset(&server, "file.txt", b"data", "text/plain", "test-token").await;

    let response = server
        .delete("/api/v1/assets/file.txt")
        .json(&serde_json::json!({"service_token": "wrong-token"}))
        .await;
    response.assert_status_unauthorized();
}

// --- Editor upload tests ---

#[tokio::test]
async fn editor_upload_creates_asset() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let png_bytes: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
    ];

    let form = MultipartForm::new()
        .add_part("file", Part::bytes(png_bytes.clone()).file_name("photo.png").mime_type("image/png"));

    let response = server
        .post("/api/v1/editor/upload-asset")
        .multipart(form)
        .await;

    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    let key = body["key"].as_str().unwrap();
    let url = body["url"].as_str().unwrap();

    assert!(key.starts_with("editor/"), "Key should start with editor/, got: {}", key);
    assert!(key.contains("photo.png"), "Key should contain filename, got: {}", key);
    assert!(url.starts_with("/api/v1/assets/"), "URL should start with /api/v1/assets/, got: {}", url);
    assert_eq!(body["content_type"], "image/png");
    assert_eq!(body["size_bytes"], 8);

    // Verify asset exists in repo
    let asset = env.asset_repo.find_by_key(key).await.unwrap().unwrap();
    assert_eq!(asset.uploaded_by, "web-editor");
    assert_eq!(asset.content_type, "image/png");

    // Verify content in S3
    let stored = env.storage.get_object(&format!("assets/{}", key)).await.unwrap().unwrap();
    assert_eq!(stored, png_bytes);
}

#[tokio::test]
async fn editor_upload_serves_correctly() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let content = b"test file content";
    let form = MultipartForm::new()
        .add_part("file", Part::bytes(content.to_vec()).file_name("test.txt").mime_type("text/plain"));

    let upload_response = server
        .post("/api/v1/editor/upload-asset")
        .multipart(form)
        .await;

    let body: serde_json::Value = upload_response.json();
    let url = body["url"].as_str().unwrap();

    // Serve it back
    let response = server.get(url).await;
    response.assert_status_ok();

    let headers = response.headers();
    assert_eq!(headers.get("content-type").unwrap(), "text/plain");

    let served_content = response.into_bytes();
    assert_eq!(served_content.as_ref(), content);
}

#[tokio::test]
async fn editor_upload_missing_file_field() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let form = MultipartForm::new()
        .add_text("wrong_field", "some data");

    let response = server
        .post("/api/v1/editor/upload-asset")
        .multipart(form)
        .await;

    response.assert_status_bad_request();
}
