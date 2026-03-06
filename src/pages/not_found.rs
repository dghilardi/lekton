use leptos::prelude::*;

/// 404 Not Found page.
#[component]
pub fn NotFound() -> impl IntoView {
    view! {
        <div class="hero min-h-[60vh]">
            <div class="hero-content text-center">
                <div class="max-w-md">
                    <h1 class="text-9xl font-bold text-primary">"404"</h1>
                    <p class="py-6 text-xl">"The page you are looking for does not exist."</p>
                    <a href="/" class="btn btn-primary">"Back to Home"</a>
                </div>
            </div>
        </div>
    }
}
