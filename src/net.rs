//! Shared outbound HTTP client construction.
//!
//! All external dependencies (OIDC, embedding, reranker, …) build their
//! `reqwest` clients through here so they inherit bounded connect/total
//! timeouts. Without a timeout a stuck dependency can hang a request — and the
//! task/connection behind it — indefinitely.

use std::time::Duration;

/// Total per-request timeout for outbound HTTP to external dependencies.
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Connection-establishment timeout.
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;

/// A `reqwest::ClientBuilder` pre-configured with bounded timeouts, so callers
/// that need extra configuration (default headers, etc.) keep the timeouts.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS))
}

/// A ready-to-use `reqwest::Client` with bounded timeouts.
pub fn http_client() -> reqwest::Client {
    http_client_builder()
        .build()
        .expect("failed to build HTTP client with default timeouts")
}
