//! Server-Sent Events responses that survive reverse proxies.

use std::convert::Infallible;
use std::time::Duration;

use axum::http::header::{HeaderName, CACHE_CONTROL};
use axum::http::HeaderValue;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use futures::Stream;

/// Interval between SSE keep-alive comments. Comfortably below the 30–60 s
/// read timeouts common on reverse proxies, so a slow first token (retrieval,
/// a cold model) does not get the connection closed as idle.
pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Wrap an event stream as an SSE response that proxies forward as it is
/// produced.
///
/// `X-Accel-Buffering: no` turns nginx response buffering off for this
/// response — with `proxy_buffering on` nginx otherwise holds the stream back
/// until it ends or a timeout cuts it — and `Cache-Control: no-cache` keeps
/// intermediaries from storing it. Keep-alive comments (`:` lines) are ignored
/// by SSE clients.
pub fn proxy_friendly<S>(stream: S) -> impl IntoResponse
where
    S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    (
        [
            (
                HeaderName::from_static("x-accel-buffering"),
                HeaderValue::from_static("no"),
            ),
            (CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn proxy_friendly_disables_proxy_buffering() {
        let response = proxy_friendly(futures::stream::empty()).into_response();
        let headers = response.headers();
        assert_eq!(headers.get("x-accel-buffering").unwrap(), "no");
        assert_eq!(headers.get(CACHE_CONTROL).unwrap(), "no-cache");
        assert_eq!(headers.get("content-type").unwrap(), "text/event-stream");
    }
}
