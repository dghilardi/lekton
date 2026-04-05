use leptos::prelude::*;

use super::navigation::NavigationTree;
use super::search::SearchModal;
use super::theme::ThemeToggle;
use super::user_menu::UserMenu;
use super::custom_css::RuntimeCustomCss;
use crate::app::{get_navigation, get_navbar_groups};

#[component]
pub fn TopNavbarLinks() -> impl IntoView {
    let nav_resource = Resource::new(|| (), |_| get_navigation());
    let groups_resource = Resource::new(|| (), |_| get_navbar_groups());

    view! {
        <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
            {move || {
                let nav_res = nav_resource.get();
                let groups_res = groups_resource.get();
                
                if let (Some(Ok(items)), Some(Ok(groups))) = (nav_res, groups_res) {
                    let mut standalone = vec![];
                    
                    for item in &items {
                        let mut in_group = false;
                        for group in &groups {
                            if group.items.contains(&item.slug) {
                                in_group = true;
                                break;
                            }
                        }
                        if !in_group {
                            standalone.push(item.clone());
                        }
                    }

                    view! {
                        {standalone.into_iter().map(|item| {
                            view! {
                                <a href=format!("/docs/{}", item.slug) class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">
                                    {item.title}
                                </a>
                            }
                        }).collect::<Vec<_>>()}

                        {groups.into_iter().map(|group| {
                            let group_items: Vec<_> = items.iter().filter(|i| group.items.contains(&i.slug)).collect();
                            if group_items.is_empty() {
                                return view! { <span></span> }.into_any();
                            }
                            view! {
                                <div class="dropdown dropdown-hover dropdown-bottom">
                                    <div tabindex="0" role="button" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50 m-1">
                                        {group.title.clone()}
                                        <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="ml-1 opacity-60"><path d="m6 9 6 6 6-6"/></svg>
                                    </div>
                                    <ul tabindex="0" class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-52 border border-base-200">
                                        {group_items.into_iter().map(|i| {
                                            view! {
                                                <li><a href=format!("/docs/{}", i.slug) class="active:!bg-primary active:!text-primary-content">{i.title.clone()}</a></li>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </ul>
                                </div>
                            }.into_any()
                        }).collect::<Vec<_>>()}
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}
        </Suspense>
    }
}

/// Main layout: navbar + sidebar + content area.
#[component]
pub fn Layout(children: Children) -> impl IntoView {
    let (search_modal_open, set_search_modal_open) = signal(false);

    use leptos::ev;
    window_event_listener(ev::keydown, move |ev| {
        if (ev.ctrl_key() || ev.meta_key()) && ev.key() == "k" {
            ev.prevent_default();
            ev.stop_propagation();
            set_search_modal_open.set(true);
        }
    });

    view! {
        // Runtime custom CSS injection (loaded from MongoDB settings)
        <RuntimeCustomCss />

        <div class="min-h-screen bg-base-100/50">
            // Navbar
            <header class="bg-base-100/80 backdrop-blur-md fixed top-0 inset-x-0 z-50 border-b border-base-200 px-4 h-16 flex items-center justify-between shadow-sm">
                // Left
                <div class="flex items-center gap-2 z-10">
                    <label for="sidebar-drawer" class="btn btn-square btn-ghost drawer-button lg:hidden">
                        <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" class="inline-block w-5 h-5 stroke-current"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h16"></path></svg>
                    </label>
                    <a class="flex items-center gap-2 text-xl font-bold tracking-tight hover:opacity-80 transition-opacity" href="/">
                        <svg class="w-6 h-6 text-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L2 7l10 5 10-5-10-5Z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>
                        <span class="truncate">"Lekton"</span>
                    </a>
                    <div class="hidden lg:flex items-center gap-1 ml-4 pl-4 border-l border-base-300">
                        <TopNavbarLinks />
                    </div>
                </div>
                // Center (Absolutey Centered)
                <div class="hidden sm:flex absolute inset-0 pointer-events-none items-center justify-center">
                    <div class="w-full max-w-md px-4 pointer-events-auto">
                        <button
                            class="btn btn-ghost bg-base-200/50 hover:bg-base-200 border border-base-300 hover:border-base-content/20 w-full justify-between shadow-sm flex-nowrap h-10 min-h-10 px-3 transition-colors font-normal text-base-content/70"
                            on:click=move |_| set_search_modal_open.set(true)
                        >
                            <div class="flex items-center gap-2 overflow-hidden">
                                <svg class="w-4 h-4 opacity-70 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path>
                                </svg>
                                <span class="truncate">"Search documentation..."</span>
                            </div>
                            <kbd class="kbd kbd-sm bg-base-100 border-none shadow-sm opacity-80 flex-shrink-0">"Ctrl K"</kbd>
                        </button>
                    </div>
                </div>
                // Right
                <div class="flex items-center gap-2 z-10 flex-nowrap shrink-0">
                    // Mobile search icon
                    <button class="btn btn-circle btn-ghost sm:hidden" on:click=move |_| set_search_modal_open.set(true)>
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path></svg>
                    </button>
                    // Theme toggle
                    <ThemeToggle />
                    // User area — shows login button or user info
                    <UserMenu />
                </div>
            </header>

            // Global search modal
            <SearchModal is_open=search_modal_open set_is_open=set_search_modal_open />

            // Main content area with sidebar
            <div class="drawer lg:drawer-open pt-16">
                <input id="sidebar-drawer" type="checkbox" class="drawer-toggle" />
                <div class="drawer-content lg:col-start-2 flex flex-col items-center bg-base-100 min-w-0">
                    <div class="w-full max-w-6xl p-6 lg:p-10 min-h-[calc(100vh-4rem)]">
                        {children()}
                    </div>
                </div>

                // Sidebar
                <div class="drawer-side z-40">
                    <label for="sidebar-drawer" aria-label="close sidebar" class="drawer-overlay"></label>
                    <div class="menu bg-base-200 min-h-full h-[calc(100vh-4rem)] w-64 p-4 text-base-content border-r border-base-300 pt-6 overflow-y-auto block">
                        <ul class="flex flex-col gap-1">
                            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"Overview"</li>
                            <li>
                                <a href="/" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                                    <svg class="w-4 h-4 opacity-70 group-hover:opacity-100 transition-opacity" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg>
                                    "Home"
                                </a>
                            </li>
                        </ul>
                        <ul class="flex flex-col gap-1 mt-6">
                            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"Documentation"</li>
                            <NavigationTree />
                        </ul>
                        <ul class="flex flex-col gap-1 mt-6">
                            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"API Resources"</li>
                            <li>
                                <a href="/schemas" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                                    <svg class="w-4 h-4 opacity-70 group-hover:opacity-100 transition-opacity" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 7h10"/><path d="M7 12h10"/><path d="M7 17h10"/></svg>
                                    "Schema Registry"
                                </a>
                            </li>
                        </ul>
                        // RAG Chat section (visible only when RAG is enabled and user is logged in)
                        {move || {
                            let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
                            let is_rag = use_context::<Signal<bool>>();
                            let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                            let rag_enabled = is_rag.map(|sig| sig.get()).unwrap_or(false);
                            if logged_in && rag_enabled {
                                view! {
                                    <ul class="flex flex-col gap-1 mt-6">
                                        <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"AI Assistant"</li>
                                        <li>
                                            <a href="/chat" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                                                <svg class="w-4 h-4 opacity-70 group-hover:opacity-100 transition-opacity" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                                                "Chat"
                                            </a>
                                        </li>
                                    </ul>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }
                        }}
                        // Admin section (visible only to admin users)
                        {move || {
                            let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
                            let is_admin = current_user
                                .and_then(|sig| sig.get())
                                .map(|u| u.is_admin)
                                .unwrap_or(false);
                            if is_admin {
                                view! {
                                    <ul class="flex flex-col gap-1 mt-6">
                                        <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"Admin"</li>
                                        <li>
                                            <a href="/admin/settings" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                                                <svg class="w-4 h-4 opacity-70 group-hover:opacity-100 transition-opacity" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>
                                                "Settings"
                                            </a>
                                        </li>
                                    </ul>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }
                        }}
                    </div>
                </div>
            </div>
        </div>
    }
}
