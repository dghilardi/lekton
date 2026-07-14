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

/// Flattened item for the ordering UI.
#[derive(Debug, Clone)]
struct OrderableItem {
    slug: String,
    title: String,
    level: u32,
}

/// Extract sections/categories (nodes with children) from the nav tree, recursively.
fn collect_sections(items: &[NavItem], level: u32, out: &mut Vec<OrderableItem>) {
    for item in items {
        if !item.children.is_empty() {
            out.push(OrderableItem {
                slug: item.slug.clone(),
                title: item.title.clone(),
                level,
            });
            collect_sections(&item.children, level + 1, out);
        }
    }
}

/// Admin component for reordering navigation sections and categories.
#[component]
pub fn NavigationOrderEditor() -> impl IntoView {
    let (items, set_items) = signal(Vec::<OrderableItem>::new());
    let (original_slugs, set_original_slugs) = signal(Vec::<String>::new());
    let (saving, set_saving) = signal(false);
    let (message, set_message) = signal(Option::<(bool, String)>::None);
    let (dragging_idx, set_dragging_idx) = signal(Option::<usize>::None);

    // Load nav tree and existing weights
    let nav_resource = LocalResource::new(|| with_auth_retry(get_navigation));
    let order_resource = LocalResource::new(|| with_auth_retry(get_navigation_order));

    // Merge nav tree with existing weights to build the orderable list
    let _ = Effect::new(move |_| {
        let nav = nav_resource.get();
        let order = order_resource.get();

        if let (Some(Ok(nav_items)), Some(Ok(order_entries))) = (nav, order) {
            let mut sections = Vec::new();
            collect_sections(&nav_items, 0, &mut sections);

            // If there are existing weights, reorder sections by weight at each level
            if !order_entries.is_empty() {
                let weight_map: std::collections::HashMap<String, i32> = order_entries
                    .iter()
                    .map(|e| (e.slug.clone(), e.weight))
                    .collect();

                // Group by level and parent prefix, then sort by weight within each group
                // Simple approach: sort the whole list respecting hierarchy
                reorder_by_weights(&mut sections, &weight_map);
            }

            set_original_slugs.set(sections.iter().map(|s| s.slug.clone()).collect());
            set_items.set(sections);
        }
    });

    let has_changes = move || {
        let current: Vec<String> = items.get().iter().map(|s| s.slug.clone()).collect();
        current != original_slugs.get()
    };

    let save_action = Action::new_local(move |_: &()| {
        let current_items = items.get_untracked();
        async move {
            set_saving.set(true);
            set_message.set(None);

            // Build entries with weights based on position within each level
            let entries: Vec<NavigationOrderEntry> = current_items
                .iter()
                .enumerate()
                .map(|(i, item)| NavigationOrderEntry {
                    slug: item.slug.clone(),
                    weight: (i as i32) * 10,
                })
                .collect();

            let result = with_auth_retry(|| save_navigation_order(entries.clone())).await;
            set_saving.set(false);

            match result {
                Ok(msg) => {
                    set_original_slugs.set(current_items.iter().map(|s| s.slug.clone()).collect());
                    set_message.set(Some((true, msg)));
                }
                Err(e) => {
                    set_message.set(Some((false, e.to_string())));
                }
            }
        }
    });

    // Find the subtree range for an item: [idx .. end) where end is the first
    // item at the same or higher level, i.e. the item plus all its descendants.
    fn subtree_range(items: &[OrderableItem], idx: usize) -> std::ops::Range<usize> {
        let level = items[idx].level;
        let mut end = idx + 1;
        while end < items.len() && items[end].level > level {
            end += 1;
        }
        idx..end
    }

    let move_item = move |idx: usize, direction: i32| {
        set_items.update(|items| {
            let level = items[idx].level;
            let src_range = subtree_range(items, idx);

            if direction < 0 {
                // Move up: find previous sibling at the same level
                if src_range.start == 0 {
                    return;
                }
                // Walk backwards from src_range.start to find the previous sibling
                let mut prev_idx = src_range.start - 1;
                while prev_idx > 0 && items[prev_idx].level > level {
                    prev_idx -= 1;
                }
                if items[prev_idx].level != level {
                    return;
                }
                // Extract subtree and reinsert before previous sibling
                let subtree: Vec<_> = items.drain(src_range.clone()).collect();
                items.splice(prev_idx..prev_idx, subtree);
            } else {
                // Move down: find next sibling at the same level
                if src_range.end >= items.len() {
                    return;
                }
                let next_range = subtree_range(items, src_range.end);
                if items[next_range.start].level != level {
                    return;
                }
                // Extract the next sibling's subtree and insert before current
                let next_subtree: Vec<_> = items.drain(next_range.clone()).collect();
                items.splice(src_range.start..src_range.start, next_subtree);
            }
        });
    };

    let on_drag_start = move |idx: usize| {
        set_dragging_idx.set(Some(idx));
    };

    let on_drag_over = move |idx: usize| {
        if let Some(from) = dragging_idx.get_untracked() {
            if from == idx {
                return;
            }
            set_items.update(|items| {
                if from >= items.len() || idx >= items.len() {
                    return;
                }
                let from_level = items[from].level;
                let idx_level = items[idx].level;
                if from_level != idx_level {
                    return;
                }

                // Find the root of the sibling subtree that idx belongs to.
                // Walk backwards from idx to find the first item at the same level.
                let mut target_root = idx;
                while target_root > 0 && items[target_root].level > from_level {
                    target_root -= 1;
                }
                if items[target_root].level != from_level {
                    return;
                }

                // Don't move onto ourselves
                let src = subtree_range(items, from);
                if src.contains(&target_root) {
                    return;
                }

                let dst = subtree_range(items, target_root);

                // Swap the two subtrees by extracting both and reinserting
                if from < target_root {
                    // Moving down: extract dst first (higher indices), then src
                    let dst_subtree: Vec<_> = items.drain(dst.clone()).collect();
                    let src_subtree: Vec<_> = items.drain(src.clone()).collect();
                    // Reinsert: src was at src.start, dst was at dst.start
                    // After removing both, insert at src.start: dst then src
                    items.splice(src.start..src.start, dst_subtree);
                    let new_src_start = src.start + (dst.len());
                    items.splice(new_src_start..new_src_start, src_subtree);
                    set_dragging_idx.set(Some(new_src_start));
                } else {
                    // Moving up: extract src first (higher indices), then dst
                    let src_subtree: Vec<_> = items.drain(src.clone()).collect();
                    let dst_subtree: Vec<_> = items.drain(dst.clone()).collect();
                    // Reinsert at dst.start: src then dst
                    items.splice(dst.start..dst.start, src_subtree.clone());
                    let new_dst_start = dst.start + src_subtree.len();
                    items.splice(new_dst_start..new_dst_start, dst_subtree);
                    set_dragging_idx.set(Some(dst.start));
                }
            });
        }
    };

    let on_drag_end = move || {
        set_dragging_idx.set(None);
    };

    view! {
        <div class="card bg-base-100 shadow-xl border border-base-200">
            <div class="card-body p-0">
                <p class="px-8 pt-8 text-sm text-base-content/60">
                    "Drag items or use the arrow buttons to reorder sections. Documents within each section are sorted by their own weight or alphabetically."
                </p>

                <div class="px-8 py-4">
                    <Suspense fallback=move || view! {
                        <div class="flex flex-col items-center justify-center py-12 gap-4">
                            <span class="loading loading-spinner loading-lg text-primary"></span>
                            <p class="text-sm font-medium animate-pulse">"Loading navigation tree..."</p>
                        </div>
                    }>
                        {move || {
                            let current_items = items.get();
                            if current_items.is_empty() {
                                return view! {
                                    <div class="flex flex-col items-center justify-center py-10 px-4 text-center border-2 border-dashed border-base-300 rounded-xl bg-base-200/20">
                                        <div class="w-16 h-16 bg-base-300/30 rounded-full flex items-center justify-center mb-4">
                                            <svg class="w-8 h-8 text-base-content/30" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 10h16M4 14h16M4 18h16"></path>
                                            </svg>
                                        </div>
                                        <h3 class="font-bold text-lg text-base-content/70">"No sections found"</h3>
                                        <p class="text-sm text-base-content/65 max-w-xs mt-1">"Sections will appear here once documents with hierarchical slugs are ingested."</p>
                                    </div>
                                }.into_any();
                            }

                            view! {
                                <div class="space-y-1">
                                    {current_items.into_iter().enumerate().map(|(idx, item)| {
                                        let indent = item.level;
                                        let slug = item.slug.clone();
                                        let title = item.title.clone();
                                        let level_label = if indent == 0 { "Section" } else { "Category" };

                                        view! {
                                            <div
                                                class="flex items-center gap-2 p-3 rounded-lg border border-base-200 bg-base-100 hover:bg-base-200/30 transition-colors cursor-grab active:cursor-grabbing"
                                                style=format!("margin-left: {}rem", indent as f32 * 1.5)
                                                draggable="true"
                                                on:dragstart=move |_| on_drag_start(idx)
                                                on:dragover=move |ev| {
                                                    ev.prevent_default();
                                                    on_drag_over(idx);
                                                }
                                                on:dragend=move |_| on_drag_end()
                                            >
                                                // Drag handle
                                                <svg class="w-5 h-5 text-base-content/30 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 8h16M4 16h16"></path>
                                                </svg>

                                                // Title and info
                                                <div class="flex-1 min-w-0">
                                                    <div class="flex items-center gap-2">
                                                        <span class="font-medium text-sm truncate">{title}</span>
                                                        <span class="badge badge-ghost badge-xs text-[10px] uppercase">{level_label}</span>
                                                    </div>
                                                    <span class="text-xs text-base-content/65 font-mono truncate block">{slug}</span>
                                                </div>

                                                // Move buttons
                                                <div class="flex gap-1 flex-shrink-0">
                                                    <button
                                                        class="btn btn-ghost btn-xs btn-square"
                                                        title="Move up"
                                                        on:click=move |_| move_item(idx, -1)
                                                    >
                                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 15l7-7 7 7"></path>
                                                        </svg>
                                                    </button>
                                                    <button
                                                        class="btn btn-ghost btn-xs btn-square"
                                                        title="Move down"
                                                        on:click=move |_| move_item(idx, 1)
                                                    >
                                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"></path>
                                                        </svg>
                                                    </button>
                                                </div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    </Suspense>
                </div>

                <div class="bg-base-200/30 p-8 pt-6 border-t border-base-200">
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
                                on:click=move |_| {
                                    let orig = original_slugs.get_untracked();
                                    let current = items.get_untracked();
                                    let by_slug: std::collections::HashMap<String, OrderableItem> =
                                        current.into_iter().map(|i| (i.slug.clone(), i)).collect();
                                    let restored: Vec<OrderableItem> = orig
                                        .iter()
                                        .filter_map(|s| by_slug.get(s).cloned())
                                        .collect();
                                    set_items.set(restored);
                                }
                            >
                                "Discard"
                            </button>
                            <button
                                class="btn btn-primary w-full sm:w-64 shadow-lg shadow-primary/20"
                                disabled=move || !has_changes() || saving.get()
                                on:click=move |_| { save_action.dispatch(()); }
                            >
                                {move || if saving.get() {
                                    view! { <span class="loading loading-spinner loading-sm"></span> }.into_any()
                                } else {
                                    view! { "Save Order" }.into_any()
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Reorder sections in-place based on weight map, preserving hierarchy.
fn reorder_by_weights(
    sections: &mut Vec<OrderableItem>,
    weights: &std::collections::HashMap<String, i32>,
) {
    // Group items by (level, parent_prefix) and sort within each group
    // We need to identify groups of siblings at the same level
    let mut i = 0;
    while i < sections.len() {
        let level = sections[i].level;
        // Find the range of consecutive items at this level (siblings)
        let start = i;
        let mut end = i + 1;
        while end < sections.len() {
            if sections[end].level == level {
                // Check this is actually a sibling, not a same-level item from another parent
                // by ensuring no lower-level item from a different subtree is between them
                end += 1;
            } else if sections[end].level > level {
                // Child of current group, skip
                end += 1;
            } else {
                // Higher level item, different group
                break;
            }
        }

        // Collect the indices of items at this level within [start..end]
        let sibling_indices: Vec<usize> = (start..end)
            .filter(|&j| sections[j].level == level)
            .collect();

        if sibling_indices.len() > 1 {
            // Extract siblings with their subtrees
            let mut groups: Vec<Vec<OrderableItem>> = Vec::new();
            for &si in sibling_indices.iter().rev() {
                // Find the subtree: from si to next sibling (or end)
                let subtree_end = sibling_indices
                    .iter()
                    .find(|&&j| j > si && sections[j].level == level)
                    .copied()
                    .unwrap_or(end);
                let subtree: Vec<OrderableItem> = sections[si..subtree_end].to_vec();
                groups.push(subtree);
            }
            groups.reverse();

            // Sort groups by weight of the root item
            groups.sort_by(|a, b| {
                let aw = weights.get(&a[0].slug).copied().unwrap_or(i32::MAX);
                let bw = weights.get(&b[0].slug).copied().unwrap_or(i32::MAX);
                aw.cmp(&bw)
                    .then_with(|| a[0].title.to_lowercase().cmp(&b[0].title.to_lowercase()))
            });

            // Reconstruct the range
            let mut new_range: Vec<OrderableItem> = Vec::new();
            for group in groups {
                new_range.extend(group);
            }

            // Replace range in sections
            sections.splice(start..end, new_range);
        }

        // Move past this group
        i = end;
        if i <= start {
            i = start + 1; // Safety: always advance
        }
    }
}
