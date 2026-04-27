use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::components::*;
use leptos_router::path;
use serde::{Deserialize, Serialize};

use crate::components::Layout;
use crate::editor::component::EditorPage;
use crate::pages::{
    AdminSettingsPage, ChatPage, DocPage, HomePage, LoginPage, NotFound, ProfilePage, PromptsPage,
};
use crate::schema::component::{SchemaListPage, SchemaViewerPage};
// Re-export server functions so existing `use crate::app::*` imports keep working.
pub use crate::server::access_levels::*;
pub use crate::server::auth_fns::*;
pub use crate::server::custom_css::*;
pub use crate::server::docs::*;
pub use crate::server::feedback::*;
pub use crate::server::nav::NavigationOrderEntry;
pub use crate::server::nav::*;
pub use crate::server::pats::*;
pub use crate::server::prompts::*;
pub use crate::server::reindex::*;
pub use crate::server::search::*;
pub use crate::server::service_tokens::*;
pub use crate::server::users::*;

/// Newtype wrapper for the demo-mode signal, used as Leptos context.
#[derive(Clone, Copy)]
pub struct IsDemoMode(pub Signal<bool>);

/// Newtype wrapper for the RAG-enabled signal, used as Leptos context.
#[derive(Clone, Copy)]
pub struct IsRagEnabled(pub Signal<bool>);

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
    pub schema_repo: Arc<dyn crate::db::schema_repository::SchemaRepository>,
    pub settings_repo: Arc<dyn crate::db::settings_repository::SettingsRepository>,
    pub asset_repo: Arc<dyn crate::db::asset_repository::AssetRepository>,
    pub storage_client: Arc<dyn crate::storage::client::StorageClient>,
    pub search_service: Option<Arc<dyn crate::search::client::SearchService>>,
    pub service_token: String,
    pub service_token_repo: Arc<dyn crate::db::service_token_repository::ServiceTokenRepository>,
    pub document_version_repo:
        Arc<dyn crate::db::document_version_repository::DocumentVersionRepository>,
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
    pub search_reindex_state: Option<Arc<crate::search::reindex::SearchReindexState>>,
    pub schema_endpoint_reindex_state: Arc<crate::schema::reindex::SchemaEndpointReindexState>,
    pub chat_repo: Option<Arc<dyn crate::db::chat_repository::ChatRepository>>,
    pub chat_service: Option<Arc<crate::rag::chat::ChatService>>,
    pub feedback_repo: Option<Arc<dyn crate::db::feedback_repository::FeedbackRepository>>,
    pub documentation_feedback_repo:
        Arc<dyn crate::db::documentation_feedback_repository::DocumentationFeedbackRepository>,
    pub embedding_cache_repo:
        Option<Arc<dyn crate::db::embedding_cache_repository::EmbeddingCacheRepository>>,
    #[from_ref(skip)]
    pub insecure_cookies: bool,
    #[from_ref(skip)]
    pub max_attachment_size_bytes: u64,
}

#[cfg(feature = "ssr")]
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en" data-theme="light">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <script>
                    r#"(function(){var t=localStorage.getItem('lekton-theme');if(t==='dark'||t==='light'){document.documentElement.setAttribute('data-theme',t)}else{var d=window.matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light';document.documentElement.setAttribute('data-theme',d)}})()"#
                </script>
                <AutoReload options=options.clone() />
                <HydrationScripts options=options />
                <Meta name="description" content="Lekton: A dynamic, high-performance Internal Developer Portal with RBAC and unified schema registry." />
                <Stylesheet id="leptos" href="/pkg/lekton.css" />
                <Link rel="stylesheet" href="/custom.css" />
                <script type="module" src="/js/tiptap-bundle.min.js"></script>
                <script type="module" src="/js/tiptap.js"></script>
                <script src="/js/mermaid-loader.js"></script>
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

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let user_resource =
        LocalResource::new(|| crate::auth::refresh_client::with_auth_bootstrap(get_current_user));
    let demo_mode_resource = LocalResource::new(get_is_demo_mode);
    let rag_resource = LocalResource::new(get_is_rag_enabled);

    let current_user: Signal<Option<crate::auth::models::AuthenticatedUser>> =
        Signal::derive(move || user_resource.get().and_then(|res| res.ok()).flatten());

    let is_demo_mode: Signal<bool> = Signal::derive(move || {
        demo_mode_resource
            .get()
            .and_then(|res| res.ok())
            .unwrap_or(true)
    });

    let is_rag_enabled: Signal<bool> =
        Signal::derive(move || rag_resource.get().and_then(|res| res.ok()).unwrap_or(false));

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
                    <Route path=path!("/schemas/*name") view=SchemaViewerPage />
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
