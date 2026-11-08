//! Request-scoped SSR reads, serialized into hydration and retried through the
//! shared auth orchestrator on subsequent client navigations.
use leptos::prelude::*;
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;

#[component]
pub fn ReaderErrorNotice(error: ServerFnError, context: &'static str) -> impl IntoView {
    if crate::auth::refresh_client::is_auth_error(&error) {
        view! {
            <div class="alert alert-warning" role="status">
                <span>"Your session needs to be renewed. "<a class="link" href="/login">"Log in again"</a>" to continue."</span>
            </div>
        }.into_any()
    } else {
        view! {
            <div class="alert alert-error" role="alert">
                <span>{format!("{context}: {error}")}</span>
            </div>
        }
        .into_any()
    }
}

pub fn reader_resource<S, T, F, Fut>(
    source: impl Fn() -> S + Send + Sync + 'static,
    fetcher: F,
) -> Resource<Result<T, ServerFnError>>
where
    S: PartialEq + Clone + Send + Sync + 'static,
    T: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    F: Fn(S) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<T, ServerFnError>> + Send + 'static,
{
    let resource = Resource::new_blocking(source, move |input| {
        let fetcher = fetcher.clone();
        #[cfg(feature = "hydrate")]
        {
            // Browser auth refresh joins a thread-local future. It is polled
            // only on the browser thread, never on the server's executor.
            send_wrapper::SendWrapper::new(async move {
                crate::auth::refresh_client::with_auth_retry(move || fetcher(input.clone())).await
            })
        }
        #[cfg(not(feature = "hydrate"))]
        fetcher(input)
    });

    // An expired SSR access cookie cannot use the path-restricted refresh
    // cookie. Bootstrap refreshes on the client; retry only that failed read,
    // rather than refetching every successfully hydrated resource.
    #[cfg(feature = "hydrate")]
    if let Some(user) = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>() {
        Effect::new(move || {
            if user.get().is_some()
                && matches!(resource.get_untracked(), Some(Err(ServerFnError::ServerError(ref message))) if message == crate::auth::models::UNAUTHORIZED_SENTINEL)
            {
                resource.refetch();
            }
        });
    }
    resource
}
