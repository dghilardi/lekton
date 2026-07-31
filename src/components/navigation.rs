use leptos::prelude::*;

use crate::app::{get_navigation, NavItem};
use crate::auth::refresh_client::with_auth_retry;

/// Recursive navigation item component for rendering tree structure.
#[component]
pub fn NavigationItem(item: NavItem, #[prop(optional)] level: u32) -> impl IntoView {
    let has_children = !item.children.is_empty();
    let slug = item.slug.clone();
    let children = item.children.clone();

    if has_children {
        if level >= 3 {
            view! {
                <li class="menu-title text-[10px] mt-2 mb-1">{item.title}</li>
                {children.into_iter().map(|child| {
                    view! {
                        <NavigationItem item=child level=level + 1 />
                    }
                }).collect::<Vec<_>>()}
            }
            .into_any()
        } else {
            view! {
                <li>
                    <details open=true>
                        <summary class="hover:bg-base-200/50 transition-colors font-medium text-base-content/80 text-sm hover:text-base-content">{item.title}</summary>
                        <ul class="before:w-[1px] before:bg-base-300 ml-2 border-l border-base-200/50 mt-1">
                            {children.into_iter().map(|child| {
                                view! {
                                    <NavigationItem item=child level=level + 1 />
                                }
                            }).collect::<Vec<_>>()}
                        </ul>
                    </details>
                </li>
            }.into_any()
        }
    } else {
        let location = leptos_router::hooks::use_location();
        let path = format!("/docs/{}", slug);
        let href_for_check = path.clone();
        let is_active = move || location.pathname.get() == href_for_check;
        view! {
            <li>
                <a
                    href=move || crate::components::pinned_doc_href(&path)
                    aria-current=move || if is_active() { Some("page") } else { None }
                    class="hover:bg-base-200/50 hover:text-primary transition-colors text-base-content/70 text-sm py-1.5"
                >
                    {item.title}
                </a>
            </li>
        }.into_any()
    }
}

/// Navigation tree component that fetches and renders the sidebar navigation.
#[component]
pub fn NavigationTree() -> impl IntoView {
    // The pins are part of the URL, so the tree re-resolves whenever they change.
    let query = leptos_router::hooks::use_query_map();
    let nav_resource = LocalResource::new(move || {
        let pins = query
            .read()
            .get_all(crate::versioning::PIN_PARAM)
            .unwrap_or_default();
        with_auth_retry(move || get_navigation(Some(pins.clone())))
    });

    let location = leptos_router::hooks::use_location();

    view! {
        <Suspense fallback=move || view! {
            <li><span class="loading loading-spinner loading-sm"></span></li>
        }>
            {move || {
                nav_resource.try_get().flatten().map(|result| match result {
                    Ok(items) => {
                        let path = location.pathname.get();
                        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                        let current_root = if parts.len() >= 2 && parts[0] == "docs" {
                            parts[1].to_string()
                        } else {
                            String::new()
                        };
                        // Root-level sections live in the navbar only.
                        // The sidebar shows the *children* of the selected section.
                        // If the current page is a leaf (no children) or unknown,
                        // fall back to showing all top-level items.
                        let display_items = if current_root.is_empty() {
                            vec![]
                        } else {
                            match items.iter().position(|i| i.slug == current_root) {
                                Some(idx) if !items[idx].children.is_empty() => {
                                    items[idx].children.clone()
                                }
                                _ => items,
                            }
                        };
                        view! {
                            {display_items.into_iter().map(|item| {
                                view! {
                                    <NavigationItem item=item level=0 />
                                }
                            }).collect::<Vec<_>>()}
                        }.into_any()
                    }
                    Err(e) => {
                        view! {
                            <li class="text-error">{format!("Error loading navigation: {}", e)}</li>
                        }.into_any()
                    }
                })
            }}
        </Suspense>
    }
}
