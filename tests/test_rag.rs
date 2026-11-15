/// Integration tests for the RAG pipeline (Qdrant + embedding).
///
/// These tests spin up a real Qdrant container via testcontainers and exercise the
/// full index_document → search path using a deterministic in-process mock embedding
/// service. No external embedding API or LLM is needed.
#[cfg(feature = "ssr")]
mod rag_integration {
    use std::sync::Arc;

    use async_trait::async_trait;
    use testcontainers::core::{ContainerPort, WaitFor};
    use testcontainers::runners::AsyncRunner;
    use testcontainers::GenericImage;

    use lekton::error::AppError;
    use lekton::rag::embedding::EmbeddingService;
    use lekton::rag::service::{DefaultRagService, RagService};
    use lekton::rag::vectorstore::{QdrantVectorStore, VectorStore};
    use lekton::usage::UsageKey;

    const DIMENSIONS: u32 = 32;
    const COLLECTION: &str = "test-rag";

    /// Deterministic mock: encodes each byte of the text into a fixed-length vector,
    /// then L2-normalises it. The same text always returns the same vector, so a
    /// round-trip index + search with identical text yields similarity = 1.0.
    struct DeterministicEmbedding;

    #[async_trait]
    impl EmbeddingService for DeterministicEmbedding {
        async fn embed(
            &self,
            _key: &UsageKey,
            texts: &[String],
        ) -> Result<Vec<Vec<f32>>, AppError> {
            Ok(texts
                .iter()
                .map(|text| {
                    let mut v = vec![0.0f32; DIMENSIONS as usize];
                    for (i, byte) in text.bytes().enumerate() {
                        v[i % DIMENSIONS as usize] += byte as f32;
                    }
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
                    v.iter_mut().for_each(|x| *x /= norm);
                    v
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn index_and_retrieve_document() {
        let qdrant = GenericImage::new("qdrant/qdrant", "v1.19.1")
            .with_exposed_port(ContainerPort::Tcp(6334))
            .with_wait_for(WaitFor::message_on_stdout("Qdrant gRPC listening"))
            .start()
            .await
            .expect("failed to start Qdrant container");

        let port = qdrant
            .get_host_port_ipv4(6334)
            .await
            .expect("failed to get Qdrant gRPC port");
        let url = format!("http://localhost:{port}");

        let vectorstore = Arc::new(
            QdrantVectorStore::new(&url, COLLECTION).expect("failed to create QdrantVectorStore"),
        );

        vectorstore
            .ensure_collection(DIMENSIONS)
            .await
            .expect("ensure_collection failed");

        let svc = DefaultRagService::new(
            Arc::new(DeterministicEmbedding),
            vectorstore.clone(),
            256,
            32,
        );

        let slug = "test-doc";
        let content = "The capital of France is Paris. Paris is known for the Eiffel Tower.";

        svc.index_document(
            slug,
            "Test Document",
            content,
            "public",
            false,
            &[],
            None,
            None,
        )
        .await
        .expect("index_document failed");

        // Embed the query with the same mock service and search
        let query = content.to_string();
        let query_vec = DeterministicEmbedding
            .embed(&UsageKey::System, &[query])
            .await
            .expect("embed failed")
            .into_iter()
            .next()
            .unwrap();

        let results = vectorstore
            .search(query_vec, 5, Some(&["public".to_string()]), false)
            .await
            .expect("search failed");

        assert!(
            !results.is_empty(),
            "expected at least one result from the indexed document"
        );
        assert!(
            results.iter().any(|r| r.document_slug == slug),
            "expected slug '{slug}' in results, got: {:?}",
            results.iter().map(|r| &r.document_slug).collect::<Vec<_>>()
        );
    }

    async fn start_qdrant() -> (testcontainers::ContainerAsync<GenericImage>, String) {
        let qdrant = GenericImage::new("qdrant/qdrant", "v1.19.1")
            .with_exposed_port(ContainerPort::Tcp(6334))
            .with_wait_for(WaitFor::message_on_stdout("Qdrant gRPC listening"))
            .start()
            .await
            .expect("failed to start Qdrant container");
        let port = qdrant
            .get_host_port_ipv4(6334)
            .await
            .expect("failed to get Qdrant gRPC port");
        (qdrant, format!("http://localhost:{port}"))
    }

    #[tokio::test]
    async fn sweep_orphans_removes_points_mongo_no_longer_knows() {
        use lekton::rag::service::AttachmentPage;
        use qdrant_client::qdrant::{PointStruct, UpsertPointsBuilder};
        use std::collections::HashSet;

        let (_qdrant, url) = start_qdrant().await;
        let vectorstore = Arc::new(QdrantVectorStore::new(&url, COLLECTION).unwrap());
        vectorstore.ensure_collection(DIMENSIONS).await.unwrap();
        let svc = DefaultRagService::new(
            Arc::new(DeterministicEmbedding),
            vectorstore.clone(),
            256,
            32,
        );

        for slug in ["docs/live", "docs/renamed-away"] {
            svc.index_document(
                slug,
                slug,
                "Some body text.",
                "internal",
                false,
                &[],
                None,
                None,
            )
            .await
            .unwrap();
        }
        let page = [AttachmentPage {
            page_number: Some(1),
            text: "Datasheet text.".to_string(),
        }];
        for key in ["live.pdf", "gone.pdf"] {
            svc.index_attachment(key, key, &page, &["internal".to_string()], &[])
                .await
                .unwrap();
        }
        // A point written before `source_kind` existed.
        let client = qdrant_client::Qdrant::from_url(&url).build().unwrap();
        let mut payload = qdrant_client::Payload::new();
        payload.insert("document_slug", "docs/legacy-gone");
        payload.insert("chunk_text", "old format");
        client
            .upsert_points(
                UpsertPointsBuilder::new(
                    COLLECTION,
                    vec![PointStruct::new(
                        42u64,
                        vec![0.5f32; DIMENSIONS as usize],
                        payload,
                    )],
                )
                .wait(true),
            )
            .await
            .unwrap();

        let before = vectorstore.list_point_sources().await.unwrap();
        assert_eq!(before.len(), 5);

        let live_slugs: HashSet<String> = ["docs/live".to_string()].into();
        let live_keys: HashSet<String> = ["live.pdf".to_string()].into();
        let deleted = svc.sweep_orphans(&live_slugs, &live_keys).await.unwrap();
        assert_eq!(deleted, 3);

        let mut remaining: Vec<String> = vectorstore
            .list_point_sources()
            .await
            .unwrap()
            .into_iter()
            .map(|p| match p.source_kind.as_deref() {
                Some("attachment") => p.attachment_key.unwrap(),
                _ => p.document_slug.unwrap_or_default(),
            })
            .collect();
        remaining.sort();
        assert_eq!(remaining, vec!["docs/live", "live.pdf"]);
    }
}
