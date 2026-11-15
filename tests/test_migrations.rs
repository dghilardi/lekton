mod common;

use mongodb::bson::doc;

/// Minimal legacy `documents` row: no `release`, no `is_latest`, as written by
/// any Lekton build predating release versioning.
fn legacy_doc(slug: &str) -> mongodb::bson::Document {
    doc! {
        "slug": slug,
        "title": "Legacy",
        "s3_key": format!("docs/{slug}.md"),
        "access_level": "public",
        "service_owner": "test-team",
        "last_updated": mongodb::bson::DateTime::now(),
        "tags": [],
        "links_out": [],
        "backlinks": [],
        "source_id": "legacy-source",
    }
}

/// Runs the full migration plan the way `main.rs` does at startup.
async fn run_plan(db: &mongodb::Database) {
    lekton::db::migrations::build_plan()
        .run(db.clone())
        .await
        .expect("migration plan failed");
}

#[tokio::test]
async fn migration_014_backfills_is_latest_on_legacy_documents() {
    let env = common::TestEnv::start().await;
    let documents = env.db.collection::<mongodb::bson::Document>("documents");

    // A legacy row must exist *before* the plan runs for the backfill to be
    // exercised at all.
    documents
        .insert_one(legacy_doc("legacy/intro"))
        .await
        .expect("insert legacy doc");

    run_plan(&env.db).await;

    let stored = documents
        .find_one(doc! { "slug": "legacy/intro" })
        .await
        .expect("query")
        .expect("legacy doc still present");

    assert_eq!(
        stored.get_bool("is_latest"),
        Ok(true),
        "legacy documents must be backfilled as latest so they keep resolving"
    );
    assert!(
        stored.get("release").is_none()
            || stored.get("release") == Some(&mongodb::bson::Bson::Null),
        "the backfill must not invent a release: the source is not release-managed"
    );
}

#[tokio::test]
async fn migration_014_replaces_the_unique_slug_index() {
    let env = common::TestEnv::start().await;
    let documents = env.db.collection::<mongodb::bson::Document>("documents");

    run_plan(&env.db).await;

    let names = documents.list_index_names().await.expect("list indexes");

    assert!(
        !names.iter().any(|n| n == "slug_1"),
        "the unique slug index from migration 008 must be gone, found: {names:?}"
    );
    for expected in [
        "slug_1_release_1",
        "source_id_1_release_1",
        "is_latest_1_source_id_1",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing index {expected}, found: {names:?}"
        );
    }
}

#[tokio::test]
async fn same_slug_is_allowed_across_releases_but_not_within_one() {
    let env = common::TestEnv::start().await;
    let documents = env.db.collection::<mongodb::bson::Document>("documents");

    run_plan(&env.db).await;

    let mut first = legacy_doc("api/authorization");
    first.insert("release", "1.0.0");
    documents
        .insert_one(first)
        .await
        .expect("first release of the slug");

    // The whole point of the migration: the same slug in a different release.
    let mut other_release = legacy_doc("api/authorization");
    other_release.insert("release", "1.2.0");
    documents
        .insert_one(other_release)
        .await
        .expect("the same slug must be insertable in another release");

    // ...while a collision inside one release is still rejected.
    let mut duplicate = legacy_doc("api/authorization");
    duplicate.insert("release", "1.2.0");
    let err = documents
        .insert_one(duplicate)
        .await
        .expect_err("a duplicate (slug, release) must be rejected");

    assert!(
        err.to_string().contains("E11000") || err.to_string().contains("duplicate key"),
        "expected a duplicate-key error, got: {err}"
    );
}

#[tokio::test]
async fn migration_014_creates_release_registry_indexes() {
    let env = common::TestEnv::start().await;

    run_plan(&env.db).await;

    let releases = env
        .db
        .collection::<mongodb::bson::Document>("source_releases");
    let aliases = env
        .db
        .collection::<mongodb::bson::Document>("source_release_aliases");

    assert!(
        releases
            .list_index_names()
            .await
            .expect("list source_releases indexes")
            .iter()
            .any(|n| n == "source_id_1_release_1"),
        "source_releases needs a unique (source_id, release) index"
    );
    assert!(
        aliases
            .list_index_names()
            .await
            .expect("list source_release_aliases indexes")
            .iter()
            .any(|n| n == "source_id_1"),
        "source_release_aliases needs a unique source_id index (one alias per source)"
    );

    // One alias per source, enforced by the index rather than by convention.
    aliases
        .insert_one(doc! { "source_id": "svc", "latest_release": "1.2.0" })
        .await
        .expect("first alias");
    let err = aliases
        .insert_one(doc! { "source_id": "svc", "latest_release": "1.3.0" })
        .await
        .expect_err("a second alias for the same source must be rejected");
    assert!(
        err.to_string().contains("E11000") || err.to_string().contains("duplicate key"),
        "expected a duplicate-key error, got: {err}"
    );
}

/// Duplicate slugs must fail the plan with a message naming them, not with an
/// opaque E11000 from an index build.
///
/// Note on which pre-flight fires: 008's condition (unique `slug`) is stricter
/// than 014's (unique `slug` + `release`), so on a fresh database 008 always
/// catches duplicates first and 014's own pre-flight is unreachable here. 014
/// keeps it as insurance for a database whose `slug_1` index was dropped by
/// hand — cheap, and the alternative is an unreadable index-build failure.
#[tokio::test]
async fn plan_refuses_to_run_over_duplicate_slugs() {
    let env = common::TestEnv::start().await;
    let documents = env.db.collection::<mongodb::bson::Document>("documents");

    documents
        .insert_many(vec![legacy_doc("dup/page"), legacy_doc("dup/page")])
        .await
        .expect("insert duplicates");

    let err = lekton::db::migrations::build_plan()
        .run(env.db.clone())
        .await
        .expect_err("the plan must refuse to build a unique index over duplicates");

    assert!(
        err.to_string().contains("008_add_documents_indexes"),
        "the returned error must name the migration that failed, got: {err}"
    );

    // The framework deliberately returns only the change id and persists the
    // detail in the changelog — which is what main.rs's startup message points
    // the operator at. Assert the detail actually lands there, otherwise a
    // duplicate is undiagnosable in production.
    let entry = env
        .db
        .collection::<mongodb::bson::Document>("__migrations")
        .find_one(doc! { "change_id": "008_add_documents_indexes" })
        .await
        .expect("query changelog")
        .expect("a failed migration must be recorded");

    let recorded = format!("{entry:?}");
    assert!(
        recorded.contains("dup/page"),
        "the changelog entry must name the offending slug so an operator can fix it, got: {recorded}"
    );
}

/// The rename must carry existing history over, field and all.
#[tokio::test]
async fn migration_015_renames_the_history_collection_and_its_field() {
    let env = common::TestEnv::start().await;

    // Legacy history, as written before the rename. Several revisions of the
    // *same* slug on purpose: with one row the field rename can never hit the
    // unique index it has to get out of the way of first.
    let legacy = env
        .db
        .collection::<mongodb::bson::Document>("document_versions");
    for revision in 1i64..=3 {
        legacy
            .insert_one(doc! {
                "id": format!("rev-{revision}"),
                "slug": "guides/intro",
                "version": revision,
                "content_hash": format!("sha256:{revision}"),
                "s3_key": format!("docs/history/guides_intro/{revision}.md"),
                "updated_by": "legacy",
                "created_at": mongodb::bson::DateTime::now(),
            })
            .await
            .expect("insert legacy revision");
    }
    // The index that made the naive rename order fail.
    legacy
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "slug": 1, "version": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .expect("recreate the pre-rename unique index");

    run_plan(&env.db).await;

    let names = env
        .db
        .list_collection_names()
        .await
        .expect("list collections");
    assert!(
        !names.iter().any(|n| n == "document_versions"),
        "the ambiguous name must be gone, found: {names:?}"
    );

    let revisions = env
        .db
        .collection::<mongodb::bson::Document>("document_revisions");
    assert_eq!(
        revisions.count_documents(doc! {}).await.expect("count"),
        3,
        "every revision must survive the rename"
    );
    assert_eq!(
        revisions
            .count_documents(doc! { "version": { "$exists": true } })
            .await
            .expect("count"),
        0,
        "no row may keep the old field: a half-renamed collection is the failure \
         this ordering exists to prevent"
    );
    let moved = revisions
        .find_one(doc! { "id": "rev-3" })
        .await
        .expect("query")
        .expect("history must survive the rename");
    assert_eq!(moved.get_i64("revision"), Ok(3), "got: {moved:?}");

    let indexes = env
        .db
        .collection::<mongodb::bson::Document>("document_revisions")
        .list_index_names()
        .await
        .expect("list indexes");
    assert!(
        indexes.iter().any(|n| n == "slug_1_revision_1"),
        "the unique index must follow the field rename, found: {indexes:?}"
    );
    assert!(
        !indexes.iter().any(|n| n.contains("version")),
        "no index may be left pointing at a field that no longer exists: {indexes:?}"
    );
}

/// A database that never wrote history has no collection to rename; the plan must
/// still complete and leave usable indexes behind.
#[tokio::test]
async fn migration_015_is_safe_with_no_history_at_all() {
    let env = common::TestEnv::start().await;

    run_plan(&env.db).await;

    let indexes = env
        .db
        .collection::<mongodb::bson::Document>("document_revisions")
        .list_index_names()
        .await
        .expect("list indexes");
    assert!(
        indexes.iter().any(|n| n == "slug_1_revision_1"),
        "found: {indexes:?}"
    );
}

#[tokio::test]
async fn migration_021_replaces_raw_service_tokens_in_asset_uploaders() {
    let env = common::TestEnv::start().await;
    let assets = env.db.collection::<mongodb::bson::Document>("assets");
    let tokens = env
        .db
        .collection::<mongodb::bson::Document>("service_tokens");

    tokens
        .insert_one(doc! {
            "id": "st-1",
            "name": "ci-docs",
            "token_hash": lekton::auth::token_service::TokenService::hash_token("raw-known-secret"),
        })
        .await
        .unwrap();
    for (key, uploader) in [
        ("attachments/a/known.pdf", "raw-known-secret"),
        ("attachments/a/legacy.pdf", "abcd"),
        // Legacy env tokens are arbitrary strings and may look like an email.
        ("attachments/a/legacy-at.pdf", "ci@secret"),
        // Already rewritten by an earlier, interrupted run: keep the name.
        ("attachments/a/done.pdf", "service-token:ci-docs"),
        // Editor and admin uploads record the user's email under editor/.
        ("editor/1_a.png", "editor@example.com"),
    ] {
        assets
            .insert_one(doc! { "key": key, "uploaded_by": uploader })
            .await
            .unwrap();
    }

    run_plan(&env.db).await;

    let uploader = |key: &'static str| {
        let assets = assets.clone();
        async move {
            assets
                .find_one(doc! { "key": key })
                .await
                .unwrap()
                .unwrap()
                .get_str("uploaded_by")
                .unwrap()
                .to_string()
        }
    };
    assert_eq!(
        uploader("attachments/a/known.pdf").await,
        "service-token:ci-docs"
    );
    // The env-configured legacy token has no stored hash to match.
    assert_eq!(
        uploader("attachments/a/legacy.pdf").await,
        "service-token:unknown"
    );
    assert_eq!(
        uploader("attachments/a/legacy-at.pdf").await,
        "service-token:unknown"
    );
    assert_eq!(
        uploader("attachments/a/done.pdf").await,
        "service-token:ci-docs"
    );
    assert_eq!(uploader("editor/1_a.png").await, "editor@example.com");
}

async fn index_names(db: &mongodb::Database, collection: &str) -> Vec<String> {
    db.collection::<mongodb::bson::Document>(collection)
        .list_index_names()
        .await
        .unwrap_or_else(|e| panic!("list {collection} indexes: {e}"))
}

/// The query planner's winning plan for `filter` on `collection`, as text.
async fn winning_plan(
    db: &mongodb::Database,
    collection: &str,
    filter: mongodb::bson::Document,
) -> String {
    let explain = db
        .run_command(doc! {
            "explain": { "find": collection, "filter": filter },
            "verbosity": "queryPlanner",
        })
        .await
        .expect("explain");
    format!(
        "{:?}",
        explain
            .get_document("queryPlanner")
            .and_then(|plan| plan.get_document("winningPlan"))
            .expect("winning plan")
    )
}

#[tokio::test]
async fn migration_022_indexes_assets_refresh_tokens_and_learn_collections() {
    let env = common::TestEnv::start().await;

    run_plan(&env.db).await;

    for (collection, expected) in [
        (
            "assets",
            &["key_1", "referenced_by_1", "extraction_status_1"][..],
        ),
        ("refresh_tokens", &["id_1", "user_id_1", "family_id_1"][..]),
        ("learn_paths", &["id_1", "user_id_1_updated_at_-1"][..]),
        (
            "learn_lessons",
            &["id_1", "path_id_1_seq_1", "user_id_1"][..],
        ),
        (
            "learn_records",
            &["path_id_1_created_at_-1", "user_id_1"][..],
        ),
        ("learn_preferences", &["user_id_1"][..]),
    ] {
        let names = index_names(&env.db, collection).await;
        for name in expected {
            assert!(
                names.iter().any(|n| n == name),
                "{collection} needs index {name}, has {names:?}"
            );
        }
    }

    // Asset references are matched as whole `{slug, release}` subdocuments;
    // the multikey index must actually serve that equality.
    let assets = env.db.collection::<mongodb::bson::Document>("assets");
    for i in 0..20 {
        assets
            .insert_one(doc! {
                "key": format!("attachments/a/{i}.pdf"),
                "referenced_by": [{ "slug": format!("docs/{i}"), "release": mongodb::bson::Bson::Null }],
            })
            .await
            .unwrap();
    }
    let plan = winning_plan(
        &env.db,
        "assets",
        doc! { "referenced_by": { "slug": "docs/3", "release": mongodb::bson::Bson::Null } },
    )
    .await;
    assert!(
        plan.contains("referenced_by_1") && plan.contains("IXSCAN"),
        "reference lookup must use the referenced_by index, plan: {plan}"
    );

    let err = assets
        .insert_one(doc! { "key": "attachments/a/3.pdf" })
        .await
        .expect_err("a second asset with the same key must be rejected");
    assert!(
        err.to_string().contains("E11000"),
        "expected a duplicate-key error, got: {err}"
    );

    let preferences = env
        .db
        .collection::<mongodb::bson::Document>("learn_preferences");
    preferences
        .insert_one(doc! { "user_id": "u1", "persist": true })
        .await
        .unwrap();
    preferences
        .insert_one(doc! { "user_id": "u1", "persist": false })
        .await
        .expect_err("one learn preference per user");
}

/// Duplicate asset keys must stop 022 with a message naming the key, like the
/// other unique-index migrations.
#[tokio::test]
async fn migration_022_refuses_duplicate_asset_keys() {
    let env = common::TestEnv::start().await;
    let assets = env.db.collection::<mongodb::bson::Document>("assets");
    assets
        .insert_many(vec![
            doc! { "key": "attachments/dup/file.pdf" },
            doc! { "key": "attachments/dup/file.pdf" },
        ])
        .await
        .unwrap();

    let err = lekton::db::migrations::build_plan()
        .run(env.db.clone())
        .await
        .expect_err("the plan must refuse to build a unique key index over duplicates");
    assert!(
        err.to_string()
            .contains("022_add_assets_refresh_tokens_learn_indexes"),
        "the error must name the migration, got: {err}"
    );

    let entry = env
        .db
        .collection::<mongodb::bson::Document>("__migrations")
        .find_one(doc! { "change_id": "022_add_assets_refresh_tokens_learn_indexes" })
        .await
        .unwrap()
        .expect("a failed migration must be recorded");
    assert!(
        format!("{entry:?}").contains("attachments/dup/file.pdf"),
        "the changelog must name the duplicate key, got: {entry:?}"
    );
}
