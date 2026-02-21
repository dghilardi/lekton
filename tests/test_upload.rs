mod common;

use axum_test::multipart::{MultipartForm, Part};

#[tokio::test]
async fn upload_image_success() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    // Create a minimal 1x1 PNG
    let png_bytes: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, // bit depth, color type, CRC
        0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, // IDAT chunk
        0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, // compressed data
        0x00, 0x02, 0x00, 0x01, 0xE2, 0x21, 0xBC, 0x33, // CRC
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, // IEND chunk
        0xAE, 0x42, 0x60, 0x82,
    ];

    let form = MultipartForm::new()
        .add_part("file", Part::bytes(png_bytes).file_name("test.png").mime_type("image/png"));

    let response = server
        .post("/api/v1/upload-image")
        .multipart(form)
        .await;

    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    let url = body["url"].as_str().expect("Response should contain url");
    assert!(
        url.starts_with("/api/v1/image/"),
        "URL should start with /api/v1/image/, got: {}",
        url
    );
    assert!(
        url.contains("test.png"),
        "URL should contain the filename, got: {}",
        url
    );
}

#[tokio::test]
async fn upload_rejects_non_image() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let form = MultipartForm::new()
        .add_part("file", Part::bytes(b"hello world".to_vec()).file_name("test.txt").mime_type("text/plain"));

    let response = server
        .post("/api/v1/upload-image")
        .multipart(form)
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn upload_missing_file_field() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let form = MultipartForm::new()
        .add_part("wrong_field", Part::bytes(b"data".to_vec()).file_name("test.png").mime_type("image/png"));

    let response = server
        .post("/api/v1/upload-image")
        .multipart(form)
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn serve_image_returns_correct_content_type() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    // Upload a PNG
    let png_bytes: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE,
        0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54,
        0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00,
        0x00, 0x02, 0x00, 0x01, 0xE2, 0x21, 0xBC, 0x33,
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
        0xAE, 0x42, 0x60, 0x82,
    ];

    let form = MultipartForm::new()
        .add_part("file", Part::bytes(png_bytes.clone()).file_name("serve_test.png").mime_type("image/png"));

    let upload_response = server
        .post("/api/v1/upload-image")
        .multipart(form)
        .await;

    let body: serde_json::Value = upload_response.json();
    let url = body["url"].as_str().unwrap();

    // Serve the image back
    let response = server.get(url).await;
    response.assert_status_ok();

    let content_type = response
        .headers()
        .get("content-type")
        .expect("Content-Type header should be present")
        .to_str()
        .unwrap();
    assert_eq!(content_type, "image/png");
}

#[tokio::test]
async fn serve_nonexistent_image() {
    let env = common::TestEnv::start().await;
    let server = env.server_permissive();

    let response = server
        .get("/api/v1/image/nonexistent_12345.png")
        .await;

    response.assert_status_not_found();
}
