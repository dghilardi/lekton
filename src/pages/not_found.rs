use leptos::prelude::*;

use crate::components::{use_login_href, PageState, StatePanel};

/// 404 Not Found page.
#[component]
pub fn NotFound() -> impl IntoView {
    view! {
        <leptos_meta::Title text=crate::components::page_title(&["Page not found"]) />
        <NotFoundState />
    }
}

/// The not-found state, with status 404 on server renders. Restricted pages
/// read as missing too, so their existence does not leak; anonymous readers
/// are offered sign-in, which returns them here.
#[component]
pub fn NotFoundState() -> impl IntoView {
    #[cfg(feature = "ssr")]
    if let Some(response) = use_context::<leptos_axum::ResponseOptions>() {
        response.set_status(axum::http::StatusCode::NOT_FOUND);
    }
    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
    let anonymous = Signal::derive(move || current_user.is_none_or(|user| user.get().is_none()));
    let login_href = use_login_href();
    view! {
        <StatePanel
            kind=PageState::NotFound
            title="Page not found"
            message=move || if anonymous.get() {
                "There is nothing at this address, or it is only visible after signing in."
            } else {
                "There is nothing at this address, or your account cannot see it."
            }
        >
            <a href="/" class="btn btn-primary">"Go to home"</a>
            <Show when=move || anonymous.get()>
                <a href=move || login_href.get() class="btn btn-ghost">"Sign in"</a>
            </Show>
        </StatePanel>
    }
}
