mod common;

use lekton::auth::models::AccessLevel;
use lekton::db::repository::DocumentRepository;
use lekton::storage::client::StorageClient;

#[tokio::test]
async fn ingest_then_search_then_retrieve() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let slug = format!("lifecycle-full-{}", uuid::Uuid::new_v4());
    let keyword = format!("lifecyclekw{}", uuid::Uuid::new_v4().simple());
    let content = format!("# {keyword} Guide\n\nDetailed instructions for the {keyword} process.");

    // 1. Ingest
    env.ingest(&server, &slug, &format!("{keyword} Guide"), &content, "developer")
        .await;

    env.wait_for_search_indexing().await;

    // 2. Search finds it
    let results = env
        .search
        .search(&keyword, AccessLevel::Developer)
        .await
        .unwrap();
    assert!(
        results.iter().any(|r| r.slug == slug),
        "Should find the document via search"
    );

    // 3. Content retrievable from S3
    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    let stored = env.storage.get_object(&doc.s3_key).await.unwrap().unwrap();
    assert_eq!(String::from_utf8(stored).unwrap(), content);
}

#[tokio::test]
async fn document_hierarchy_navigation() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let prefix = format!("hierarchy-{}", uuid::Uuid::new_v4().simple());
    let parent = format!("{prefix}-parent");
    let child_a = format!("{prefix}-child-a");
    let child_b = format!("{prefix}-child-b");

    // Create parent
    env.ingest(&server, &parent, "Parent Doc", "# Parent", "public")
        .await;

    // Create children with parent_slug and ordering
    server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": child_b,
            "title": "Child B",
            "content": "# Child B",
            "access_level": "public",
            "service_owner": "test-team",
            "tags": ["test"],
            "parent_slug": parent,
            "order": 2,
            "is_hidden": false
        }))
        .await;

    server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": child_a,
            "title": "Child A",
            "content": "# Child A",
            "access_level": "public",
            "service_owner": "test-team",
            "tags": ["test"],
            "parent_slug": parent,
            "order": 1,
            "is_hidden": false
        }))
        .await;

    // list_accessible should return them sorted by order
    let docs = env.repo.list_accessible(AccessLevel::Admin).await.unwrap();
    let our_docs: Vec<_> = docs
        .iter()
        .filter(|d| d.slug.starts_with(&prefix))
        .collect();

    assert_eq!(our_docs.len(), 3, "Should have parent + 2 children");

    // Children should have correct parent_slug
    let child_a_doc = our_docs.iter().find(|d| d.slug == child_a).unwrap();
    let child_b_doc = our_docs.iter().find(|d| d.slug == child_b).unwrap();
    assert_eq!(child_a_doc.parent_slug.as_deref(), Some(parent.as_str()));
    assert_eq!(child_b_doc.parent_slug.as_deref(), Some(parent.as_str()));

    // Order: child_a (order=1) should come before child_b (order=2)
    let child_a_pos = our_docs.iter().position(|d| d.slug == child_a).unwrap();
    let child_b_pos = our_docs.iter().position(|d| d.slug == child_b).unwrap();
    assert!(
        child_a_pos < child_b_pos,
        "Child A (order=1) should come before Child B (order=2)"
    );
}

#[tokio::test]
async fn hidden_documents_excluded_from_listing() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let visible_slug = format!("visible-{}", uuid::Uuid::new_v4());
    let hidden_slug = format!("hidden-{}", uuid::Uuid::new_v4());

    env.ingest(&server, &visible_slug, "Visible Doc", "# Visible", "public")
        .await;

    server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": hidden_slug,
            "title": "Hidden Doc",
            "content": "# Hidden",
            "access_level": "public",
            "service_owner": "test-team",
            "tags": ["test"],
            "order": 0,
            "is_hidden": true
        }))
        .await;

    // list_accessible should NOT include the hidden doc
    let docs = env.repo.list_accessible(AccessLevel::Admin).await.unwrap();
    let slugs: Vec<&str> = docs.iter().map(|d| d.slug.as_str()).collect();

    assert!(
        slugs.contains(&visible_slug.as_str()),
        "Visible doc should be in listing"
    );
    assert!(
        !slugs.contains(&hidden_slug.as_str()),
        "Hidden doc should NOT be in listing"
    );

    // But it should still be accessible directly
    let hidden = env.repo.find_by_slug(&hidden_slug).await.unwrap();
    assert!(hidden.is_some(), "Hidden doc should be accessible by slug");
}

#[tokio::test]
async fn backlink_graph_consistency() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let doc_a = format!("backlink-a-{}", uuid::Uuid::new_v4());
    let doc_b = format!("backlink-b-{}", uuid::Uuid::new_v4());
    let doc_c = format!("backlink-c-{}", uuid::Uuid::new_v4());

    // Create target documents B and C
    env.ingest(&server, &doc_b, "Doc B", "# Doc B", "public")
        .await;
    env.ingest(&server, &doc_c, "Doc C", "# Doc C", "public")
        .await;

    // Create A linking to B
    let content_a_v1 = format!("# Doc A\n\nSee [B](/docs/{}).", doc_b);
    env.ingest(&server, &doc_a, "Doc A", &content_a_v1, "public")
        .await;

    // B should have backlink from A
    let b = env.repo.find_by_slug(&doc_b).await.unwrap().unwrap();
    assert!(
        b.backlinks.contains(&doc_a),
        "B should have backlink from A after first ingest"
    );

    // Update A to link to C instead of B
    let content_a_v2 = format!("# Doc A\n\nSee [C](/docs/{}).", doc_c);
    env.ingest(&server, &doc_a, "Doc A Updated", &content_a_v2, "public")
        .await;

    // B should no longer have backlink from A
    let b = env.repo.find_by_slug(&doc_b).await.unwrap().unwrap();
    assert!(
        !b.backlinks.contains(&doc_a),
        "B should NOT have backlink from A after update. Backlinks: {:?}",
        b.backlinks
    );

    // C should now have backlink from A
    let c = env.repo.find_by_slug(&doc_c).await.unwrap().unwrap();
    assert!(
        c.backlinks.contains(&doc_a),
        "C should have backlink from A after update. Backlinks: {:?}",
        c.backlinks
    );
}

#[tokio::test]
async fn access_level_enforcement() {
    let env = common::TestEnv::start().await;
    let server = env.server();

    let public_slug = format!("acl-public-{}", uuid::Uuid::new_v4());
    let dev_slug = format!("acl-dev-{}", uuid::Uuid::new_v4());
    let admin_slug = format!("acl-admin-{}", uuid::Uuid::new_v4());

    env.ingest(&server, &public_slug, "Public Doc", "# Public", "public")
        .await;
    env.ingest(&server, &dev_slug, "Dev Doc", "# Dev", "developer")
        .await;
    env.ingest(&server, &admin_slug, "Admin Doc", "# Admin", "admin")
        .await;

    // Public access: only sees public docs
    let public_docs = env.repo.list_accessible(AccessLevel::Public).await.unwrap();
    let public_slugs: Vec<&str> = public_docs.iter().map(|d| d.slug.as_str()).collect();
    assert!(public_slugs.contains(&public_slug.as_str()));
    assert!(!public_slugs.contains(&dev_slug.as_str()));
    assert!(!public_slugs.contains(&admin_slug.as_str()));

    // Developer access: sees public + developer
    let dev_docs = env.repo.list_accessible(AccessLevel::Developer).await.unwrap();
    let dev_slugs: Vec<&str> = dev_docs.iter().map(|d| d.slug.as_str()).collect();
    assert!(dev_slugs.contains(&public_slug.as_str()));
    assert!(dev_slugs.contains(&dev_slug.as_str()));
    assert!(!dev_slugs.contains(&admin_slug.as_str()));

    // Admin access: sees everything
    let admin_docs = env.repo.list_accessible(AccessLevel::Admin).await.unwrap();
    let admin_slugs: Vec<&str> = admin_docs.iter().map(|d| d.slug.as_str()).collect();
    assert!(admin_slugs.contains(&public_slug.as_str()));
    assert!(admin_slugs.contains(&dev_slug.as_str()));
    assert!(admin_slugs.contains(&admin_slug.as_str()));
}
