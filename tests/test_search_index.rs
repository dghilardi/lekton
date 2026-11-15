/// Integration tests for the Meilisearch index primitives (listing, task
/// outcomes) against a real Meilisearch container. Only Meilisearch is started.
#[cfg(feature = "ssr")]
mod search_index {
    use testcontainers::runners::AsyncRunner;
    use testcontainers::ContainerAsync;
    use testcontainers::ImageExt;
    use testcontainers_modules::meilisearch::Meilisearch;

    use lekton::rag::service::AttachmentPage;
    use lekton::search::attachment_search::{
        AttachmentSearchService, MeilisearchAttachmentService,
    };
    use lekton::search::client::{slug_to_id, MeilisearchService, SearchDocument, SearchService};

    async fn start_meilisearch() -> (ContainerAsync<Meilisearch>, String) {
        // Same Meilisearch version as docker-compose.yml and the e2e workflow.
        let container = Meilisearch::default()
            .with_tag("v1.54.3")
            .start()
            .await
            .expect("failed to start Meilisearch container");
        let port = container
            .get_host_port_ipv4(7700)
            .await
            .expect("failed to get Meilisearch port");
        (container, format!("http://127.0.0.1:{port}"))
    }

    fn search_doc(slug: &str) -> SearchDocument {
        SearchDocument {
            id: slug_to_id(slug),
            slug: slug.to_string(),
            title: slug.to_string(),
            access_level: "public".to_string(),
            is_draft: false,
            service_owner: "team".to_string(),
            tags: vec![],
            content_preview: "preview".to_string(),
            content: "preview".to_string(),
            last_updated: 0,
            source_id: None,
            release: None,
        }
    }

    /// Meilisearch applies writes asynchronously; poll until `check` holds.
    async fn eventually<F, Fut>(mut check: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..100 {
            if check().await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("condition not reached in time");
    }

    #[tokio::test]
    async fn lists_every_indexed_slug_and_attachment_key() {
        let (_meili, url) = start_meilisearch().await;
        let search = MeilisearchService::new(&url, None::<String>).unwrap();
        let attachments = MeilisearchAttachmentService::new(&url, None::<String>).unwrap();

        // Nothing written yet: the indexes do not exist, which lists as empty.
        assert!(search.list_indexed_slugs().await.unwrap().is_empty());
        assert!(attachments
            .list_indexed_attachment_keys()
            .await
            .unwrap()
            .is_empty());

        search.configure_index().await.unwrap();
        attachments.configure_index().await.unwrap();
        for slug in ["docs/a", "docs/b/c"] {
            search.index_document(&search_doc(slug)).await.unwrap();
        }
        let pages = [
            AttachmentPage {
                page_number: Some(1),
                text: "page one".to_string(),
            },
            AttachmentPage {
                page_number: Some(2),
                text: "page two".to_string(),
            },
        ];
        attachments
            .index_pages("x.pdf", "x.pdf", "docs/a", "A", &pages, &[])
            .await
            .unwrap();

        eventually(|| async {
            let mut slugs = search.list_indexed_slugs().await.unwrap();
            slugs.sort();
            let keys = attachments.list_indexed_attachment_keys().await.unwrap();
            slugs == ["docs/a", "docs/b/c"] && keys == ["x.pdf"]
        })
        .await;
    }

    #[tokio::test]
    async fn unlinked_attachment_pages_surface_once_a_document_links_them() {
        let (_meili, url) = start_meilisearch().await;
        let attachments = MeilisearchAttachmentService::new(&url, None::<String>).unwrap();
        attachments.configure_index().await.unwrap();

        // Extracted before any document referenced the file.
        let pages = [AttachmentPage {
            page_number: Some(1),
            text: "torque wrench calibration".to_string(),
        }];
        attachments
            .index_pages("spec.pdf", "spec.pdf", "", "", &pages, &[])
            .await
            .unwrap();
        eventually(|| async {
            attachments.list_indexed_attachment_keys().await.unwrap() == ["spec.pdf"]
        })
        .await;
        assert!(
            attachments.search("torque", None).await.unwrap().is_empty(),
            "an unlinked page has nowhere to point a hit at"
        );

        attachments
            .update_reference("spec.pdf", "docs/tools", "Tools", &["public".to_string()])
            .await
            .unwrap();
        eventually(|| async {
            let hits = attachments
                .search("torque", Some(&["public".to_string()]))
                .await
                .unwrap();
            hits.len() == 1
                && hits[0].document_slug == "docs/tools"
                && hits[0].document_title == "Tools"
                && hits[0].filename == "spec.pdf"
        })
        .await;
    }

    #[tokio::test]
    async fn attachment_search_finds_page_text_past_the_preview_with_a_snippet() {
        let (_meili, url) = start_meilisearch().await;
        let attachments = MeilisearchAttachmentService::new(&url, None::<String>).unwrap();
        attachments.configure_index().await.unwrap();

        let text = format!(
            "{} The torquecode E4711 means the sensor is loose. {} pagetailmarker",
            "Opening filler about the device housing. ".repeat(10),
            "Closing filler about warranty terms. ".repeat(40),
        );
        assert!(text.find("E4711").unwrap() > 300);
        let pages = [AttachmentPage {
            page_number: Some(7),
            text,
        }];
        let public = ["public".to_string()];
        attachments
            .index_pages(
                "manual.pdf",
                "manual.pdf",
                "docs/m",
                "Manual",
                &pages,
                &public,
            )
            .await
            .unwrap();

        let hits = attachments.search("E4711", Some(&public)).await.unwrap();
        assert_eq!(hits.len(), 1, "a term past the preview is searchable");
        let hit = &hits[0];
        assert_eq!(hit.page, Some(7));
        // A snippet around the match, never the whole page.
        assert!(hit.content_preview.contains("E4711"), "{hit:?}");
        assert!(!hit.content_preview.contains("pagetailmarker"), "{hit:?}");
        assert!(
            hit.content_preview.split_whitespace().count() <= 30,
            "{hit:?}"
        );
        let [start, end] = hit.preview_matches[0];
        let matched: String = hit
            .content_preview
            .chars()
            .skip(start)
            .take(end - start)
            .collect();
        assert_eq!(matched, "E4711");

        // Hidden from a reader without the level, as before.
        assert!(attachments
            .search("E4711", Some(&["internal".to_string()]))
            .await
            .unwrap()
            .is_empty());
    }

    /// Pre-create `index` with a primary key the services do not use, so every
    /// write they send (which names `id` as the primary key) is rejected.
    async fn break_index(url: &str, index: &str) {
        let client = meilisearch_sdk::client::Client::new(url, None::<String>).unwrap();
        let task = client
            .create_index(index, Some("other"))
            .await
            .unwrap()
            .wait_for_completion(&client, None, None)
            .await
            .unwrap();
        assert!(task.is_success(), "index must not exist yet: {task:?}");
    }

    #[tokio::test]
    async fn a_document_write_meilisearch_rejects_is_reported() {
        let (_meili, url) = start_meilisearch().await;
        let search = MeilisearchService::new(&url, None::<String>).unwrap();

        // An id Meilisearch refuses.
        let mut invalid = search_doc("docs/a");
        invalid.id = "not a valid id!".to_string();
        assert!(
            search.index_document(&invalid).await.is_err(),
            "a failed indexing task must not report success"
        );

        // The same write with a valid id goes through.
        let valid = search_doc("docs/a");
        search.index_document(&valid).await.unwrap();
        assert_eq!(search.list_indexed_slugs().await.unwrap(), ["docs/a"]);
    }

    #[tokio::test]
    async fn an_attachment_write_meilisearch_rejects_is_reported() {
        let (_meili, url) = start_meilisearch().await;
        let attachments = MeilisearchAttachmentService::new(&url, None::<String>).unwrap();
        break_index(&url, "attachment_chunks").await;

        let pages = [AttachmentPage {
            page_number: Some(1),
            text: "text".to_string(),
        }];
        assert!(attachments
            .index_pages("x.pdf", "x.pdf", "docs/a", "A", &pages, &[])
            .await
            .is_err());
    }
}
