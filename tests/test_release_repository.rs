mod common;

/// A re-sync of the same release must refresh `last_synced_at` without moving
/// `first_synced_at`, which is what `list_by_source` orders on.
#[tokio::test]
async fn register_is_idempotent_and_preserves_first_synced_at() {
    let env = common::TestEnv::start().await;

    env.release_repo
        .register("assets-manager", "1.0.0")
        .await
        .expect("first register");

    let first = env
        .release_repo
        .list_by_source("assets-manager")
        .await
        .expect("list")
        .into_iter()
        .next()
        .expect("one release");

    // Mongo stores millisecond precision; wait so a moved timestamp is visible.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    env.release_repo
        .register("assets-manager", "1.0.0")
        .await
        .expect("re-register");

    let releases = env
        .release_repo
        .list_by_source("assets-manager")
        .await
        .expect("list again");

    assert_eq!(releases.len(), 1, "re-registering must not duplicate");
    assert_eq!(
        releases[0].first_synced_at, first.first_synced_at,
        "first_synced_at must be stable across re-syncs"
    );
    assert!(
        releases[0].last_synced_at > first.last_synced_at,
        "last_synced_at must advance on re-sync"
    );
}

#[tokio::test]
async fn list_by_source_is_newest_published_first_and_scoped_per_source() {
    let env = common::TestEnv::start().await;

    for release in ["1.0.0", "1.1.0", "1.2.0"] {
        env.release_repo
            .register("assets-manager", release)
            .await
            .expect("register");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    env.release_repo
        .register("cloud-common", "9.9.9")
        .await
        .expect("register other source");

    let releases = env
        .release_repo
        .list_by_source("assets-manager")
        .await
        .expect("list");

    let tags: Vec<&str> = releases.iter().map(|r| r.release.as_str()).collect();
    assert_eq!(
        tags,
        vec!["1.2.0", "1.1.0", "1.0.0"],
        "releases must come back newest-published first"
    );
    assert!(
        releases.iter().all(|r| r.source_id == "assets-manager"),
        "another source's releases must not leak in"
    );
}

/// This predicate is what makes the sync fail fast when `--version` is omitted
/// for a source that has already published releases.
#[tokio::test]
async fn a_source_becomes_release_managed_on_its_first_release() {
    let env = common::TestEnv::start().await;

    assert!(
        !env.release_repo
            .is_release_managed("assets-manager")
            .await
            .expect("query"),
        "an untouched source must not be release-managed"
    );

    env.release_repo
        .register("assets-manager", "1.0.0")
        .await
        .expect("register");

    assert!(
        env.release_repo
            .is_release_managed("assets-manager")
            .await
            .expect("query"),
        "publishing a release must make the source release-managed"
    );
    assert!(
        !env.release_repo
            .is_release_managed("cloud-common")
            .await
            .expect("query"),
        "the flag must be per source, not global"
    );
}

#[tokio::test]
async fn latest_alias_starts_unset_and_moves_in_place() {
    let env = common::TestEnv::start().await;

    assert_eq!(
        env.release_repo
            .latest("assets-manager")
            .await
            .expect("query"),
        None,
        "publishing without --latest must leave the alias unset"
    );

    env.release_repo
        .set_latest("assets-manager", "1.0.0")
        .await
        .expect("set alias");
    assert_eq!(
        env.release_repo
            .latest("assets-manager")
            .await
            .expect("query"),
        Some("1.0.0".to_string())
    );

    // Promoting another release replaces the alias rather than adding a second
    // one — the property the unique source_id index guarantees.
    env.release_repo
        .set_latest("assets-manager", "1.2.0")
        .await
        .expect("move alias");
    assert_eq!(
        env.release_repo
            .latest("assets-manager")
            .await
            .expect("query"),
        Some("1.2.0".to_string()),
        "the alias must move, not accumulate"
    );

    assert_eq!(
        env.release_repo
            .latest("cloud-common")
            .await
            .expect("query"),
        None,
        "the alias must be per source"
    );
}
