use crate::models::document::Document;
use crate::state::AppState;
use aws_sdk_s3::primitives::ByteStream;
use chrono::Utc;
use mongodb::bson::doc;

pub async fn seed_demo_data(state: &AppState) {
    tracing::info!("Starting demo data seeding...");

    // Using include_str! to embed demo content directly into the binary
    let demo_docs = vec![
        (
            "getting-started",
            "Getting Started with Lekton",
            include_str!("../demo_data/getting_started.md"),
            vec!["intro", "guide"],
        ),
        (
            "architecture",
            "Lekton Architecture",
            include_str!("../demo_data/architecture.md"),
            vec!["architecture", "technical"],
        ),
        (
            "api-reference",
            "API Reference",
            include_str!("../demo_data/api_reference.md"),
            vec!["api", "reference"],
        ),
    ];

    let collection = state.documents_collection();

    for (slug, title, content, tags) in demo_docs {
        let slug_str = slug.to_string();
        // Check if document already exists
        let filter = doc! { "slug": &slug_str };
        match collection.find_one(filter.clone()).await {
            Ok(Some(_)) => {
                tracing::info!("Document '{}' already exists, skipping.", slug);
                continue;
            }
            Err(e) => {
                tracing::error!("Failed to check for existing document '{}': {}", slug, e);
                continue;
            }
            Ok(None) => {}
        }

        // Upload to S3
        let s3_key = format!("{}.md", slug);
        let body = ByteStream::from(content.as_bytes().to_vec());

        match state
            .s3
            .put_object()
            .bucket(&state.config.s3_bucket)
            .key(&s3_key)
            .body(body)
            .send()
            .await
        {
            Ok(_) => tracing::info!("Uploaded '{}' to S3.", s3_key),
            Err(e) => {
                tracing::error!("Failed to upload '{}' to S3: {}", s3_key, e);
                // We could continue, but if S3 fails, the document is broken.
                // However, for demo, maybe we proceed to insert metadata anyway or skip.
                // Let's log and skip inserting metadata to avoid broken state.
                continue;
            }
        }

        // Create Document object
        let document = Document {
            id: None, // MongoDB will generate this
            slug: slug_str.clone(),
            title: title.to_string(),
            s3_key: s3_key.clone(),
            access_level: "public".to_string(),
            service_owner: "system".to_string(),
            last_updated: Utc::now(),
            tags: tags.into_iter().map(String::from).collect(),
            links_out: vec![],
            backlinks: vec![],
        };

        // Insert into MongoDB
        match collection.insert_one(document.clone()).await {
            Ok(insert_result) => {
                tracing::info!("Inserted metadata for '{}' into MongoDB.", slug);

                // Index in Meilisearch if available
                if let Some(meili) = &state.meili {
                    let index = meili.index("documents");
                    // Retrieve the full document with ID or construct it
                    if let Ok(Some(inserted_doc)) = collection.find_one(filter).await {
                        match index.add_documents(&[inserted_doc], Some("_id")).await {
                            Ok(task) => tracing::info!(
                                "Triggered Meilisearch indexing for '{}': {:?}",
                                slug,
                                task.task_uid
                            ),
                            Err(e) => {
                                tracing::error!("Failed to index '{}' in Meilisearch: {}", slug, e)
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to insert metadata for '{}': {}", slug, e);
            }
        }
    }

    tracing::info!("Demo data seeding completed.");
}
