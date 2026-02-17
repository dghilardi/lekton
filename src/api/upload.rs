use axum::extract::Multipart;
use serde::Serialize;

use crate::error::AppError;

/// Response from a successful image upload.
#[derive(Debug, Serialize)]
pub struct UploadResponse {
    /// The URL path where the image can be accessed.
    pub url: String,
}

/// Axum handler for `POST /api/v1/upload-image`.
///
/// Accepts a multipart form with a single file field named "file".
/// Uploads the image to S3 under the `images/` prefix.
pub async fn upload_image_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    mut multipart: Multipart,
) -> Result<axum::Json<UploadResponse>, AppError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name != "file" {
            continue;
        }

        let file_name = field
            .file_name()
            .unwrap_or("upload.bin")
            .to_string();

        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();

        // Only allow image types
        if !content_type.starts_with("image/") {
            return Err(AppError::BadRequest(
                "Only image files are allowed".into(),
            ));
        }

        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("Failed to read file: {e}")))?;

        // Generate a unique key
        let timestamp = chrono::Utc::now().timestamp_millis();
        let sanitized_name = file_name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
            .collect::<String>();
        let s3_key = format!("images/{}_{}", timestamp, sanitized_name);

        state.storage_client
            .put_object(&s3_key, data.to_vec())
            .await?;

        // Return the URL path (served through a future image proxy or direct S3 access)
        let url = format!("/api/v1/image/{}", s3_key.trim_start_matches("images/"));

        return Ok(axum::Json(UploadResponse { url }));
    }

    Err(AppError::BadRequest("No file field found in request".into()))
}

/// Axum handler for `GET /api/v1/image/:filename`.
///
/// Serves an image from S3 storage.
pub async fn serve_image_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;

    let s3_key = format!("images/{}", filename);

    let data = state.storage_client
        .get_object(&s3_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Image not found".into()))?;

    // Infer content type from extension
    let content_type = if filename.ends_with(".png") {
        "image/png"
    } else if filename.ends_with(".jpg") || filename.ends_with(".jpeg") {
        "image/jpeg"
    } else if filename.ends_with(".gif") {
        "image/gif"
    } else if filename.ends_with(".webp") {
        "image/webp"
    } else if filename.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    };

    Ok((
        [(axum::http::header::CONTENT_TYPE, content_type)],
        data,
    ).into_response())
}
