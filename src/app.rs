use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::components::*;
use leptos_router::path;
use serde::{Deserialize, Serialize};

use crate::components::Layout;
use crate::editor::component::EditorPage;
use crate::pages::{
    AdminSettingsPage, ChatPage, DocPage, HomePage, LearnDashboardPage, LearnPathPage, LoginPage,
    NotFound, ProfilePage, PromptsPage,
};
use crate::schema::component::{SchemaListPage, SchemaViewerPage};
// Re-export server functions so existing `use crate::app::*` imports keep working.
pub use crate::server::access_levels::*;
pub use crate::server::auth_fns::*;
pub use crate::server::custom_css::*;
pub use crate::server::docs::*;
pub use crate::server::feedback::*;
pub use crate::server::learn::*;
pub use crate::server::nav::NavigationOrderEntry;
pub use crate::server::nav::*;
pub use crate::server::pats::*;
pub use crate::server::prompts::*;
pub use crate::server::reindex::*;
pub use crate::server::search::*;
pub use crate::server::service_tokens::*;
pub use crate::server::sources::*;
pub use crate::server::users::*;

/// Newtype wrapper for the demo-mode signal, used as Leptos context.
#[derive(Clone, Copy)]
pub struct IsDemoMode(pub Signal<bool>);

/// Newtype wrapper for the RAG-enabled signal, used as Leptos context.
#[derive(Clone, Copy)]
pub struct IsRagEnabled(pub Signal<bool>);

/// Resolved runtime feature toggles, sent to the client so the UI can hide
/// disabled functionality. Mirrors the validated `[features]` config: because
/// enabled-but-misconfigured features fail fast at startup, a `true` flag here
/// means the feature is fully available.
///
/// `Default` is all-`false`: the conservative state shown before the flags
/// resolve on the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FeatureFlags {
    pub mcp: bool,
    pub rag: bool,
    pub editor: bool,
    pub schema_registry: bool,
    pub search: bool,
    pub prompt_library: bool,
    pub documentation_feedback: bool,
    pub attachment_indexing: bool,
    pub document_upload: bool,
    pub sources: bool,
    pub learn: bool,
    pub doc_versioning: bool,
}

/// Newtype wrapper for the feature-flags signal, used as Leptos context.
///
/// Holds `None` until the flags resolve on the client. Consumers choose how to
/// treat the unresolved state: navigation hides entries (default `false`),
/// while route guards render optimistically (default `true`) to avoid a
/// not-found flash on first paint.
#[derive(Clone, Copy)]
pub struct Features(pub Signal<Option<FeatureFlags>>);

/// Reactive accessor for one feature flag, for navigation/buttons: the entry
/// stays hidden until the flags resolve.
pub fn use_feature(
    selector: impl Fn(FeatureFlags) -> bool + Send + Sync + 'static,
) -> Signal<bool> {
    let features = use_context::<Features>();
    Signal::derive(move || {
        features
            .and_then(|f| f.0.get())
            .map(&selector)
            .unwrap_or(false)
    })
}

/// Reactive accessor for route guards: renders optimistically (treats the
/// unresolved state as enabled) so an enabled page does not flash the
/// not-found view before the flags arrive.
fn route_feature(selector: impl Fn(FeatureFlags) -> bool + Send + Sync + 'static) -> Signal<bool> {
    let features = use_context::<Features>();
    Signal::derive(move || {
        features
            .and_then(|f| f.0.get())
            .map(&selector)
            .unwrap_or(true)
    })
}

#[cfg(feature = "ssr")]
impl axum::extract::FromRef<AppState> for crate::auth::extractor::DemoMode {
    fn from_ref(state: &AppState) -> Self {
        crate::auth::extractor::DemoMode(state.demo_mode)
    }
}

#[cfg(feature = "ssr")]
use std::sync::Arc;

#[cfg(feature = "ssr")]
#[derive(Clone, axum::extract::FromRef)]
pub struct AppState {
    pub document_repo: Arc<dyn crate::db::repository::DocumentRepository>,
    /// Release catalogue and the `latest` alias, backing the version selector
    /// and the release-scoped sync.
    pub release_repo: Arc<dyn crate::db::release_repository::ReleaseRepository>,
    pub schema_repo: Arc<dyn crate::db::schema_repository::SchemaRepository>,
    pub settings_repo: Arc<dyn crate::db::settings_repository::SettingsRepository>,
    pub asset_repo: Arc<dyn crate::db::asset_repository::AssetRepository>,
    pub storage_client: Arc<dyn crate::storage::client::StorageClient>,
    pub search_service: Option<Arc<dyn crate::search::client::SearchService>>,
    pub service_token: String,
    pub service_token_repo: Arc<dyn crate::db::service_token_repository::ServiceTokenRepository>,
    pub document_revision_repo:
        Arc<dyn crate::db::document_revision_repository::DocumentRevisionRepository>,
    pub prompt_repo: Arc<dyn crate::db::prompt_repository::PromptRepository>,
    pub prompt_version_repo: Arc<dyn crate::db::prompt_version_repository::PromptVersionRepository>,
    pub user_prompt_preference_repo:
        Arc<dyn crate::db::user_prompt_preference_repository::UserPromptPreferenceRepository>,
    pub demo_mode: bool,
    pub leptos_options: LeptosOptions,
    // ── Auth (phase 5) ────────────────────────────────────────────────────────
    pub user_repo: Arc<dyn crate::db::user_repository::UserRepository>,
    pub access_level_repo: Arc<dyn crate::db::access_level_repository::AccessLevelRepository>,
    pub navigation_order_repo:
        Arc<dyn crate::db::navigation_order_repository::NavigationOrderRepository>,
    pub token_service: Arc<crate::auth::token_service::TokenService>,
    pub auth_provider: Option<Arc<dyn crate::auth::provider::AuthProvider>>,
    pub rag_service: Option<Arc<dyn crate::rag::service::RagService>>,
    pub reindex_state: Option<Arc<crate::rag::reindex::ReindexState>>,
    /// Handle for enqueuing attachments for RAG extraction. Present only when
    /// RAG and the `attachment_indexing` feature are both enabled.
    #[from_ref(skip)]
    pub attachment_queue: Option<crate::rag::attachment_extraction::AttachmentQueue>,
    /// The attachment extraction service backing `attachment_queue`, kept
    /// alongside it so a full RAG re-index can force-reprocess every
    /// attachment synchronously (bypassing the bounded background queue).
    pub attachment_service:
        Option<Arc<crate::rag::attachment_extraction::AttachmentExtractionService>>,
    /// Keyword search over PDF attachment content, backed by a dedicated
    /// Meilisearch index. Present only when the `search` feature is enabled.
    pub attachment_search_service:
        Option<Arc<dyn crate::search::attachment_search::AttachmentSearchService>>,
    pub search_reindex_state: Option<Arc<crate::search::reindex::SearchReindexState>>,
    pub schema_endpoint_reindex_state: Arc<crate::schema::reindex::SchemaEndpointReindexState>,
    pub chat_repo: Option<Arc<dyn crate::db::chat_repository::ChatRepository>>,
    pub chat_service: Option<Arc<crate::rag::chat::ChatService>>,
    /// Learn-mode persistence (paths/lessons/records). Present only when the
    /// `learn` feature is enabled.
    pub learn_repo: Option<Arc<dyn crate::db::learn_repository::LearnRepository>>,
    /// Learn-mode orchestration (lesson generation + grading). Present only
    /// when the `learn` feature is enabled and its LLM provider initialised.
    pub learn_service: Option<Arc<crate::learn::service::LearnService>>,
    pub feedback_repo: Option<Arc<dyn crate::db::feedback_repository::FeedbackRepository>>,
    pub documentation_feedback_repo:
        Arc<dyn crate::db::documentation_feedback_repository::DocumentationFeedbackRepository>,
    pub document_source_repo: Arc<dyn crate::db::source_repository::DocumentSourceRepository>,
    pub embedding_cache_repo:
        Option<Arc<dyn crate::db::embedding_cache_repository::EmbeddingCacheRepository>>,
    #[from_ref(skip)]
    pub insecure_cookies: bool,
    #[from_ref(skip)]
    pub max_attachment_size_bytes: u64,
    #[from_ref(skip)]
    pub features: FeatureFlags,
}

#[cfg(feature = "ssr")]
pub fn shell(options: LeptosOptions, features: FeatureFlags) -> impl IntoView {
    // The WYSIWYG editor bundle is only needed when the editor feature is on.
    let editor_scripts = features.editor.then(|| {
        view! {
            <script type="module" src="/js/tiptap-bundle.min.js"></script>
            <script type="module" src="/js/tiptap.js"></script>
        }
    });

    // Favicon follows the same override priority as the navbar logo
    // (logo-{theme}.svg > logo.svg > the built-in Lekton mark), resolved here at
    // SSR time so a deployment's custom logo also becomes the browser tab icon.
    let resolve_icon = |theme_file: &str| {
        if std::path::Path::new(&format!("public/{theme_file}")).exists() {
            format!("/{theme_file}")
        } else if std::path::Path::new("public/logo.svg").exists() {
            "/logo.svg".to_string()
        } else {
            "/favicon.svg".to_string()
        }
    };
    let icon_light = resolve_icon("logo-light.svg");
    let icon_dark = resolve_icon("logo-dark.svg");
    let favicon = if icon_light == icon_dark {
        view! { <link rel="icon" href=icon_light /> }.into_any()
    } else {
        // Two icons keyed to the OS colour scheme (favicons can't follow the
        // in-app theme toggle; prefers-color-scheme is the closest approximation).
        view! {
            <link rel="icon" href=icon_light media="(prefers-color-scheme: light)" />
            <link rel="icon" href=icon_dark media="(prefers-color-scheme: dark)" />
        }
        .into_any()
    };
    view! {
        <!DOCTYPE html>
        <html lang="en" data-theme="light">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <script>
                    r#"(function(){var t=localStorage.getItem('lekton-theme');if(t==='dark'||t==='light'){document.documentElement.setAttribute('data-theme',t)}else{var d=window.matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light';document.documentElement.setAttribute('data-theme',d)}})()"#
                </script>
                // lekton_logged_in is NOT httpOnly so JS can read it.
                // If absent the user is anonymous: play entrance animations immediately
                // (before the body renders) so they fire from SSR HTML without WASM.
                <script>
                    r#"(function(){if(!/(?:^|;\s*)lekton_logged_in=/.test(document.cookie)){document.documentElement.classList.add('lekton-play')}})()"#
                </script>
                <AutoReload options=options.clone() />
                <HydrationScripts options=options />
                <MetaTags />
                <meta name="description" content="Lekton: A dynamic, high-performance Internal Developer Portal with RBAC and unified schema registry." />
                {favicon}
                <Stylesheet id="leptos" href="/pkg/lekton.css" />
                <Link rel="stylesheet" href="/custom.css" />
                {editor_scripts}
                <script src="/js/mermaid-loader.js"></script>
                <script src="/js/code-blocks.js"></script>
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

/// Resolve `(allowed_levels, include_draft)` for an optionally-authenticated user.
///
/// This is the single source of truth for user visibility used by all HTTP handlers
/// that need to filter documents, schemas, or assets by access level.
///
/// - Admin → `(None, true)` (unrestricted).
/// - Demo user → `(["public"], false)`.
/// - Authenticated user → effective levels + `"public"` + `"loggeduser"`, `can_read_draft`.
/// - Unauthenticated → `(["public"], false)`.
#[cfg(feature = "ssr")]
pub async fn resolve_user_visibility(
    state: &AppState,
    user: Option<&crate::auth::models::AuthenticatedUser>,
) -> Result<(Option<Vec<String>>, bool), crate::error::AppError> {
    match user {
        Some(u) if u.is_admin => Ok((None, true)),
        Some(u) if state.demo_mode && u.user_id.starts_with("demo-") => {
            Ok((Some(vec!["public".to_string()]), false))
        }
        Some(u) => {
            let user_doc = state.user_repo.find_user_by_id(&u.user_id).await?;
            let (levels, include_draft) = match user_doc {
                Some(doc) => {
                    let mut levels = doc.effective_access_levels;
                    if !levels.contains(&"public".to_string()) {
                        levels.push("public".to_string());
                    }
                    if !levels.contains(&"loggeduser".to_string()) {
                        levels.push("loggeduser".to_string());
                    }
                    (levels, doc.can_read_draft)
                }
                None => (vec!["public".to_string(), "loggeduser".to_string()], false),
            };
            Ok((Some(levels), include_draft))
        }
        None => Ok((Some(vec!["public".to_string()]), false)),
    }
}

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let user_resource =
        LocalResource::new(|| crate::auth::refresh_client::with_auth_bootstrap(get_current_user));
    let demo_mode_resource = LocalResource::new(get_is_demo_mode);
    let features_resource = LocalResource::new(get_feature_flags);

    let current_user: Signal<Option<crate::auth::models::AuthenticatedUser>> =
        Signal::derive(move || user_resource.get().and_then(|res| res.ok()).flatten());

    let is_demo_mode: Signal<bool> = Signal::derive(move || {
        demo_mode_resource
            .get()
            .and_then(|res| res.ok())
            .unwrap_or(true)
    });

    let features: Signal<Option<FeatureFlags>> =
        Signal::derive(move || features_resource.get().and_then(|res| res.ok()));

    let is_rag_enabled: Signal<bool> =
        Signal::derive(move || features.get().map(|f| f.rag).unwrap_or(false));

    // Add 'lekton-play' to <html> once the auth check completes. This unpauses
    // the entrance animations and fades out the splash screen (CSS-driven).
    // For anonymous users the inline <head> script already added it before body
    // render — this Effect is a no-op for them. For authenticated users it fires
    // when user_resource resolves (Ok or Err — either way the layout reveals).
    Effect::new(move || {
        if user_resource.get().is_some() {
            #[cfg(feature = "hydrate")]
            if let Some(root) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.document_element())
            {
                let cls = root.class_name();
                if !cls.contains("lekton-play") {
                    root.set_class_name(&format!("{cls} lekton-play"));
                }
            }
        }
    });

    provide_context(current_user);
    provide_context(IsDemoMode(is_demo_mode));
    provide_context(IsRagEnabled(is_rag_enabled));
    provide_context(Features(features));
    provide_context(crate::pages::chat::ChatContext::new());

    view! {
        <Title text="Lekton — Internal Developer Portal" />

        // Splash screen: visible by default in SSR HTML, hidden via CSS once
        // 'lekton-play' is on <html>. For anonymous users the inline <head>
        // script applies that class before the body renders, so the splash is
        // effectively skipped. Authenticated users see it until the access
        // token is validated.
        <div class="lekton-splash">
            <span class="loading loading-spinner loading-xl text-primary"></span>
        </div>

        <Router>
            <Layout>
                <Routes fallback=|| view! { <NotFound /> }>
                    <Route path=path!("/") view=HomePage />
                    <Route path=path!("/login") view=LoginPage />
                    <Route path=path!("/docs/*slug") view=DocPage />
                    <Route path=path!("/edit") view=EditorRoute />
                    <Route path=path!("/edit/*slug") view=EditorRoute />
                    <Route path=path!("/schemas") view=SchemaListRoute />
                    <Route path=path!("/schemas/*name") view=SchemaViewerRoute />
                    <Route path=path!("/chat") view=ChatRoute />
                    <Route path=path!("/learn") view=LearnRoute />
                    <Route path=path!("/learn/:path_id") view=LearnPathRoute />
                    <Route path=path!("/prompts") view=PromptsRoute />
                    <Route path=path!("/profile") view=ProfilePage />
                    <Route path=path!("/admin/:section") view=AdminSettingsPage />
                </Routes>
            </Layout>
        </Router>
    }
}

/// Route guards: render the page only when its feature is enabled, otherwise
/// fall back to the not-found view. Optimistic while flags are unresolved so an
/// enabled page never flashes not-found on first paint.
#[component]
fn EditorRoute() -> impl IntoView {
    let enabled = route_feature(|f| f.editor);
    view! { <Show when=move || enabled.get() fallback=|| view! { <NotFound /> }><EditorPage /></Show> }
}

#[component]
fn SchemaListRoute() -> impl IntoView {
    let enabled = route_feature(|f| f.schema_registry);
    view! { <Show when=move || enabled.get() fallback=|| view! { <NotFound /> }><SchemaListPage /></Show> }
}

#[component]
fn SchemaViewerRoute() -> impl IntoView {
    let enabled = route_feature(|f| f.schema_registry);
    view! { <Show when=move || enabled.get() fallback=|| view! { <NotFound /> }><SchemaViewerPage /></Show> }
}

#[component]
fn ChatRoute() -> impl IntoView {
    let enabled = route_feature(|f| f.rag);
    view! { <Show when=move || enabled.get() fallback=|| view! { <NotFound /> }><ChatPage /></Show> }
}

#[component]
fn PromptsRoute() -> impl IntoView {
    let enabled = route_feature(|f| f.prompt_library);
    view! { <Show when=move || enabled.get() fallback=|| view! { <NotFound /> }><PromptsPage /></Show> }
}

#[component]
fn LearnRoute() -> impl IntoView {
    let enabled = route_feature(|f| f.learn);
    view! { <Show when=move || enabled.get() fallback=|| view! { <NotFound /> }><LearnDashboardPage /></Show> }
}

#[component]
fn LearnPathRoute() -> impl IntoView {
    let enabled = route_feature(|f| f.learn);
    view! { <Show when=move || enabled.get() fallback=|| view! { <NotFound /> }><LearnPathPage /></Show> }
}

#[cfg(test)]
mod prompt_library_tests {
    use crate::server::prompts::build_prompt_library_state;
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
        assert!(!doc_is_accessible(
            "architect",
            false,
            Some(&allowed),
            false
        ));
        assert!(!doc_is_accessible(
            "cloud-internal",
            false,
            Some(&allowed),
            false
        ));
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
