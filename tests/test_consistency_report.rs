mod common;

use lekton::jobs::consistency_report::{build_report, Sources};

#[tokio::test]
async fn reports_storage_missing_and_orphan_objects_without_mutation() {
    let env = common::TestEnv::start().await;
    let server = env.server();
    env.ingest(&server, "report/doc", "Report", "# Content", "public")
        .await;
    let doc = env.repo.find_by_slug("report/doc").await.unwrap().unwrap();
    env.storage.delete_object(&doc.s3_key).await.unwrap();
    env.storage
        .put_object("unowned.txt", b"orphan".to_vec())
        .await
        .unwrap();
    let report = build_report(Sources {
        references: env.app_state.storage_reference_repo.as_ref(),
        documents: env.repo.as_ref(),
        assets: env.app_state.asset_repo.as_ref(),
        storage: env.storage.as_ref(),
        search: None,
        attachment_search: None,
        rag: None,
    })
    .await
    .unwrap();
    let count = |id: &str| report.findings.iter().find(|f| f.id == id).unwrap().count;
    assert_eq!(count("s3_orphans"), 1);
    assert_eq!(count("s3_missing"), 1);
    assert_eq!(report.skipped.len(), 3);
    assert_eq!(
        env.storage
            .get_object("unowned.txt")
            .await
            .unwrap()
            .unwrap(),
        b"orphan"
    );
    assert!(env.repo.find_by_slug("report/doc").await.unwrap().is_some());
}

#[tokio::test]
async fn reports_missing_and_stale_search_entries_without_repairing_them() {
    let env = common::TestEnv::start().await;
    let server = env.server();
    env.ingest(&server, "report/live", "Report", "# Content", "public")
        .await;
    let doc = env.repo.find_by_slug("report/live").await.unwrap().unwrap();
    env.search.delete_document(&doc.slug).await.unwrap();
    let mut stale = lekton::search::client::build_search_document(&doc, "# Content");
    stale.slug = "report/deleted".into();
    stale.id = lekton::search::client::slug_to_id(&stale.slug);
    env.search.index_document(&stale).await.unwrap();
    // Search mutations are asynchronous; the listing must see their settled state.
    for _ in 0..100 {
        let slugs = env.search.list_indexed_slugs().await.unwrap();
        if slugs == vec![stale.slug.clone()] {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let before = env.search.list_indexed_slugs().await.unwrap();
    assert_eq!(before, vec![stale.slug.clone()]);
    let report = build_report(Sources {
        references: env.app_state.storage_reference_repo.as_ref(),
        documents: env.repo.as_ref(),
        assets: env.app_state.asset_repo.as_ref(),
        storage: env.storage.as_ref(),
        search: Some(env.search.as_ref()),
        attachment_search: None,
        rag: None,
    })
    .await
    .unwrap();
    let finding = |id: &str| report.findings.iter().find(|f| f.id == id).unwrap();
    assert_eq!(finding("search_orphans").samples, vec![stale.slug]);
    assert_eq!(finding("search_missing").samples, vec![doc.slug]);
    assert_eq!(env.search.list_indexed_slugs().await.unwrap(), before);
}
