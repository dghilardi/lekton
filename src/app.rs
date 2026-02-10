use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::components::*;
use leptos_router::path;

use crate::rendering::markdown::render_markdown;

/// Shared application state (available on both client and server).
///
/// On the server side, the full `AppState` with DB/Storage is used.
/// This struct holds the subset needed for the Leptos app.
#[cfg(feature = "ssr")]
use std::sync::Arc;

#[cfg(feature = "ssr")]
#[derive(Clone, axum::extract::FromRef)]
pub struct AppState {
    pub document_repo: Arc<dyn crate::db::repository::DocumentRepository>,
    pub storage_client: Arc<dyn crate::storage::client::StorageClient>,
    pub service_token: String,
    pub leptos_options: LeptosOptions,
}

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Html attr:lang="en" attr:data-theme="light" />
        <Title text="Lekton — Internal Developer Portal" />
        <Meta charset="utf-8" />
        <Meta name="viewport" content="width=device-width, initial-scale=1" />
        <Meta name="description" content="Lekton: A dynamic, high-performance Internal Developer Portal with RBAC and unified schema registry." />

        // Runtime customizable stylesheet — loaded AFTER the main CSS
        <Link rel="stylesheet" href="/custom.css" />

        <Router>
            <Layout>
                <Routes fallback=|| view! { <NotFound /> }>
                    <Route path=path!("/") view=HomePage />
                    <Route path=path!("/docs/:slug") view=DocPage />
                </Routes>
            </Layout>
        </Router>
    }
}

/// Main layout: navbar + sidebar + content area.
#[component]
fn Layout(children: Children) -> impl IntoView {
    view! {
        <div class="min-h-screen bg-base-200">
            // Navbar
            <div class="navbar bg-base-100 shadow-lg sticky top-0 z-50">
                <div class="flex-1">
                    <a class="btn btn-ghost text-xl font-bold" href="/">
                        "🔥 Lekton"
                    </a>
                </div>
                <div class="flex-none gap-2">
                    <div class="form-control">
                        <input
                            type="text"
                            placeholder="Search docs..."
                            class="input input-bordered w-24 md:w-auto"
                        />
                    </div>
                    <div class="dropdown dropdown-end">
                        <div tabindex="0" role="button" class="btn btn-ghost btn-circle avatar">
                            <div class="w-10 rounded-full bg-primary text-primary-content flex items-center justify-center">
                                <span class="text-lg">"U"</span>
                            </div>
                        </div>
                    </div>
                </div>
            </div>

            // Main content area with sidebar
            <div class="drawer lg:drawer-open">
                <input id="sidebar-drawer" type="checkbox" class="drawer-toggle" />
                <div class="drawer-content p-6">
                    <div class="max-w-4xl mx-auto">
                        {children()}
                    </div>
                </div>

                // Sidebar
                <div class="drawer-side">
                    <label for="sidebar-drawer" aria-label="close sidebar" class="drawer-overlay"></label>
                    <ul class="menu bg-base-100 min-h-full w-64 p-4 text-base-content">
                        <li class="menu-title">"Documentation"</li>
                        <li><a href="/">"🏠 Home"</a></li>
                        <li><a href="/docs/getting-started">"🚀 Getting Started"</a></li>
                        <li><a href="/docs/architecture">"🏗️ Architecture"</a></li>
                        <li class="menu-title">"API Schemas"</li>
                        <li><a href="/docs/api-reference">"📡 API Reference"</a></li>
                    </ul>
                </div>
            </div>
        </div>
    }
}

/// Home page component.
#[component]
fn HomePage() -> impl IntoView {
    view! {
        <div class="hero min-h-[60vh]">
            <div class="hero-content text-center">
                <div class="max-w-2xl">
                    <h1 class="text-5xl font-bold">"Welcome to Lekton"</h1>
                    <p class="py-6 text-lg text-base-content/70">
                        "Your dynamic Internal Developer Portal. Search documentation, explore API schemas, and collaborate — all in one place."
                    </p>
                    <div class="flex gap-4 justify-center">
                        <a href="/docs/getting-started" class="btn btn-primary btn-lg">
                            "Get Started"
                        </a>
                        <a href="/docs/api-reference" class="btn btn-outline btn-lg">
                            "API Schemas"
                        </a>
                    </div>
                </div>
            </div>
        </div>

        // Feature cards
        <div class="grid grid-cols-1 md:grid-cols-3 gap-6 mt-8">
            <FeatureCard
                title="Dynamic Docs"
                description="CI/CD integration for live documentation updates. No rebuilds needed."
                icon="📝"
            />
            <FeatureCard
                title="Granular RBAC"
                description="Role-based access control ensures sensitive docs are only visible to authorized users."
                icon="🔒"
            />
            <FeatureCard
                title="Schema Registry"
                description="Unified OpenAPI, AsyncAPI, and JSON Schema viewer with versioning."
                icon="📡"
            />
        </div>
    }
}

/// A feature card component for the home page.
#[component]
fn FeatureCard(title: &'static str, description: &'static str, icon: &'static str) -> impl IntoView {
    view! {
        <div class="card bg-base-100 shadow-xl hover:shadow-2xl transition-shadow">
            <div class="card-body items-center text-center">
                <span class="text-4xl">{icon}</span>
                <h2 class="card-title">{title}</h2>
                <p class="text-base-content/70">{description}</p>
            </div>
        </div>
    }
}

/// Document viewer page — renders markdown content from the server.
#[component]
fn DocPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();

    let slug = move || {
        params.read().get("slug").unwrap_or_default()
    };

    // For Phase 1 (MVP), render a placeholder.
    // In production, this will fetch from the server via a server function.
    let content = move || {
        let current_slug = slug();
        let sample_md = format!(
            "# {}\n\nThis is the documentation page for `{}`.\n\n\
             > **Note:** In the full implementation, this content will be \
             fetched from S3 via a Leptos server function.\n\n\
             ## Features\n\n\
             - Dynamic content loading\n\
             - Role-based access control\n\
             - Version history\n",
            current_slug, current_slug
        );
        render_markdown(&sample_md)
    };

    view! {
        <article class="prose prose-lg max-w-none">
            <div inner_html=content />
        </article>
    }
}

/// 404 Not Found page.
#[component]
fn NotFound() -> impl IntoView {
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
