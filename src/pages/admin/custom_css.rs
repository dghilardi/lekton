use leptos::prelude::*;

#[allow(unused_imports)]
use crate::app::{
    admin_list_pats, admin_toggle_pat, create_admin_access_level, create_service_token,
    deactivate_service_token, delete_admin_access_level, get_custom_css, get_navigation,
    get_navigation_order, get_rag_reindex_status, get_schema_endpoint_reindex_status,
    get_search_reindex_status, list_admin_access_levels, list_admin_users,
    list_documentation_feedback, list_service_tokens, mark_documentation_feedback_duplicate,
    resolve_documentation_feedback, save_custom_css, save_navigation_order,
    set_admin_user_access_levels, trigger_rag_reindex, trigger_schema_endpoint_reindex,
    trigger_search_reindex, update_admin_access_level, AccessLevelInfo, CreateTokenResult,
    DocumentationFeedbackAdminItem, DocumentationFeedbackAdminListResult, NavItem,
    NavigationOrderEntry, ServiceTokenInfo,
};
#[allow(unused_imports)]
use crate::auth::refresh_client::with_auth_retry;

/// Component for editing custom application CSS.
#[component]
pub fn CustomCssEditor() -> impl IntoView {
    let (css, set_css) = signal(String::new());
    let (original_css, set_original_css) = signal(String::new());
    let (saving, set_saving) = signal(false);
    let (message, set_message) = signal(Option::<(bool, String)>::None);

    let load_resource = LocalResource::new(|| with_auth_retry(get_custom_css));

    let _ = Effect::new(move |_| {
        if let Some(Ok(loaded_css)) = load_resource.get() {
            set_css.set(loaded_css.clone());
            set_original_css.set(loaded_css);
        }
    });

    let save_action = crate::components::browser_action(move |new_css: &String| {
        let new_css = new_css.clone();
        async move {
            set_saving.set(true);
            set_message.set(None);
            let result = with_auth_retry(|| save_custom_css(new_css.clone())).await;
            set_saving.set(false);
            match result {
                Ok(msg) => {
                    set_original_css.set(new_css);
                    set_message.set(Some((true, msg)));
                }
                Err(e) => {
                    set_message.set(Some((false, e.to_string())));
                }
            }
        }
    });

    let has_changes = move || css.get() != original_css.get();

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200">
            <div class="card-body p-0">
                <div class="p-8 flex flex-col gap-6">
                    <div class="grid gap-6 xl:grid-cols-[minmax(0,1fr)_18rem] xl:items-start">
                        <div class="form-control">
                            <textarea
                                class="textarea textarea-bordered w-full min-h-[28rem] xl:min-h-[34rem] resize-y font-mono text-sm leading-7 focus:textarea-primary transition-all shadow-inner bg-base-200/20"
                                placeholder={"/* A theme assigns variables; components read them.\n   Reference: docs/theming.md */\n\n:root {\n  --lekton-font-family: \"Inter\", sans-serif;\n  --lekton-code-scheme: dark;\n  --lekton-code-bg: #0b1220;\n  --lekton-code-text: #e2e8f0;\n}\n\n[data-theme=\"light\"] {\n  --color-primary: #0d9488;\n  --lekton-sidebar-bg: #0f172a;\n  --lekton-sidebar-text: #e2e8f0;\n}\n\n[data-theme=\"dark\"] {\n  --color-primary: #2dd4bf;\n}"}
                                prop:value=move || css.get()
                                on:input=move |ev| set_css.set(event_target_value(&ev))
                            ></textarea>
                        </div>

                        <ThemingReference />
                    </div>

                    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
                        <div class="min-h-[2.5rem]">
                            <Show when=move || message.get().is_some()>
                                {move || {
                                    let (success, text) =
                                        message.get().unwrap_or((false, String::new()));
                                    let alert_class = if success { "alert-success bg-success/10 text-success" } else { "alert-error bg-error/10 text-error" };
                                    view! {
                                        <div class=format!("alert {alert_class} py-2 px-4 shadow-sm border-none flex items-center gap-2 text-sm font-semibold")>
                                            {if success {
                                                view! { <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"></path></svg> }.into_any()
                                            } else {
                                                view! { <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"></path></svg> }.into_any()
                                            }}
                                            <span>{text}</span>
                                        </div>
                                    }
                                }}
                            </Show>
                        </div>

                        <div class="flex flex-col sm:flex-row gap-3 w-full sm:w-auto">
                            <button
                                class="btn btn-ghost w-full sm:w-auto"
                                disabled=move || !has_changes() || saving.get()
                                on:click=move |_| set_css.set(original_css.get())
                            >
                                "Discard"
                            </button>
                            <button
                                class="btn btn-primary w-full sm:w-64 shadow-lg shadow-primary/20"
                                disabled=move || !has_changes() || saving.get()
                                on:click=move |_| { save_action.dispatch(css.get()); }
                            >
                                {move || if saving.get() {
                                    view! { <span class="loading loading-spinner loading-sm"></span> }.into_any()
                                } else {
                                    view! { "Save Changes" }.into_any()
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Theming variables, grouped as in `docs/theming.md`.
const THEMING_GROUPS: &[(&str, &[&str])] = &[
    (
        "Palette & shape (DaisyUI)",
        &[
            "--color-primary",
            "--color-base-100 … 300",
            "--color-base-content",
            "--radius-box / -field / -selector",
        ],
    ),
    (
        "Typography & layout",
        &[
            "--lekton-font-family",
            "--lekton-heading-font",
            "--lekton-mono-font",
            "--lekton-sidebar-width",
            "--lekton-content-max-width",
        ],
    ),
    (
        "Header",
        &[
            "--lekton-header-bg",
            "--lekton-header-text",
            "--lekton-header-hover-bg",
            "--lekton-header-border-color / -width",
            "--lekton-header-shadow",
        ],
    ),
    (
        "Sidebar",
        &[
            "--lekton-sidebar-bg",
            "--lekton-sidebar-text",
            "--lekton-sidebar-text-muted",
            "--lekton-sidebar-heading",
            "--lekton-sidebar-accent",
            "--lekton-sidebar-hover-bg",
            "--lekton-sidebar-active-bg / -text",
            "--lekton-sidebar-border",
            "--lekton-sidebar-tree-line",
        ],
    ),
    (
        "Content",
        &[
            "--lekton-content-text",
            "--lekton-heading-color",
            "--lekton-link-color",
            "--lekton-link-decoration",
            "--lekton-table-header-bg / -text",
            "--lekton-table-border",
        ],
    ),
    (
        "Code",
        &[
            "--lekton-code-bg / -text / -border",
            "--lekton-code-scheme (light | dark)",
            "--lekton-code-toolbar-text",
            "--lekton-code-copy-border / -hover-bg",
            "--lekton-inline-code-bg / -text / -border",
        ],
    ),
];

/// Side panel listing the theming variables, so a theme can be written
/// without hunting for selectors.
#[component]
fn ThemingReference() -> impl IntoView {
    view! {
        <div class="space-y-4 text-sm">
            <div class="rounded-2xl border border-base-200 bg-base-200/20 p-4">
                <p class="text-xs font-semibold uppercase tracking-[0.14em] text-base-content/70">"Theming variables"</p>
                <p class="mt-3 leading-6 text-base-content/70">
                    "Assign variables on "
                    <code>":root"</code>
                    " (both modes) or on "
                    <code>"[data-theme=\"light\"]"</code>
                    " / "
                    <code>"[data-theme=\"dark\"]"</code>
                    ". Components read them, so a theme does not need component selectors. Full reference and an example theme: "
                    <a
                        class="link link-primary"
                        href="https://github.com/dghilardi/lekton/blob/main/docs/theming.md"
                        target="_blank"
                        rel="noopener noreferrer"
                    >
                        "docs/theming.md"
                    </a>
                    "."
                </p>
            </div>
            {THEMING_GROUPS
                .iter()
                .map(|(title, vars)| {
                    view! {
                        <details class="rounded-2xl border border-base-200 bg-base-200/20 px-4 py-3">
                            <summary class="cursor-pointer text-xs font-semibold uppercase tracking-[0.14em] text-base-content/70">
                                {*title}
                            </summary>
                            <ul class="mt-2 space-y-1 text-base-content/70">
                                {vars
                                    .iter()
                                    .map(|var| view! { <li><code class="text-xs">{*var}</code></li> })
                                    .collect::<Vec<_>>()}
                            </ul>
                        </details>
                    }
                })
                .collect::<Vec<_>>()}
            <p class="px-1 text-xs leading-5 text-base-content/70">
                "Stable hooks when a variable is not enough: "
                <code>"[data-lekton-header]"</code> ", "
                <code>"[data-lekton-sidebar]"</code> ", "
                <code>"[data-lekton-markdown]"</code> ", "
                <code>"[data-lekton-chat-session]"</code> "."
            </p>
        </div>
    }
}
