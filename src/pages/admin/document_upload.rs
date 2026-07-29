//! Admin guided document-upload form: upload a PDF, assign metadata, optionally
//! generate an AI summary, and create (or edit) a document that links to it.

use leptos::prelude::*;

use crate::server::document_upload::{
    get_document_for_edit, save_document_with_attachment, DocumentUploadForm,
};

#[cfg(feature = "hydrate")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "hydrate")]
#[wasm_bindgen(module = "/public/js/document-upload.js")]
extern "C" {
    #[wasm_bindgen(js_name = "uploadDocumentPdf")]
    fn upload_document_pdf_js() -> js_sys::Promise;

    #[wasm_bindgen(js_name = "streamDocumentSummary")]
    fn stream_document_summary_js(key: &str) -> js_sys::Promise;
}

/// Result of the JS upload helper (a JSON string).
#[cfg(feature = "hydrate")]
#[derive(serde::Deserialize)]
struct UploadJsResult {
    key: Option<String>,
    file_name: Option<String>,
    error: Option<String>,
}

/// Flatten the navigation tree into `(slug, indented_title)` pairs for the
/// parent-document picker.
fn flatten_nav(items: &[crate::app::NavItem], depth: usize, out: &mut Vec<(String, String)>) {
    for item in items {
        let prefix = "\u{00a0}\u{00a0}".repeat(depth);
        out.push((item.slug.clone(), format!("{prefix}{}", item.title)));
        flatten_nav(&item.children, depth + 1, out);
    }
}

#[component]
pub fn DocumentUploadManager() -> impl IntoView {
    let rag_enabled = crate::app::use_feature(|f| f.rag);

    // Edit mode is driven by an `?edit=<slug>` query parameter.
    let query = leptos_router::hooks::use_query_map();
    let edit_slug = Memo::new(move |_| query.read().get("edit").filter(|s| !s.is_empty()));

    let levels_resource = LocalResource::new(list_levels);
    let nav_resource = LocalResource::new(|| crate::server::nav::get_navigation(vec![]));
    let edit_resource = LocalResource::new(move || {
        let slug = edit_slug.get();
        async move {
            match slug {
                Some(s) => Some(get_document_for_edit(s).await),
                None => None,
            }
        }
    });

    let (title, set_title) = signal(String::new());
    let (summary, set_summary) = signal(String::new());
    let (access_level, set_access_level) = signal(String::new());
    let (parent_slug, set_parent_slug) = signal(String::new());
    let (order, set_order) = signal(0u32);
    let (asset_key, set_asset_key) = signal(String::new());
    let (file_name, set_file_name) = signal(String::new());

    let (uploading, set_uploading) = signal(false);
    let (generating, set_generating) = signal(false);
    let (error_msg, set_error_msg) = signal(Option::<String>::None);
    let (success_msg, set_success_msg) = signal(Option::<String>::None);
    // The slug currently loaded into the form, so prefill only runs once per edit.
    let (loaded_slug, set_loaded_slug) = signal(Option::<String>::None);

    // Prefill the form when editing.
    Effect::new(move || {
        if let Some(Some(Ok(data))) = edit_resource.get() {
            if loaded_slug.get_untracked().as_deref() != Some(data.slug.as_str()) {
                set_loaded_slug.set(Some(data.slug.clone()));
                set_title.set(data.title);
                set_summary.set(data.summary);
                set_access_level.set(data.access_level);
                set_parent_slug.set(data.parent_slug.unwrap_or_default());
                set_order.set(data.order);
                set_asset_key.set(data.asset_key.clone());
                set_file_name.set(
                    data.asset_key
                        .rsplit('/')
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                );
            }
        }
    });

    // JsFuture is !Send, so we can't use Action::new (which requires Send).
    // Mirror the on_upload pattern: plain closure + spawn_local.
    let on_generate = move |_| {
        let key = asset_key.get();
        if key.is_empty() {
            return;
        }
        set_error_msg.set(None);
        #[cfg(feature = "hydrate")]
        {
            set_generating.set(true);
            leptos::task::spawn_local(async move {
                let result =
                    wasm_bindgen_futures::JsFuture::from(stream_document_summary_js(&key)).await;
                match result {
                    Ok(val) => {
                        if let Some(text) = val.as_string() {
                            set_summary.set(text);
                        }
                    }
                    Err(e) => {
                        let msg =
                            js_sys::Reflect::get(&e, &wasm_bindgen::JsValue::from_str("message"))
                                .ok()
                                .and_then(|v| v.as_string())
                                .unwrap_or_else(|| "Summary generation failed".into());
                        set_error_msg.set(Some(msg));
                    }
                }
                set_generating.set(false);
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = key;
    };
    #[cfg(not(feature = "hydrate"))]
    let _ = set_generating;

    let save = Action::new(move |_: &()| {
        let form = DocumentUploadForm {
            slug: edit_slug.get_untracked(),
            title: title.get_untracked(),
            summary: summary.get_untracked(),
            access_level: access_level.get_untracked(),
            asset_key: asset_key.get_untracked(),
            parent_slug: {
                let p = parent_slug.get_untracked();
                if p.is_empty() {
                    None
                } else {
                    Some(p)
                }
            },
            order: order.get_untracked(),
        };
        async move {
            set_error_msg.set(None);
            set_success_msg.set(None);
            if form.title.trim().is_empty() {
                set_error_msg.set(Some("Title is required".into()));
                return;
            }
            if form.access_level.trim().is_empty() {
                set_error_msg.set(Some("Select an access level".into()));
                return;
            }
            if form.asset_key.is_empty() {
                set_error_msg.set(Some("Upload a PDF first".into()));
                return;
            }
            match save_document_with_attachment(form).await {
                Ok(slug) => {
                    set_success_msg.set(Some(slug));
                }
                Err(e) => set_error_msg.set(Some(clean_err(&e.to_string()))),
            }
        }
    });
    let saving = save.pending();

    // Trigger the JS file picker + upload (hydrate only).
    let on_upload = move |_| {
        set_error_msg.set(None);
        #[cfg(feature = "hydrate")]
        {
            set_uploading.set(true);
            leptos::task::spawn_local(async move {
                let result = wasm_bindgen_futures::JsFuture::from(upload_document_pdf_js()).await;
                set_uploading.set(false);
                match result {
                    Ok(val) => {
                        // null = cancelled.
                        if val.is_null() || val.is_undefined() {
                            return;
                        }
                        if let Some(json) = val.as_string() {
                            match serde_json::from_str::<UploadJsResult>(&json) {
                                Ok(r) => {
                                    if let Some(err) = r.error {
                                        set_error_msg.set(Some(err));
                                    } else if let Some(key) = r.key {
                                        set_asset_key.set(key);
                                        set_file_name.set(r.file_name.unwrap_or_default());
                                    }
                                }
                                Err(e) => set_error_msg.set(Some(format!("Upload error: {e}"))),
                            }
                        }
                    }
                    Err(_) => set_error_msg.set(Some("Upload failed".into())),
                }
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = set_uploading;
    };

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200 overflow-hidden">
            <div class="card-body p-8 space-y-6">
                <Show when=move || edit_slug.get().is_some()>
                    <h2 class="card-title text-xl">"Edit document"</h2>
                </Show>

                {move || error_msg.get().map(|e| view! {
                    <div class="alert alert-error text-sm">
                        <span>{e}</span>
                        <button class="btn btn-ghost btn-xs" on:click=move |_| set_error_msg.set(None)>"✕"</button>
                    </div>
                })}

                {move || success_msg.get().map(|slug| {
                    let href = format!("/docs/{slug}");
                    view! {
                        <div class="alert alert-success text-sm">
                            <span>"Document saved."</span>
                            <a class="link link-hover font-medium" href=href>"Open it →"</a>
                        </div>
                    }
                })}

                // ── PDF upload ───────────────────────────────────────────────
                <div class="form-control">
                    <label class="label"><span class="label-text font-medium">"PDF file"</span></label>
                    <div class="flex items-center gap-3">
                        <button
                            class="btn btn-outline btn-sm"
                            prop:disabled=move || uploading.get()
                            on:click=on_upload
                        >
                            {move || if uploading.get() {
                                view! { <span class="loading loading-spinner loading-xs"></span> }.into_any()
                            } else {
                                view! { <span>"Choose PDF…"</span> }.into_any()
                            }}
                        </button>
                        <span class="text-sm text-base-content/70">
                            {move || {
                                let f = file_name.get();
                                if f.is_empty() { "No file selected".to_string() } else { f }
                            }}
                        </span>
                    </div>
                </div>

                // ── Title ────────────────────────────────────────────────────
                <div class="form-control">
                    <label class="label"><span class="label-text font-medium">"Title"</span></label>
                    <input
                        type="text"
                        class="input input-bordered w-full"
                        placeholder="e.g. Employee Handbook 2026"
                        prop:value=move || title.get()
                        on:input=move |e| set_title.set(event_target_value(&e))
                    />
                </div>

                // ── Description / summary ────────────────────────────────────
                <div class="form-control">
                    <div class="flex items-center justify-between">
                        <label class="label"><span class="label-text font-medium">"Description"</span></label>
                        <Show when=move || rag_enabled.get()>
                            <button
                                class="btn btn-ghost btn-xs gap-1 text-primary hover:bg-primary/10"
                                prop:disabled=move || generating.get() || asset_key.get().is_empty()
                                on:click=on_generate
                            >
                                {move || if generating.get() {
                                    view! { <span class="loading loading-spinner loading-xs"></span> }.into_any()
                                } else {
                                    view! { <span>"✨ Generate with AI"</span> }.into_any()
                                }}
                            </button>
                        </Show>
                    </div>
                    <textarea
                        class="textarea textarea-bordered w-full min-h-24"
                        placeholder="A short description of what this document covers."
                        prop:value=move || summary.get()
                        on:input=move |e| set_summary.set(event_target_value(&e))
                    ></textarea>
                </div>

                // ── Access level ─────────────────────────────────────────────
                <div class="form-control">
                    <label class="label"><span class="label-text font-medium">"Access level"</span></label>
                    <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
                        {move || {
                            let levels = levels_resource.get().and_then(|r| r.ok()).unwrap_or_default();
                            view! {
                                <select
                                    class="select select-bordered w-full"
                                    prop:value=move || access_level.get()
                                    on:change=move |e| set_access_level.set(event_target_value(&e))
                                >
                                    <option value="" disabled selected=move || access_level.get().is_empty()>
                                        "Select an access level…"
                                    </option>
                                    {levels.into_iter().map(|l| {
                                        let name = l.name.clone();
                                        let name_sel = l.name.clone();
                                        let label = if l.label.is_empty() { l.name.clone() } else { l.label.clone() };
                                        view! {
                                            <option value=name.clone() selected=move || access_level.get() == name_sel>
                                                {format!("{label} ({name})")}
                                            </option>
                                        }
                                    }).collect::<Vec<_>>()}
                                </select>
                            }
                        }}
                    </Suspense>
                </div>

                // ── Parent + order ───────────────────────────────────────────
                <div class="grid grid-cols-1 sm:grid-cols-3 gap-4">
                    <div class="form-control sm:col-span-2">
                        <label class="label"><span class="label-text font-medium">"Parent (optional)"</span></label>
                        <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
                            {move || {
                                let nav = nav_resource.get().and_then(|r| r.ok()).unwrap_or_default();
                                let mut flat = Vec::new();
                                flatten_nav(&nav, 0, &mut flat);
                                view! {
                                    <select
                                        class="select select-bordered w-full"
                                        prop:value=move || parent_slug.get()
                                        on:change=move |e| set_parent_slug.set(event_target_value(&e))
                                    >
                                        <option value="">"— Top level —"</option>
                                        {flat.into_iter().map(|(slug, label)| {
                                            let slug_sel = slug.clone();
                                            view! {
                                                <option value=slug.clone() selected=move || parent_slug.get() == slug_sel>
                                                    {label}
                                                </option>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                }
                            }}
                        </Suspense>
                    </div>
                    <div class="form-control">
                        <label class="label"><span class="label-text font-medium">"Order"</span></label>
                        <input
                            type="number"
                            min="0"
                            class="input input-bordered w-full"
                            prop:value=move || order.get().to_string()
                            on:input=move |e| {
                                if let Ok(n) = event_target_value(&e).parse::<u32>() {
                                    set_order.set(n);
                                }
                            }
                        />
                    </div>
                </div>

                // ── Submit ───────────────────────────────────────────────────
                <div class="flex justify-end pt-2">
                    <button
                        class="btn btn-primary"
                        prop:disabled=move || saving.get()
                        on:click=move |_| { save.dispatch(()); }
                    >
                        {move || if saving.get() {
                            view! { <span class="loading loading-spinner loading-sm"></span> }.into_any()
                        } else if edit_slug.get().is_some() {
                            view! { <span>"Save changes"</span> }.into_any()
                        } else {
                            view! { <span>"Create document"</span> }.into_any()
                        }}
                    </button>
                </div>
            </div>
        </div>
    }
}

/// Wrapper so `LocalResource` gets a plain async fn (the server fn lives in the
/// access-levels server module).
async fn list_levels() -> Result<Vec<crate::server::access_levels::AccessLevelInfo>, ServerFnError>
{
    crate::server::access_levels::list_admin_access_levels().await
}

/// Strip the `ServerFnError`/sentinel noise from an error string for display.
fn clean_err(e: &str) -> String {
    e.strip_prefix("error running server function: ")
        .unwrap_or(e)
        .to_string()
}
