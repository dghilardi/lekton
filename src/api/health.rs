use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;

use crate::app::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

#[derive(Serialize)]
pub struct ReadyResponse {
    pub status: &'static str,
    pub mongo: &'static str,
    pub rag: &'static str,
    pub search: &'static str,
}

pub async fn liveness_handler() -> (StatusCode, Json<HealthResponse>) {
    (StatusCode::OK, Json(HealthResponse { status: "ok" }))
}

pub async fn readiness_handler(State(state): State<AppState>) -> (StatusCode, Json<ReadyResponse>) {
    let mongo = match state.settings_repo.get_settings().await {
        Ok(_) => "ok",
        Err(_) => "error",
    };

    let rag = if state.rag_service.is_some() {
        "ok"
    } else {
        "disabled"
    };

    let search = if state.search_service.is_some() {
        "ok"
    } else {
        "disabled"
    };

    let status = if mongo == "ok" { "ok" } else { "degraded" };
    let code = if mongo == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        code,
        Json(ReadyResponse {
            status,
            mongo,
            rag,
            search,
        }),
    )
}
