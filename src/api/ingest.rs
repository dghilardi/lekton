use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use crate::state::AppState;
use chrono::Utc;
use axum::response::IntoResponse;
use http::StatusCode;

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub slug: String,
    pub title: String,
    pub content: String,
    pub access_level: String,
    pub service_owner: String,
    pub tags: Vec<String>,
}

#[derive(Serialize)]
pub struct IngestResponse {
    pub message: String,
    pub slug: String,
}

pub use crate::models::search::SearchDocument;

pub async fn ingest_handler(
    State(state): State<AppState>,
    Json(payload): Json<IngestRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // 1. Link Validation
    let links = crate::models::link_validator::LinkValidator::extract_links(&payload.content);
    for link in &links {
        let filter = mongodb::bson::doc! { "slug": link };
        let exists = state.documents_collection()
            .find_one(filter)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("MongoDB error during validation: {}", e)))?
            .is_some();
        
        if !exists {
            // In a real scenario, we might want this to be a warning or a strict error based on config
            tracing::warn!("Document {} links to non-existent slug: {}", payload.slug, link);
        }
    }

    let s3_key = format!("docs/{}.md", payload.slug);

    // 2. Upload to S3
    state.s3.put_object()
        .bucket(&state.config.s3_bucket)
        .key(&s3_key)
        .body(payload.content.clone().into_bytes().into())
        .send()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("S3 error: {}", e)))?;

    // 3. Update MongoDB
    let filter = mongodb::bson::doc! { "slug": &payload.slug };
    let update = mongodb::bson::doc! {
        "$set": {
            "title": &payload.title,
            "s3_key": &s3_key,
            "access_level": &payload.access_level,
            "service_owner": &payload.service_owner,
            "last_updated": Utc::now(),
            "tags": &payload.tags,
            "links_out": &links,
        },
        "$setOnInsert": {
            "backlinks": Vec::<String>::new(),
        }
    };

    state.documents_collection()
        .update_one(filter, update)
        .upsert(true)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("MongoDB error: {}", e)))?;

    // 4. Index in Meilisearch
    if let Some(ref meili) = state.meili {
        let index = meili.index("documents");
        let search_doc = SearchDocument {
            id: payload.slug.replace('/', "_"), // Meilisearch IDs must be alphanumeric
            slug: payload.slug.clone(),
            title: payload.title,
            content: payload.content,
            access_level: payload.access_level,
            tags: payload.tags,
        };
        
        index.add_documents(&[search_doc], Some("id"))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Meilisearch error: {}", e)))?;
    }

    Ok(Json(IngestResponse {
        message: "Document ingested successfully".to_string(),
        slug: payload.slug,
    }))
}
