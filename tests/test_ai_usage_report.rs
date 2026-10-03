//! Exercise the admin report through its actual Leptos server-function endpoint.

mod common;

use std::sync::Arc;

use axum::{
    body::Body, extract::State, http::Request, response::IntoResponse, routing::post, Router,
};
use lekton::app::AppState;
use lekton::db::usage_models::LlmUsageEvent;
use lekton::db::usage_repository::{MongoUsageEventRepository, UsageEventRepository};
use lekton::server::usage::{ConsumerUsage, ListTopConsumers};
use leptos::{prelude::provide_context, server_fn::ServerFn};

async fn server_fn(State(state): State<AppState>, request: Request<Body>) -> impl IntoResponse {
    leptos_axum::handle_server_fns_with_context(move || provide_context(state.clone()), request)
        .await
}

fn report_server(env: &common::TestEnv, demo_mode: bool) -> axum_test::TestServer {
    let mut state = env.app_state.clone();
    state.demo_mode = demo_mode;
    state.usage_event_repo = Some(Arc::new(MongoUsageEventRepository::new(&env.db)));
    axum_test::TestServer::new(
        Router::new()
            .route(ListTopConsumers::PATH, post(server_fn))
            .with_state(state),
    )
}

fn event(kind: &str, id: Option<&str>, model: &str) -> LlmUsageEvent {
    LlmUsageEvent {
        actor_kind: kind.into(),
        actor_id: id.map(str::to_string),
        feature: "chat".into(),
        model: model.into(),
        prompt_tokens: 10,
        completion_tokens: 5,
        estimated: false,
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn report_resolves_user_emails_without_losing_usage_or_other_callers() {
    let env = common::TestEnv::start().await;
    let admin = env
        .create_test_user("admin-1", "admin@test.com", true)
        .await;
    env.create_test_user("user-1", "person@test.com", false)
        .await;
    env.create_test_user("blank-email", " \t ", false).await;
    let repo = MongoUsageEventRepository::new(&env.db);
    repo.record_events(vec![
        event("user", Some("user-1"), "model-a"),
        event("user", Some("user-1"), "model-b"),
        event("user", Some("missing-user"), "model-a"),
        event("user", Some("blank-email"), "model-a"),
        event("user", Some("demo-demo"), "model-a"),
        // A token ID matching a user ID must not acquire that user's email.
        event("service_token", Some("user-1"), "model-a"),
        event("system", None, "model-a"),
        event("anonymous", None, "model-a"),
    ])
    .await
    .unwrap();

    let server = report_server(&env, true);
    let response = server
        .post(ListTopConsumers::PATH)
        .add_cookie(env.auth_cookie(&admin))
        .form(&ListTopConsumers { days: 7, limit: 25 })
        .await;
    response.assert_status_ok();
    let rows: Vec<ConsumerUsage> = response.json();
    assert_eq!(rows.len(), 7);
    let user = rows
        .iter()
        .find(|row| row.actor_kind == "user" && row.actor_id.as_deref() == Some("user-1"))
        .unwrap();
    assert_eq!(user.actor_email.as_deref(), Some("person@test.com"));
    assert_eq!(user.caller_label(), "person@test.com");
    assert_eq!(
        (user.calls, user.prompt_tokens, user.completion_tokens),
        (2, 20, 10)
    );

    let demo = rows
        .iter()
        .find(|row| row.actor_id.as_deref() == Some("demo-demo"))
        .unwrap();
    assert_eq!(demo.caller_label(), "demo@demo.lekton.dev");

    for id in ["missing-user", "blank-email"] {
        let row = rows
            .iter()
            .find(|row| row.actor_id.as_deref() == Some(id))
            .unwrap();
        assert!(row.actor_email.is_none());
        assert_eq!(row.caller_label(), id);
    }
    let token = rows
        .iter()
        .find(|row| row.actor_kind == "service_token")
        .unwrap();
    assert!(token.actor_email.is_none());
    assert_eq!(token.caller_label(), "user-1");
    for kind in ["system", "anonymous"] {
        let row = rows.iter().find(|row| row.actor_kind == kind).unwrap();
        assert!(row.actor_email.is_none());
        assert_eq!(row.caller_label(), "—");
    }

    let server = report_server(&env, false);
    let response = server
        .post(ListTopConsumers::PATH)
        .add_cookie(env.auth_cookie(&admin))
        .form(&ListTopConsumers { days: 7, limit: 25 })
        .await;
    response.assert_status_ok();
    let rows: Vec<ConsumerUsage> = response.json();
    let demo = rows
        .iter()
        .find(|row| row.actor_id.as_deref() == Some("demo-demo"))
        .unwrap();
    assert!(demo.actor_email.is_none());
    assert_eq!(demo.caller_label(), "demo-demo");
}

#[tokio::test]
async fn report_does_not_disclose_user_emails_to_non_admins() {
    let env = common::TestEnv::start().await;
    let user = env
        .create_test_user("user-1", "person@test.com", false)
        .await;
    MongoUsageEventRepository::new(&env.db)
        .record_events(vec![event("user", Some("user-1"), "model-a")])
        .await
        .unwrap();
    let server = report_server(&env, false);

    for cookie in [None, Some(env.auth_cookie(&user))] {
        let request = server
            .post(ListTopConsumers::PATH)
            .form(&ListTopConsumers { days: 7, limit: 25 });
        let response = match cookie {
            Some(cookie) => request.add_cookie(cookie).await,
            None => request.await,
        };
        assert!(!response.status_code().is_success());
        assert!(!response.text().contains("person@test.com"));
    }
}
