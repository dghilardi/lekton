//! Prometheus usage metrics.
//!
//! Gated behind `features.metrics` (off by default). When enabled, [`install`]
//! registers a global recorder and returns a [`PrometheusHandle`] that
//! [`render`] serialises at `GET /metrics`. The [`track_metrics`] middleware
//! records HTTP request counts and latencies; product-usage counters are
//! emitted inline at the relevant handlers via the `metrics` macros.
//!
//! Product counters are plain `metrics::counter!` calls scattered across the
//! codebase. Those macros are cheap no-ops when no recorder is installed, so
//! they need no feature gating — only the exporter and the `/metrics` route
//! depend on the flag.

use std::time::Instant;

use axum::{
    body::Body,
    extract::MatchedPath,
    http::{header, HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

/// Latency histogram buckets, in seconds.
const LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Install the global Prometheus recorder. Returns a handle used by [`render`].
///
/// Panics if a recorder is already installed — call exactly once at startup.
pub fn install() -> PrometheusHandle {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("http_request_duration_seconds".to_string()),
            LATENCY_BUCKETS,
        )
        .expect("valid latency buckets")
        .install_recorder()
        .expect("failed to install Prometheus recorder")
}

/// Render the metrics exposition, guarding it with an optional bearer token.
///
/// When `expected_token` is set, requests must carry a matching
/// `Authorization: Bearer <token>` header, otherwise `401` is returned.
pub fn render(
    handle: &PrometheusHandle,
    expected_token: Option<&str>,
    headers: &HeaderMap,
) -> Response {
    if let Some(expected) = expected_token {
        let provided = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if provided != Some(expected) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        handle.render(),
    )
        .into_response()
}

/// Middleware recording `http_requests_total` and `http_request_duration_seconds`.
///
/// Labels use the matched route template (e.g. `/api/v1/image/{filename}`) to
/// keep cardinality bounded; requests without a matched route (static assets)
/// are grouped under `other`.
pub async fn track_metrics(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "other".to_string());

    let response = next.run(req).await;

    let status = response.status().as_u16().to_string();
    let labels = [("method", method), ("path", path), ("status", status)];

    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_request_duration_seconds", &labels)
        .record(start.elapsed().as_secs_f64());

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_handle() -> PrometheusHandle {
        // A local (non-installed) recorder gives us a handle without touching
        // the global recorder, so the test stays isolated.
        PrometheusBuilder::new().build_recorder().handle()
    }

    #[test]
    fn render_without_token_is_open() {
        let handle = test_handle();
        let resp = render(&handle, None, &HeaderMap::new());
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn render_with_token_rejects_missing_and_wrong() {
        let handle = test_handle();

        let resp = render(&handle, Some("secret"), &HeaderMap::new());
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer wrong".parse().unwrap());
        let resp = render(&handle, Some("secret"), &headers);
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn render_with_token_accepts_match() {
        let handle = test_handle();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        let resp = render(&handle, Some("secret"), &headers);
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
