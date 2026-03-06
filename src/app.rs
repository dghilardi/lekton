use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::components::*;
use leptos_router::path;
use serde::{Deserialize, Serialize};

use crate::components::Layout;
use crate::editor::component::EditorPage;
use crate::pages::{DocPage, HomePage, LoginPage, NotFound};
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

    let all_items: Vec<NavItem> = docs.into_iter().map(|doc| NavItem {
        slug: doc.slug,
        title: doc.title,
        parent_slug: doc.parent_slug,
        order: doc.order,
        children: vec![],
    }).collect();

    let items_by_slug: HashMap<String, NavItem> = all_items.into_iter()
        .map(|item| (item.slug.clone(), item))
        .collect();

    let mut roots = Vec::new();
    let mut children_by_parent: HashMap<String, Vec<NavItem>> = HashMap::new();

    for (_slug, item) in items_by_slug.iter() {
        if let Some(parent) = &item.parent_slug {
            children_by_parent.entry(parent.clone())
                .or_insert_with(Vec::new)
                .push(item.clone());
        } else {
            roots.push(item.clone());
        }
    }

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

    let set_clear_cookie = |name: &str, path: &str| -> Result<(), ServerFnError> {
        let value = axum::http::HeaderValue::from_str(&clear_cookie(name, path))
            .map_err(|e| ServerFnError::new(format!("Invalid cookie header: {e}")))?;
        response.append_header(axum::http::header::SET_COOKIE, value);
        Ok(())
    };

    if state.demo_mode {
        set_clear_cookie("lekton_demo_user", "/")?;
    } else {
        set_clear_cookie("lekton_access_token", "/")?;
        set_clear_cookie("lekton_refresh_token", "/auth/refresh")?;
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

/// Server function to fetch a document's rendered HTML content, TOC, and metadata.
#[server(GetDocHtml, "/api")]
pub async fn get_doc_html(
    slug: String,
) -> Result<Option<crate::pages::DocPageData>, ServerFnError> {
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

    Ok(Some(crate::pages::DocPageData {
        title: doc.title,
        html,
        headings,
        last_updated,
        tags: doc.tags,
    }))
}

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let user_resource = LocalResource::new(get_current_user);

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
