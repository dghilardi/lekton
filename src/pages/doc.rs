use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::app::get_doc_html;
use crate::auth::refresh_client::with_auth_retry;
use crate::components::MarkdownContent;

/// Data returned for rendering a document page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocPageData {
    pub title: String,
    pub html: String,
    pub headings: Vec<crate::rendering::markdown::TocHeading>,
    pub last_updated: String,
    pub tags: Vec<String>,
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
                        view! {
                            <li>
                                <a href=format!("/docs/{}", path) class="hover:underline">{label}</a>
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

/// Document viewer page — renders markdown content fetched from S3.
#[component]
pub fn DocPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let slug = move || params.read().get("slug").unwrap_or_default();

    let doc_resource = LocalResource::new(move || {
        let slug = slug();
        with_auth_retry(move || get_doc_html(slug.clone()))
    });

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
                        let can_edit = move || {
                            current_user
                                .and_then(|s| s.get())
                                .map(|u| u.is_admin)
                                .unwrap_or(false)
                        };
                        view! {
                            <div class="flex gap-8 items-start">
                                <div class="flex-1 min-w-0">
                                    // Breadcrumb row + edit button — single meta strip
                                    <div class="flex items-center justify-between gap-4 mb-5">
                                        <Breadcrumbs slug=current_slug.clone() />
                                        <Show when=can_edit>
                                            <a
                                                href={let s = current_slug.clone(); move || format!("/edit/{}", s)}
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
                                    </div>
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
                                    // Last Updated footer
                                    <div class="divider mt-12"></div>
                                    <div class="flex items-center gap-2 text-sm text-base-content/50 pb-4">
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z">
                                            </path>
                                        </svg>
                                        <span>"Last updated: " {data.last_updated}</span>
                                    </div>
                                </div>
                                <TableOfContents headings=data.headings />
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
    }
}
