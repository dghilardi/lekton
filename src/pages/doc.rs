use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::app::get_doc_html;
use crate::auth::refresh_client::with_auth_retry;
use crate::components::MarkdownContent;
use crate::server::document_upload::archive_document;

/// Data returned for rendering a document page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocPageData {
    pub title: String,
    pub html: String,
    pub headings: Vec<crate::rendering::markdown::TocHeading>,
    pub last_updated: String,
    pub tags: Vec<String>,
    /// True when the document was created through the admin upload form, so the
    /// page can offer to edit it via that form rather than the markdown editor.
    pub is_upload_doc: bool,
    /// True when the document is managed by an external source (ingest API /
    /// lekton-sync). Such pages are read-only in the portal — editing them would
    /// be overwritten on the next sync — so no edit affordance is shown.
    pub is_sync_doc: bool,
    /// Asset key of the linked PDF for upload documents. When present (together
    /// with `is_upload_doc`), the page renders a specialized PDF layout with a
    /// prominent download card and the summary, instead of the bare stub body.
    /// `html` then holds only the rendered summary. `None` for other documents.
    pub pdf_asset_key: Option<String>,
    /// For externally-managed (sync) documents whose source is registered with a
    /// recognized provider repo URL, a link to view the source file on that repo.
    /// `None` when the source is unregistered, has no repo URL, or the host is
    /// not a known provider. Shown to all users.
    pub source_view_url: Option<String>,
    /// The import source owning this document, needed to build a release pin.
    /// `None` for documents with no source.
    pub source_id: Option<String>,
    /// The release being shown. `None` when the source is not release-managed.
    pub current_release: Option<String>,
    /// Every release this source has published, newest-published first. Empty
    /// unless versioning is enabled and the source publishes releases; the
    /// selector stays hidden below two entries, since a single release is not a
    /// choice.
    pub releases: Vec<String>,
    /// Which release currently carries the `latest` alias, so the selector can
    /// mark it and link to the unpinned URL for it.
    pub latest_release: Option<String>,
}

/// Breadcrumbs component to show document hierarchy based on slug.
#[component]
fn Breadcrumbs(slug: String) -> impl IntoView {
    let parts: Vec<&str> = slug.split('/').collect();

    let breadcrumb_items: Vec<_> = parts
        .iter()
        .enumerate()
        .map(|(idx, part)| {
            let is_last = idx == parts.len() - 1;
            let path = parts[..=idx].join("/");
            let label = part
                .split('-')
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
        })
        .collect();

    view! {
        <div class="breadcrumbs text-sm">
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
                        let href = format!("/docs/{}", path);
                        view! {
                            <li>
                                <a
                                    href=move || crate::components::pinned_doc_href(&href)
                                    class="hover:underline"
                                >
                                    {label}
                                </a>
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
        }
        .into_any();
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
    }
    .into_any()
}

/// Specialized layout for upload documents backed by a PDF: a prominent
/// open/download card plus the AI-generated summary, in place of the bare
/// markdown stub. Used when the document has a linked PDF asset.
#[component]
fn PdfDocContent(
    title: String,
    /// Pre-rendered, sanitized HTML of the summary (may be empty).
    summary_html: String,
    /// Asset key of the linked PDF (served at `/api/v1/assets/{key}`).
    asset_key: String,
    tags: Vec<String>,
) -> impl IntoView {
    let has_tags = !tags.is_empty();
    let has_summary = !summary_html.trim().is_empty();
    let asset_url = format!("/api/v1/assets/{asset_key}");
    let download_url = asset_url.clone();

    view! {
        <header class="mb-6">
            <h1 class="text-3xl font-bold mb-3 break-words">{title}</h1>
            <Show when=move || has_tags>
                <div class="flex flex-wrap gap-2">
                    {tags.iter().map(|tag| {
                        let tag_text = tag.clone();
                        view! { <span class="badge badge-outline badge-sm">{tag_text}</span> }
                    }).collect::<Vec<_>>()}
                </div>
            </Show>
        </header>

        // Prominent PDF card — the primary affordance for this page.
        <div class="card bg-base-200/60 border border-base-300 mb-8">
            <div class="card-body flex-row items-center gap-4 py-4">
                <div class="flex items-center justify-center w-11 h-11 rounded-lg bg-error/10 text-error flex-shrink-0">
                    <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" />
                    </svg>
                </div>
                <div class="flex-1 min-w-0">
                    <div class="font-semibold leading-tight">"PDF document"</div>
                    <div class="text-sm text-base-content/60">"View the full source file"</div>
                </div>
                <div class="flex gap-2 flex-shrink-0">
                    <a href=asset_url target="_blank" rel="noopener"
                        class="btn btn-primary btn-sm gap-1.5">
                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                        </svg>
                        "Open"
                    </a>
                    <a href=download_url download
                        class="btn btn-ghost btn-sm gap-1.5">
                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                        </svg>
                        "Download"
                    </a>
                </div>
            </div>
        </div>

        // AI-generated summary of the document.
        <Show when=move || has_summary>
            <section>
                <h2 class="text-xs font-semibold uppercase tracking-wider text-base-content/65 mb-3">
                    "Summary"
                </h2>
                <div class="prose prose-lg max-w-none">
                    <MarkdownContent html=summary_html.clone() />
                </div>
            </section>
        </Show>
    }
}

/// Document viewer page — renders markdown content fetched from S3.
#[component]
pub fn DocPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let slug = move || params.read().get("slug").unwrap_or_default();

    // Release pins live in the URL, so changing one re-runs the resource and the
    // page re-resolves against the newly pinned release.
    let query = leptos_router::hooks::use_query_map();
    let pins = move || {
        query
            .read()
            .get_all(crate::versioning::PIN_PARAM)
            .unwrap_or_default()
    };

    let doc_resource = LocalResource::new(move || {
        let slug = slug();
        let pins = pins();
        with_auth_retry(move || get_doc_html(slug.clone(), Some(pins.clone())))
    });

    let show_archive_confirm = RwSignal::new(false);
    let archiving = RwSignal::new(false);
    let archive_error = RwSignal::new(Option::<String>::None);
    let navigate = leptos_router::hooks::use_navigate();

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
                        let editor_enabled = crate::app::use_feature(|f| f.editor);
                        let upload_enabled = crate::app::use_feature(|f| f.document_upload);
                        let is_admin = move || {
                            current_user
                                .and_then(|s| s.get())
                                .map(|u| u.is_admin)
                                .unwrap_or(false)
                        };
                        let is_upload_doc = data.is_upload_doc;
                        let is_sync_doc = data.is_sync_doc;
                        // Edit affordance by provenance: upload-origin docs use the
                        // upload form, hand-made docs use the markdown editor, and
                        // externally-managed (sync) docs are read-only (no button).
                        let can_edit_upload = move || upload_enabled.get() && is_admin() && is_upload_doc;
                        let can_edit = move || {
                            editor_enabled.get() && is_admin() && !is_upload_doc && !is_sync_doc
                        };
                        let edit_href = format!("/edit/{current_slug}");
                        let upload_edit_href = format!("/admin/upload?edit={current_slug}");
                        // Externally-managed docs are read-only in the portal, but
                        // when their source repo is registered with a recognized
                        // provider we link out to the file. Shown to all users.
                        let source_view_url = data.source_view_url.clone();

                        // Release selector inputs; the component hides itself when
                        // the source has fewer than two releases.
                        let sel_source_id = data.source_id.clone();
                        let sel_current = data.current_release.clone();
                        let sel_releases = data.releases.clone();
                        let sel_latest = data.latest_release.clone();

                        // Upload documents backed by a PDF get a specialized layout
                        // (download card + summary, no table of contents); everything
                        // else renders the full markdown body with its ToC.
                        let render_pdf = is_upload_doc && data.pdf_asset_key.is_some();
                        let content = if render_pdf {
                            let asset_key = data.pdf_asset_key.clone().unwrap_or_default();
                            view! {
                                <PdfDocContent
                                    title=data.title.clone()
                                    summary_html=data.html.clone()
                                    asset_key=asset_key
                                    tags=tags.clone()
                                />
                            }.into_any()
                        } else {
                            view! {
                                // Tags — shown between breadcrumb and content
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
                                // The markdown H1 serves as the page title — no separate h1 here
                                <article class="prose prose-lg max-w-none">
                                    <MarkdownContent html=data.html />
                                </article>
                            }.into_any()
                        };
                        let toc = if render_pdf {
                            ().into_any()
                        } else {
                            view! { <TableOfContents headings=data.headings /> }.into_any()
                        };
                        view! {
                            <div class="flex gap-8 items-start">
                                <div class="flex-1 min-w-0 max-w-4xl">
                                    // Breadcrumb row + edit button — single meta strip
                                    <div class="flex items-center justify-between gap-4 mb-5">
                                        <Breadcrumbs slug=current_slug.clone() />
                                        <crate::components::ReleaseSelector
                                            source_id=sel_source_id.clone()
                                            current_release=sel_current.clone()
                                            releases=sel_releases.clone()
                                            latest_release=sel_latest.clone()
                                        />
                                        <Show when=can_edit>
                                            <a
                                                href=edit_href.clone()
                                                class="btn btn-ghost btn-sm flex-shrink-0 gap-1.5 text-base-content/60 hover:text-primary"
                                            >
                                                <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                        d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z">
                                                    </path>
                                                </svg>
                                                "Edit"
                                            </a>
                                        </Show>
                                        <Show when=can_edit_upload>
                                            <a
                                                href=upload_edit_href.clone()
                                                class="btn btn-ghost btn-sm flex-shrink-0 gap-1.5 text-base-content/60 hover:text-primary"
                                            >
                                                <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                        d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z">
                                                    </path>
                                                </svg>
                                                "Edit"
                                            </a>
                                        </Show>
                                        <Show when=can_edit_upload>
                                            <button
                                                type="button"
                                                on:click=move |_| show_archive_confirm.set(true)
                                                class="btn btn-ghost btn-sm flex-shrink-0 gap-1.5 text-base-content/60 hover:text-error"
                                            >
                                                <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                        d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16">
                                                    </path>
                                                </svg>
                                                "Archive"
                                            </button>
                                        </Show>
                                        <Show when={
                                            let has = source_view_url.is_some();
                                            move || has
                                        }>
                                            <a
                                                href=source_view_url.clone().unwrap_or_default()
                                                target="_blank"
                                                rel="noopener"
                                                class="btn btn-ghost btn-sm flex-shrink-0 gap-1.5 text-base-content/60 hover:text-primary"
                                            >
                                                <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                        d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14">
                                                    </path>
                                                </svg>
                                                "View source"
                                            </a>
                                        </Show>
                                    </div>
                                    {content}
                                    // Last Updated footer
                                    <div class="divider mt-12"></div>
                                    <div class="flex items-center gap-2 text-sm text-base-content/65 pb-4">
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z">
                                            </path>
                                        </svg>
                                        <span>"Last updated: " {data.last_updated}</span>
                                    </div>
                                </div>
                                {toc}
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
        <Show when=move || show_archive_confirm.get()>
            {
                let navigate = navigate.clone();
                view! {
                    <div class="modal modal-open">
                        <div class="modal-box">
                            <h3 class="font-bold text-lg">"Archive this document?"</h3>
                            <p class="py-4 text-sm text-base-content/70">
                                "The page will no longer be listed or searchable. The linked PDF is kept and can be re-linked from a new upload. This can be undone by re-uploading with the same slug."
                            </p>
                            <Show when=move || archive_error.get().is_some()>
                                <div class="alert alert-error text-sm mb-2">
                                    <span>{move || archive_error.get().unwrap_or_default()}</span>
                                </div>
                            </Show>
                            <div class="modal-action">
                                <button
                                    type="button"
                                    class="btn btn-ghost"
                                    disabled=move || archiving.get()
                                    on:click=move |_| show_archive_confirm.set(false)
                                >
                                    "Cancel"
                                </button>
                                <button
                                    type="button"
                                    class="btn btn-error"
                                    disabled=move || archiving.get()
                                    on:click=move |_| {
                                        let current_slug = slug();
                                        let navigate = navigate.clone();
                                        archiving.set(true);
                                        archive_error.set(None);
                                        leptos::task::spawn_local(async move {
                                            match with_auth_retry(move || archive_document(current_slug.clone())).await {
                                                Ok(()) => {
                                                    navigate("/", Default::default());
                                                }
                                                Err(e) => {
                                                    archiving.set(false);
                                                    archive_error.set(Some(e.to_string()));
                                                }
                                            }
                                        });
                                    }
                                >
                                    {move || if archiving.get() { "Archiving…" } else { "Archive" }}
                                </button>
                            </div>
                        </div>
                    </div>
                }
            }
        </Show>
    }
}
