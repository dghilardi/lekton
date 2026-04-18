//! Client-side token refresh orchestrator (hydrate build only).
//!
//! # Responsibilities
//!
//! 1. **Detect** auth errors from server function results using
//!    [`is_auth_error`] — matches the [`UNAUTHORIZED_SENTINEL`] emitted by
//!    `require_any_user` / `require_admin_user`.
//!
//! 2. **Deduplicate** concurrent refresh calls: if three server functions all
//!    fail with a 401 at the same time, only **one** real HTTP request is sent
//!    to `POST /auth/refresh`; the other two futures join the same in-flight
//!    future and wake up together when it resolves.
//!
//! 3. **Retry** the original call after a successful refresh.
//!
//! 4. **Bootstrap** an existing session on first load by attempting one silent
//!    refresh when the app has no current user yet.
//!
//! 5. **Redirect** to `/login` when the refresh itself fails during a normal
//!    authenticated operation (revoked or expired refresh token → the session
//!    is truly dead).
//!
//! # Usage
//!
//! ```rust,ignore
//! // Before:
//! match list_user_pats().await {
//!     Ok(t)  => pats.set(t),
//!     Err(e) => tracing::error!("{e}"),
//! }
//!
//! // After:
//! match with_auth_retry(list_user_pats).await {
//!     Ok(t)  => pats.set(t),
//!     Err(e) => tracing::error!("{e}"),  // already redirected to /login if 401
//! }
//! ```

#[cfg(feature = "hydrate")]
mod inner {
    use std::cell::RefCell;
    use std::future::Future;
    use std::pin::Pin;

    use futures::future::{FutureExt, Shared};
    use leptos::server_fn::error::ServerFnError;

    use crate::auth::models::UNAUTHORIZED_SENTINEL;

    // ── Type aliases ──────────────────────────────────────────────────────────

    /// Outcome of a single refresh attempt.
    type RefreshResult = Result<(), String>;

    /// A shared, clone-able future that all concurrent waiters can `.await`.
    type SharedRefreshFut = Shared<Pin<Box<dyn Future<Output = RefreshResult> + 'static>>>;

    // ── In-flight deduplication state ─────────────────────────────────────────

    thread_local! {
        /// Holds the in-flight refresh future while a refresh is occurring.
        ///
        /// WASM is single-threaded, so `RefCell` is sufficient (no lock
        /// contention). There are no `.await` points between the check and the
        /// store, so there is no interleaving risk.
        static REFRESH_IN_FLIGHT: RefCell<Option<SharedRefreshFut>> =
            RefCell::new(None);
    }

    // ── Public helpers ────────────────────────────────────────────────────────

    /// Returns `true` when `err` carries the [`UNAUTHORIZED_SENTINEL`], meaning
    /// the server rejected the request because the access token is expired or
    /// absent.  `false` for every other error (forbidden, not-found, etc.).
    ///
    /// Uses a direct pattern match on `ServerFnError::ServerError(msg)` to
    /// avoid dependence on the `Display` formatting (which wraps the message
    /// with "error running server function: …").
    pub fn is_auth_error(err: &ServerFnError) -> bool {
        matches!(err, ServerFnError::ServerError(msg) if msg == UNAUTHORIZED_SENTINEL)
    }

    /// Attempt to refresh the token pair.
    ///
    /// If a refresh is already in progress (started by a concurrent caller),
    /// this function joins that same future instead of launching a second
    /// request.  After the shared future resolves, every waiter gets the same
    /// `Ok(())`/`Err(msg)` result.
    ///
    /// On success the browser will have stored the new `lekton_access_token`
    /// and `lekton_refresh_token` cookies (set by the `/auth/refresh` response
    /// headers).
    pub async fn try_refresh() -> RefreshResult {
        // Step 1: try to grab the in-flight future (no await here → safe).
        let existing: Option<SharedRefreshFut> = REFRESH_IN_FLIGHT.with(|r| r.borrow().clone());

        if let Some(fut) = existing {
            // Another caller already started a refresh — join it.
            return fut.await;
        }

        // Step 2: we are the first caller; create, store, then await.
        let refresh_fut: SharedRefreshFut = do_refresh().boxed_local().shared();
        REFRESH_IN_FLIGHT.with(|r| *r.borrow_mut() = Some(refresh_fut.clone()));

        let result = refresh_fut.await;

        // Step 3: clear the slot so future callers start a fresh refresh.
        REFRESH_IN_FLIGHT.with(|r| *r.borrow_mut() = None);

        result
    }

    /// Call `f()`, and if it returns an [`UNAUTHORIZED_SENTINEL`] error:
    ///
    /// 1. Call [`try_refresh`] (deduplicated).
    /// 2. On success — retry `f()` once and return its result.
    /// 3. On failure — redirect the browser to `/login` (best-effort) and
    ///    return the original error.
    ///
    /// All other errors are passed through unchanged.
    pub async fn with_auth_retry<T, F, Fut>(f: F) -> Result<T, ServerFnError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, ServerFnError>>,
    {
        match f().await {
            Err(ref e) if is_auth_error(e) => {
                match try_refresh().await {
                    Ok(()) => f().await,
                    Err(_) => {
                        // Refresh failed — session is dead, redirect to login.
                        redirect_to_login();
                        f().await // result will be ignored after navigation
                    }
                }
            }
            other => other,
        }
    }

    /// Bootstrap helper for "who am I?" style calls that return `Option<T>`.
    ///
    /// On the first app load, browsers do not send expired access-token
    /// cookies, so the server returns `Ok(None)` instead of the shared auth
    /// sentinel. In that case we attempt one silent refresh without redirecting
    /// to `/login`; anonymous visitors should stay anonymous, while callers
    /// with a valid refresh token recover their session seamlessly.
    pub async fn with_auth_bootstrap<T, F, Fut>(f: F) -> Result<Option<T>, ServerFnError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<Option<T>, ServerFnError>>,
    {
        let initial = f().await;

        if !should_attempt_bootstrap_refresh(&initial) {
            return initial;
        }

        match try_refresh().await {
            Ok(()) => f().await,
            Err(_) => Ok(None),
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// POST to `/auth/refresh` and return whether it succeeded.
    ///
    /// The refresh-token cookie is path-restricted to `/auth/refresh` by the
    /// server, so the browser sends it automatically on this exact path.
    /// `gloo_net` defaults to same-origin credentials, which is correct here.
    async fn do_refresh() -> RefreshResult {
        use gloo_net::http::Request;

        let resp = Request::post("/auth/refresh")
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if resp.ok() {
            Ok(())
        } else {
            Err(format!("refresh failed with status {}", resp.status()))
        }
    }

    /// Navigate to `/login` via `window.location.href`.
    ///
    /// Works from any async context (unlike Leptos `use_navigate` which
    /// requires component context).
    fn redirect_to_login() {
        if let Some(window) = web_sys::window() {
            let _ = window.location().set_href("/login");
        }
    }

    fn should_attempt_bootstrap_refresh<T>(result: &Result<Option<T>, ServerFnError>) -> bool {
        matches!(result, Ok(None)) || matches!(result, Err(err) if is_auth_error(err))
    }
}

// Re-export the public API at module level when the hydrate feature is active.
#[cfg(feature = "hydrate")]
pub use inner::{is_auth_error, try_refresh, with_auth_bootstrap, with_auth_retry};

// ── SSR passthrough ───────────────────────────────────────────────────────────
//
// In the SSR build, components are rendered on the server where a token refresh
// is not meaningful (each request is stateless).  We expose no-op versions of
// the same functions so page components can call `with_auth_retry(...)` without
// `#[cfg]` guards.

/// SSR stub: returns `true` when the error is the [`UNAUTHORIZED_SENTINEL`].
/// Same logic as the hydrate version; extracted so it compiles in all targets.
#[cfg(not(feature = "hydrate"))]
pub fn is_auth_error(err: &leptos::server_fn::error::ServerFnError) -> bool {
    use crate::auth::models::UNAUTHORIZED_SENTINEL;
    matches!(err, leptos::server_fn::error::ServerFnError::ServerError(msg) if msg == UNAUTHORIZED_SENTINEL)
}

/// SSR stub: no refresh is possible during server-side rendering; just calls
/// `f()` directly and returns its result unchanged.
#[cfg(not(feature = "hydrate"))]
pub async fn with_auth_retry<T, F, Fut>(f: F) -> Result<T, leptos::server_fn::error::ServerFnError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, leptos::server_fn::error::ServerFnError>>,
{
    f().await
}

/// SSR stub: bootstrap reads are also a passthrough on the server.
#[cfg(not(feature = "hydrate"))]
pub async fn with_auth_bootstrap<T, F, Fut>(
    f: F,
) -> Result<Option<T>, leptos::server_fn::error::ServerFnError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, leptos::server_fn::error::ServerFnError>>,
{
    f().await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // `is_auth_error` depends on ServerFnError which is available in all
    // build configurations, so we can test it without the hydrate feature.
    use crate::auth::models::UNAUTHORIZED_SENTINEL;
    use leptos::server_fn::error::ServerFnError;

    fn auth_err() -> ServerFnError {
        ServerFnError::new(UNAUTHORIZED_SENTINEL)
    }

    fn other_err(msg: &str) -> ServerFnError {
        ServerFnError::new(msg)
    }

    fn should_attempt_bootstrap_refresh<T>(result: &Result<Option<T>, ServerFnError>) -> bool {
        matches!(result, Ok(None))
            || matches!(result, Err(err) if matches!(err, ServerFnError::ServerError(msg) if msg == UNAUTHORIZED_SENTINEL))
    }

    /// Replicate the detection logic so the test works without the hydrate
    /// feature (which is WASM-only and cannot run in regular `cargo test`).
    fn is_auth_error(err: &ServerFnError) -> bool {
        matches!(err, ServerFnError::ServerError(msg) if msg == UNAUTHORIZED_SENTINEL)
    }

    #[test]
    fn sentinel_error_detected() {
        assert!(is_auth_error(&auth_err()));
    }

    #[test]
    fn other_errors_not_detected() {
        assert!(!is_auth_error(&other_err("Admin privileges required")));
        assert!(!is_auth_error(&other_err("PAT not found")));
        assert!(!is_auth_error(&other_err("internal server error")));
        assert!(!is_auth_error(&other_err("")));
    }

    #[test]
    fn sentinel_does_not_match_superstring() {
        // A message containing the sentinel as a substring must not match.
        assert!(!is_auth_error(&other_err("not unauthorized")));
        assert!(!is_auth_error(&other_err("unauthorized access attempt")));
    }

    #[test]
    fn bootstrap_refresh_runs_for_missing_user() {
        assert!(should_attempt_bootstrap_refresh::<()>(&Ok::<
            Option<()>,
            ServerFnError,
        >(None),));
    }

    #[test]
    fn bootstrap_refresh_runs_for_auth_sentinel() {
        assert!(should_attempt_bootstrap_refresh::<()>(&Err(auth_err())));
    }

    #[test]
    fn bootstrap_refresh_skips_authenticated_and_other_errors() {
        assert!(!should_attempt_bootstrap_refresh(&Ok(Some(()))));
        assert!(!should_attempt_bootstrap_refresh::<()>(&Err(other_err(
            "forbidden"
        ))));
    }
}
