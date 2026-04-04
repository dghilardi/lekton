use leptos::prelude::*;

/// Login page.
///
/// In demo mode, shows a username/password form with demo credentials.
/// In OAuth2/OIDC mode, redirects the browser to `/auth/login` which starts
/// the external provider flow.
#[component]
pub fn LoginPage() -> impl IntoView {
    let is_demo_mode =
        use_context::<Signal<bool>>().expect("LoginPage must be inside App");

    view! {
        {move || {
            if is_demo_mode.get() {
                view! { <DemoLoginForm /> }.into_any()
            } else {
                view! { <OAuthRedirect /> }.into_any()
            }
        }}
    }
}

/// Redirects to `/auth/login` to start the OAuth2/OIDC flow.
/// Shows a brief loading state while the redirect is set up.
#[component]
fn OAuthRedirect() -> impl IntoView {
    view! {
        <div class="hero min-h-[60vh]">
            <div class="hero-content text-center">
                <div>
                    <span class="loading loading-spinner loading-lg"></span>
                    <p class="mt-4 text-base-content/70">"Redirecting to sign in..."</p>
                </div>
            </div>
        </div>

        // Redirect via JS — works both on initial SSR and on client navigation.
        <script>"window.location.replace('/auth/login');"</script>
    }
}

/// Demo mode login form with hardcoded test credentials.
#[component]
fn DemoLoginForm() -> impl IntoView {
    view! {
        <div class="hero min-h-[60vh]">
            <div class="hero-content">
                <div class="card bg-base-100 shadow-2xl w-full max-w-md">
                    <div class="card-body">
                        <h2 class="card-title text-2xl justify-center">"Demo Login"</h2>
                        <p class="text-center text-base-content/70 text-sm">
                            "Sign in with demo credentials to explore Lekton."
                        </p>

                        <form id="login-form" class="mt-4">
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
                            <div id="login-error" class="alert alert-error mt-4 hidden">
                                <span>"Invalid credentials"</span>
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
                                        <td><span class="badge badge-error">"Admin"</span></td>
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
