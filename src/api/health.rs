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

    // Actually probe the enabled dependencies rather than reporting "ok" merely
    // because the service handle exists: an initialised-but-unreachable backend
    // must show as an error, not healthy.
    let rag = match &state.rag_service {
        Some(svc) => match svc.health_check().await {
            Ok(()) => "ok",
            Err(_) => "error",
        },
        None => "disabled",
    };

    let search = match &state.search_service {
        Some(svc) => match svc.health_check().await {
            Ok(()) => "ok",
            Err(_) => "error",
        },
        None => "disabled",
    };

    // Ready only when Mongo is up and no *enabled* dependency is failing its probe.
    let deps_ok = mongo == "ok" && rag != "error" && search != "error";
    let status = if deps_ok { "ok" } else { "degraded" };
    let code = if deps_ok {
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
