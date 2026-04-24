mod common;

#[tokio::test]
async fn seed_defaults_creates_expected_levels() {
    let env = common::TestEnv::start().await;

    env.access_level_repo.seed_defaults().await.unwrap();

    let levels = env.access_level_repo.list_all().await.unwrap();
    let names: Vec<&str> = levels.iter().map(|l| l.name.as_str()).collect();

    assert!(names.contains(&"public"));
    assert!(names.contains(&"internal"));
    assert!(names.contains(&"developer"));
    assert!(names.contains(&"architect"));

    // Verify "public" is a system level
    let public = levels.iter().find(|l| l.name == "public").unwrap();
    assert!(public.is_system);
}

#[tokio::test]
async fn seed_defaults_idempotent() {
    let env = common::TestEnv::start().await;

    env.access_level_repo.seed_defaults().await.unwrap();
    let first_count = env.access_level_repo.list_all().await.unwrap().len();

    env.access_level_repo.seed_defaults().await.unwrap();
    let second_count = env.access_level_repo.list_all().await.unwrap().len();

    assert_eq!(
        first_count, second_count,
        "seed_defaults must be idempotent"
    );
}

#[tokio::test]
async fn delete_system_level_forbidden() {
    let env = common::TestEnv::start().await;

    env.access_level_repo.seed_defaults().await.unwrap();

    let result = env.access_level_repo.delete("public").await;
    assert!(result.is_err(), "deleting system level should fail");
}

#[tokio::test]
async fn exists_returns_correct_value() {
    let env = common::TestEnv::start().await;

    env.access_level_repo.seed_defaults().await.unwrap();

    assert!(env.access_level_repo.exists("public").await.unwrap());
    assert!(!env.access_level_repo.exists("nonexistent").await.unwrap());
}

#[tokio::test]
async fn list_all_returns_all_seeded_levels() {
    let env = common::TestEnv::start().await;

    env.access_level_repo.seed_defaults().await.unwrap();

    let levels = env.access_level_repo.list_all().await.unwrap();
    assert!(
        levels.len() >= 4,
        "seed_defaults should create at least 4 levels"
    );
}
