//! One presentation for pages that cannot show what was asked for: missing,
//! behind sign-in, not permitted, failed, or managed elsewhere.
use leptos::prelude::*;

/// Why a page shows a state instead of its content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageState {
    /// Nothing at this address the reader can see.
    NotFound,
    /// The page needs a signed-in reader.
    SignIn,
    /// Signed in, but without the rights this page needs.
    Forbidden,
    /// The page failed to load.
    Error,
    /// The content is maintained outside the portal.
    Managed,
}

impl PageState {
    fn icon(self) -> &'static str {
        match self {
            // Magnifier with a slash: nothing found.
            PageState::NotFound => "M21 21l-4.35-4.35M11 19a8 8 0 100-16 8 8 0 000 16zM8 8l6 6",
            // Padlock.
            PageState::SignIn | PageState::Forbidden => {
                "M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
            }
            // Warning triangle.
            PageState::Error => {
                "M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
            }
            // Arrow out of a box: lives elsewhere.
            PageState::Managed => "M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14",
        }
    }

    fn tone(self) -> &'static str {
        match self {
            PageState::Error => "bg-error/10 text-error",
            PageState::Forbidden => "bg-warning/15 text-base-content",
            _ => "bg-primary/10 text-primary",
        }
    }
}

/// A page-level state: icon, heading, one plain sentence and the ways out.
/// The heading is the page's `h1`, since the state replaces its content.
#[component]
pub fn StatePanel(
    kind: PageState,
    #[prop(into)] title: TextProp,
    #[prop(into)] message: TextProp,
    /// Actions, most useful first (links or buttons styled `btn`).
    #[prop(optional)]
    children: Option<Children>,
) -> impl IntoView {
    view! {
        <section class="lekton-state mx-auto max-w-xl py-12 text-center" data-state=format!("{kind:?}")>
            <div class=format!("mx-auto mb-5 flex h-12 w-12 items-center justify-center rounded-full {}", kind.tone()) aria-hidden="true">
                <svg class="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d=kind.icon()></path>
                </svg>
            </div>
            <h1 class="text-2xl font-semibold text-balance">{move || title.get()}</h1>
            <p class="mt-3 text-base-content/75 text-pretty">{move || message.get()}</p>
            {children.map(|children| view! {
                <div class="mt-6 flex flex-wrap items-center justify-center gap-3">{children()}</div>
            })}
        </section>
    }
}

/// For pages that need more than the reader has: sign-in (returning here) for
/// anonymous readers, a plain refusal for signed-in ones. Nothing is shown
/// until the session is known, so a signed-in admin never sees a flash of it.
#[component]
pub fn RequireRole(
    signed_in: Signal<bool>,
    /// What the page is, as a subject: "Administration", "Editing pages".
    action: &'static str,
) -> impl IntoView {
    let resolved = use_context::<crate::app::AuthResolved>().map(|resolved| resolved.0);
    let login_href = use_login_href();
    move || {
        if !resolved.is_none_or(|resolved| resolved.get()) {
            return ().into_any();
        }
        if signed_in.get() {
            #[cfg(feature = "ssr")]
            if let Some(response) = use_context::<leptos_axum::ResponseOptions>() {
                response.set_status(axum::http::StatusCode::FORBIDDEN);
            }
            view! {
                <StatePanel
                    kind=PageState::Forbidden
                    title="You don't have access to this page"
                    message=format!("{action} is limited to administrators. Ask one if you need something changed here.")
                >
                    <a href="/" class="btn btn-primary">"Go to home"</a>
                </StatePanel>
            }
            .into_any()
        } else {
            view! {
                <StatePanel
                    kind=PageState::SignIn
                    title="Sign in to continue"
                    message=format!("{action} needs a signed-in administrator. You will come back here after signing in.")
                >
                    <a href=move || login_href.get() class="btn btn-primary">"Sign in"</a>
                    <a href="/" class="btn btn-ghost">"Go to home"</a>
                </StatePanel>
            }
            .into_any()
        }
    }
}

/// The sign-in page for the current URL, returning here afterwards.
/// Always the in-app page, which forwards to the identity provider when the
/// portal uses one, so it also works for client-side navigation.
pub fn use_login_href() -> Signal<String> {
    let location = leptos_router::hooks::use_location();
    Signal::derive(move || {
        let search = location.search.get();
        let here = if search.is_empty() {
            location.pathname.get()
        } else {
            format!("{}?{search}", location.pathname.get())
        };
        crate::auth::return_to::login_href("/login", &here)
    })
}

/// Text for a failed server call that a reader can act on, without the
/// framework's "error running server function" wrapper.
pub fn readable_error(error: &ServerFnError) -> String {
    let message = match error {
        ServerFnError::ServerError(message) => message.trim(),
        ServerFnError::Request(_) | ServerFnError::Response(_) => {
            return "The server could not be reached. Check your connection and try again."
                .to_string()
        }
        _ => "",
    };
    // Variants other than plain server errors carry framework detail; so does
    // an empty message.
    if message.is_empty() {
        "Something went wrong on the server. Try again in a moment.".to_string()
    } else {
        message.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_errors_drop_the_framework_wrapper() {
        assert_eq!(
            readable_error(&ServerFnError::ServerError("Schema not found".into())),
            "Schema not found"
        );
        assert_eq!(
            readable_error(&ServerFnError::Request("connection refused".into())),
            "The server could not be reached. Check your connection and try again."
        );
        assert!(
            !readable_error(&ServerFnError::Deserialization("bad".into()))
                .contains("server function")
        );
    }
}
