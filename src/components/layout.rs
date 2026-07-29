use leptos::prelude::*;

use super::contextual_sidebars::{AdminSidebar, ChatSidebar, DocsSidebar, RegistrySidebar};
use super::custom_css::RuntimeCustomCss;
use super::logo::BrandedLogo;
use super::search::SearchModal;
use super::theme::ThemeToggle;
use super::user_menu::UserMenu;
use crate::app::{get_navbar_groups, get_navigation};
use crate::auth::refresh_client::with_auth_retry;

const MAX_DOCS_ITEMS: usize = 5;

#[component]
fn TopNavbarLinksSkeleton() -> impl IntoView {
    view! {
        <div class="flex w-full items-center gap-3 min-w-0 animate-pulse">
            <div class="flex flex-1 items-center gap-2 min-w-0 overflow-hidden">
                <span class="h-8 w-20 rounded-md bg-base-200/80"></span>
                <span class="h-8 w-24 rounded-md bg-base-200/80"></span>
                <span class="h-8 w-16 rounded-md bg-base-200/80 hidden xl:inline-flex"></span>
            </div>
            <div class="w-px h-5 bg-base-300 self-center shrink-0"></div>
            <div class="flex items-center gap-2 shrink-0">
                <span class="h-8 w-16 rounded-md bg-base-200/80"></span>
                <span class="h-8 w-14 rounded-md bg-base-200/80 hidden lg:inline-flex"></span>
            </div>
        </div>
    }
}

#[component]
pub fn TopNavbarLinks() -> impl IntoView {
    // Reader-facing navbar: follows the URL's release pins like the sidebar.
    let nav_query = leptos_router::hooks::use_query_map();
    let nav_resource = LocalResource::new(move || {
        let pins = nav_query
            .read()
            .get_all(crate::versioning::PIN_PARAM)
            .unwrap_or_default();
        with_auth_retry(move || get_navigation(Some(pins.clone())))
    });
    let groups_resource = Resource::new(|| (), |_| get_navbar_groups());
    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
    let is_rag = use_context::<crate::app::IsRagEnabled>();
    let schema_enabled = crate::app::use_feature(|f| f.schema_registry);
    let learn_enabled = crate::app::use_feature(|f| f.learn);

    view! {
        <Suspense fallback=move || view! { <TopNavbarLinksSkeleton /> }>
            {move || {
                let nav_res = nav_resource.get();
                let groups_res = groups_resource.get();

                if let (Some(Ok(items)), Some(Ok(groups))) = (nav_res, groups_res) {
                    let standalone: Vec<_> = items.iter()
                        .filter(|item| !groups.iter().any(|g| g.items.contains(&item.slug)))
                        .cloned()
                        .collect();

                    // Tier 1 max-items logic
                    let n_s = standalone.len();
                    let vis_s = n_s.min(MAX_DOCS_ITEMS);
                    let vis_g = groups.len().min(MAX_DOCS_ITEMS.saturating_sub(n_s));

                    let t1_standalone = standalone[..vis_s].to_vec();
                    let t1_groups = groups[..vis_g].to_vec();
                    let altro_standalone = standalone[vis_s..].to_vec();
                    let altro_groups = groups[vis_g..].to_vec();
                    let has_overflow = !altro_standalone.is_empty() || !altro_groups.is_empty();

                    // Clones for each tier
                    let t2_standalone = standalone.clone();
                    let t2_groups = groups.clone();
                    let t3_standalone = standalone;
                    let t3_groups = groups;

                    // Items for group filtering per tier
                    let items_t1g = items.clone();
                    let items_altro = items.clone();
                    let items_t2 = items.clone();
                    let items_t3 = items;

                    view! {
                        // ── TIER 1: xl+ — full text, max items, "Altro" overflow ──────────────
                        <div class="hidden xl:flex w-full items-center gap-3 min-w-0">
                            <div class="flex flex-1 items-center gap-2 min-w-0 overflow-x-clip">
                            {t1_standalone.into_iter().map(|item| {
                                view! {
                                    <a href=format!("/docs/{}", item.slug)
                                       class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">
                                        {item.title}
                                    </a>
                                }
                            }).collect::<Vec<_>>()}

                            {t1_groups.into_iter().map(|group| {
                                let group_items: Vec<_> = items_t1g.iter()
                                    .filter(|i| group.items.contains(&i.slug))
                                    .cloned()
                                    .collect();
                                if group_items.is_empty() {
                                    return view! { <span></span> }.into_any();
                                }
                                view! {
                                    <div class="dropdown dropdown-hover dropdown-bottom">
                                        <div tabindex="0" role="button" aria-haspopup="menu"
                                             class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50 m-1">
                                            {group.title.clone()}
                                            <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="ml-1 opacity-60"><path d="m6 9 6 6 6-6"/></svg>
                                        </div>
                                        <ul tabindex="0" class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-52 border border-base-200">
                                            {group_items.into_iter().map(|i| view! {
                                                <li><a href=format!("/docs/{}", i.slug) class="active:!bg-primary active:!text-primary-content">{i.title.clone()}</a></li>
                                            }).collect::<Vec<_>>()}
                                        </ul>
                                    </div>
                                }.into_any()
                            }).collect::<Vec<_>>()}
                            </div>

                            // "Altro" overflow dropdown — kept outside the clipped row (shrink-0) so
                            // it never gets squeezed out when the centered search bar (2xl+) halves
                            // the space available to the nav-links row.
                            {if has_overflow {
                                view! {
                                    <div class="dropdown dropdown-hover dropdown-bottom shrink-0">
                                        <div tabindex="0" role="button" aria-haspopup="menu"
                                             class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50 m-1">
                                            "Altro"
                                            <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="ml-1 opacity-60"><path d="m6 9 6 6 6-6"/></svg>
                                        </div>
                                        <ul tabindex="0" class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-52 border border-base-200">
                                            {altro_standalone.into_iter().map(|item| view! {
                                                <li><a href=format!("/docs/{}", item.slug) class="active:!bg-primary active:!text-primary-content">{item.title}</a></li>
                                            }).collect::<Vec<_>>()}
                                            {altro_groups.into_iter().flat_map(|group| {
                                                let gis: Vec<_> = items_altro.iter()
                                                    .filter(|i| group.items.contains(&i.slug))
                                                    .cloned()
                                                    .collect();
                                                let mut all = vec![
                                                    view! { <li class="menu-title">{group.title}</li> }.into_any()
                                                ];
                                                all.extend(gis.into_iter().map(|i| view! {
                                                    <li><a href=format!("/docs/{}", i.slug) class="active:!bg-primary active:!text-primary-content">{i.title}</a></li>
                                                }.into_any()));
                                                all
                                            }).collect::<Vec<_>>()}
                                        </ul>
                                    </div>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }}

                            <div class="w-px h-5 bg-base-300 self-center shrink-0"></div>

                            <div class="flex items-center gap-2 shrink-0">
                            {move || if schema_enabled.get() { view! {
                            <a href="/schemas" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">
                                "Registry"
                            </a>
                            }.into_any() } else { view! { <span></span> }.into_any() }}
                            {move || {
                                let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                let rag_enabled = is_rag.map(|sig| sig.0.get()).unwrap_or(false);
                                if logged_in && rag_enabled {
                                    view! { <a href="/chat" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">"Chat"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            {move || {
                                let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                if logged_in && learn_enabled.get() {
                                    view! { <a href="/learn" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">"Learn"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            {move || {
                                let is_admin = current_user.and_then(|sig| sig.get()).map(|u| u.is_admin).unwrap_or(false);
                                if is_admin {
                                    view! { <a href="/admin/tokens" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">"Admin"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            </div>
                        </div>

                        // ── TIER 2: lg–xl — "Docs ▾" dropdown + text system links ─────────────
                        <div class="hidden lg:flex xl:hidden w-full items-center gap-3 min-w-0">
                            <div class="flex flex-1 items-center min-w-0 overflow-x-clip">
                            <div class="dropdown dropdown-hover dropdown-bottom">
                                <div tabindex="0" role="button" aria-haspopup="menu"
                                     class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50 m-1">
                                    "Docs"
                                    <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="ml-1 opacity-60"><path d="m6 9 6 6 6-6"/></svg>
                                </div>
                                <ul tabindex="0" class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-52 border border-base-200">
                                    {t2_standalone.into_iter().map(|item| view! {
                                        <li><a href=format!("/docs/{}", item.slug) class="active:!bg-primary active:!text-primary-content">{item.title}</a></li>
                                    }).collect::<Vec<_>>()}
                                    {t2_groups.into_iter().flat_map(|group| {
                                        let gis: Vec<_> = items_t2.iter()
                                            .filter(|i| group.items.contains(&i.slug))
                                            .cloned()
                                            .collect();
                                        let mut all = vec![
                                            view! { <li class="menu-title">{group.title}</li> }.into_any()
                                        ];
                                        all.extend(gis.into_iter().map(|i| view! {
                                            <li><a href=format!("/docs/{}", i.slug) class="active:!bg-primary active:!text-primary-content">{i.title}</a></li>
                                        }.into_any()));
                                        all
                                    }).collect::<Vec<_>>()}
                                </ul>
                            </div>
                            </div>

                            <div class="w-px h-5 bg-base-300 self-center shrink-0"></div>

                            <div class="flex items-center gap-2 shrink-0">
                            {move || if schema_enabled.get() { view! {
                            <a href="/schemas" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">
                                "Registry"
                            </a>
                            }.into_any() } else { view! { <span></span> }.into_any() }}
                            {move || {
                                let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                let rag_enabled = is_rag.map(|sig| sig.0.get()).unwrap_or(false);
                                if logged_in && rag_enabled {
                                    view! { <a href="/chat" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">"Chat"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            {move || {
                                let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                if logged_in && learn_enabled.get() {
                                    view! { <a href="/learn" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">"Learn"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            {move || {
                                let is_admin = current_user.and_then(|sig| sig.get()).map(|u| u.is_admin).unwrap_or(false);
                                if is_admin {
                                    view! { <a href="/admin/tokens" class="btn btn-ghost btn-sm font-normal text-base-content/80 hover:text-base-content hover:bg-base-200/50">"Admin"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            </div>
                        </div>

                        // ── TIER 3: <lg — one labelled menu (icon-only items are ambiguous on touch) ─
                        <div class="flex lg:hidden items-center">
                            <div class="dropdown dropdown-bottom">
                                <div tabindex="0" role="button" aria-haspopup="menu"
                                     class="btn btn-ghost btn-sm gap-1.5 px-2.5 text-base-content/80 hover:text-base-content hover:bg-base-200/50"
                                     aria-label="Navigation menu">
                                    // Grid icon — distinct from the sidebar drawer's hamburger
                                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/></svg>
                                    <span class="text-sm font-medium">"Menu"</span>
                                    <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="opacity-60"><path d="m6 9 6 6 6-6"/></svg>
                                </div>
                                <ul tabindex="0" class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-56 border border-base-200 max-h-[70vh] flex-nowrap overflow-y-auto">
                                    <li class="menu-title">"Documentation"</li>
                                    {t3_standalone.into_iter().map(|item| view! {
                                        <li><a href=format!("/docs/{}", item.slug) class="active:!bg-primary active:!text-primary-content">{item.title}</a></li>
                                    }).collect::<Vec<_>>()}
                                    {t3_groups.into_iter().flat_map(|group| {
                                        let gis: Vec<_> = items_t3.iter()
                                            .filter(|i| group.items.contains(&i.slug))
                                            .cloned()
                                            .collect();
                                        let mut all = vec![
                                            view! { <li class="menu-title">{group.title}</li> }.into_any()
                                        ];
                                        all.extend(gis.into_iter().map(|i| view! {
                                            <li><a href=format!("/docs/{}", i.slug) class="active:!bg-primary active:!text-primary-content">{i.title}</a></li>
                                        }.into_any()));
                                        all
                                    }).collect::<Vec<_>>()}

                                    // Top-level sections
                                    {move || if schema_enabled.get() {
                                        view! { <li><a href="/schemas" class="active:!bg-primary active:!text-primary-content">"Registry"</a></li> }.into_any()
                                    } else { view! { <li class="hidden"></li> }.into_any() }}
                                    {move || {
                                        let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                        let rag_enabled = is_rag.map(|sig| sig.0.get()).unwrap_or(false);
                                        if logged_in && rag_enabled {
                                            view! { <li><a href="/chat" class="active:!bg-primary active:!text-primary-content">"Chat"</a></li> }.into_any()
                                        } else { view! { <li class="hidden"></li> }.into_any() }
                                    }}
                                    {move || {
                                        let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                        if logged_in && learn_enabled.get() {
                                            view! { <li><a href="/learn" class="active:!bg-primary active:!text-primary-content">"Learn"</a></li> }.into_any()
                                        } else { view! { <li class="hidden"></li> }.into_any() }
                                    }}
                                    {move || {
                                        let is_admin = current_user.and_then(|sig| sig.get()).map(|u| u.is_admin).unwrap_or(false);
                                        if is_admin {
                                            view! { <li><a href="/admin/tokens" class="active:!bg-primary active:!text-primary-content">"Admin"</a></li> }.into_any()
                                        } else { view! { <li class="hidden"></li> }.into_any() }
                                    }}
                                </ul>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! { <TopNavbarLinksSkeleton /> }.into_any()
                }
            }}
        </Suspense>
    }
}

/// Main layout: navbar + sidebar + content area.
#[component]
pub fn Layout(children: Children) -> impl IntoView {
    let (search_modal_open, set_search_modal_open) = signal(false);
    let search_enabled = crate::app::use_feature(|f| f.search);
    let location = leptos_router::hooks::use_location();
    let path = Memo::new(move |_| location.pathname.get());
    let has_context_sidebar = Memo::new(move |_| {
        let path = path.get();
        path.starts_with("/docs")
            || path.starts_with("/schemas")
            || path.starts_with("/chat")
            || path.starts_with("/admin")
    });
    let is_chat_layout = Memo::new(move |_| path.get().starts_with("/chat"));

    use leptos::ev;
    window_event_listener(ev::keydown, move |ev| {
        if search_enabled.get() && (ev.ctrl_key() || ev.meta_key()) && ev.key() == "k" {
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
            <header class="bg-base-100/80 backdrop-blur-md fixed top-0 inset-x-0 z-50 border-b border-base-200 px-4 h-16 flex items-center gap-2 shadow-sm lekton-header-enter">
                // Left — shrinks only when space is truly exhausted
                <div class="flex items-center gap-2 min-w-0 flex-1">
                    <Show when=move || has_context_sidebar.get()>
                        <label
                            for="sidebar-drawer"
                            aria-label="Open navigation menu"
                            class="btn btn-square btn-ghost drawer-button lg:hidden"
                        >
                            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" class="inline-block w-5 h-5 stroke-current"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h16"></path></svg>
                        </label>
                    </Show>
                    <BrandedLogo />
                    <div class="flex items-center gap-1 min-w-0 ml-2 pl-2 sm:ml-4 sm:pl-4 border-l border-base-300 overflow-visible lg:overflow-x-clip">
                        <TopNavbarLinks />
                    </div>
                </div>
                // Center — visible at md+, replaced by icon on smaller screens
                <Show when=move || search_enabled.get()>
                <div class="hidden 2xl:flex flex-1 min-w-0 items-center justify-center">
                    <div class="w-full max-w-md">
                        <button
                            class="btn btn-ghost bg-base-200/50 hover:bg-base-200 border border-base-300 hover:border-primary/30 w-full justify-between shadow-sm flex-nowrap h-11 min-h-[2.75rem] px-4 transition-all font-normal text-base-content/80 group/btn"
                            on:click=move |_| set_search_modal_open.set(true)
                        >
                            <div class="flex items-center gap-3 overflow-hidden">
                                <svg class="w-4 h-4 opacity-70 flex-shrink-0 group-hover/btn:text-primary transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path>
                                </svg>
                                <span class="truncate">"Search documentation..."</span>
                            </div>
                            <kbd class="kbd kbd-sm bg-base-100 border-none shadow-sm opacity-80 flex-shrink-0 group-hover/btn:bg-primary group-hover/btn:text-primary-content transition-colors">"Ctrl K"</kbd>
                        </button>
                    </div>
                </div>
                </Show>
                // Right — never shrinks
                <div class="flex items-center gap-2 flex-nowrap shrink-0">
                    // Search icon — shown when full search bar is hidden
                    <Show when=move || search_enabled.get()>
                    <button
                        class="btn btn-circle btn-ghost 2xl:hidden"
                        aria-label="Open search"
                        on:click=move |_| set_search_modal_open.set(true)
                    >
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"></path></svg>
                    </button>
                    </Show>
                    // Theme toggle
                    <ThemeToggle />
                    // User area — shows login button or user info
                    <UserMenu />
                </div>
            </header>

            // Global search modal
            <Show when=move || search_enabled.get()>
                <SearchModal is_open=search_modal_open set_is_open=set_search_modal_open />
            </Show>

            // Main content area with sidebar
            <div class=move || {
                if has_context_sidebar.get() {
                    "drawer lg:drawer-open pt-16"
                } else {
                    "drawer pt-16"
                }
            }>
                <input id="sidebar-drawer" type="checkbox" class="drawer-toggle" aria-label="Toggle navigation sidebar" />
                <div class="drawer-content lg:col-start-2 flex flex-col bg-base-100 min-w-0">
                    <div class=move || {
                        if is_chat_layout.get() {
                            "w-full h-[calc(100vh-4rem)] flex flex-col overflow-hidden lekton-content-enter"
                        } else {
                            "w-full max-w-[var(--lekton-content-max-width)] mx-auto px-4 py-6 sm:px-6 lg:px-10 lg:py-10 min-h-[calc(100vh-4rem)] lekton-content-enter"
                        }
                    }>
                        {children()}
                    </div>
                </div>

                // Sidebar
                <Show when=move || has_context_sidebar.get()>
                    <div class="drawer-side z-40">
                        <label for="sidebar-drawer" aria-label="close sidebar" class="drawer-overlay"></label>
                        <div class="menu bg-base-200 min-h-full h-[calc(100vh-4rem)] w-[var(--lekton-sidebar-width)] p-4 text-base-content border-r border-base-300 pt-6 overflow-y-auto block">
                            {move || {
                                let path = path.get();

                                if path.starts_with("/docs") {
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
                </Show>
            </div>
        </div>
    }
}
