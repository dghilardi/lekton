use leptos::prelude::*;

use crate::app::{get_navigation, NavItem};
use crate::auth::refresh_client::with_auth_retry;

fn first_doc_href(items: &[NavItem]) -> Option<String> {
    for item in items {
        if !item.slug.is_empty() {
            return Some(format!("/docs/{}", item.slug));
        }
        if let Some(child_href) = first_doc_href(&item.children) {
            return Some(child_href);
        }
    }
    None
}

/// Home page component.
#[component]
pub fn HomePage() -> impl IntoView {
    let navigation_resource = LocalResource::new(|| with_auth_retry(get_navigation));
    let schema_registry_enabled = crate::app::use_feature(|f| f.schema_registry);

    let get_started_href = Signal::derive(move || {
        navigation_resource
            .get()
            .and_then(|result| result.ok())
            .and_then(|items| first_doc_href(&items))
            .unwrap_or_else(|| {
                if schema_registry_enabled.get() {
                    "/schemas".to_string()
                } else {
                    "/login".to_string()
                }
            })
    });

    view! {
        <div class="hero py-16 md:py-24">
            <div class="hero-content text-center">
                <div class="max-w-2xl">
                    <h1 class="text-5xl font-bold text-balance">"Welcome to Lekton"</h1>
                    <p class="py-6 text-lg text-base-content/70 text-pretty">
                        "Your dynamic Internal Developer Portal. Search documentation, explore API schemas, and collaborate — all in one place."
                    </p>
                    <div class="flex gap-4 justify-center">
                        <a href=get_started_href class="btn btn-primary btn-lg">
                            "Get Started"
                        </a>
                        <Show when=move || schema_registry_enabled.get()>
                            <a href="/schemas" class="btn btn-outline btn-lg">
                                "API Schemas"
                            </a>
                        </Show>
                    </div>
                </div>
            </div>
        </div>

        // Feature cards
        <div class="grid grid-cols-1 md:grid-cols-3 gap-6">
            <FeatureCard
                title="Dynamic Docs"
                description="CI/CD integration for live documentation updates. No rebuilds needed."
                icon_path="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
            />
            <FeatureCard
                title="Granular RBAC"
                description="Role-based access control ensures sensitive docs are only visible to authorized users."
                icon_path="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
            />
            <FeatureCard
                title="Schema Registry"
                description="Unified OpenAPI, AsyncAPI, and JSON Schema viewer with versioning."
                icon_path="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"
            />
        </div>
    }
}

/// A feature card component for the home page. Static and informational — no
/// hover affordance, since the card is not a link.
#[component]
fn FeatureCard(
    title: &'static str,
    description: &'static str,
    icon_path: &'static str,
) -> impl IntoView {
    view! {
        <div class="card bg-base-100 shadow-sm border border-base-200">
            <div class="card-body items-center text-center gap-3">
                <div class="p-3 bg-primary/10 rounded-2xl text-primary">
                    <svg class="w-7 h-7" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d=icon_path></path>
                    </svg>
                </div>
                <h2 class="card-title">{title}</h2>
                <p class="text-base-content/70">{description}</p>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::first_doc_href;
    use crate::app::NavItem;

    #[test]
    fn first_doc_href_returns_first_top_level_item() {
        let items = vec![NavItem {
            slug: "getting-started".into(),
            title: "Getting Started".into(),
            parent_slug: None,
            order: 0,
            children: vec![],
        }];

        assert_eq!(
            first_doc_href(&items).as_deref(),
            Some("/docs/getting-started")
        );
    }

    #[test]
    fn first_doc_href_descends_into_children() {
        let items = vec![NavItem {
            slug: String::new(),
            title: "Section".into(),
            parent_slug: None,
            order: 0,
            children: vec![NavItem {
                slug: "guides/intro".into(),
                title: "Intro".into(),
                parent_slug: Some("guides".into()),
                order: 0,
                children: vec![],
            }],
        }];

        assert_eq!(
            first_doc_href(&items).as_deref(),
            Some("/docs/guides/intro")
        );
    }
}
