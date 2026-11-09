use leptos::prelude::*;

use crate::app::IsDemoMode;

/// Login page.
///
/// In demo mode, shows a username/password form with demo credentials.
/// In OAuth2/OIDC mode, redirects the browser to `/auth/login` which starts
/// the external provider flow.
#[component]
pub fn LoginPage() -> impl IntoView {
    let is_demo_mode = use_context::<IsDemoMode>()
        .expect("LoginPage must be inside App")
        .0;
    let query = leptos_router::hooks::use_query_map();
    // Where to go once signed in: a checked local path, else home.
    let return_to = Signal::derive(move || {
        crate::auth::return_to::safe_return_path(
            query
                .read()
                .get(crate::auth::return_to::NEXT_PARAM)
                .as_deref(),
        )
        .unwrap_or_else(|| "/".to_string())
    });

    view! {
        <leptos_meta::Title text=crate::components::page_title(&["Sign in"]) />
        {move || {
            if is_demo_mode.get() {
                view! { <DemoLoginForm return_to=return_to /> }.into_any()
            } else {
                view! { <OAuthRedirect return_to=return_to /> }.into_any()
            }
        }}
    }
}

/// Redirects to `/auth/login` to start the OAuth2/OIDC flow.
/// Shows a brief loading state while the redirect is set up.
#[component]
fn OAuthRedirect(return_to: Signal<String>) -> impl IntoView {
    let provider_href =
        Signal::derive(move || crate::auth::return_to::login_href("/auth/login", &return_to.get()));
    Effect::new(move |_| {
        let _href = provider_href.get();
        #[cfg(feature = "hydrate")]
        {
            use leptos::web_sys::window;
            if let Some(w) = window() {
                let _ = w.location().replace(&_href);
            }
        }
    });

    view! {
        <div class="hero min-h-[60vh]">
            <div class="hero-content text-center">
                <div>
                    <span class="loading loading-spinner loading-lg"></span>
                    <p class="mt-4 text-base-content/70">"Redirecting to sign in..."</p>
                    <p class="text-sm mt-4">
                        "If you are not redirected automatically, "
                        <a href=move || provider_href.get() rel="external" class="link link-primary">"click here"</a>.
                    </p>
                </div>
            </div>
        </div>
    }
}

/// Demo mode login form with hardcoded test credentials.
#[component]
fn DemoLoginForm(return_to: Signal<String>) -> impl IntoView {
    view! {
        <div class="hero min-h-[60vh]">
            <div class="hero-content">
                <div class="card bg-base-100 shadow-2xl w-full max-w-md">
                    <div class="card-body">
                        <h2 class="card-title text-2xl justify-center">"Demo Login"</h2>
                        <p class="text-center text-base-content/70 text-sm">
                            "Sign in with demo credentials to explore Lekton."
                        </p>

                        // login.js returns here after signing in.
                        <form id="login-form" class="mt-4" data-return-to=move || return_to.get()>
                            <div class="form-control">
                                <label class="label">
                                    <span class="label-text">"Username"</span>
                                </label>
                                <input
                                    id="login-username"
                                    type="text"
                                    name="username"
                                    placeholder="demo"
                                    class="input input-bordered"
                                    required
                                />
                            </div>
                            <div class="form-control mt-2">
                                <label class="label">
                                    <span class="label-text">"Password"</span>
                                </label>
                                <input
                                    id="login-password"
                                    type="password"
                                    name="password"
                                    placeholder="demo"
                                    class="input input-bordered"
                                    required
                                />
                            </div>
                            <div id="login-error" class="alert alert-error mt-4 hidden" role="alert">
                                <span id="login-error-message">"Unable to sign in"</span>
                            </div>
                            <div class="form-control mt-6">
                                <button type="submit" class="btn btn-primary">"Sign In"</button>
                            </div>
                        </form>

                        <div class="divider">"Demo Accounts"</div>
                        <div class="overflow-x-auto">
                            <table class="table table-sm">
                                <thead>
                                    <tr>
                                        <th>"Username"</th>
                                        <th>"Password"</th>
                                        <th>"Role"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    <tr>
                                        <td><code>"demo"</code></td>
                                        <td><code>"demo"</code></td>
                                        <td><span class="badge badge-info">"Developer"</span></td>
                                    </tr>
                                    <tr>
                                        <td><code>"admin"</code></td>
                                        <td><code>"admin"</code></td>
                                        <td><span class="badge badge-primary">"Admin"</span></td>
                                    </tr>
                                    <tr>
                                        <td><code>"public"</code></td>
                                        <td><code>"public"</code></td>
                                        <td><span class="badge badge-ghost">"Public"</span></td>
                                    </tr>
                                </tbody>
                            </table>
                        </div>
                    </div>
                </div>
            </div>
        </div>

        // Client-side login JavaScript (loaded from external file)
        <script src="/js/login.js" defer></script>
    }
}
