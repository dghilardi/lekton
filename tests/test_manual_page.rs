mod common;

use lekton::editor::component::{load_editor_page, save_editor_page, EditorPageInput};

fn input(
    slug: &str,
    title: &str,
    html: &str,
    access_level: &str,
    parent: Option<&str>,
    order: u32,
) -> EditorPageInput {
    EditorPageInput {
        slug: slug.to_string(),
        title: title.to_string(),
        html_content: html.to_string(),
        access_level: access_level.to_string(),
        parent_slug: parent.map(str::to_string),
        order,
    }
}

/// Full hand-authored-page flow exercised against real MongoDB + MinIO:
/// create a page, load it for editing, then edit its body and metadata.
#[tokio::test]
async fn create_then_edit_hand_authored_page() {
    let env = common::TestEnv::start().await;
    let slug = format!("guides/manual-{}", uuid::Uuid::new_v4());

    // Nothing exists yet -> creation mode.
    let before = load_editor_page(env.repo.as_ref(), env.storage.as_ref(), &slug)
        .await
        .unwrap();
    assert!(before.is_none(), "no page should exist before creation");

    // Create.
    save_editor_page(
        env.repo.as_ref(),
        env.asset_repo.as_ref(),
        None,
        None,
        None,
        env.storage.as_ref(),
        input(
            &slug,
            "Manual Page",
            "<h1>Manual Page</h1><p>First draft.</p>",
            "public",
            Some("guides"),
            2,
        ),
    )
    .await
    .unwrap();

    // It is a hand-authored (web-editor) page with the given metadata.
    let doc = env.repo.find_by_slug(&slug).await.unwrap().unwrap();
    assert_eq!(doc.service_owner, "web-editor");
    assert_eq!(doc.source_id, None);
    assert_eq!(doc.access_level, "public");
    assert_eq!(doc.parent_slug.as_deref(), Some("guides"));
    assert_eq!(doc.order, 2);

    // The editor can load it back for editing.
    let loaded = load_editor_page(env.repo.as_ref(), env.storage.as_ref(), &slug)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.title, "Manual Page");
    assert_eq!(loaded.access_level, "public");
    assert!(loaded.html.contains("First draft."));

    // Edit: change the body and the metadata the form owns.
    save_editor_page(
        env.repo.as_ref(),
        env.asset_repo.as_ref(),
        None,
        None,
        None,
        env.storage.as_ref(),
        input(
            &slug,
            "Manual Page v2",
            "<h1>Manual Page</h1><p>Revised.</p>",
            "internal",
            None,
            5,
        ),
    )
    .await
    .unwrap();

    let edited = load_editor_page(env.repo.as_ref(), env.storage.as_ref(), &slug)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edited.title, "Manual Page v2");
    assert_eq!(edited.access_level, "internal");
    assert_eq!(edited.parent_slug, None);
    assert_eq!(edited.order, 5);
    assert!(edited.html.contains("Revised."));
    assert!(
        !edited.html.contains("First draft."),
        "the body should be overwritten on edit"
    );
}

/// Pages managed by an external source (ingest / lekton-sync) carry a
/// `source_id` and must stay read-only in the editor: both loading and saving
/// over them must fail.
#[tokio::test]
async fn editor_refuses_externally_managed_pages() {
    let env = common::TestEnv::start().await;
    let server = env.server();
    let slug = format!("synced-{}", uuid::Uuid::new_v4());

    server
        .post("/api/v1/ingest")
        .json(&serde_json::json!({
            "service_token": "test-token",
            "slug": slug,
            "source_path": format!("docs/{}.md", slug),
            "source_id": "test-source",
            "title": "Synced",
            "content": "# Synced\n\nManaged externally.",
            "access_level": "public",
            "service_owner": "test-team",
            "order": 0,
            "is_hidden": false
        }))
        .await;

    let load = load_editor_page(env.repo.as_ref(), env.storage.as_ref(), &slug).await;
    assert!(load.is_err(), "loading a managed page must error");

    let save = save_editor_page(
        env.repo.as_ref(),
        env.asset_repo.as_ref(),
        None,
        None,
        None,
        env.storage.as_ref(),
        input(&slug, "Hijack", "<p>nope</p>", "public", None, 0),
    )
    .await;
    assert!(save.is_err(), "saving over a managed page must error");
}
