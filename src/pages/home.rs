use leptos::prelude::*;

use crate::app::NavItem;

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

/// Whether a navigation entry is an onboarding page, by slug or title.
fn is_getting_started(item: &NavItem) -> bool {
    let segment = item.slug.rsplit('/').next().unwrap_or_default();
    let title = item.title.to_ascii_lowercase();
    [
        "getting-started",
        "get-started",
        "quickstart",
        "quick-start",
    ]
    .iter()
    .any(|name| segment.starts_with(name))
        || title.starts_with("getting started")
        || title.starts_with("get started")
        || title.starts_with("quick start")
        || title.starts_with("quickstart")
}

/// Where "Get Started" leads: an onboarding page when the portal has one
/// (shallowest first), otherwise the first section's landing, otherwise the
/// first document. Navigation order alone can put a policy page first.
fn get_started_href(items: &[NavItem]) -> Option<String> {
    let mut level: Vec<&NavItem> = items.iter().collect();
    while !level.is_empty() {
        if let Some(item) = level
            .iter()
            .find(|item| !item.slug.is_empty() && is_getting_started(item))
        {
            return Some(format!("/docs/{}", item.slug));
        }
        level = level.iter().flat_map(|item| item.children.iter()).collect();
    }
    items
        .iter()
        .find(|item| !item.children.is_empty())
        .and_then(|section| {
            if section.slug.is_empty() {
                first_doc_href(&section.children)
            } else {
                Some(format!("/docs/{}", section.slug))
            }
        })
        .or_else(|| first_doc_href(items))
}

/// Home page component.
#[component]
pub fn HomePage() -> impl IntoView {
    // The landing page follows the URL's release pins like the rest of the
    // shell, through the layout's shared navigation.
    let navigation_resource = crate::components::use_navigation();
    let current_user = expect_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
    let schema_registry_enabled = crate::app::use_feature(|f| f.schema_registry);

    let auth_resolved = expect_context::<crate::app::AuthResolved>().0;
    let get_started_action = Signal::derive(move || {
        let navigation = navigation_resource.get().and_then(Result::ok);
        if !auth_resolved.get() || navigation.is_none() {
            return (None, "Get Started");
        }
        if let Some(href) = navigation.as_deref().and_then(get_started_href) {
            return (
                Some(crate::components::pinned_doc_href(&href)),
                "Get Started",
            );
        }
        if current_user.get().is_none() {
            (Some("/login".to_string()), "Log in to explore")
        } else if schema_registry_enabled.get() {
            (Some("/schemas".to_string()), "Get Started")
        } else {
            (Some("/profile".to_string()), "View your profile")
        }
    });

    view! {
        <div class="hero py-16 md:py-24">
            <div class="hero-content text-center">
                <div class="max-w-2xl">
                    <h1 class="text-5xl font-bold text-balance">"Welcome to Lekton"</h1>
                    <p class="py-6 text-lg text-base-content/70 text-pretty">
                        "Your dynamic Internal Developer Portal. Search documentation, explore API schemas, and collaborate — all in one place."
                    </p>
                    <Suspense fallback=|| view! { <span class="btn btn-primary btn-lg" aria-disabled="true">"Get Started"</span> }>
                    <div class="flex flex-wrap gap-4 justify-center">
                        <a href=move || get_started_action.get().0 aria-disabled=move || get_started_action.get().0.is_none().to_string() class="btn btn-primary btn-lg">
                            {move || get_started_action.get().1}
                        </a>
                        <Show when=move || schema_registry_enabled.get() && get_started_action.get().0.as_deref() != Some("/schemas")>
                            <a href="/schemas" class="btn btn-outline btn-lg">
                                "API Schemas"
                            </a>
                        </Show>
                    </div>
                    </Suspense>
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
    use super::{first_doc_href, get_started_href};
    use crate::app::NavItem;

    fn item(slug: &str, title: &str, children: Vec<NavItem>) -> NavItem {
        NavItem {
            slug: slug.into(),
            title: title.into(),
            parent_slug: slug.rsplit_once('/').map(|(parent, _)| parent.into()),
            order: 0,
            children,
        }
    }

    #[test]
    fn get_started_prefers_an_onboarding_page_over_the_first_entry() {
        let items = vec![
            item("data-retention-policy", "Data Retention Policy", vec![]),
            item(
                "handbook",
                "Handbook",
                vec![item("handbook/getting-started", "Getting started", vec![])],
            ),
        ];
        assert_eq!(
            get_started_href(&items).as_deref(),
            Some("/docs/handbook/getting-started")
        );

        let items = vec![
            item(
                "handbook",
                "Handbook",
                vec![item("handbook/faq", "FAQ", vec![])],
            ),
            item("lekton", "Get started with Lekton", vec![]),
        ];
        assert_eq!(get_started_href(&items).as_deref(), Some("/docs/lekton"));
    }

    #[test]
    fn get_started_falls_back_to_the_first_section_then_the_first_document() {
        let items = vec![
            item("data-retention-policy", "Data Retention Policy", vec![]),
            item(
                "handbook",
                "Handbook",
                vec![item("handbook/faq", "FAQ", vec![])],
            ),
        ];
        assert_eq!(get_started_href(&items).as_deref(), Some("/docs/handbook"));

        let items = vec![item("policy", "Policy", vec![]), item("faq", "FAQ", vec![])];
        assert_eq!(get_started_href(&items).as_deref(), Some("/docs/policy"));
    }

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
