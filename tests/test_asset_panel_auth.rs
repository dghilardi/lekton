//! The editor's asset panel server functions must be admin-only: one lists every
//! asset, the other deletes an asset from storage and from the indexes.

mod common;

use axum::{
    body::Body, extract::State, http::Request, response::IntoResponse, routing::post, Router,
};
use chrono::Utc;
use lekton::app::AppState;
use lekton::db::models::{Asset, ExtractionStatus};
use lekton::editor::asset_panel::{DeleteAssetByKey, ListAllAssets};
use leptos::{prelude::provide_context, server_fn::ServerFn};

const KEY: &str = "panel/secret.pdf";

async fn server_fn(State(state): State<AppState>, request: Request<Body>) -> impl IntoResponse {
    leptos_axum::handle_server_fns_with_context(move || provide_context(state.clone()), request)
        .await
}

fn panel_server(env: &common::TestEnv) -> axum_test::TestServer {
    axum_test::TestServer::new(
        Router::new()
            .route(ListAllAssets::PATH, post(server_fn))
            .route(DeleteAssetByKey::PATH, post(server_fn))
            .with_state(env.app_state.clone()),
    )
}

async fn seed_asset(env: &common::TestEnv) {
    let s3_key = format!("assets/{KEY}");
    env.storage
        .put_object(&s3_key, b"%PDF-1.4".to_vec())
        .await
        .unwrap();
    env.asset_repo
        .create_or_update(Asset {
            key: KEY.to_string(),
            content_type: "application/pdf".to_string(),
            size_bytes: 8,
            s3_key,
            uploaded_at: Utc::now(),
            uploaded_by: "admin@test.com".to_string(),
            referenced_by: vec![],
            content_hash: None,
            extraction_status: ExtractionStatus::Skipped,
            extraction_error: None,
            extracted_content_hash: None,
            extracted_at: None,
            indexed_chunks: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn asset_panel_rejects_anonymous_and_non_admin_callers() {
    let env = common::TestEnv::start().await;
    let user = env
        .create_test_user("user-1", "person@test.com", false)
        .await;
    seed_asset(&env).await;
    let server = panel_server(&env);

    for cookie in [None, Some(env.auth_cookie(&user))] {
        let list = server.post(ListAllAssets::PATH).form(&ListAllAssets {});
        let delete = server.post(DeleteAssetByKey::PATH).form(&DeleteAssetByKey {
            key: KEY.to_string(),
        });
        let (list, delete) = match &cookie {
            Some(c) => (
                list.add_cookie(c.clone()).await,
                delete.add_cookie(c.clone()).await,
            ),
            None => (list.await, delete.await),
        };
        assert!(
            !list.status_code().is_success(),
            "list allowed for {cookie:?}"
        );
        assert!(!list.text().contains(KEY));
        assert!(
            !delete.status_code().is_success(),
            "delete allowed for {cookie:?}"
        );
    }

    assert!(env.asset_repo.find_by_key(KEY).await.unwrap().is_some());
    assert!(env
        .storage
        .get_object(&format!("assets/{KEY}"))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn asset_panel_lets_admins_list_and_delete() {
    let env = common::TestEnv::start().await;
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;
    seed_asset(&env).await;
    let server = panel_server(&env);

    let list = server
        .post(ListAllAssets::PATH)
        .form(&ListAllAssets {})
        .add_cookie(env.auth_cookie(&admin))
        .await;
    list.assert_status_ok();
    assert!(list.text().contains(KEY));

    server
        .post(DeleteAssetByKey::PATH)
        .form(&DeleteAssetByKey {
            key: KEY.to_string(),
        })
        .add_cookie(env.auth_cookie(&admin))
        .await
        .assert_status_ok();
    assert!(env.asset_repo.find_by_key(KEY).await.unwrap().is_none());
}
