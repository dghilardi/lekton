//! Real Qdrant coverage for the lexical-document passage boundary. No LLM/API credentials.
#![cfg(feature = "ssr")]
use lekton::rag::vectorstore::{
    ChunkPayload, QdrantVectorStore, SourceKind, VectorPoint, VectorStore,
};
use testcontainers::{
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
    GenericImage,
};

fn point_id(id: u64) -> String {
    uuid::Uuid::from_u128(id as u128).to_string()
}

fn point(
    id: u64,
    slug: &str,
    index: u32,
    vector: Vec<f32>,
    level: &str,
    draft: bool,
    kind: SourceKind,
) -> VectorPoint {
    VectorPoint {
        id: point_id(id),
        vector,
        payload: ChunkPayload {
            chunk_text: format!("passage {index}: TP-200 /api/v1/devices ERR_DEVICE_42"),
            document_slug: slug.into(),
            document_title: "TP-200 reference".into(),
            source_kind: kind,
            attachment_key: (kind == SourceKind::Attachment).then(|| "manual.pdf".into()),
            source_page: (kind == SourceKind::Attachment).then_some(2),
            access_levels: vec![level.into()],
            is_draft: draft,
            tags: vec![],
            chunk_index: index,
            section_path: vec!["Troubleshooting".into()],
            section_anchor: "troubleshooting".into(),
            source_id: None,
            release: None,
        },
    }
}

#[tokio::test]
async fn lexical_document_passages_are_relevance_ranked_and_visibility_filtered_in_qdrant() {
    let container = GenericImage::new("qdrant/qdrant", "v1.19.1")
        .with_exposed_port(ContainerPort::Tcp(6334))
        .with_wait_for(WaitFor::message_on_stdout("Qdrant gRPC listening"))
        .start()
        .await
        .expect("start Qdrant");
    let port = container.get_host_port_ipv4(6334).await.unwrap();
    let store =
        QdrantVectorStore::new(&format!("http://127.0.0.1:{port}"), "hybrid-passages").unwrap();
    store.ensure_collection(2).await.unwrap();
    let mut points: Vec<_> = (0..30)
        .map(|i| {
            point(
                i + 10,
                "lexical",
                i as u32,
                vec![-1.0, 0.0],
                "public",
                false,
                SourceKind::Document,
            )
        })
        .collect();
    points.extend([
        point(
            1,
            "global-vector",
            0,
            vec![1.0, 0.0],
            "public",
            false,
            SourceKind::Document,
        ),
        point(
            600,
            "lexical",
            999,
            vec![0.8, 0.6],
            "public",
            false,
            SourceKind::Document,
        ),
        point(
            601,
            "lexical",
            1000,
            vec![0.999, 0.01],
            "private",
            false,
            SourceKind::Document,
        ),
        point(
            602,
            "lexical",
            1001,
            vec![0.995, 0.1],
            "public",
            true,
            SourceKind::Document,
        ),
        point(
            603,
            "lexical",
            1002,
            vec![0.99, 0.1],
            "public",
            false,
            SourceKind::Attachment,
        ),
        point(
            604,
            "forbidden",
            0,
            vec![1.0, 0.0],
            "private",
            false,
            SourceKind::Document,
        ),
    ]);
    store.upsert_chunks(points).await.unwrap();
    let public = vec!["public".into()];
    let global = store
        .search(vec![1.0, 0.0], 1, Some(&public), false)
        .await
        .unwrap();
    assert_eq!(global[0].document_slug, "global-vector");
    let passages = store
        .search_document_chunks(vec![1.0, 0.0], "lexical", 3, Some(&public), false, false)
        .await
        .unwrap();
    assert_eq!(passages.len(), 3);
    assert_eq!(passages[0].point_id, point_id(600));
    assert_eq!(
        passages[0].chunk_index, 999,
        "select relevant late passage, not first three chunks"
    );
    assert!(passages.iter().all(|c| c.document_slug == "lexical"
        && c.source_kind == SourceKind::Document
        && ![point_id(601), point_id(602), point_id(603)].contains(&c.point_id)));
    assert_eq!(passages[0].section_anchor, "troubleshooting");
    assert!(store
        .search_document_chunks(vec![1.0, 0.0], "forbidden", 3, Some(&public), false, true)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .search_document_chunks(vec![1.0, 0.0], "lexical", 3, Some(&[]), true, true)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .search_document_chunks(vec![1.0, 0.0], "lexical\" OR true", 3, None, true, true)
        .await
        .unwrap()
        .is_empty());
    let admin = store
        .search_document_chunks(vec![1.0, 0.0], "lexical", 3, None, true, true)
        .await
        .unwrap();
    for id in [601, 602, 603] {
        assert!(admin.iter().any(|c| c.point_id == point_id(id)));
    }
    let attachment = admin.iter().find(|c| c.point_id == point_id(603)).unwrap();
    assert_eq!(attachment.source_kind, SourceKind::Attachment);
    assert_eq!(attachment.attachment_key.as_deref(), Some("manual.pdf"));
    assert_eq!(attachment.source_page, Some(2));
}
