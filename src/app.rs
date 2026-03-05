use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::components::*;
use leptos_router::path;
use serde::{Deserialize, Serialize};

use crate::editor::component::EditorPage;
use crate::rendering::markdown::render_markdown;
use crate::schema::component::{SchemaListPage, SchemaViewerPage};
use crate::search::client::SearchHit;

/// Shared application state (server-side only).
#[cfg(feature = "ssr")]
use std::sync::Arc;

#[cfg(feature = "ssr")]
#[derive(Clone, axum::extract::FromRef)]
pub struct AppState {
    pub document_repo: Arc<dyn crate::db::repository::DocumentRepository>,
    pub schema_repo: Arc<dyn crate::db::schema_repository::SchemaRepository>,
    pub settings_repo: Arc<dyn crate::db::settings_repository::SettingsRepository>,
    pub asset_repo: Arc<dyn crate::db::asset_repository::AssetRepository>,
    pub storage_client: Arc<dyn crate::storage::client::StorageClient>,
    pub search_service: Option<Arc<dyn crate::search::client::SearchService>>,
    pub service_token: String,
    pub demo_mode: bool,
    pub leptos_options: LeptosOptions,
    // ── Auth (phase 5) ────────────────────────────────────────────────────────
    pub user_repo: Arc<dyn crate::db::user_repository::UserRepository>,
    pub access_level_repo: Arc<dyn crate::db::access_level_repository::AccessLevelRepository>,
    pub token_service: Arc<crate::auth::token_service::TokenService>,
    pub auth_provider: Option<Arc<dyn crate::auth::provider::AuthProvider>>,
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
                // Inline script to apply saved theme before first paint (prevents FOUC)
                <script>
                    r#"(function(){var t=localStorage.getItem('lekton-theme');if(t==='dark'||t==='light'){document.documentElement.setAttribute('data-theme',t)}else{var d=window.matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light';document.documentElement.setAttribute('data-theme',d)}})()"#
                </script>
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

/// Helper to derive `(allowed_levels, include_draft)` from the current
/// request's JWT cookie.  Anonymous callers get public-only, non-draft access.
///
/// Uses `CookieJar` (state-free extractor) and validates the token directly
/// via `state.token_service` — `OptionalAuthUser` cannot be used inside Leptos
/// server functions because `leptos_axum::extract()` uses an empty `()` state.
#[cfg(feature = "ssr")]
async fn request_document_visibility(
    state: &AppState,
) -> Result<(Option<Vec<String>>, bool), ServerFnError> {
    use axum_extra::extract::CookieJar;
    use crate::auth::extractor::ACCESS_TOKEN_COOKIE;
    use crate::auth::models::UserContext;
    use crate::auth::token_service::TokenService;

    let jar: CookieJar = leptos_axum::extract().await?;

    // Try JWT access token first (normal OAuth2/OIDC flow).
    let maybe_user = jar
        .get(ACCESS_TOKEN_COOKIE)
        .and_then(|c| state.token_service.validate_access_token(c.value()).ok())
        .map(|claims| TokenService::claims_to_user(&claims));

    if let Some(auth_user) = maybe_user {
        let perms = state
            .user_repo
            .get_permissions(&auth_user.user_id)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        return Ok(UserContext { user: auth_user, permissions: perms }.document_visibility());
    }

    // Fall back to demo session cookie when demo mode is active.
    if state.demo_mode {
        if let Some(cookie) = jar.get("lekton_demo_user") {
            if let Ok(demo_user) = serde_json::from_str::<crate::auth::models::AuthenticatedUser>(cookie.value()) {
                // Admins see everything; non-admins are treated as public readers.
                if demo_user.is_admin {
                    return Ok((None, true));
                } else {
                    return Ok((Some(vec!["public".to_string()]), false));
                }
            }
        }
    }

    // Anonymous access: public, non-draft only.
    Ok((Some(vec!["public".to_string()]), false))
}

/// Server function to search documents.
#[server(SearchDocs, "/api")]
pub async fn search_docs(
    query: String,
) -> Result<Vec<SearchHit>, ServerFnError> {
    use crate::search::client::SearchService;

    let state = expect_context::<AppState>();

    let search_service = state.search_service.as_ref()
        .ok_or_else(|| ServerFnError::new("Search not available"))?;

    let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
    let results = search_service
        .search(&query, allowed_levels.as_deref(), include_draft)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(results)
}

/// Server function to fetch navigation tree.
#[server(GetNavigation, "/api")]
pub async fn get_navigation() -> Result<Vec<NavItem>, ServerFnError> {
    use crate::db::repository::DocumentRepository;
    use std::collections::HashMap;

    let state = expect_context::<AppState>();

    let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
    let docs = state.document_repo
        .list_by_access_levels(allowed_levels.as_deref(), include_draft)
        .await
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

/// Server function to get the currently authenticated user from the JWT cookie.
///
/// Returns `None` for anonymous requests.  Both demo mode (session cookie)
/// and production OIDC mode (JWT cookie) are handled transparently.
#[server(GetCurrentUser, "/api")]
pub async fn get_current_user() -> Result<Option<crate::auth::models::AuthenticatedUser>, ServerFnError> {
    use axum_extra::extract::CookieJar;
    use crate::auth::extractor::ACCESS_TOKEN_COOKIE;
    use crate::auth::token_service::TokenService;

    let state = expect_context::<AppState>();
    let jar: CookieJar = leptos_axum::extract().await?;

    // Try production JWT first
    if let Some(token_user) = jar
        .get(ACCESS_TOKEN_COOKIE)
        .and_then(|c| state.token_service.validate_access_token(c.value()).ok())
        .map(|claims| TokenService::claims_to_user(&claims))
    {
        return Ok(Some(token_user));
    }

    // Fall back to demo mode session cookie
    if state.demo_mode {
        if let Some(cookie) = jar.get("lekton_demo_user") {
            if let Ok(user) =
                serde_json::from_str::<crate::auth::models::AuthenticatedUser>(cookie.value())
            {
                return Ok(Some(user));
            }
        }
    }

    Ok(None)
}

/// Server function to log out the current user.
///
/// Clears the JWT and refresh token cookies (or the demo session cookie in
/// demo mode).  Revocation of the refresh token is handled client-side via
/// the `/auth/logout` endpoint.
#[server(LogoutUser, "/api")]
pub async fn logout_user() -> Result<(), ServerFnError> {
    use leptos_axum::ResponseOptions;

    let state = expect_context::<AppState>();
    let response = expect_context::<ResponseOptions>();

    let clear_cookie = |name: &str, path: &str| -> String {
        format!("{name}=; Path={path}; HttpOnly; SameSite=Lax; Max-Age=0")
    };

    if state.demo_mode {
        response.append_header(
            axum::http::header::SET_COOKIE,
            axum::http::HeaderValue::from_str(&clear_cookie("lekton_demo_user", "/")).unwrap(),
        );
    } else {
        response.append_header(
            axum::http::header::SET_COOKIE,
            axum::http::HeaderValue::from_str(&clear_cookie("lekton_access_token", "/")).unwrap(),
        );
        response.append_header(
            axum::http::header::SET_COOKIE,
            axum::http::HeaderValue::from_str(&clear_cookie("lekton_refresh_token", "/auth/refresh")).unwrap(),
        );
    }

    Ok(())
}

/// Server function to get the current custom CSS.
#[server(GetCustomCss, "/api")]
pub async fn get_custom_css() -> Result<String, ServerFnError> {
    let state = expect_context::<AppState>();
    let settings = state.settings_repo.get_settings().await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(settings.custom_css)
}

/// Server function to save custom CSS (admin only).
#[server(SaveCustomCss, "/api")]
pub async fn save_custom_css(css: String) -> Result<String, ServerFnError> {
    let state = expect_context::<AppState>();
    state.settings_repo.set_custom_css(&css).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok("Custom CSS saved successfully".to_string())
}

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    // Load the current user on the client side only (LocalResource skips SSR).
    // Using LocalResource avoids hydration mismatches: the server cannot read
    // the browser's httpOnly cookies, so we let the client fetch this on hydration.
    let user_resource = LocalResource::new(get_current_user);

    // Derive a flat signal: None = anonymous (or loading), Some(user) = authenticated.
    let current_user: Signal<Option<crate::auth::models::AuthenticatedUser>> =
        Signal::derive(move || {
            user_resource.get().and_then(|res| res.ok()).flatten()
        });

    provide_context(current_user);

    view! {
        <Title text="Lekton — Internal Developer Portal" />

        <Router>
            <Layout>
                <Routes fallback=|| view! { <NotFound /> }>
                    <Route path=path!("/") view=HomePage />
                    <Route path=path!("/login") view=LoginPage />
                    <Route path=path!("/docs/:slug") view=DocPage />
                    <Route path=path!("/edit/:slug") view=EditorPage />
                    <Route path=path!("/schemas") view=SchemaListPage />
                    <Route path=path!("/schemas/:name") view=SchemaViewerPage />
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
                <details open=true>
                    <summary class="hover:bg-base-200/50 transition-colors font-medium text-base-content/80 text-sm hover:text-base-content">{item.title}</summary>
                    <ul class="before:w-[1px] before:bg-base-300 ml-2 border-l border-base-200/50 mt-1">
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
                <a 
                    href=format!("/docs/{}", slug)
                    class="hover:bg-base-200/50 hover:text-primary transition-colors text-base-content/70 data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium text-sm py-1.5"
                >
                    {item.title}
                </a>
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

/// Runtime custom CSS component — injects user-defined CSS from settings.
#[component]
fn RuntimeCustomCss() -> impl IntoView {
    let css_resource = Resource::new(|| (), |_| get_custom_css());

    view! {
        <Suspense fallback=|| ()>
            {move || {
                css_resource.get().map(|result| match result {
                    Ok(css) if !css.is_empty() => {
                        view! {
                            <style>{css}</style>
                        }.into_any()
                    }
                    _ => view! { <span /> }.into_any(),
                })
            }}
        </Suspense>
    }
}

/// User menu in the navbar: shows login link for anonymous users, or a
/// dropdown with the user's email and a logout button when authenticated.
#[component]
fn UserMenu() -> impl IntoView {
    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>()
        .expect("UserMenu must be inside App");

    let logout_action = Action::new(|_: &()| async move {
        let _ = logout_user().await;
        // Force a full page reload so the cookie change is reflected
        #[cfg(feature = "hydrate")]
        {
            use leptos::web_sys::window;
            if let Some(w) = window() {
                let _ = w.location().assign("/");
            }
        }
    });

    view! {
        {move || {
            match current_user.get() {
                Some(user) => {
                    let display = user.name.clone().unwrap_or_else(|| user.email.clone());
                    let is_admin = user.is_admin;
                    view! {
                        <div class="dropdown dropdown-end">
                            <div tabindex="0" role="button" class="btn btn-ghost btn-sm gap-2 font-medium">
                                <span class="truncate max-w-[120px]">{display}</span>
                                {if is_admin {
                                    view! { <span class="badge badge-error badge-xs">"Admin"</span> }.into_any()
                                } else {
                                    view! { <span /> }.into_any()
                                }}
                            </div>
                            <ul tabindex="0" class="dropdown-content menu bg-base-100 rounded-box z-[1] w-52 p-2 shadow border border-base-200 mt-2">
                                <li class="menu-title text-xs opacity-60 px-2 pb-1 truncate">{user.email.clone()}</li>
                                <div class="divider my-1"></div>
                                <li>
                                    <button
                                        class="text-error"
                                        on:click=move |_| { logout_action.dispatch(()); }
                                    >
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1" />
                                        </svg>
                                        "Log Out"
                                    </button>
                                </li>
                            </ul>
                        </div>
                    }.into_any()
                }
                None => view! {
                    <a href="/login" class="btn btn-ghost btn-sm font-medium whitespace-nowrap">"Log In"</a>
                }.into_any(),
            }
        }}
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
        // Runtime custom CSS injection (loaded from MongoDB settings)
        <RuntimeCustomCss />

        <div class="min-h-screen bg-base-100/50">
            // Navbar
            <header class="bg-base-100/80 backdrop-blur-md fixed top-0 inset-x-0 z-50 border-b border-base-200 px-4 h-16 flex items-center justify-between shadow-sm">
                // Left
                <div class="flex items-center gap-2 z-10">
                    <label for="sidebar-drawer" class="btn btn-square btn-ghost drawer-button lg:hidden">
                        <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" class="inline-block w-5 h-5 stroke-current"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h16"></path></svg>
                    </label>
                    <a class="flex items-center gap-2 text-xl font-bold tracking-tight hover:opacity-80 transition-opacity" href="/">
                        <svg class="w-6 h-6 text-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L2 7l10 5 10-5-10-5Z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>
                        <span class="truncate">"Lekton"</span>
                    </a>
                </div>
                // Center (Absolutey Centered)
                <div class="hidden sm:flex absolute inset-0 pointer-events-none items-center justify-center">
                    <div class="w-full max-w-md px-4 pointer-events-auto">
                        <button 
                            class="btn btn-ghost bg-base-200/50 hover:bg-base-200 border border-base-300 hover:border-base-content/20 w-full justify-between shadow-sm flex-nowrap h-10 min-h-10 px-3 transition-colors font-normal text-base-content/70"
                            on:click=move |_| set_search_modal_open.set(true)
                        >
                            <div class="flex items-center gap-2 overflow-hidden">
                                <svg class="w-4 h-4 opacity-70 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path>
                                </svg>
                                <span class="truncate">"Search documentation..."</span>
                            </div>
                            <kbd class="kbd kbd-sm bg-base-100 border-none shadow-sm opacity-80 flex-shrink-0">"Ctrl K"</kbd>
                        </button>
                    </div>
                </div>
                // Right
                <div class="flex items-center gap-2 z-10 flex-nowrap shrink-0">
                    // Mobile search icon
                    <button class="btn btn-circle btn-ghost sm:hidden" on:click=move |_| set_search_modal_open.set(true)>
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path></svg>
                    </button>
                    // Theme toggle
                    <ThemeToggle />
                    // User area — shows login button or user info
                    <UserMenu />
                </div>
            </header>

            // Global search modal
            <SearchModal is_open=search_modal_open set_is_open=set_search_modal_open />

            // Main content area with sidebar
            <div class="drawer lg:drawer-open pt-16">
                <input id="sidebar-drawer" type="checkbox" class="drawer-toggle" />
                <div class="drawer-content lg:col-start-2 flex flex-col items-center bg-base-100 min-w-0">
                    <div class="w-full max-w-6xl p-6 lg:p-10 min-h-[calc(100vh-4rem)]">
                        {children()}
                    </div>
                </div>

                // Sidebar
                <div class="drawer-side z-40">
                    <label for="sidebar-drawer" aria-label="close sidebar" class="drawer-overlay"></label>
                    <div class="menu bg-base-200 min-h-full h-[calc(100vh-4rem)] w-64 p-4 text-base-content border-r border-base-300 pt-6 overflow-y-auto block">
                        <ul class="flex flex-col gap-1">
                            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"Overview"</li>
                            <li>
                                <a href="/" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                                    <svg class="w-4 h-4 opacity-70 group-hover:opacity-100 transition-opacity" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg>
                                    "Home"
                                </a>
                            </li>
                        </ul>
                        <ul class="flex flex-col gap-1 mt-6">
                            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"Documentation"</li>
                            <NavigationTree />
                        </ul>
                        <ul class="flex flex-col gap-1 mt-6">
                            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"API Resources"</li>
                            <li>
                                <a href="/schemas" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                                    <svg class="w-4 h-4 opacity-70 group-hover:opacity-100 transition-opacity" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 7h10"/><path d="M7 12h10"/><path d="M7 17h10"/></svg>
                                    "Schema Registry"
                                </a>
                            </li>
                        </ul>
                    </div>
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


/// Data returned for rendering a document page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocPageData {
    pub title: String,
    pub html: String,
    pub headings: Vec<crate::rendering::markdown::TocHeading>,
    pub last_updated: String,
    pub tags: Vec<String>,
}

/// Server function to fetch a document's rendered HTML content, TOC, and metadata.
#[server(GetDocHtml, "/api")]
pub async fn get_doc_html(
    slug: String,
) -> Result<Option<DocPageData>, ServerFnError> {
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
    let last_updated = doc.last_updated.format("%B %d, %Y").to_string();

    Ok(Some(DocPageData {
        title: doc.title,
        html,
        headings,
        last_updated,
        tags: doc.tags,
    }))
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
                    Ok(Some(data)) => {
                        let current_slug = slug();
                        let has_tags = !data.tags.is_empty();
                        let tags = data.tags.clone();
                        let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
                        let can_edit = move || {
                            current_user
                                .and_then(|s| s.get())
                                .map(|u| u.is_admin)
                                .unwrap_or(false)
                        };
                        view! {
                            <div class="flex gap-8 items-start">
                                <div class="flex-1 min-w-0">
                                    <Breadcrumbs slug=current_slug.clone() />
                                    <div class="flex justify-between items-center mb-6">
                                        <h1 class="text-3xl font-bold">{data.title}</h1>
                                        <Show when=can_edit>
                                            <a
                                                href={let s = current_slug.clone(); move || format!("/edit/{}", s)}
                                                class="btn btn-outline btn-sm"
                                            >
                                                "Edit"
                                            </a>
                                        </Show>
                                    </div>
                                    // Tags
                                    <Show when=move || has_tags>
                                        <div class="flex flex-wrap gap-2 mb-6">
                                            {tags.iter().map(|tag| {
                                                let tag_text = tag.clone();
                                                view! {
                                                    <span class="badge badge-outline badge-sm">{tag_text}</span>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    </Show>
                                    <article class="prose prose-lg max-w-none">
                                        <div inner_html=data.html />
                                    </article>
                                    // Last Updated footer
                                    <div class="divider mt-12"></div>
                                    <div class="flex items-center gap-2 text-sm text-base-content/50 pb-4">
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z">
                                            </path>
                                        </svg>
                                        <span>"Last updated: " {data.last_updated}</span>
                                    </div>
                                </div>
                                <TableOfContents headings=data.headings />
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
                    <div class="p-4 border-b border-base-200 bg-base-100/50 rounded-t-lg">
                        <div class="flex items-center gap-3">
                            <svg class="w-6 h-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path>
                            </svg>
                            <input
                                type="text"
                                placeholder="Search documentation..."
                                class="w-full bg-transparent focus:outline-none text-xl placeholder:text-base-content/30"
                                prop:value=query
                                on:input=move |ev| {
                                    set_query.set(event_target_value(&ev));
                                }
                                on:keydown=on_keydown
                                autofocus
                            />
                            <kbd class="kbd kbd-sm bg-base-200 border-none shadow-sm text-xs font-semibold">"ESC"</kbd>
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

/// Theme toggle component — cycles through system/light/dark themes.
///
/// Persists choice in localStorage and applies it to the `<html>` element's `data-theme`.
/// Uses three states: "system" (follows OS preference), "light", and "dark".
#[component]
fn ThemeToggle() -> impl IntoView {
    let (theme, set_theme) = signal("system".to_string());

    // On mount (client-side), read the saved theme from localStorage
    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::prelude::*;

        // Read initial value
        let saved = js_sys::eval("localStorage.getItem('lekton-theme') || 'system'")
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| "system".to_string());
        set_theme.set(saved);
    }

    let cycle_theme = move |_| {
        let next = match theme.get().as_str() {
            "system" => "light",
            "light" => "dark",
            "dark" => "system",
            _ => "system",
        };
        set_theme.set(next.to_string());

        // Apply theme via JS — works on both SSR and hydrate
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::prelude::*;

            let js_code = format!(
                r#"(function(){{
                    var theme = '{}';
                    if (theme === 'system') {{
                        localStorage.removeItem('lekton-theme');
                        var actual = window.matchMedia('(prefers-color-scheme:dark)').matches ? 'dark' : 'light';
                        document.documentElement.setAttribute('data-theme', actual);
                    }} else {{
                        localStorage.setItem('lekton-theme', theme);
                        document.documentElement.setAttribute('data-theme', theme);
                    }}
                }})()"#,
                next
            );
            let _ = js_sys::eval(&js_code);
        }
    };

    view! {
        <div class="tooltip tooltip-bottom" data-tip=move || {
            match theme.get().as_str() {
                "light" => "Light mode (click for dark)",
                "dark" => "Dark mode (click for system)",
                _ => "System theme (click for light)",
            }
        }>
            <button
                class="btn btn-ghost btn-sm btn-square"
                on:click=cycle_theme
                aria-label="Toggle theme"
            >
                {move || match theme.get().as_str() {
                    "light" => view! {
                        // Sun icon
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z">
                            </path>
                        </svg>
                    }.into_any(),
                    "dark" => view! {
                        // Moon icon
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M20.354 15.354A9 9 0 018.646 3.646 9.003 9.003 0 0012 21a9.003 9.003 0 008.354-5.646z">
                            </path>
                        </svg>
                    }.into_any(),
                    _ => view! {
                        // Monitor icon (system)
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z">
                            </path>
                        </svg>
                    }.into_any(),
                }}
            </button>
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
