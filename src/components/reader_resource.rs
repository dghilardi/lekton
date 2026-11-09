//! Request-scoped SSR reads, serialized into hydration and retried through the
//! shared auth orchestrator on subsequent client navigations.
use leptos::prelude::*;
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;

/// A failed read inside a page region: session renewal with a way back here,
/// or the error in plain words.
#[component]
pub fn ReaderErrorNotice(error: ServerFnError, context: &'static str) -> impl IntoView {
    if crate::auth::refresh_client::is_auth_error(&error) {
        let login_href = super::use_login_href();
        view! {
            <div class="alert alert-warning" role="status">
                <span>"Your session needs to be renewed. "<a class="link" href=move || login_href.get()>"Log in again"</a>" to continue."</span>
            </div>
        }.into_any()
    } else {
        view! {
            <div class="alert alert-error" role="alert">
                <span>{format!("{context}. {}", super::readable_error(&error))}</span>
            </div>
        }
        .into_any()
    }
}

/// A failed read that replaces a whole page: sign-in for expired or missing
/// sessions, otherwise the error in plain words with a way to retry.
#[component]
pub fn ReaderErrorState(error: ServerFnError) -> impl IntoView {
    use super::{PageState, StatePanel};
    if crate::auth::refresh_client::is_auth_error(&error) {
        let login_href = super::use_login_href();
        view! {
            <StatePanel
                kind=PageState::SignIn
                title="Sign in to continue"
                message="Your session has ended. Sign in again to come back to this page."
            >
                <a href=move || login_href.get() class="btn btn-primary">"Sign in"</a>
            </StatePanel>
        }
        .into_any()
    } else {
        let location = leptos_router::hooks::use_location();
        view! {
            <StatePanel
                kind=PageState::Error
                title="This page could not be loaded"
                message=super::readable_error(&error)
            >
                <a
                    href=move || {
                        let search = location.search.get();
                        if search.is_empty() { location.pathname.get() } else { format!("{}?{search}", location.pathname.get()) }
                    }
                    rel="external"
                    class="btn btn-primary"
                >"Try again"</a>
                <a href="/" class="btn btn-ghost">"Go to home"</a>
            </StatePanel>
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
