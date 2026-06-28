use leptos::prelude::*;
use leptos_router::hooks::use_params;
use leptos_router::params::Params;

use crate::app::CreateTokenResult;

mod access_levels;
mod custom_css;
mod document_upload;
mod documentation_feedback;
mod navigation_order;
mod pats;
mod reindex;
mod service_tokens;
mod users;

use access_levels::AccessLevelManager;
use custom_css::CustomCssEditor;
use document_upload::DocumentUploadManager;
use documentation_feedback::DocumentationFeedbackAdminPanel;
use navigation_order::NavigationOrderEditor;
use pats::AdminPatManager;
use reindex::{RagReindexSection, SchemaEndpointReindexSection, SearchReindexSection};
use service_tokens::ServiceTokenManager;
use users::UserManager;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct AdminParams {
    pub section: String,
}

/// Admin settings page with service token management and theming.
#[component]
pub fn AdminSettingsPage() -> impl IntoView {
    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();

    let is_admin = move || {
        current_user
            .and_then(|sig| sig.get())
            .map(|u| u.is_admin)
            .unwrap_or(false)
    };

    let params = use_params::<AdminParams>();
    let section = move || {
        params.with(|p| {
            p.as_ref()
                .map(|p| p.section.clone())
                .unwrap_or_else(|_| "tokens".to_string())
        })
    };

    view! {
        <Show
            when=is_admin
            fallback=|| view! {
                <div class="flex items-center justify-center min-h-[50vh]">
                    <div class="alert alert-error max-w-md shadow-lg border-none bg-error/10 text-error">
                        <svg xmlns="http://www.w3.org/2000/svg" class="h-6 w-6 shrink-0 stroke-current text-error" fill="none" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
                        </svg>
                        <span class="font-medium">"Access denied. Admin privileges required."</span>
                    </div>
                </div>
            }
        >
            <div class="animate-in fade-in slide-in-from-bottom-4 duration-500">
                <AdminSettingsContent section=section />
            </div>
        </Show>
    }
}

/// Inner content, rendered only for admins.
#[component]
fn AdminSettingsContent(section: impl Fn() -> String + Send + Sync + 'static) -> impl IntoView {
    // Created token (shown once in modal)
    let (created_token, set_created_token) = signal(Option::<CreateTokenResult>::None);

    let section = std::sync::Arc::new(section);
    let section2 = section.clone();

    view! {
        <div class="max-w-5xl mx-auto space-y-8 pb-20">
            <header class="flex flex-col items-start gap-4 border-b border-base-200 pb-8 sm:flex-row sm:items-center">
                <div class="p-3 bg-primary/10 rounded-2xl text-primary">
                    <svg class="w-8 h-8" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"></path>
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"></path>
                    </svg>
                </div>
                <div>
                   {move || {
                       let current_section = section();
                       let title = match current_section.as_str() {
                           "tokens" => "Service Tokens",
                           "pats" => "Personal Access Tokens",
                           "documentation-feedback" => "Documentation Feedback",
                           "navigation" => "Navigation Setup",
                           "css" => "Visual Customization",
                           "rag" => "Index Management",
                           "access-levels" => "Access Levels",
                           "users" => "User Management",
                           "upload" => "Upload Document",
                           _ => "Administration",
                       };
                       let subtitle = match current_section.as_str() {
                           "documentation-feedback" => "Review MCP-reported documentation gaps, resolve them, and keep the registry tidy.",
                           "access-levels" => "Manage content access levels and their inheritance hierarchy.",
                           "users" => "Assign access levels and permissions to registered users.",
                           "rag" => "Rebuild derived search and retrieval indexes from the canonical document store.",
                           "upload" => "Upload a PDF and publish it as a page with a description and download link.",
                           _ => "Manage your instance configuration, service tokens, and theming.",
                       };
                       view! {
                           <>
                               <h1 class="text-4xl font-extrabold tracking-tight">{title}</h1>
                               <p class="text-base-content/60 mt-1">{subtitle}</p>
                           </>
                       }
                   }}
                </div>
            </header>

            <div class="grid grid-cols-1 gap-8">
                {move || match section2().as_str() {
                    "tokens" => view! { <ServiceTokenManager set_created_token=set_created_token /> }.into_any(),
                    "pats" => view! { <AdminPatManager /> }.into_any(),
                    "documentation-feedback" => view! { <DocumentationFeedbackAdminPanel /> }.into_any(),
                    "navigation" => view! { <NavigationOrderEditor /> }.into_any(),
                    "css" => view! { <CustomCssEditor /> }.into_any(),
                    "rag" => {
                        let search_enabled = crate::app::use_feature(|f| f.search);
                        let rag_enabled = crate::app::use_feature(|f| f.rag);
                        let schema_enabled = crate::app::use_feature(|f| f.schema_registry);
                        view! {
                            <div class="space-y-6">
                                <Show when=move || search_enabled.get()>
                                    <SearchReindexSection />
                                </Show>
                                <Show when=move || rag_enabled.get()>
                                    <RagReindexSection />
                                </Show>
                                <Show when=move || schema_enabled.get()>
                                    <SchemaEndpointReindexSection />
                                </Show>
                            </div>
                        }.into_any()
                    }
                    "access-levels" => view! { <AccessLevelManager /> }.into_any(),
                    "users" => view! { <UserManager /> }.into_any(),
                    "upload" => {
                        let upload_enabled = crate::app::use_feature(|f| f.document_upload);
                        view! {
                            <Show
                                when=move || upload_enabled.get()
                                fallback=|| view! { <div class="alert alert-warning">"Document upload is disabled."</div> }
                            >
                                <DocumentUploadManager />
                            </Show>
                        }.into_any()
                    }
                    _ => view! { <div class="alert alert-warning">"Page not found"</div> }.into_any(),
                }}
            </div>
        </div>

        // Created token modal
        <CreatedTokenModal token=created_token set_token=set_created_token />
    }
}

/// Modal shown once after creating a token, displaying the raw token value.
#[component]
fn CreatedTokenModal(
    token: ReadSignal<Option<CreateTokenResult>>,
    set_token: WriteSignal<Option<CreateTokenResult>>,
) -> impl IntoView {
    let (copied, set_copied) = signal(false);

    view! {
        <Show when=move || token.get().is_some()>
            <div class="fixed inset-0 z-[200] flex items-center justify-center bg-black/60 backdrop-blur-sm animate-in fade-in duration-300">
                <div class="bg-base-100 rounded-3xl shadow-2xl w-full max-w-xl mx-4 p-10 relative overflow-hidden animate-in zoom-in-95 duration-300">
                    <div class="absolute top-0 inset-x-0 h-2 bg-warning"></div>

                    <div class="mb-8 flex items-center gap-4">
                        <div class="w-12 h-12 bg-warning/10 rounded-2xl flex items-center justify-center text-warning">
                          <svg xmlns="http://www.w3.org/2000/svg" class="h-8 w-8" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
                          </svg>
                        </div>
                        <div>
                          <h3 class="font-black text-3xl tracking-tight">"Token Created"</h3>
                          <p class="text-base-content/60 font-medium">"This is your only chance to copy it."</p>
                        </div>
                    </div>

                    <div class="bg-warning/10 border border-warning/20 rounded-2xl p-6 mb-8 flex items-start gap-4">
                        <div class="text-warning mt-1">
                          <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path></svg>
                        </div>
                        <p class="text-base-content text-sm font-semibold leading-relaxed">
                          "For security reasons, we do not store the raw token. If you lose it, you will need to deactivate it and create a new one."
                        </p>
                    </div>

                    {move || token.get().map(|t| {
                        let raw = t.raw_token.clone();
                        #[cfg(feature = "hydrate")]
                        let raw_for_copy = t.raw_token.clone();
                        let name = t.name.clone();
                        let scopes_str = t.allowed_scopes.join(", ");
                        view! {
                            <div class="space-y-6">
                                <div class="form-control">
                                    <label class="label pt-0"><span class="label-text font-bold text-xs uppercase tracking-widest text-base-content/50">"Generated Token"</span></label>
                                    <div class="relative group">
                                        <input
                                            type="text"
                                            readonly
                                            class="input input-bordered w-full font-mono text-lg py-8 pr-16 bg-base-200/50 border-base-300 focus:outline-none focus:border-warning/50 selection:bg-warning/20"
                                            value=raw
                                        />
                                        <button
                                            class="btn btn-warning shadow-lg shadow-warning/20 absolute right-2 top-1/2 -translate-y-1/2 normal-case font-bold"
                                            on:click=move |_| {
                                                #[cfg(feature = "hydrate")]
                                                {
                                                    let raw = raw_for_copy.clone();
                                                    let _ = js_sys::eval(&format!(
                                                        "navigator.clipboard.writeText('{}')",
                                                        raw.replace('\'', "\\'")
                                                    ));
                                                    set_copied.set(true);
                                                }
                                            }
                                        >
                                            {move || if copied.get() {
                                              view! { <span class="flex items-center gap-1"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"></path></svg>"Copied"</span> }.into_any()
                                            } else {
                                              view! { <span class="flex items-center gap-1"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3"></path></svg>"Copy"</span> }.into_any()
                                            }}
                                        </button>
                                    </div>
                                </div>

                                <div class="grid grid-cols-2 gap-8 py-6 border-y border-base-200">
                                    <div>
                                      <p class="text-[10px] font-black uppercase text-base-content/40 tracking-widest mb-1">"Internal Name"</p>
                                      <p class="font-bold text-base">{name}</p>
                                    </div>
                                    <div>
                                      <p class="text-[10px] font-black uppercase text-base-content/40 tracking-widest mb-1">"Scopes"</p>
                                      <p class="font-mono text-xs truncate" title=scopes_str.clone()>{scopes_str.clone()}</p>
                                    </div>
                                </div>
                            </div>
                        }
                    })}

                    <div class="flex justify-end pt-8">
                        <button
                            class="btn btn-ghost hover:bg-base-200 w-full sm:w-64 font-bold"
                            on:click=move |_| {
                                set_token.set(None);
                                set_copied.set(false);
                            }
                        >
                            "I have saved the token"
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    }
}
