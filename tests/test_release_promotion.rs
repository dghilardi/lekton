mod common;

use lekton::db::models::Document;

fn doc(slug: &str, release: &str, is_latest: bool) -> Document {
    Document {
        slug: slug.to_string(),
        title: slug.to_string(),
        summary: None,
        s3_key: format!("docs/{}.md", slug.replace('/', "_")),
        access_level: "public".to_string(),
        is_draft: false,
        service_owner: "team".to_string(),
        last_updated: chrono::Utc::now(),
        tags: vec![],
        links_out: vec![],
        backlinks: vec![],
        parent_slug: None,
        order: 0,
        is_hidden: false,
        content_hash: None,
        metadata_hash: None,
        is_archived: false,
        source_path: Some(format!("{slug}.md")),
        source_id: Some("assets-manager".to_string()),
        release: Some(release.to_string()),
        is_latest,
        needs_reindex: false,
        skip_rag: false,
    }
}

/// The flag mirrors the alias, so promoting has to move it off the old release
/// and onto the new one in a single sweep.
#[tokio::test]
async fn promotion_moves_the_is_latest_flag_across_releases() {
    let env = common::TestEnv::start().await;

    env.repo
        .create_or_update(doc("api/auth", "1.0.0", true))
        .await
        .unwrap();
    env.repo
        .create_or_update(doc("api/auth", "1.2.0", false))
        .await
        .unwrap();

    let affected = env
        .repo
        .promote_release("assets-manager", "1.2.0")
        .await
        .expect("promote");

    assert_eq!(
        affected.len(),
        2,
        "both the release gaining the flag and the one losing it are affected"
    );

    let copies = env.repo.find_all_by_slug("api/auth").await.unwrap();
    let latest: Vec<&str> = copies
        .iter()
        .filter(|d| d.is_latest)
        .map(|d| d.release.as_deref().unwrap())
        .collect();
    assert_eq!(
        latest,
        vec!["1.2.0"],
        "exactly one release may carry the flag"
    );

    // Only the copy that *gained* the alias is stale: it is the one that must
    // now appear in the index. The demoted copy is out of scope for indexing, so
    // flagging it would leave a false positive nothing can ever clear.
    let gained = copies.iter().find(|d| d.is_latest).expect("one is latest");
    assert!(gained.needs_reindex, "the new latest must be flagged stale");
    assert!(
        copies
            .iter()
            .filter(|d| !d.is_latest)
            .all(|d| !d.needs_reindex),
        "a demoted release must not be left permanently flagged"
    );
}

/// Re-running a promotion must not mark the whole release stale again, otherwise
/// every idempotent sync would trigger a pointless re-index.
#[tokio::test]
async fn re_promoting_the_current_release_is_a_no_op() {
    let env = common::TestEnv::start().await;

    env.repo
        .create_or_update(doc("api/auth", "1.2.0", true))
        .await
        .unwrap();

    let affected = env
        .repo
        .promote_release("assets-manager", "1.2.0")
        .await
        .expect("promote");

    assert!(
        affected.is_empty(),
        "nothing changed, so nothing needs re-indexing: {affected:?}"
    );
    let stored = env
        .repo
        .find_all_by_slug("api/auth")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(stored.is_latest);
    assert!(
        !stored.needs_reindex,
        "an unchanged document must not be flagged stale"
    );
}

#[tokio::test]
async fn promotion_only_touches_its_own_source() {
    let env = common::TestEnv::start().await;

    env.repo
        .create_or_update(doc("api/auth", "1.0.0", true))
        .await
        .unwrap();
    let other = Document {
        source_id: Some("cloud-common".to_string()),
        ..doc("common/amqp", "5.0.0", true)
    };
    env.repo.create_or_update(other).await.unwrap();

    env.repo
        .create_or_update(doc("api/auth", "1.2.0", false))
        .await
        .unwrap();
    env.repo
        .promote_release("assets-manager", "1.2.0")
        .await
        .unwrap();

    let untouched = env
        .repo
        .find_all_by_slug("common/amqp")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        untouched.is_latest && !untouched.needs_reindex,
        "another source's alias must be unaffected"
    );
}

#[tokio::test]
async fn promoting_an_unpublished_release_is_rejected() {
    let env = common::TestEnv::start().await;

    env.release_repo
        .register("assets-manager", "1.0.0")
        .await
        .unwrap();

    let err = lekton::api::releases::process_promote_release(
        env.repo.as_ref(),
        env.release_repo.as_ref(),
        env.service_token_repo.as_ref(),
        Some("test-token"),
        lekton::api::releases::PromoteReleaseRequest {
            service_token: "test-token".to_string(),
            source_id: "assets-manager".to_string(),
            release: "9.9.9".to_string(),
        },
    )
    .await
    .expect_err("a typo must not point latest at nothing");

    assert!(
        err.to_string().contains("not published"),
        "the error must say the release does not exist, got: {err}"
    );
    assert_eq!(
        env.release_repo.latest("assets-manager").await.unwrap(),
        None,
        "the alias must be left alone"
    );
}

#[tokio::test]
async fn promotion_sets_the_alias_and_reports_the_reindex_backlog() {
    let env = common::TestEnv::start().await;

    env.release_repo
        .register("assets-manager", "1.2.0")
        .await
        .unwrap();
    env.repo
        .create_or_update(doc("api/auth", "1.0.0", true))
        .await
        .unwrap();
    env.repo
        .create_or_update(doc("api/auth", "1.2.0", false))
        .await
        .unwrap();

    let (response, _affected) = lekton::api::releases::process_promote_release(
        env.repo.as_ref(),
        env.release_repo.as_ref(),
        env.service_token_repo.as_ref(),
        Some("test-token"),
        lekton::api::releases::PromoteReleaseRequest {
            service_token: "test-token".to_string(),
            source_id: "assets-manager".to_string(),
            release: "1.2.0".to_string(),
        },
    )
    .await
    .expect("promote");

    assert_eq!(response.reindex_pending, 2);
    assert_eq!(
        env.release_repo.latest("assets-manager").await.unwrap(),
        Some("1.2.0".to_string()),
        "the alias must follow the promotion"
    );
}

/// The promotion reindex must repoint the index at the new release and drop the
/// slugs the new release no longer ships.
#[tokio::test]
async fn promotion_reindex_follows_the_new_latest_and_drops_removed_slugs() {
    let env = common::TestEnv::start().await;

    // `kept` exists in both releases; `dropped` only in the old one.
    for (slug, release, latest) in [
        ("api/kept", "1.0.0", true),
        ("api/kept", "1.2.0", false),
        ("api/dropped", "1.0.0", true),
    ] {
        env.repo
            .create_or_update(doc(slug, release, latest))
            .await
            .unwrap();
    }
    // Bodies must exist in storage for the reindex to have something to index.
    for slug in ["api/kept", "api/dropped"] {
        env.storage
            .put_object(
                &format!("docs/{}.md", slug.replace('/', "_")),
                format!("# {slug}\n\nbody").into_bytes(),
            )
            .await
            .unwrap();
    }

    let affected = env
        .repo
        .promote_release("assets-manager", "1.2.0")
        .await
        .unwrap();

    lekton::api::releases::reindex_promoted(
        env.repo.as_ref(),
        env.release_repo.as_ref(),
        "assets-manager",
        env.storage.as_ref(),
        Some(env.search.as_ref()),
        None,
        &affected,
    )
    .await;

    // `kept` is latest under 1.2.0 now, and indexing it succeeded, so the stale
    // flag is cleared.
    let kept_latest = env
        .repo
        .find_all_by_slug("api/kept")
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.is_latest)
        .expect("one copy is latest");
    assert_eq!(kept_latest.release.as_deref(), Some("1.2.0"));
    assert!(
        !kept_latest.needs_reindex,
        "a successfully re-indexed document must not stay flagged stale"
    );

    // `dropped` is latest nowhere: it was removed from the index, and there is no
    // latest row left to clear a flag on.
    let dropped = env.repo.find_all_by_slug("api/dropped").await.unwrap();
    assert!(
        dropped.iter().all(|d| !d.is_latest),
        "a slug the promoted release dropped must not be latest anywhere"
    );
}

/// Regression: a slow reindex must not resurrect a demotion.
///
/// Publishing two releases back to back leaves the first promotion's reindex
/// still running while the second lands. That task holds a snapshot taken when
/// its document was latest; writing the whole document back from it re-set
/// `is_latest`, leaving two releases of one source claiming the alias.
#[tokio::test]
async fn a_stale_reindex_snapshot_cannot_undo_a_later_demotion() {
    let env = common::TestEnv::start().await;

    env.repo
        .create_or_update(doc("api/only-in-old", "1.0.0", true))
        .await
        .unwrap();
    env.storage
        .put_object("docs/api_only-in-old.md", b"# Old\n\nbody".to_vec())
        .await
        .unwrap();

    // Snapshot taken while it is still latest — what the in-flight task holds.
    let stale = env
        .repo
        .find_all_by_slug("api/only-in-old")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(stale.is_latest);

    // A newer release lands and takes the alias.
    env.repo
        .create_or_update(doc("api/other", "1.2.0", false))
        .await
        .unwrap();
    env.repo
        .promote_release("assets-manager", "1.2.0")
        .await
        .unwrap();

    // The late task finishes and clears its flag.
    env.repo
        .clear_needs_reindex(&stale.slug, stale.release.as_deref())
        .await
        .unwrap();

    let after = env
        .repo
        .find_all_by_slug("api/only-in-old")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        !after.is_latest,
        "clearing the flag must not resurrect a demoted release"
    );
    assert!(!after.needs_reindex, "and must still clear the flag");

    let claiming: Vec<String> = env
        .repo
        .list_all()
        .await
        .unwrap()
        .into_iter()
        .filter(|d| d.source_id.as_deref() == Some("assets-manager") && d.is_latest)
        .map(|d| d.release.unwrap_or_default())
        .collect();
    assert_eq!(
        claiming,
        vec!["1.2.0".to_string()],
        "exactly one release may claim the alias"
    );
}
