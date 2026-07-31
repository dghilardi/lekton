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
