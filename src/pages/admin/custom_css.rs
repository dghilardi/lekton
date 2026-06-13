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

    let save_action = Action::new_local(move |new_css: &String| {
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
                <div class="p-8 pb-4">
                    <div class="flex items-center gap-3 mb-2">
                        <svg class="w-6 h-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 21a4 4 0 01-4-4V5a2 2 0 012-2h4a2 2 0 012 2v12a4 4 0 01-4 4zm0 0h12a2 2 0 002-2v-4a2 2 0 00-2-2h-2.343M11 7.343l1.172-1.172a4 4 0 115.656 5.656L15 13"></path>
                        </svg>
                        <h2 class="card-title text-2xl">"Theming & Custom CSS"</h2>
                    </div>
                    <p class="text-base-content/60">
                        "Customize the visual appearance of your Lekton instance. The CSS below is injected into every page at runtime."
                    </p>
                </div>

                <div class="p-8 pt-0 flex flex-col gap-6">
                    <div class="grid gap-6 xl:grid-cols-[minmax(0,1fr)_18rem] xl:items-start">
                        <div class="form-control">
                            <textarea
                                class="textarea textarea-bordered min-h-[28rem] xl:min-h-[34rem] resize-y font-mono text-sm leading-7 focus:textarea-primary transition-all shadow-inner bg-base-200/20"
                                placeholder={"/* Example:\n:root,\n[data-theme=\"light\"] {\n  --color-primary: #0f766e;\n  --lekton-content-max-width: 78rem;\n}\n\n[data-theme=\"dark\"] {\n  --color-primary: #2dd4bf;\n}\n\n.navbar {\n  border-bottom-color: color-mix(in oklab, var(--color-primary) 45%, transparent);\n}\n*/"}
                                prop:value=move || css.get()
                                on:input=move |ev| set_css.set(event_target_value(&ev))
                            ></textarea>
                        </div>

                        <div class="space-y-4">
                            <div class="rounded-2xl border border-base-200 bg-base-200/20 p-4">
                                <p class="text-xs font-semibold uppercase tracking-[0.14em] text-base-content/55">"Theme tokens"</p>
                                <div class="mt-3 space-y-2 text-sm text-base-content/70">
                                    <p><code>"--color-primary"</code> " / " <code>"--color-base-100"</code></p>
                                    <p><code>"--color-base-content"</code> " / " <code>"--color-base-300"</code></p>
                                    <p><code>"--radius-box"</code> " / " <code>"--radius-field"</code></p>
                                </div>
                            </div>

                            <div class="rounded-2xl border border-base-200 bg-base-200/20 p-4">
                                <p class="text-xs font-semibold uppercase tracking-[0.14em] text-base-content/55">"Layout tokens"</p>
                                <div class="mt-3 space-y-2 text-sm text-base-content/70">
                                    <p><code>"--lekton-sidebar-width"</code></p>
                                    <p><code>"--lekton-header-height"</code></p>
                                    <p><code>"--lekton-content-max-width"</code></p>
                                </div>
                            </div>

                            <div class="rounded-2xl border border-base-200 bg-base-200/20 p-4">
                                <p class="text-xs font-semibold uppercase tracking-[0.14em] text-base-content/55">"Recommended strategy"</p>
                                <p class="mt-3 text-sm leading-6 text-base-content/70">
                                    "Prefer overriding shared primitives like "
                                    <code>"body"</code>
                                    ", "
                                    <code>".navbar"</code>
                                    ", "
                                    <code>".card"</code>
                                    ", "
                                    <code>".btn"</code>
                                    " and the theme variables above, so custom branding stays consistent across docs, registry, and admin pages."
                                </p>
                            </div>
                        </div>
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
