use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::components::*;
use leptos_router::path;
use serde::{Deserialize, Serialize};

use crate::editor::component::EditorPage;
use crate::rendering::markdown::render_markdown;
use crate::search::client::SearchHit;

/// Shared application state (server-side only).
#[cfg(feature = "ssr")]
use std::sync::Arc;

#[cfg(feature = "ssr")]
#[derive(Clone, axum::extract::FromRef)]
pub struct AppState {
    pub document_repo: Arc<dyn crate::db::repository::DocumentRepository>,
    pub schema_repo: Arc<dyn crate::db::schema_repository::SchemaRepository>,
    pub storage_client: Arc<dyn crate::storage::client::StorageClient>,
    pub search_service: Option<Arc<dyn crate::search::client::SearchService>>,
    pub service_token: String,
    pub demo_mode: bool,
    pub leptos_options: LeptosOptions,
}

/// The HTML shell for the application.
#[cfg(feature = "ssr")]
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en" data-theme="light">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <AutoReload options=options.clone() />
                <HydrationScripts options=options />
                <Meta name="description" content="Lekton: A dynamic, high-performance Internal Developer Portal with RBAC and unified schema registry." />
                <Stylesheet id="leptos" href="/pkg/lekton.css" />
                <Link rel="stylesheet" href="/custom.css" />
                <script type_="module" src="/js/tiptap-bundle.min.js"></script>
                <script type_="module" src="/js/tiptap.js"></script>
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

/// Simplified document info for navigation tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavItem {
    pub slug: String,
    pub title: String,
    pub parent_slug: Option<String>,
    pub order: u32,
    pub children: Vec<NavItem>,
}

/// Server function to search documents.
#[server(SearchDocs, "/api")]
pub async fn search_docs(
    query: String,
) -> Result<Vec<SearchHit>, ServerFnError> {
    use crate::auth::models::AccessLevel;
    use crate::search::client::SearchService;

    let state = expect_context::<AppState>();

    let search_service = state.search_service.as_ref()
        .ok_or_else(|| ServerFnError::new("Search not available"))?;

    // Default to developer access for now; a full implementation would
    // use the authenticated user's access level.
    let results = search_service.search(&query, AccessLevel::Developer).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(results)
}

/// Server function to fetch navigation tree.
#[server(GetNavigation, "/api")]
pub async fn get_navigation() -> Result<Vec<NavItem>, ServerFnError> {
    use crate::auth::models::AccessLevel;
    use crate::db::repository::DocumentRepository;
    use std::collections::HashMap;

    let state = expect_context::<AppState>();

    // Fetch all accessible documents (default to developer access)
    let docs = state.document_repo.list_accessible(AccessLevel::Developer).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Create all nav items first
    let all_items: Vec<NavItem> = docs.into_iter().map(|doc| NavItem {
        slug: doc.slug,
        title: doc.title,
        parent_slug: doc.parent_slug,
        order: doc.order,
        children: vec![],
    }).collect();

    // Build a map for quick parent lookup
    let mut items_by_slug: HashMap<String, NavItem> = all_items.into_iter()
        .map(|item| (item.slug.clone(), item))
        .collect();

    // Separate root items and child items
    let mut roots = Vec::new();
    let mut children_by_parent: HashMap<String, Vec<NavItem>> = HashMap::new();

    for (slug, item) in items_by_slug.iter() {
        if let Some(parent) = &item.parent_slug {
            children_by_parent.entry(parent.clone())
                .or_insert_with(Vec::new)
                .push(item.clone());
        } else {
            roots.push(item.clone());
        }
    }

    // Recursively attach children to parents
    fn attach_children(item: &mut NavItem, children_map: &HashMap<String, Vec<NavItem>>) {
        if let Some(children) = children_map.get(&item.slug) {
            item.children = children.clone();
            for child in &mut item.children {
                attach_children(child, children_map);
            }
        }
    }

    for root in &mut roots {
        attach_children(root, &children_by_parent);
    }

    Ok(roots)
}

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Title text="Lekton — Internal Developer Portal" />

        <Router>
            <Layout>
                <Routes fallback=|| view! { <NotFound /> }>
                    <Route path=path!("/") view=HomePage />
                    <Route path=path!("/login") view=LoginPage />
                    <Route path=path!("/docs/:slug") view=DocPage />
                    <Route path=path!("/edit/:slug") view=EditorPage />
                </Routes>
            </Layout>
        </Router>
    }
}

/// Recursive navigation item component for rendering tree structure.
#[component]
fn NavigationItem(item: NavItem, #[prop(optional)] level: u32) -> impl IntoView {
    let has_children = !item.children.is_empty();
    let slug = item.slug.clone();
    let children = item.children.clone();
    
    if has_children {
        // Parent item with collapsible children using DaisyUI collapse
        view! {
            <li>
                <details>
                    <summary>{item.title}</summary>
                    <ul>
                        {children.into_iter().map(|child| {
                            view! {
                                <NavigationItem item=child level=level + 1 />
                            }
                        }).collect::<Vec<_>>()}
                    </ul>
                </details>
            </li>
        }.into_any()
    } else {
        // Leaf item with direct link
        view! {
            <li>
                <a href=format!("/docs/{}", slug)>{item.title}</a>
            </li>
        }.into_any()
    }
}

/// Navigation tree component that fetches and renders the sidebar navigation.
#[component]
fn NavigationTree() -> impl IntoView {
    let nav_resource = Resource::new(
        || (), // No dependencies, fetch once
        |_| get_navigation(),
    );

    view! {
        <Suspense fallback=move || view! {
            <li><span class="loading loading-spinner loading-sm"></span></li>
        }>
            {move || {
                nav_resource.get().map(|result| match result {
                    Ok(items) => {
                        view! {
                            {items.into_iter().map(|item| {
                                view! {
                                    <NavigationItem item=item level=0 />
                                }
                            }).collect::<Vec<_>>()}
                        }.into_any()
                    }
                    Err(e) => {
                        view! {
                            <li class="text-error">{format!("Error loading navigation: {}", e)}</li>
                        }.into_any()
                    }
                })
            }}
        </Suspense>
    }
}

/// Main layout: navbar + sidebar + content area.
#[component]
fn Layout(children: Children) -> impl IntoView {
    let (search_modal_open, set_search_modal_open) = signal(false);

    // Global keyboard listener for Ctrl+K / Cmd+K at document level
    use leptos::ev;
    window_event_listener(ev::keydown, move |ev| {
        if (ev.ctrl_key() || ev.meta_key()) && ev.key() == "k" {
            ev.prevent_default();
            ev.stop_propagation();
            set_search_modal_open.set(true);
        }
    });

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
                    // Search button that opens modal
                    <button 
                        class="btn btn-ghost btn-sm gap-2"
                        on:click=move |_| set_search_modal_open.set(true)
                    >
                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path>
                        </svg>
                        <span class="hidden md:inline">"Search"</span>
                        <kbd class="kbd kbd-xs hidden md:inline">"Ctrl+K"</kbd>
                    </button>
                    // User area — shows login button or user info
                    <div id="user-area" class="flex items-center gap-2">
                        <a href="/login" class="btn btn-primary btn-sm">"Login"</a>
                    </div>
                </div>
            </div>

            // Global search modal
            <SearchModal is_open=search_modal_open set_is_open=set_search_modal_open />

            // Main content area with sidebar
            <div class="drawer lg:drawer-open">
                <input id="sidebar-drawer" type="checkbox" class="drawer-toggle" />
                <div class="drawer-content p-6">
                    <div class="max-w-7xl mx-auto">
                        {children()}
                    </div>
                </div>

                // Sidebar
                <div class="drawer-side">
                    <label for="sidebar-drawer" aria-label="close sidebar" class="drawer-overlay"></label>
                    <ul class="menu bg-base-100 min-h-full w-64 p-4 text-base-content">
                        <li class="menu-title">"Documentation"</li>
                        <li><a href="/">"🏠 Home"</a></li>
                        <NavigationTree />
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

/// Login page for demo mode.
#[component]
fn LoginPage() -> impl IntoView {
    view! {
        <div class="hero min-h-[60vh]">
            <div class="hero-content">
                <div class="card bg-base-100 shadow-2xl w-full max-w-md">
                    <div class="card-body">
                        <h2 class="card-title text-2xl justify-center">"🔐 Demo Login"</h2>
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

        // Client-side login JavaScript
        <script>
            r###"
            document.getElementById('login-form').addEventListener('submit', async (e) => {
                e.preventDefault();
                const username = document.getElementById('login-username').value;
                const password = document.getElementById('login-password').value;
                const errorEl = document.getElementById('login-error');

                try {
                    const res = await fetch('/api/auth/login', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ username, password })
                    });
                    if (res.ok) {
                        window.location.href = '/';
                    } else {
                        errorEl.classList.remove('hidden');
                    }
                } catch (err) {
                    errorEl.classList.remove('hidden');
                }
            });
            "###
        </script>
    }
}


/// Server function to fetch a document's rendered HTML content and TOC headings.
#[server(GetDocHtml, "/api")]
pub async fn get_doc_html(
    slug: String,
) -> Result<Option<(String, String, Vec<crate::rendering::markdown::TocHeading>)>, ServerFnError> {
    use crate::db::repository::DocumentRepository;
    use crate::storage::client::StorageClient;
    use crate::rendering::markdown::{extract_headings, render_markdown};

    let state = expect_context::<AppState>();

    let doc = state.document_repo.find_by_slug(&slug).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(doc) = doc else {
        return Ok(None);
    };

    let content_bytes = state.storage_client.get_object(&doc.s3_key).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(content_bytes) = content_bytes else {
        return Ok(None);
    };

    let raw = String::from_utf8(content_bytes)
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let html = render_markdown(&raw);
    let headings = extract_headings(&raw);
    
    Ok(Some((doc.title, html, headings)))
}

/// Breadcrumbs component to show document hierarchy based on slug.
#[component]
fn Breadcrumbs(slug: String) -> impl IntoView {
    // Parse slug like "engineering/deployment-guide" into breadcrumb trail
    let parts: Vec<&str> = slug.split('/').collect();
    
    // Build breadcrumb items
    let breadcrumb_items: Vec<_> = parts.iter().enumerate().map(|(idx, part)| {
        let is_last = idx == parts.len() - 1;
        let path = parts[..=idx].join("/");
        let label = part.split('-')
            .map(|word| {
                let mut c = word.chars();
                match c.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        
        (path, label, is_last)
    }).collect();
    
    view! {
        <div class="breadcrumbs text-sm mb-4">
            <ul>
                <li>
                    <a href="/" class="hover:underline">"Docs"</a>
                </li>
                {breadcrumb_items.into_iter().map(|(path, label, is_last)| {
                    if is_last {
                        view! {
                            <li>{label}</li>
                        }.into_any()
                    } else {
                        view! {
                            <li>
                                <a href=format!("/docs/{}", path) class="hover:underline">{label}</a>
                            </li>
                        }.into_any()
                    }
                }).collect::<Vec<_>>()}
            </ul>
        </div>
    }
}

/// Table of Contents component for document navigation.
#[component]
fn TableOfContents(headings: Vec<crate::rendering::markdown::TocHeading>) -> impl IntoView {
    if headings.is_empty() {
        return view! {
            <div></div>
        }.into_any();
    }

    view! {
        <nav class="sticky top-20 hidden xl:block w-64 ml-8">
            <div class="text-sm font-semibold mb-4">"On This Page"</div>
            <ul class="space-y-2 text-sm">
                {headings.into_iter().map(|heading| {
                    let indent_class = if heading.level == 3 {
                        "ml-4"
                    } else {
                        ""
                    };
                    let href = format!("#{}", heading.id);
                    
                    view! {
                        <li class=indent_class>
                            <a 
                                href=href
                                class="text-base-content/70 hover:text-primary transition-colors"
                            >
                                {heading.text}
                            </a>
                        </li>
                    }
                }).collect::<Vec<_>>()}
            </ul>
        </nav>
    }.into_any()
}

/// Document viewer page — renders markdown content fetched from S3.
#[component]
fn DocPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let slug = move || params.read().get("slug").unwrap_or_default();

    let doc_resource = Resource::new(
        move || slug(),
        |slug| get_doc_html(slug),
    );

    view! {
        <Suspense fallback=move || view! {
            <div class="flex justify-center py-12">
                <span class="loading loading-spinner loading-lg"></span>
            </div>
        }>
            {move || {
                doc_resource.get().map(|result| match result {
                    Ok(Some((title, html, headings))) => {
                        let current_slug = slug();
                        view! {
                            <div class="flex gap-8">
                                <div class="flex-1 min-w-0">
                                    <Breadcrumbs slug=current_slug.clone() />
                                    <div class="flex justify-between items-center mb-6">
                                        <h1 class="text-3xl font-bold">{title}</h1>
                                        <a
                                            href=move || format!("/edit/{}", current_slug)
                                            class="btn btn-outline btn-sm"
                                        >
                                            "Edit"
                                        </a>
                                    </div>
                                    <article class="prose prose-lg max-w-none">
                                        <div inner_html=html />
                                    </article>
                                </div>
                                <TableOfContents headings=headings />
                            </div>
                        }.into_any()
                    }
                    Ok(None) => {
                        view! {
                            <div class="alert alert-warning">
                                <span>{format!("Document '{}' not found.", slug())}</span>
                            </div>
                        }.into_any()
                    }
                    Err(e) => {
                        view! {
                            <div class="alert alert-error">
                                <span>{format!("Error loading document: {e}")}</span>
                            </div>
                        }.into_any()
                    }
                })
            }}
        </Suspense>
    }
}

/// Global search modal triggered by Ctrl+K (or Cmd+K on Mac).
#[component]
fn SearchModal(is_open: ReadSignal<bool>, set_is_open: WriteSignal<bool>) -> impl IntoView {
    let (query, set_query) = signal(String::new());
    
    let search_resource = Resource::new(
        move || query.get(),
        |q| async move {
            if q.len() < 2 {
                return Ok(vec![]);
            }
            search_docs(q).await
        },
    );

    // Close modal on Escape key
    let on_keydown = move |ev: leptos::web_sys::KeyboardEvent| {
        if ev.key() == "Escape" {
            set_is_open.set(false);
        }
    };

    view! {
        <Show when=move || is_open.get()>
            <div 
                class="fixed inset-0 z-[200] flex items-start justify-center pt-20 bg-black/50 backdrop-blur-sm"
                on:click=move |_| set_is_open.set(false)
            >
                <div 
                    class="bg-base-100 rounded-lg shadow-2xl w-full max-w-2xl mx-4"
                    on:click=move |ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                >
                    // Search input
                    <div class="p-4 border-b border-base-300">
                        <div class="flex items-center gap-3">
                            <svg class="w-5 h-5 text-base-content/50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path>
                            </svg>
                            <input
                                type="text"
                                placeholder="Search documentation..."
                                class="input input-ghost w-full focus:outline-none text-lg"
                                prop:value=query
                                on:input=move |ev| {
                                    set_query.set(event_target_value(&ev));
                                }
                                on:keydown=on_keydown
                                autofocus
                            />
                            <kbd class="kbd kbd-sm">ESC</kbd>
                        </div>
                    </div>

                    // Results area
                    <div class="max-h-96 overflow-y-auto">
                        <Suspense fallback=move || view! {
                            <div class="flex justify-center p-8">
                                <span class="loading loading-spinner loading-lg"></span>
                            </div>
                        }>
                            {move || {
                                let q = query.get();
                                if q.len() < 2 {
                                    return Some(view! {
                                        <div class="p-8 text-center text-base-content/50">
                                            "Type at least 2 characters to search..."
                                        </div>
                                    }.into_any());
                                }

                                search_resource.get().map(|result| match result {
                                    Ok(hits) if hits.is_empty() => {
                                        view! {
                                            <div class="p-8 text-center text-base-content/50">
                                                "No results found for \"" {q.clone()} "\""
                                            </div>
                                        }.into_any()
                                    }
                                    Ok(hits) => {
                                        view! {
                                            <div class="divide-y divide-base-300">
                                                {hits.into_iter().map(|hit| {
                                                    let slug = hit.slug.clone();
                                                    let title = hit.title.clone();
                                                    let preview = hit.content_preview.clone();
                                                    let tags = hit.tags.clone();
                                                    let has_tags = !tags.is_empty();
                                                    
                                                    view! {
                                                        <a
                                                            href=format!("/docs/{}", slug)
                                                            class="block p-4 hover:bg-base-200 transition-colors"
                                                            on:click=move |_| set_is_open.set(false)
                                                        >
                                                            <div class="font-semibold text-lg mb-1">{title}</div>
                                                            <div class="text-sm text-base-content/70 mb-2">{preview}</div>
                                                            <Show when=move || has_tags>
                                                                <div class="flex gap-2 flex-wrap">
                                                                    {tags.iter().map(|tag| {
                                                                        let tag_text = tag.clone();
                                                                        view! {
                                                                            <span class="badge badge-sm badge-outline">{tag_text}</span>
                                                                        }
                                                                    }).collect::<Vec<_>>()}
                                                                </div>
                                                            </Show>
                                                        </a>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        }.into_any()
                                    }
                                    Err(e) => {
                                        view! {
                                            <div class="p-8 text-center text-error">
                                                "Search error: " {e.to_string()}
                                            </div>
                                        }.into_any()
                                    }
                                })
                            }}
                        </Suspense>
                    </div>

                    // Footer with keyboard hints
                    <div class="p-3 border-t border-base-300 bg-base-200/50 rounded-b-lg">
                        <div class="flex items-center justify-between text-xs text-base-content/50">
                            <div class="flex items-center gap-4">
                                <span>"Press ESC to close"</span>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </Show>
    }
}

/// Search bar component with live results dropdown.
#[component]
fn SearchBar() -> impl IntoView {
    let (query, set_query) = signal(String::new());
    let (show_results, set_show_results) = signal(false);

    let search_resource = Resource::new(
        move || query.get(),
        |q| async move {
            if q.len() < 2 {
                return Ok(vec![]);
            }
            search_docs(q).await
        },
    );

    view! {
        <div class="dropdown dropdown-end">
            <div class="form-control">
                <input
                    type="text"
                    placeholder="Search docs..."
                    class="input input-bordered w-24 md:w-64"
                    prop:value=query
                    on:input=move |ev| {
                        let val = event_target_value(&ev);
                        set_query.set(val.clone());
                        set_show_results.set(val.len() >= 2);
                    }
                    on:focus=move |_| {
                        if query.get().len() >= 2 {
                            set_show_results.set(true);
                        }
                    }
                />
            </div>
            <Show when=move || show_results.get()>
                <ul class="dropdown-content menu bg-base-100 rounded-box z-[100] w-80 p-2 shadow-lg mt-2 max-h-80 overflow-y-auto">
                    <Suspense fallback=move || view! { <li><span class="loading loading-spinner loading-sm"></span></li> }>
                        {move || {
                            search_resource.get().map(|result| match result {
                                Ok(hits) if hits.is_empty() => {
                                    view! {
                                        <li class="text-base-content/50 p-2">"No results found"</li>
                                    }.into_any()
                                }
                                Ok(hits) => {
                                    view! {
                                        {hits.into_iter().map(|hit| {
                                            let slug = hit.slug.clone();
                                            view! {
                                                <li>
                                                    <a href=format!("/docs/{}", slug) class="flex flex-col items-start">
                                                        <span class="font-semibold">{hit.title}</span>
                                                        <span class="text-xs text-base-content/50 truncate w-full">
                                                            {hit.content_preview}
                                                        </span>
                                                    </a>
                                                </li>
                                            }
                                        }).collect::<Vec<_>>()}
                                    }.into_any()
                                }
                                Err(_) => {
                                    view! {
                                        <li class="text-error p-2">"Search error"</li>
                                    }.into_any()
                                }
                            })
                        }}
                    </Suspense>
                </ul>
            </Show>
        </div>
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
