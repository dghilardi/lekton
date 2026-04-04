use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::components::*;
use leptos_router::path;
use serde::{Deserialize, Serialize};

use crate::db::settings_repository::NavGroup;

use crate::components::Layout;
use crate::editor::component::EditorPage;
use crate::pages::{AdminSettingsPage, DocPage, HomePage, LoginPage, NotFound};
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
    pub service_token_repo: Arc<dyn crate::db::service_token_repository::ServiceTokenRepository>,
    pub document_version_repo: Arc<dyn crate::db::document_version_repository::DocumentVersionRepository>,
    pub demo_mode: bool,
    pub leptos_options: LeptosOptions,
    // ── Auth (phase 5) ────────────────────────────────────────────────────────
    pub user_repo: Arc<dyn crate::db::user_repository::UserRepository>,
    pub access_level_repo: Arc<dyn crate::db::access_level_repository::AccessLevelRepository>,
    pub navigation_order_repo: Arc<dyn crate::db::navigation_order_repository::NavigationOrderRepository>,
    pub token_service: Arc<crate::auth::token_service::TokenService>,
    pub auth_provider: Option<Arc<dyn crate::auth::provider::AuthProvider>>,
    pub rag_service: Option<Arc<dyn crate::rag::service::RagService>>,
    pub reindex_state: Option<Arc<crate::rag::reindex::ReindexState>>,
    pub chat_repo: Option<Arc<dyn crate::db::chat_repository::ChatRepository>>,
    pub chat_service: Option<Arc<crate::rag::chat::ChatService>>,
    /// Whether cookies should be set without the `Secure` flag (HTTP local dev).
    #[from_ref(skip)]
    pub insecure_cookies: bool,
    /// Maximum allowed attachment upload size in bytes.
    #[from_ref(skip)]
    pub max_attachment_size_bytes: u64,
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
    use crate::db::navigation_order_repository::NavigationOrderRepository;
    use std::collections::HashMap;

    let state = expect_context::<AppState>();

    let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
    let (docs, nav_order_entries) = tokio::join!(
        state.document_repo.list_by_access_levels(allowed_levels.as_deref(), include_draft),
        state.navigation_order_repo.list_all(),
    );
    let docs = docs.map_err(|e| ServerFnError::new(e.to_string()))?;
    let nav_order_entries = nav_order_entries.map_err(|e| ServerFnError::new(e.to_string()))?;

    // Build a weight lookup: slug -> weight
    let nav_weights: HashMap<String, i32> = nav_order_entries
        .into_iter()
        .map(|e| (e.slug, e.weight))
        .collect();

    let mut all_items: Vec<NavItem> = docs.into_iter().map(|doc| {
        let parent_slug = doc.parent_slug.or_else(|| {
            if let Some((parent, _)) = doc.slug.rsplit_once('/') {
                Some(parent.to_string())
            } else {
                None
            }
        });
        NavItem {
            slug: doc.slug,
            title: doc.title,
            parent_slug,
            order: doc.order,
            children: vec![],
        }
    }).collect();

    let mut items_by_slug: HashMap<String, NavItem> = all_items.iter().cloned()
        .map(|item| (item.slug.clone(), item))
        .collect();

    for item in &all_items {
        let mut current_parent = item.parent_slug.clone();
        while let Some(parent_slug) = current_parent {
            if !items_by_slug.contains_key(&parent_slug) {
                let title_part = parent_slug.split('/').last().unwrap_or(&parent_slug);
                let title = title_part.split('-')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str()
                        }
                    }).collect::<Vec<_>>().join(" ");
                
                let next_parent = if let Some((p, _)) = parent_slug.rsplit_once('/') {
                    Some(p.to_string())
                } else {
                    None
                };

                let missing_node = NavItem {
                    slug: parent_slug.clone(),
                    title,
                    parent_slug: next_parent.clone(),
                    order: 0,
                    children: vec![],
                };
                
                items_by_slug.insert(parent_slug.clone(), missing_node);
                current_parent = next_parent;
            } else {
                break;
            }
        }
    }

    let mut roots = Vec::new();
    let mut children_by_parent: HashMap<String, Vec<NavItem>> = HashMap::new();

    for (_slug, item) in items_by_slug.into_iter() {
        if let Some(parent) = &item.parent_slug {
            children_by_parent.entry(parent.clone())
                .or_insert_with(Vec::new)
                .push(item);
        } else {
            roots.push(item);
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

    // Sort the tree: sections/categories use custom weight (fallback alphabetical),
    // documents use their order field (fallback alphabetical). Both types are mixed
    // together — sections are NOT prioritized over documents.
    fn sort_nav_items(items: &mut [NavItem], weights: &HashMap<String, i32>) {
        items.sort_by(|a, b| {
            let a_is_section = !a.children.is_empty();
            let b_is_section = !b.children.is_empty();

            let a_sort_key = if a_is_section {
                weights.get(&a.slug).copied().unwrap_or(i32::MAX)
            } else {
                a.order as i32
            };
            let b_sort_key = if b_is_section {
                weights.get(&b.slug).copied().unwrap_or(i32::MAX)
            } else {
                b.order as i32
            };

            a_sort_key.cmp(&b_sort_key)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        for item in items.iter_mut() {
            if !item.children.is_empty() {
                sort_nav_items(&mut item.children, weights);
            }
        }
    }

    sort_nav_items(&mut roots, &nav_weights);

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

/// Server function to check whether the app is running in demo mode.
///
/// Returns `true` for demo mode (username/password form), `false` for
/// production OAuth2/OIDC (redirect to external provider).
#[server(GetIsDemoMode, "/api")]
pub async fn get_is_demo_mode() -> Result<bool, ServerFnError> {
    let state = expect_context::<AppState>();
    Ok(state.demo_mode)
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
        format!("{name}=; Path={path}; HttpOnly; SameSite=Strict; Max-Age=0")
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

/// Server function to get navbar grouping configurations.
#[server(GetNavbarGroups, "/api")]
pub async fn get_navbar_groups() -> Result<Vec<NavGroup>, ServerFnError> {
    let state = expect_context::<AppState>();
    let settings = state.settings_repo.get_settings().await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(settings.navbar_groups)
}

/// Re-export for use in frontend components.
pub use crate::db::navigation_order_repository::NavigationOrderEntry;

/// Server function to get all navigation order entries (admin only).
#[server(GetNavigationOrder, "/api")]
pub async fn get_navigation_order() -> Result<Vec<NavigationOrderEntry>, ServerFnError> {
    use crate::db::navigation_order_repository::NavigationOrderRepository;

    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state.navigation_order_repo
        .list_all()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Server function to save navigation order (admin only).
///
/// Accepts the full ordered list of entries and replaces everything atomically.
#[server(SaveNavigationOrder, "/api")]
pub async fn save_navigation_order(entries: Vec<NavigationOrderEntry>) -> Result<String, ServerFnError> {
    use crate::db::navigation_order_repository::NavigationOrderRepository;

    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state.navigation_order_repo
        .replace_all(entries)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok("Navigation order saved successfully".to_string())
}

/// Returns `true` if a document with the given `access_level` / `is_draft` state
/// is readable by a caller whose visibility is described by `allowed_levels` and
/// `include_draft`.
///
/// `allowed_levels = None` means admin (unrestricted).
pub fn doc_is_accessible(
    access_level: &str,
    is_draft: bool,
    allowed_levels: Option<&[String]>,
    include_draft: bool,
) -> bool {
    let level_ok = match allowed_levels {
        None => true,
        Some(levels) => levels.iter().any(|l| l == access_level),
    };
    level_ok && (!is_draft || include_draft)
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
        let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
        let all_docs = state.document_repo
            .list_by_access_levels(allowed_levels.as_deref(), include_draft)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        
        let mut children: Vec<_> = all_docs.into_iter()
            .filter(|d| d.parent_slug.as_deref() == Some(slug.as_str()))
            .collect();
            
        if children.is_empty() {
            return Ok(None);
        }
        
        children.sort_by_key(|d| d.order);

        let title_part = slug.split('/').last().unwrap_or("Section");
        let title = title_part.split('-')
            .map(|word| {
                let mut c = word.chars();
                match c.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let mut html = String::from("<p class=\"text-base-content/70 pb-4 border-b border-base-200\">Select a document from this section to read.</p><div class=\"grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 mt-6\">");
        for child in children {
            html.push_str(&format!(
                "<a href=\"/docs/{}\" class=\"card bg-base-100 shadow-sm border border-base-200 hover:shadow-md transition-shadow hover:border-primary/30\"><div class=\"card-body p-5\"><h2 class=\"card-title text-lg flex items-center gap-2\"><svg class=\"w-5 h-5 text-primary opacity-80\" fill=\"none\" stroke=\"currentColor\" viewBox=\"0 0 24 24\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z\"></path></svg>{}</h2></div></a>",
                child.slug, child.title
            ));
        }
        html.push_str("</div>");

        return Ok(Some(crate::pages::DocPageData {
            title,
            html,
            headings: vec![],
            last_updated: chrono::Utc::now().format("%B %d, %Y").to_string(),
            tags: vec![],
        }));
    };

    // Enforce access control: return None (→ 404) when the caller does not have
    // permission to read this document.  Returning None instead of an error
    // avoids leaking the existence of restricted documents.
    let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
    if !doc_is_accessible(&doc.access_level, doc.is_draft, allowed_levels.as_deref(), include_draft) {
        return Ok(None);
    }

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

// ── Service token management (admin) ─────────────────────────────────────────

/// Summary of a service token for the admin UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceTokenInfo {
    pub id: String,
    pub name: String,
    pub allowed_scopes: Vec<String>,
    pub can_write: bool,
    pub is_active: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// Result of creating a new service token.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateTokenResult {
    pub id: String,
    pub name: String,
    pub raw_token: String,
    pub allowed_scopes: Vec<String>,
}

/// Helper: extract the current user and verify admin.
#[cfg(feature = "ssr")]
async fn require_admin_user(state: &AppState) -> Result<crate::auth::models::AuthenticatedUser, ServerFnError> {
    use axum_extra::extract::CookieJar;
    use crate::auth::extractor::ACCESS_TOKEN_COOKIE;
    use crate::auth::token_service::TokenService;

    let jar: CookieJar = leptos_axum::extract().await?;

    // Try JWT first
    if let Some(user) = jar
        .get(ACCESS_TOKEN_COOKIE)
        .and_then(|c| state.token_service.validate_access_token(c.value()).ok())
        .map(|claims| TokenService::claims_to_user(&claims))
    {
        if user.is_admin {
            return Ok(user);
        }
        return Err(ServerFnError::new("Admin privileges required"));
    }

    // Demo mode fallback
    if state.demo_mode {
        if let Some(cookie) = jar.get("lekton_demo_user") {
            if let Ok(user) = serde_json::from_str::<crate::auth::models::AuthenticatedUser>(cookie.value()) {
                if user.is_admin {
                    return Ok(user);
                }
            }
        }
    }

    Err(ServerFnError::new("Admin privileges required"))
}

/// List all service tokens (admin only).
#[server(ListServiceTokens, "/api")]
pub async fn list_service_tokens() -> Result<Vec<ServiceTokenInfo>, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let tokens = state.service_token_repo.list_all().await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(tokens.into_iter().map(|t| ServiceTokenInfo {
        id: t.id,
        name: t.name,
        allowed_scopes: t.allowed_scopes,
        can_write: t.can_write,
        is_active: t.is_active,
        created_at: t.created_at.format("%Y-%m-%d %H:%M").to_string(),
        last_used_at: t.last_used_at.map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
    }).collect())
}

/// Create a new scoped service token (admin only).
/// `scopes` is newline-separated.
#[server(CreateServiceToken, "/api")]
pub async fn create_service_token(
    name: String,
    scopes: String,
    can_write: bool,
) -> Result<CreateTokenResult, ServerFnError> {
    use crate::auth::token_service::TokenService;

    let state = expect_context::<AppState>();
    let user = require_admin_user(&state).await?;

    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("Token name cannot be empty"));
    }

    let allowed_scopes: Vec<String> = scopes
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if allowed_scopes.is_empty() {
        return Err(ServerFnError::new("At least one scope is required"));
    }

    let has_overlap = state.service_token_repo
        .check_scope_overlap(&allowed_scopes, None)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    if has_overlap {
        return Err(ServerFnError::new("Scopes overlap with an existing service token"));
    }

    let raw_token = uuid::Uuid::new_v4().to_string();
    let token_hash = TokenService::hash_token(&raw_token);

    let token = crate::db::service_token_models::ServiceToken {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        token_hash,
        allowed_scopes: allowed_scopes.clone(),
        can_write,
        created_by: user.user_id,
        created_at: chrono::Utc::now(),
        last_used_at: None,
        is_active: true,
    };

    state.service_token_repo.create(token).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(CreateTokenResult {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        raw_token,
        allowed_scopes,
    })
}

/// Deactivate a service token (admin only).
#[server(DeactivateServiceToken, "/api")]
pub async fn deactivate_service_token(id: String) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state.service_token_repo.deactivate(&id).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let user_resource = LocalResource::new(get_current_user);
    let demo_mode_resource = LocalResource::new(get_is_demo_mode);

    let current_user: Signal<Option<crate::auth::models::AuthenticatedUser>> =
        Signal::derive(move || {
            user_resource.get().and_then(|res| res.ok()).flatten()
        });

    // Whether the app is in demo mode (defaults to true until loaded to avoid
    // flashing the wrong UI — the demo login page is a safe fallback).
    let is_demo_mode: Signal<bool> = Signal::derive(move || {
        demo_mode_resource.get().and_then(|res| res.ok()).unwrap_or(true)
    });

    provide_context(current_user);
    provide_context(is_demo_mode);

    view! {
        <Title text="Lekton — Internal Developer Portal" />

        <Router>
            <Layout>
                <Routes fallback=|| view! { <NotFound /> }>
                    <Route path=path!("/") view=HomePage />
                    <Route path=path!("/login") view=LoginPage />
                    <Route path=path!("/docs/*slug") view=DocPage />
                    <Route path=path!("/edit/*slug") view=EditorPage />
                    <Route path=path!("/schemas") view=SchemaListPage />
                    <Route path=path!("/schemas/:name") view=SchemaViewerPage />
                    <Route path=path!("/admin/settings") view=AdminSettingsPage />
                </Routes>
            </Layout>
        </Router>
    }
}


#[cfg(test)]
mod tests {
    use super::doc_is_accessible;

    fn levels(s: &[&str]) -> Vec<String> {
        s.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn admin_can_read_any_level() {
        assert!(doc_is_accessible("architect", false, None, false));
        assert!(doc_is_accessible("cloud-internal", false, None, false));
    }

    #[test]
    fn user_can_read_allowed_level() {
        let allowed = levels(&["public", "internal"]);
        assert!(doc_is_accessible("public", false, Some(&allowed), false));
        assert!(doc_is_accessible("internal", false, Some(&allowed), false));
    }

    #[test]
    fn user_cannot_read_restricted_level() {
        let allowed = levels(&["public"]);
        assert!(!doc_is_accessible("internal", false, Some(&allowed), false));
        assert!(!doc_is_accessible("architect", false, Some(&allowed), false));
        assert!(!doc_is_accessible("cloud-internal", false, Some(&allowed), false));
    }

    #[test]
    fn draft_hidden_without_draft_permission() {
        let allowed = levels(&["internal"]);
        assert!(!doc_is_accessible("internal", true, Some(&allowed), false));
    }

    #[test]
    fn draft_visible_with_draft_permission() {
        let allowed = levels(&["internal"]);
        assert!(doc_is_accessible("internal", true, Some(&allowed), true));
    }

    #[test]
    fn admin_can_read_draft() {
        assert!(doc_is_accessible("architect", true, None, true));
    }

    #[test]
    fn wrong_level_blocks_even_with_draft_permission() {
        let allowed = levels(&["public"]);
        assert!(!doc_is_accessible("architect", true, Some(&allowed), true));
    }
}
