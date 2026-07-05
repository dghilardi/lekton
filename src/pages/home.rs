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
        <div class="hero min-h-[60vh]">
            <div class="hero-content text-center">
                <div class="max-w-2xl">
                    <h1 class="text-5xl font-bold">"Welcome to Lekton"</h1>
                    <p class="py-6 text-lg text-base-content/70">
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
        <div class="grid grid-cols-1 md:grid-cols-3 gap-6 mt-8">
            <FeatureCard
                title="Dynamic Docs"
                description="CI/CD integration for live documentation updates. No rebuilds needed."
                icon="📝"
            />
            <FeatureCard
                title="Granular RBAC"
                description="Role-based access control ensures sensitive docs are only visible to authorized users."
                icon="🔒"
            />
            <FeatureCard
                title="Schema Registry"
                description="Unified OpenAPI, AsyncAPI, and JSON Schema viewer with versioning."
                icon="📡"
            />
        </div>
    }
}

/// A feature card component for the home page.
#[component]
fn FeatureCard(
    title: &'static str,
    description: &'static str,
    icon: &'static str,
) -> impl IntoView {
    view! {
        <div class="card bg-base-100 shadow-xl hover:shadow-2xl transition-shadow">
            <div class="card-body items-center text-center">
                <span class="text-4xl">{icon}</span>
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
