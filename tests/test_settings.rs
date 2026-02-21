mod common;

use lekton::db::settings_repository::SettingsRepository;

#[tokio::test]
async fn settings_default_returns_empty_css() {
    let env = common::TestEnv::start().await;

    let settings = env.settings_repo.get_settings().await.unwrap();
    assert_eq!(settings.key, "global");
    assert!(settings.custom_css.is_empty());
}

#[tokio::test]
async fn settings_set_and_get_custom_css() {
    let env = common::TestEnv::start().await;

    let css = ":root { --lekton-font-family: monospace; }";
    env.settings_repo.set_custom_css(css).await.unwrap();

    let settings = env.settings_repo.get_settings().await.unwrap();
    assert_eq!(settings.custom_css, css);
}

#[tokio::test]
async fn settings_update_custom_css() {
    let env = common::TestEnv::start().await;

    // Set initial CSS
    env.settings_repo
        .set_custom_css("body { color: red; }")
        .await
        .unwrap();

    // Update CSS
    env.settings_repo
        .set_custom_css("body { color: blue; }")
        .await
        .unwrap();

    let settings = env.settings_repo.get_settings().await.unwrap();
    assert_eq!(settings.custom_css, "body { color: blue; }");
}

#[tokio::test]
async fn settings_clear_custom_css() {
    let env = common::TestEnv::start().await;

    env.settings_repo
        .set_custom_css("body { color: red; }")
        .await
        .unwrap();

    env.settings_repo.set_custom_css("").await.unwrap();

    let settings = env.settings_repo.get_settings().await.unwrap();
    assert!(settings.custom_css.is_empty());
}
