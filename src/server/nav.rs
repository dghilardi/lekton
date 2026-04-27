#[cfg(feature = "ssr")]
use crate::app::AppState;
use crate::app::NavItem;
#[cfg(feature = "ssr")]
use crate::server::request_document_visibility;
use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

pub use crate::db::navigation_order_repository::NavigationOrderEntry;

use crate::db::settings_repository::NavGroup;

#[server(GetNavigation, "/api")]
pub async fn get_navigation() -> Result<Vec<NavItem>, ServerFnError> {
    use std::collections::HashMap;

    let state = expect_context::<AppState>();

    let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
    let (docs, nav_order_entries) = tokio::join!(
        state
            .document_repo
            .list_by_access_levels(allowed_levels.as_deref(), include_draft),
        state.navigation_order_repo.list_all(),
    );
    let docs = docs.map_err(|e| ServerFnError::new(e.to_string()))?;
    let nav_order_entries = nav_order_entries.map_err(|e| ServerFnError::new(e.to_string()))?;

    let nav_weights: HashMap<String, i32> = nav_order_entries
        .into_iter()
        .map(|e| (e.slug, e.weight))
        .collect();

    let all_items: Vec<NavItem> = docs
        .into_iter()
        .map(|doc| {
            let parent_slug = doc.parent_slug.or_else(|| {
                if let Some((parent, _)) = doc.slug.rsplit_once('/') {
                    Some(parent.to_string())
                } else {
                    None
                }
            });
            NavItem {
                slug: doc.slug,
                title: doc.title,
                parent_slug,
                order: doc.order,
                children: vec![],
            }
        })
        .collect();

    let mut items_by_slug: HashMap<String, NavItem> = all_items
        .iter()
        .cloned()
        .map(|item| (item.slug.clone(), item))
        .collect();

    for item in &all_items {
        let mut current_parent = item.parent_slug.clone();
        while let Some(parent_slug) = current_parent {
            if !items_by_slug.contains_key(&parent_slug) {
                let title_part = parent_slug.split('/').next_back().unwrap_or(&parent_slug);
                let title = title_part
                    .split('-')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");

                let next_parent = if let Some((p, _)) = parent_slug.rsplit_once('/') {
                    Some(p.to_string())
                } else {
                    None
                };

                let missing_node = NavItem {
                    slug: parent_slug.clone(),
                    title,
                    parent_slug: next_parent.clone(),
                    order: 0,
                    children: vec![],
                };

                items_by_slug.insert(parent_slug.clone(), missing_node);
                current_parent = next_parent;
            } else {
                break;
            }
        }
    }

    let mut roots = Vec::new();
    let mut children_by_parent: HashMap<String, Vec<NavItem>> = HashMap::new();

    for (_slug, item) in items_by_slug.into_iter() {
        if let Some(parent) = &item.parent_slug {
            children_by_parent
                .entry(parent.clone())
                .or_default()
                .push(item);
        } else {
            roots.push(item);
        }
    }

    fn attach_children(item: &mut NavItem, children_map: &HashMap<String, Vec<NavItem>>) {
        if let Some(children) = children_map.get(&item.slug) {
            item.children = children.clone();
            for child in &mut item.children {
                attach_children(child, children_map);
            }
        }
    }

    for root in &mut roots {
        attach_children(root, &children_by_parent);
    }

    fn sort_nav_items(items: &mut [NavItem], weights: &HashMap<String, i32>) {
        items.sort_by(|a, b| {
            let a_is_section = !a.children.is_empty();
            let b_is_section = !b.children.is_empty();

            let a_sort_key = if a_is_section {
                weights.get(&a.slug).copied().unwrap_or(i32::MAX)
            } else {
                a.order as i32
            };
            let b_sort_key = if b_is_section {
                weights.get(&b.slug).copied().unwrap_or(i32::MAX)
            } else {
                b.order as i32
            };

            a_sort_key
                .cmp(&b_sort_key)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        for item in items.iter_mut() {
            if !item.children.is_empty() {
                sort_nav_items(&mut item.children, weights);
            }
        }
    }

    sort_nav_items(&mut roots, &nav_weights);

    Ok(roots)
}

#[server(GetNavbarGroups, "/api")]
pub async fn get_navbar_groups() -> Result<Vec<NavGroup>, ServerFnError> {
    let state = expect_context::<AppState>();
    let settings = state
        .settings_repo
        .get_settings()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(settings.navbar_groups)
}

#[server(GetNavigationOrder, "/api")]
pub async fn get_navigation_order() -> Result<Vec<NavigationOrderEntry>, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state
        .navigation_order_repo
        .list_all()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(SaveNavigationOrder, "/api")]
pub async fn save_navigation_order(
    entries: Vec<NavigationOrderEntry>,
) -> Result<String, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state
        .navigation_order_repo
        .replace_all(entries)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok("Navigation order saved successfully".to_string())
}
