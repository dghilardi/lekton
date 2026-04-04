use leptos::prelude::*;

use crate::app::{get_navigation, NavItem};

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
            }.into_any()
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
        view! {
            <li>
                <a
                    href=format!("/docs/{}", slug)
                    class="hover:bg-base-200/50 hover:text-primary transition-colors text-base-content/70 data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium text-sm py-1.5"
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
    let nav_resource = Resource::new(
        || (),
        |_| get_navigation(),
    );

    let location = leptos_router::hooks::use_location();
    let active_root = Signal::derive(move || {
        let path = location.pathname.get();
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() >= 2 && parts[0] == "docs" {
            parts[1].to_string()
        } else {
            String::new()
        }
    });

    view! {
        <Suspense fallback=move || view! {
            <li><span class="loading loading-spinner loading-sm"></span></li>
        }>
            {move || {
                nav_resource.get().map(|result| match result {
                    Ok(items) => {
                        let current_root = active_root.get();
                        // Root-level sections (docs, hackday, …) live in the
                        // navbar only.  The sidebar shows the *children* of
                        // whichever section is currently selected.
                        let display_items = if current_root.is_empty() {
                            vec![]
                        } else {
                            if let Some(root_item) = items.into_iter().find(|i| i.slug == current_root) {
                                root_item.children
                            } else {
                                vec![]
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
