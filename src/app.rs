use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::components::*;
use leptos_router::path;
use serde::{Deserialize, Serialize};

use crate::db::settings_repository::NavGroup;

use crate::components::Layout;
use crate::editor::component::EditorPage;
use crate::pages::{AdminSettingsPage, ChatPage, DocPage, HomePage, LoginPage, NotFound, ProfilePage, PromptsPage};
use crate::schema::component::{SchemaListPage, SchemaViewerPage};
use crate::search::client::SearchHit;

/// Newtype wrapper for the demo-mode signal, used as Leptos context.
/// Prevents collision with other `Signal<bool>` contexts (e.g. `IsRagEnabled`).
#[derive(Clone, Copy)]
pub struct IsDemoMode(pub Signal<bool>);

/// Newtype wrapper for the RAG-enabled signal, used as Leptos context.
#[derive(Clone, Copy)]
pub struct IsRagEnabled(pub Signal<bool>);

/// Implement `FromRef<AppState>` for `DemoMode` so that Axum extractors
/// (`RequiredAuthUser`, `OptionalAuthUser`) can fall back to the demo session
/// cookie when demo mode is active.
#[cfg(feature = "ssr")]
impl axum::extract::FromRef<AppState> for crate::auth::extractor::DemoMode {
    fn from_ref(state: &AppState) -> Self {
        crate::auth::extractor::DemoMode(state.demo_mode)
    }
}

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
    pub prompt_repo: Arc<dyn crate::db::prompt_repository::PromptRepository>,
    pub prompt_version_repo: Arc<dyn crate::db::prompt_version_repository::PromptVersionRepository>,
    pub user_prompt_preference_repo: Arc<dyn crate::db::user_prompt_preference_repository::UserPromptPreferenceRepository>,
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
    pub feedback_repo: Option<Arc<dyn crate::db::feedback_repository::FeedbackRepository>>,
    pub documentation_feedback_repo: Arc<dyn crate::db::documentation_feedback_repository::DocumentationFeedbackRepository>,
    pub embedding_cache_repo: Option<Arc<dyn crate::db::embedding_cache_repository::EmbeddingCacheRepository>>,
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

/// Server function to check whether RAG chat is available.
#[server(GetIsRagEnabled, "/api")]
pub async fn get_is_rag_enabled() -> Result<bool, ServerFnError> {
    let state = expect_context::<AppState>();
    Ok(state.rag_service.is_some() && state.chat_service.is_some())
}

/// Server function to get RAG re-index status.
#[server(GetRagReindexStatus, "/api")]
pub async fn get_rag_reindex_status() -> Result<(bool, u32, bool), ServerFnError> {
    use std::sync::atomic::Ordering;
    let state = expect_context::<AppState>();
    let rag_enabled = state.rag_service.is_some();
    match &state.reindex_state {
        Some(reindex) => Ok((
            reindex.is_running.load(Ordering::Acquire),
            reindex.progress.load(Ordering::Relaxed),
            rag_enabled,
        )),
        None => Ok((false, 0, rag_enabled)),
    }
}

/// Server function to trigger RAG re-index (admin only).
#[server(TriggerRagReindex, "/api")]
pub async fn trigger_rag_reindex() -> Result<String, ServerFnError> {
    use std::sync::atomic::Ordering;
    let state = expect_context::<AppState>();

    let rag = state
        .rag_service
        .as_ref()
        .ok_or_else(|| ServerFnError::new("RAG is not enabled"))?;

    let reindex = state
        .reindex_state
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Reindex state not available"))?;

    if reindex
        .is_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ServerFnError::new("Re-index is already in progress"));
    }

    let reindex_clone = reindex.clone();
    let document_repo = state.document_repo.clone();
    let storage = state.storage_client.clone();
    let rag_clone = rag.clone();

    tokio::spawn(async move {
        crate::rag::reindex::run_reindex(reindex_clone, document_repo, storage, rag_clone).await;
    });

    Ok("Re-index started".to_string())
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
///
/// Returns [`UNAUTHORIZED_SENTINEL`] when the caller is not authenticated at
/// all (so the client knows it should attempt a token refresh).
/// Returns a distinct "forbidden" message when the caller is authenticated but
/// lacks admin privileges (no refresh makes sense in that case).
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
        // Authenticated but not admin — 403, no refresh needed.
        return Err(ServerFnError::new("Admin privileges required"));
    }

    // Demo mode fallback
    if state.demo_mode {
        if let Some(cookie) = jar.get("lekton_demo_user") {
            if let Ok(user) = serde_json::from_str::<crate::auth::models::AuthenticatedUser>(cookie.value()) {
                if user.is_admin {
                    return Ok(user);
                }
                return Err(ServerFnError::new("Admin privileges required"));
            }
        }
    }

    // Not authenticated — 401, client should attempt refresh.
    Err(ServerFnError::new(crate::auth::models::UNAUTHORIZED_SENTINEL))
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
        token_type: "service".to_string(),
        user_id: None,
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

// ── PAT management (user self-service) ───────────────────────────────────────

/// Summary of a PAT shown in the user profile page.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatInfo {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// Result of creating a new PAT (raw token shown once).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatePatResult {
    pub id: String,
    pub name: String,
    pub raw_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptLibraryItem {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub access_level: String,
    pub status: String,
    pub owner: String,
    pub tags: Vec<String>,
    pub publish_to_mcp: bool,
    pub default_primary: bool,
    pub context_cost: String,
    pub is_favorite: bool,
    pub is_hidden: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptLibraryState {
    pub items: Vec<PromptLibraryItem>,
    pub estimated_context_cost: String,
    pub warnings: Vec<String>,
}

/// Helper: extract the current authenticated user (any role).
#[cfg(feature = "ssr")]
async fn require_any_user(state: &AppState) -> Result<crate::auth::models::AuthenticatedUser, ServerFnError> {
    use axum_extra::extract::CookieJar;
    use crate::auth::extractor::ACCESS_TOKEN_COOKIE;
    use crate::auth::token_service::TokenService;

    let jar: CookieJar = leptos_axum::extract().await?;

    if let Some(user) = jar
        .get(ACCESS_TOKEN_COOKIE)
        .and_then(|c| state.token_service.validate_access_token(c.value()).ok())
        .map(|claims| TokenService::claims_to_user(&claims))
    {
        return Ok(user);
    }

    if state.demo_mode {
        if let Some(cookie) = jar.get("lekton_demo_user") {
            if let Ok(user) = serde_json::from_str::<crate::auth::models::AuthenticatedUser>(cookie.value()) {
                return Ok(user);
            }
        }
    }

    Err(ServerFnError::new(crate::auth::models::UNAUTHORIZED_SENTINEL))
}

#[cfg(feature = "ssr")]
async fn prompt_visibility_for_user(
    state: &AppState,
    user: &crate::auth::models::AuthenticatedUser,
) -> Result<(Option<Vec<String>>, bool), ServerFnError> {
    if user.is_admin {
        return Ok((None, true));
    }

    let perms = state
        .user_repo
        .get_permissions(&user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let user_ctx = crate::auth::models::UserContext {
        user: user.clone(),
        permissions: perms,
    };
    Ok(user_ctx.document_visibility())
}

#[cfg(feature = "ssr")]
fn prompt_context_cost_label(weight: u32) -> String {
    if weight >= 12 {
        "high".to_string()
    } else if weight >= 6 {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

#[cfg(feature = "ssr")]
fn build_prompt_library_state(
    prompts: Vec<crate::db::prompt_models::Prompt>,
    preferences: Vec<crate::db::user_prompt_preference_repository::UserPromptPreference>,
) -> PromptLibraryState {
    use std::collections::HashMap;

    let pref_by_slug: HashMap<String, crate::db::user_prompt_preference_repository::UserPromptPreference> =
        preferences
            .into_iter()
            .map(|pref| (pref.prompt_slug.clone(), pref))
            .collect();

    let mut items = Vec::new();
    let mut total_context_weight = 0u32;

    for prompt in prompts {
        let pref = pref_by_slug.get(&prompt.slug);
        let is_favorite = pref.map(|p| p.is_favorite).unwrap_or(false);
        let is_hidden = pref.map(|p| p.is_hidden).unwrap_or(false);

        if prompt.publish_to_mcp && ((prompt.default_primary && !is_hidden) || is_favorite) {
            total_context_weight += prompt.context_cost.weight() as u32;
        }

        items.push(PromptLibraryItem {
            slug: prompt.slug,
            name: prompt.name,
            description: prompt.description,
            access_level: prompt.access_level,
            status: match prompt.status {
                crate::db::prompt_models::PromptStatus::Draft => "draft".to_string(),
                crate::db::prompt_models::PromptStatus::Active => "active".to_string(),
                crate::db::prompt_models::PromptStatus::Deprecated => "deprecated".to_string(),
            },
            owner: prompt.owner,
            tags: prompt.tags,
            publish_to_mcp: prompt.publish_to_mcp,
            default_primary: prompt.default_primary,
            context_cost: match prompt.context_cost {
                crate::db::prompt_models::ContextCost::Low => "low".to_string(),
                crate::db::prompt_models::ContextCost::Medium => "medium".to_string(),
                crate::db::prompt_models::ContextCost::High => "high".to_string(),
            },
            is_favorite,
            is_hidden,
        });
    }

    items.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.slug.cmp(&b.slug)));

    let mut warnings = Vec::new();
    if total_context_weight >= 12 {
        warnings.push(
            "Selected prompts add heavy context overhead; reduce favorites or hide some primary prompts.".to_string(),
        );
    } else if total_context_weight >= 8 {
        warnings.push(
            "Selected prompts may add significant context overhead.".to_string(),
        );
    }

    PromptLibraryState {
        items,
        estimated_context_cost: prompt_context_cost_label(total_context_weight),
        warnings,
    }
}

#[server(GetPromptLibraryState, "/api")]
pub async fn get_prompt_library_state() -> Result<PromptLibraryState, ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;
    let (levels, include_draft) = prompt_visibility_for_user(&state, &user).await?;

    let prompts = state
        .prompt_repo
        .list_by_access_levels(levels.as_deref(), include_draft)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let preferences = state
        .user_prompt_preference_repo
        .list_by_user_id(&user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(build_prompt_library_state(prompts, preferences))
}

#[server(SavePromptPreference, "/api")]
pub async fn save_prompt_preference(
    prompt_slug: String,
    is_favorite: bool,
    is_hidden: bool,
) -> Result<PromptLibraryState, ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;
    let (levels, include_draft) = prompt_visibility_for_user(&state, &user).await?;

    let prompt = state
        .prompt_repo
        .find_by_slug(&prompt_slug)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Prompt not found"))?;

    let allowed = user.is_admin || levels.as_ref().is_none_or(|ls| ls.contains(&prompt.access_level));
    let can_read_draft = if user.is_admin {
        true
    } else {
        include_draft
    };
    if !allowed || (prompt.status == crate::db::prompt_models::PromptStatus::Draft && !can_read_draft) {
        return Err(ServerFnError::new("Prompt not found"));
    }

    let preference = crate::db::user_prompt_preference_repository::UserPromptPreference {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user.user_id.clone(),
        prompt_slug: prompt_slug.clone(),
        is_favorite,
        is_hidden,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    state
        .user_prompt_preference_repo
        .upsert(preference)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let prompts = state
        .prompt_repo
        .list_by_access_levels(levels.as_deref(), include_draft)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let preferences = state
        .user_prompt_preference_repo
        .list_by_user_id(&user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(build_prompt_library_state(prompts, preferences))
}

/// List the current user's PATs.
#[server(ListUserPats, "/api")]
pub async fn list_user_pats() -> Result<Vec<PatInfo>, ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let tokens = state.service_token_repo
        .list_by_user_id(&user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(tokens.into_iter().map(|t| PatInfo {
        id: t.id,
        name: t.name,
        is_active: t.is_active,
        created_at: t.created_at.format("%Y-%m-%d %H:%M").to_string(),
        last_used_at: t.last_used_at.map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
    }).collect())
}

/// Create a new PAT for the current user.
#[server(CreateUserPat, "/api")]
pub async fn create_user_pat(name: String) -> Result<CreatePatResult, ServerFnError> {
    use crate::auth::token_service::TokenService;

    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("PAT name cannot be empty"));
    }

    let raw_token = uuid::Uuid::new_v4().to_string();
    let token_hash = TokenService::hash_token(&raw_token);
    let id = uuid::Uuid::new_v4().to_string();

    let token = crate::db::service_token_models::ServiceToken {
        id: id.clone(),
        name: name.clone(),
        token_hash,
        allowed_scopes: vec![],
        token_type: "pat".to_string(),
        user_id: Some(user.user_id.clone()),
        can_write: false,
        created_by: user.user_id,
        created_at: chrono::Utc::now(),
        last_used_at: None,
        is_active: true,
    };

    state.service_token_repo.create(token).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(CreatePatResult { id, name, raw_token })
}

/// Toggle a PAT active/inactive (caller must own it).
#[server(ToggleUserPat, "/api")]
pub async fn toggle_user_pat(id: String, active: bool) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let token = state.service_token_repo.find_by_id(&id).await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("PAT not found"))?;

    if token.user_id.as_deref() != Some(&user.user_id) {
        return Err(ServerFnError::new("You do not own this token"));
    }

    state.service_token_repo.set_active(&id, active).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Permanently delete a PAT (caller must own it).
#[server(DeleteUserPat, "/api")]
pub async fn delete_user_pat(id: String) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    state.service_token_repo.delete_pat(&id, &user.user_id).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

// ── PAT management (admin) ────────────────────────────────────────────────────

/// PAT summary for the admin view (includes user email).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdminPatInfo {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// Paginated list of all PATs (admin only).
#[server(AdminListPats, "/api")]
pub async fn admin_list_pats(page: u64, per_page: u64) -> Result<(Vec<AdminPatInfo>, u64), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let per_page = per_page.clamp(1, 100);
    let (tokens, total) = state.service_token_repo
        .list_pats_paginated(page, per_page)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Resolve user emails
    let mut email_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for token in &tokens {
        if let Some(uid) = &token.user_id {
            if !email_map.contains_key(uid) {
                if let Ok(Some(u)) = state.user_repo.find_user_by_id(uid).await {
                    email_map.insert(uid.clone(), u.email);
                }
            }
        }
    }

    let items = tokens.into_iter().map(|t| {
        let email = t.user_id.as_ref().and_then(|uid| email_map.get(uid).cloned());
        AdminPatInfo {
            id: t.id,
            name: t.name,
            is_active: t.is_active,
            user_id: t.user_id,
            user_email: email,
            created_at: t.created_at.format("%Y-%m-%d %H:%M").to_string(),
            last_used_at: t.last_used_at.map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
        }
    }).collect();

    Ok((items, total))
}

/// Toggle any PAT active/inactive (admin only).
#[server(AdminTogglePat, "/api")]
pub async fn admin_toggle_pat(id: String, active: bool) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let token = state.service_token_repo.find_by_id(&id).await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("PAT not found"))?;

    if !token.is_pat() {
        return Err(ServerFnError::new("This endpoint only manages PATs"));
    }

    state.service_token_repo.set_active(&id, active).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

// ── Feedback (user self-service) ─────────────────────────────────────────────

/// A single feedback item for the user's profile history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackInfo {
    pub message_id: String,
    pub session_id: String,
    pub rating: String, // "positive" | "negative"
    pub comment: Option<String>,
    pub created_at: String,
}

/// Paginated list of feedback items.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackListResult {
    pub items: Vec<FeedbackInfo>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentationFeedbackAdminItem {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub related_resources: Vec<String>,
    pub search_queries: Vec<String>,
    pub created_by: String,
    pub created_at: String,
    pub duplicate_of: Option<String>,
    pub resolution_note: Option<String>,
    pub related_feedback_ids: Vec<String>,
    pub user_goal: Option<String>,
    pub missing_information: Option<String>,
    pub impact: Option<String>,
    pub suggested_target_resource: Option<String>,
    pub target_resource_uri: Option<String>,
    pub problem_summary: Option<String>,
    pub proposal: Option<String>,
    pub supporting_resources: Vec<String>,
    pub expected_benefit: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentationFeedbackAdminListResult {
    pub items: Vec<DocumentationFeedbackAdminItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[cfg(feature = "ssr")]
fn map_documentation_feedback_item(
    item: crate::db::documentation_feedback_models::DocumentationFeedback,
) -> DocumentationFeedbackAdminItem {
    DocumentationFeedbackAdminItem {
        id: item.id,
        kind: item.kind.as_str().to_string(),
        status: item.status.as_str().to_string(),
        title: item.title,
        summary: item.summary,
        related_resources: item.related_resources,
        search_queries: item.search_queries,
        created_by: item.created_by,
        created_at: item.created_at.format("%Y-%m-%d %H:%M").to_string(),
        duplicate_of: item.duplicate_of,
        resolution_note: item.resolution_note,
        related_feedback_ids: item.related_feedback_ids,
        user_goal: item.user_goal,
        missing_information: item.missing_information,
        impact: item.impact,
        suggested_target_resource: item.suggested_target_resource,
        target_resource_uri: item.target_resource_uri,
        problem_summary: item.problem_summary,
        proposal: item.proposal,
        supporting_resources: item.supporting_resources,
        expected_benefit: item.expected_benefit,
    }
}

/// List the current user's feedback (paginated, newest first).
#[server(ListUserFeedback, "/api")]
pub async fn list_user_feedback(page: u64, per_page: u64) -> Result<FeedbackListResult, ServerFnError> {
    use crate::db::chat_models::FeedbackRating;
    use crate::db::feedback_repository::FeedbackListParams;

    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let fb_repo = state.feedback_repo
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Feedback not available"))?;

    let per_page = per_page.clamp(1, 50);
    let params = FeedbackListParams {
        page,
        per_page,
        ..Default::default()
    };

    let result = fb_repo
        .list_user_feedback(&user.user_id, params)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let items = result.items.into_iter().map(|fb| {
        let rating = match fb.rating {
            FeedbackRating::Positive => "positive".to_string(),
            FeedbackRating::Negative => "negative".to_string(),
        };
        FeedbackInfo {
            message_id: fb.message_id,
            session_id: fb.session_id,
            rating,
            comment: fb.comment,
            created_at: fb.created_at.format("%Y-%m-%d %H:%M").to_string(),
        }
    }).collect();

    Ok(FeedbackListResult {
        items,
        total: result.total,
        page: result.page,
        per_page: result.per_page,
    })
}

/// Delete the current user's feedback on a specific message.
#[server(DeleteUserFeedback, "/api")]
pub async fn delete_user_feedback(message_id: String) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let fb_repo = state.feedback_repo
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Feedback not available"))?;

    fb_repo.delete_feedback(&message_id, &user.user_id).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

#[server(ListDocumentationFeedback, "/api")]
pub async fn list_documentation_feedback(
    page: u64,
    per_page: u64,
    query: Option<String>,
    kind: Option<String>,
    status: Option<String>,
) -> Result<DocumentationFeedbackAdminListResult, ServerFnError> {
    use crate::db::documentation_feedback_models::{
        DocumentationFeedbackKind, DocumentationFeedbackStatus,
    };
    use crate::db::documentation_feedback_repository::DocumentationFeedbackListParams;

    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let kind = kind
        .map(|value| value.parse::<DocumentationFeedbackKind>())
        .transpose()
        .map_err(ServerFnError::new)?;
    let status = status
        .map(|value| value.parse::<DocumentationFeedbackStatus>())
        .transpose()
        .map_err(ServerFnError::new)?;

    let result = state
        .documentation_feedback_repo
        .list(DocumentationFeedbackListParams {
            query,
            kind,
            status,
            page,
            per_page: per_page.clamp(1, 50),
        })
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(DocumentationFeedbackAdminListResult {
        items: result
            .items
            .into_iter()
            .map(map_documentation_feedback_item)
            .collect(),
        total: result.total,
        page: result.page,
        per_page: result.per_page,
    })
}

#[server(ResolveDocumentationFeedback, "/api")]
pub async fn resolve_documentation_feedback(
    id: String,
    resolution_note: Option<String>,
) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state
        .documentation_feedback_repo
        .resolve(&id, resolution_note.filter(|value| !value.trim().is_empty()))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(MarkDocumentationFeedbackDuplicate, "/api")]
pub async fn mark_documentation_feedback_duplicate(
    id: String,
    duplicate_of: String,
    resolution_note: Option<String>,
) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let duplicate_of = duplicate_of.trim().to_string();
    if duplicate_of.is_empty() {
        return Err(ServerFnError::new("Duplicate target id is required"));
    }
    if duplicate_of == id {
        return Err(ServerFnError::new("A feedback item cannot duplicate itself"));
    }

    state
        .documentation_feedback_repo
        .find_by_id(&duplicate_of)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Duplicate target not found"))?;

    state
        .documentation_feedback_repo
        .mark_duplicate(
            &id,
            &duplicate_of,
            resolution_note.filter(|value| !value.trim().is_empty()),
        )
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let user_resource = LocalResource::new(get_current_user);
    let demo_mode_resource = LocalResource::new(get_is_demo_mode);
    let rag_resource = LocalResource::new(get_is_rag_enabled);

    let current_user: Signal<Option<crate::auth::models::AuthenticatedUser>> =
        Signal::derive(move || {
            user_resource.get().and_then(|res| res.ok()).flatten()
        });

    // Whether the app is in demo mode (defaults to true until loaded to avoid
    // flashing the wrong UI — the demo login page is a safe fallback).
    let is_demo_mode: Signal<bool> = Signal::derive(move || {
        demo_mode_resource.get().and_then(|res| res.ok()).unwrap_or(true)
    });

    let is_rag_enabled: Signal<bool> = Signal::derive(move || {
        rag_resource.get().and_then(|res| res.ok()).unwrap_or(false)
    });

    provide_context(current_user);
    provide_context(IsDemoMode(is_demo_mode));
    provide_context(IsRagEnabled(is_rag_enabled));
    provide_context(crate::pages::chat::ChatContext::new());

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
                    <Route path=path!("/chat") view=ChatPage />
                    <Route path=path!("/prompts") view=PromptsPage />
                    <Route path=path!("/profile") view=ProfilePage />
                    <Route path=path!("/admin/:section") view=AdminSettingsPage />
                </Routes>
            </Layout>
        </Router>
    }
}

#[cfg(test)]
mod prompt_library_tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn build_prompt_library_state_combines_primary_and_favorites_into_context_cost() {
        let prompts = vec![
            crate::db::prompt_models::Prompt {
                slug: "prompts/code-review".into(),
                name: "Code Review".into(),
                description: "Review code".into(),
                s3_key: "prompts/code-review.yaml".into(),
                access_level: "internal".into(),
                status: crate::db::prompt_models::PromptStatus::Active,
                owner: "platform".into(),
                last_updated: Utc::now(),
                tags: vec![],
                variables: vec![],
                publish_to_mcp: true,
                default_primary: true,
                context_cost: crate::db::prompt_models::ContextCost::Medium,
                content_hash: None,
                metadata_hash: None,
                is_archived: false,
            },
            crate::db::prompt_models::Prompt {
                slug: "prompts/git-history-sanitizer".into(),
                name: "Git History Sanitizer".into(),
                description: "Check git history".into(),
                s3_key: "prompts/git-history-sanitizer.yaml".into(),
                access_level: "internal".into(),
                status: crate::db::prompt_models::PromptStatus::Active,
                owner: "platform".into(),
                last_updated: Utc::now(),
                tags: vec![],
                variables: vec![],
                publish_to_mcp: true,
                default_primary: false,
                context_cost: crate::db::prompt_models::ContextCost::High,
                content_hash: None,
                metadata_hash: None,
                is_archived: false,
            },
        ];

        let preferences = vec![
            crate::db::user_prompt_preference_repository::UserPromptPreference {
                id: "pref-1".into(),
                user_id: "u1".into(),
                prompt_slug: "prompts/git-history-sanitizer".into(),
                is_favorite: true,
                is_hidden: false,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];

        let state = build_prompt_library_state(prompts, preferences);
        assert_eq!(state.estimated_context_cost, "medium");
        assert!(state.warnings.is_empty());
        assert_eq!(state.items.len(), 2);
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
