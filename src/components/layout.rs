use leptos::prelude::*;

use super::contextual_sidebars::{AdminSidebar, ChatSidebar, DocsSidebar, RegistrySidebar};
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

                        // Macro-areas links
                        <a href="/schemas" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">
                            "Registry"
                        </a>
                        
                        {move || {
                            let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
                            let is_rag = use_context::<crate::app::IsRagEnabled>();
                            let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                            let rag_enabled = is_rag.map(|sig| sig.0.get()).unwrap_or(false);
                            if logged_in && rag_enabled {
                                view! {
                                    <a href="/chat" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">
                                        "Chat"
                                    </a>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }
                        }}

                        {move || {
                            let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
                            let is_admin = current_user
                                .and_then(|sig| sig.get())
                                .map(|u| u.is_admin)
                                .unwrap_or(false);
                            if is_admin {
                                view! {
                                    <a href="/admin/tokens" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">
                                        "Admin"
                                    </a>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }
                        }}
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
                        {move || {
                            let location = leptos_router::hooks::use_location();
                            let path = location.pathname.get();
                            
                            if path.starts_with("/docs") || path == "/" {
                                view! { <DocsSidebar /> }.into_any()
                            } else if path.starts_with("/schemas") {
                                view! { <RegistrySidebar /> }.into_any()
                            } else if path.starts_with("/chat") {
                                view! { <ChatSidebar /> }.into_any()
                            } else if path.starts_with("/admin") {
                                view! { <AdminSidebar /> }.into_any()
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
