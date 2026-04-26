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

    const DIMENSIONS: u32 = 32;
    const COLLECTION: &str = "test-rag";

    /// Deterministic mock: encodes each byte of the text into a fixed-length vector,
    /// then L2-normalises it. The same text always returns the same vector, so a
    /// round-trip index + search with identical text yields similarity = 1.0.
    struct DeterministicEmbedding;

    #[async_trait]
    impl EmbeddingService for DeterministicEmbedding {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
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
        let qdrant = GenericImage::new("qdrant/qdrant", "v1.13.6")
            .with_exposed_port(ContainerPort::Tcp(6334))
            .with_wait_for(WaitFor::message_on_stdout("Qdrant HTTP listening"))
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

        svc.index_document(slug, "Test Document", content, "public", false, &[])
            .await
            .expect("index_document failed");

        // Embed the query with the same mock service and search
        let query = content.to_string();
        let query_vec = DeterministicEmbedding
            .embed(&[query])
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
}
