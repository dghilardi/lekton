use leptos::prelude::*;

use super::contextual_sidebars::{AdminSidebar, ChatSidebar, DocsSidebar, RegistrySidebar};
use super::custom_css::RuntimeCustomCss;
use super::logo::BrandedLogo;
use super::navigation::use_navigation;
use super::release_pin_strip::ReleasePinStrip;
use super::search::SearchModal;
use super::theme::ThemeToggle;
use super::user_menu::UserMenu;
use crate::app::{get_navbar_groups, NavItem};
use crate::db::settings_repository::NavGroup;

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
    let nav_resource = super::navigation::use_navigation();
    let groups_resource = use_navbar_groups();
    // Below lg these links live in the navigation drawer (see
    // `SectionMenuItems`), the one menu small screens have.
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
                    let standalone = sections_first(items.iter()
                        .filter(|item| !groups.iter().any(|g| g.items.contains(&item.slug)))
                        .cloned()
                        .collect());

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
                    let t2_groups = groups;

                    // Items for group filtering per tier
                    let items_t1g = items.clone();
                    let items_altro = items.clone();
                    let items_t2 = items;

                    view! {
                        // ── TIER 1: xl+ — full text, max items, "More" overflow ──────────────
                        <div class="hidden xl:flex w-full items-center gap-3 min-w-0">
                            <div class="flex flex-1 items-center gap-2 min-w-0 overflow-x-clip">
                            {t1_standalone.into_iter().map(|item| {
                                view! {
                                    <a href=crate::components::pinned_doc_href(&format!("/docs/{}", item.slug))
                                       class="btn btn-ghost btn-sm font-normal lekton-header-link">
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
                                             class="btn btn-ghost btn-sm font-normal lekton-header-link m-1">
                                            {group.title.clone()}
                                            <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="ml-1 opacity-60"><path d="m6 9 6 6 6-6"/></svg>
                                        </div>
                                        <ul tabindex="0" class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-52 border border-base-200">
                                            {group_items.into_iter().map(|i| view! {
                                                <li><a href=crate::components::pinned_doc_href(&format!("/docs/{}", i.slug)) class="active:!bg-primary active:!text-primary-content">{i.title.clone()}</a></li>
                                            }).collect::<Vec<_>>()}
                                        </ul>
                                    </div>
                                }.into_any()
                            }).collect::<Vec<_>>()}
                            </div>

                            // "More" overflow dropdown — kept outside the clipped row (shrink-0) so
                            // it never gets squeezed out when the centered search bar (2xl+) halves
                            // the space available to the nav-links row.
                            {if has_overflow {
                                view! {
                                    <div class="dropdown dropdown-hover dropdown-bottom shrink-0">
                                        <div tabindex="0" role="button" aria-haspopup="menu"
                                             class="btn btn-ghost btn-sm font-normal lekton-header-link m-1">
                                            "More"
                                            <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="ml-1 opacity-60"><path d="m6 9 6 6 6-6"/></svg>
                                        </div>
                                        <ul tabindex="0" class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-52 border border-base-200">
                                            {altro_standalone.into_iter().map(|item| view! {
                                                <li><a href=crate::components::pinned_doc_href(&format!("/docs/{}", item.slug)) class="active:!bg-primary active:!text-primary-content">{item.title}</a></li>
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
                                                    <li><a href=crate::components::pinned_doc_href(&format!("/docs/{}", i.slug)) class="active:!bg-primary active:!text-primary-content">{i.title}</a></li>
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
                            <a href="/schemas" class="btn btn-ghost btn-sm font-normal lekton-header-link">
                                "Registry"
                            </a>
                            }.into_any() } else { view! { <span></span> }.into_any() }}
                            {move || {
                                let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                let rag_enabled = is_rag.map(|sig| sig.0.get()).unwrap_or(false);
                                if logged_in && rag_enabled {
                                    view! { <a href="/chat" class="btn btn-ghost btn-sm font-normal lekton-header-link">"Chat"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            {move || {
                                let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                if logged_in && learn_enabled.get() {
                                    view! { <a href="/learn" class="btn btn-ghost btn-sm font-normal lekton-header-link">"Learn"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            {move || {
                                let is_admin = current_user.and_then(|sig| sig.get()).map(|u| u.is_admin).unwrap_or(false);
                                if is_admin {
                                    view! { <a href="/admin/tokens" class="btn btn-ghost btn-sm font-normal lekton-header-link">"Admin"</a> }.into_any()
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
                                     class="btn btn-ghost btn-sm font-normal lekton-header-link m-1">
                                    "Docs"
                                    <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="ml-1 opacity-60"><path d="m6 9 6 6 6-6"/></svg>
                                </div>
                                <ul tabindex="0" class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-52 border border-base-200">
                                    {t2_standalone.into_iter().map(|item| view! {
                                        <li><a href=crate::components::pinned_doc_href(&format!("/docs/{}", item.slug)) class="active:!bg-primary active:!text-primary-content">{item.title}</a></li>
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
                                            <li><a href=crate::components::pinned_doc_href(&format!("/docs/{}", i.slug)) class="active:!bg-primary active:!text-primary-content">{i.title}</a></li>
                                        }.into_any()));
                                        all
                                    }).collect::<Vec<_>>()}
                                </ul>
                            </div>
                            </div>

                            <div class="w-px h-5 bg-base-300 self-center shrink-0"></div>

                            <div class="flex items-center gap-2 shrink-0">
                            {move || if schema_enabled.get() { view! {
                            <a href="/schemas" class="btn btn-ghost btn-sm font-normal lekton-header-link">
                                "Registry"
                            </a>
                            }.into_any() } else { view! { <span></span> }.into_any() }}
                            {move || {
                                let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                let rag_enabled = is_rag.map(|sig| sig.0.get()).unwrap_or(false);
                                if logged_in && rag_enabled {
                                    view! { <a href="/chat" class="btn btn-ghost btn-sm font-normal lekton-header-link">"Chat"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            {move || {
                                let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
                                if logged_in && learn_enabled.get() {
                                    view! { <a href="/learn" class="btn btn-ghost btn-sm font-normal lekton-header-link">"Learn"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            {move || {
                                let is_admin = current_user.and_then(|sig| sig.get()).map(|u| u.is_admin).unwrap_or(false);
                                if is_admin {
                                    view! { <a href="/admin/tokens" class="btn btn-ghost btn-sm font-normal lekton-header-link">"Admin"</a> }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
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

/// Top-level entries with pages beneath them come before single documents,
/// each keeping the navigation order, so the header's limited slots go to
/// sections instead of burying them under "More".
fn sections_first(items: Vec<NavItem>) -> Vec<NavItem> {
    let (mut sections, leaves): (Vec<_>, Vec<_>) = items
        .into_iter()
        .partition(|item| !item.children.is_empty());
    sections.extend(leaves);
    sections
}

/// Whether a path shows a context sidebar (and so a ☰ drawer on small screens).
/// Administration navigation is only for administrators: others get the
/// sign-in or permission state on its own.
fn has_context_sidebar(path: &str, is_admin: bool) -> bool {
    path.starts_with("/docs")
        || path.starts_with("/schemas")
        || path.starts_with("/chat")
        || (path.starts_with("/admin") && is_admin)
}

fn is_admin_user(
    current_user: Option<Signal<Option<crate::auth::models::AuthenticatedUser>>>,
) -> bool {
    current_user
        .and_then(|user| user.get())
        .is_some_and(|user| user.is_admin)
}

/// Navbar groups, fetched once per page by the layout.
#[derive(Clone, Copy)]
struct SharedNavbarGroups(Resource<Result<Vec<NavGroup>, ServerFnError>>);

fn use_navbar_groups() -> Resource<Result<Vec<NavGroup>, ServerFnError>> {
    use_context::<SharedNavbarGroups>()
        .map(|shared| shared.0)
        .unwrap_or_else(|| Resource::new(|| (), |_| get_navbar_groups()))
}

/// The drawer's global destinations on small screens: the portal areas first,
/// so Registry, Chat and Learn are one tap away, then the documentation entries
/// unless the page's own sidebar already lists them.
#[component]
fn SectionMenuItems(
    items: Vec<NavItem>,
    groups: Vec<NavGroup>,
    #[prop(default = true)] include_docs: bool,
) -> impl IntoView {
    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
    let is_rag = use_context::<crate::app::IsRagEnabled>();
    let schema_enabled = crate::app::use_feature(|f| f.schema_registry);
    let learn_enabled = crate::app::use_feature(|f| f.learn);
    let standalone = sections_first(
        items
            .iter()
            .filter(|item| !groups.iter().any(|g| g.items.contains(&item.slug)))
            .cloned()
            .collect(),
    );
    let docs = include_docs.then(|| {
        view! {
            <li class="menu-title">"Documentation"</li>
            {standalone.into_iter().map(|item| view! {
                <li><a href=crate::components::pinned_doc_href(&format!("/docs/{}", item.slug))>{item.title}</a></li>
            }).collect::<Vec<_>>()}
            {groups.into_iter().flat_map(|group| {
                let gis: Vec<_> = items.iter()
                    .filter(|i| group.items.contains(&i.slug))
                    .cloned()
                    .collect();
                let mut all = vec![
                    view! { <li class="menu-title">{group.title}</li> }.into_any()
                ];
                all.extend(gis.into_iter().map(|i| view! {
                    <li><a href=crate::components::pinned_doc_href(&format!("/docs/{}", i.slug))>{i.title}</a></li>
                }.into_any()));
                all
            }).collect::<Vec<_>>()}
        }
    });

    view! {
        <li><a href="/">"Home"</a></li>
        {move || if schema_enabled.get() {
            view! { <li><a href="/schemas">"Registry"</a></li> }.into_any()
        } else { ().into_any() }}
        {move || {
            let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
            let rag_enabled = is_rag.map(|sig| sig.0.get()).unwrap_or(false);
            if logged_in && rag_enabled {
                view! { <li><a href="/chat">"Chat"</a></li> }.into_any()
            } else { ().into_any() }
        }}
        {move || {
            let logged_in = current_user.map(|sig| sig.get().is_some()).unwrap_or(false);
            if logged_in && learn_enabled.get() {
                view! { <li><a href="/learn">"Learn"</a></li> }.into_any()
            } else { ().into_any() }
        }}
        {move || {
            if is_admin_user(current_user) {
                view! { <li><a href="/admin/tokens">"Admin"</a></li> }.into_any()
            } else { ().into_any() }
        }}
        {docs}
    }
}

/// Main layout: navbar + sidebar + content area.
#[component]
pub fn Layout(children: Children) -> impl IntoView {
    let (search_modal_open, set_search_modal_open) = signal(false);
    let pin_query = leptos_router::hooks::use_query_map();
    let active_release_pins = Memo::new(move |_| {
        pin_query
            .read()
            .get_all(crate::versioning::PIN_PARAM)
            .unwrap_or_default()
    });
    provide_context(super::ActiveReleasePins(active_release_pins));
    // One navigation fetch per page, shared by the navbar, sidebar and pages.
    provide_context(super::navigation::SharedNavigation(
        super::navigation::navigation_resource(),
    ));
    let search_enabled = crate::app::use_feature(|f| f.search);
    let location = leptos_router::hooks::use_location();
    // Cheap URL checks stay uncached derived signals: unlike memos, they have
    // no shared value that concurrent Suspense boundaries could race on.
    let path = Signal::derive(move || location.pathname.get());
    let layout_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
    let has_context_sidebar =
        Signal::derive(move || has_context_sidebar(&path.get(), is_admin_user(layout_user)));
    provide_context(SharedNavbarGroups(Resource::new(
        || (),
        |_| get_navbar_groups(),
    )));
    provide_context(crate::schema::component::SharedSchemas(
        crate::components::reader_resource(
            move || path.get().starts_with("/schemas"),
            |active| async move {
                if active {
                    crate::schema::component::list_schemas().await
                } else {
                    Ok(vec![])
                }
            },
        ),
    ));
    let is_chat_layout = Signal::derive(move || path.get().starts_with("/chat"));
    // Server-side, each Suspense boundary resolves on its own worker thread,
    // and reactive_graph can hand an unset value to a reader racing another
    // thread's first computation of the same memo. Compute the memos that
    // boundaries share here, before the first one renders: the URL does not
    // change during SSR, so they then stay clean for the whole response.
    location.pathname.with_untracked(|_| ());
    location.search.with_untracked(|_| ());
    active_release_pins.with_untracked(|_| ());
    let sidebar_open = RwSignal::new(false);
    let close_sidebar = move || {
        sidebar_open.set(false);
        #[cfg(feature = "hydrate")]
        leptos::task::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(0).await;
            let _ = js_sys::eval("document.getElementById('sidebar-trigger')?.focus()");
        });
    };
    Effect::new(move || {
        let _ = path.get();
        sidebar_open.set(false);
    });
    Effect::new(move || {
        if sidebar_open.get() {
            #[cfg(feature = "hydrate")]
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(0).await;
                let _ =
                    js_sys::eval("document.querySelector('#portal-navigation button')?.focus()");
            });
        }
    });

    use leptos::ev;
    window_event_listener(ev::resize, move |_| sidebar_open.set(false));
    window_event_listener(ev::keydown, move |ev| {
        if ev.key() == "Escape" && sidebar_open.get_untracked() {
            close_sidebar();
        }
    });
    // Ctrl/Cmd+K is the portal's search everywhere. Embedded viewers (Scalar's
    // OpenAPI reference) bind the same keys, so the portal claims them in the
    // capture phase, before the event reaches them.
    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::{closure::Closure, JsCast};
        let on_search_shortcut = Closure::<dyn Fn(leptos::web_sys::KeyboardEvent)>::new(
            move |ev: leptos::web_sys::KeyboardEvent| {
                if search_enabled.get_untracked()
                    && (ev.ctrl_key() || ev.meta_key())
                    && ev.key().eq_ignore_ascii_case("k")
                {
                    ev.prevent_default();
                    ev.stop_immediate_propagation();
                    set_search_modal_open.set(true);
                }
            },
        );
        let _ = window().add_event_listener_with_callback_and_bool(
            "keydown",
            on_search_shortcut.as_ref().unchecked_ref(),
            true,
        );
        // The layout lives as long as the page, so the listener does too.
        on_search_shortcut.forget();
    }

    view! {
        // Runtime custom CSS injection (loaded from MongoDB settings)
        <RuntimeCustomCss />

        <div class="min-h-screen bg-base-100/50">
            <a href="#content" class="lekton-skip-link" inert=move || sidebar_open.get() on:click=move |ev| {
                ev.prevent_default();
                #[cfg(feature = "hydrate")]
                let _ = js_sys::eval("document.getElementById('content')?.focus(); document.getElementById('content')?.scrollIntoView()");
            }>"Skip to content"</a>
            // Desktop sidebars come after the content in the DOM (and in tab
            // order); on small screens the navigation lives in the drawer.
            <Show when=move || has_context_sidebar.get()>
                <a href="#portal-navigation" class="lekton-skip-link max-lg:hidden" inert=move || sidebar_open.get() on:click=move |ev| {
                    ev.prevent_default();
                    #[cfg(feature = "hydrate")]
                    let _ = js_sys::eval("document.getElementById('portal-navigation')?.focus()");
                }>"Skip to section navigation"</a>
            </Show>
            // Navbar
            // Colours come from the --lekton-header-* theming tokens (see style/tailwind.css).
            <header inert=move || sidebar_open.get() data-lekton-header class="bg-base-100/80 backdrop-blur-md fixed top-0 inset-x-0 z-50 border-b border-base-200 px-4 h-16 flex items-center gap-2 shadow-sm lekton-header-enter">
                // Left — shrinks only when space is truly exhausted
                <div class="flex items-center gap-2 min-w-0 flex-1">
                    <button
                        id="sidebar-trigger"
                        type="button"
                        aria-controls="portal-navigation"
                        aria-expanded=move || sidebar_open.get().to_string()
                        on:click=move |_| sidebar_open.set(true)
                        aria-label="Open navigation menu"
                        class="btn btn-square btn-ghost drawer-button size-11 lg:hidden"
                    >
                        <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" class="inline-block w-5 h-5 stroke-current"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h16"></path></svg>
                    </button>
                    <BrandedLogo />
                    <div class="hidden lg:flex items-center gap-1 min-w-0 ml-4 pl-4 border-l border-base-300 overflow-x-clip">
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
                <input id="sidebar-drawer" type="checkbox" class="drawer-toggle" tabindex="-1" aria-hidden="true" prop:checked=move || sidebar_open.get() />
                <div inert=move || sidebar_open.get() class="drawer-content lg:col-start-2 flex flex-col bg-base-100 min-w-0">
                    // Sticky under the fixed header. Lives in the shell rather than
                    // the document page because pins govern navigation and links
                    // everywhere, including search results.
                    <ReleasePinStrip />
                    <main id="content" tabindex="-1" class=move || {
                        if is_chat_layout.get() {
                            "w-full h-[calc(100vh-4rem)] flex flex-col overflow-hidden lekton-content-enter"
                        } else {
                            "w-full max-w-[var(--lekton-content-max-width)] mx-auto px-4 py-6 sm:px-6 lg:px-10 lg:py-10 min-h-[calc(100vh-4rem)] lekton-content-enter"
                        }
                    }>
                        {children()}
                    </main>
                </div>

                // Navigation: the section sidebar on large screens, and on small
                // screens the one drawer with every destination, on every page.
                <div class=move || if has_context_sidebar.get() {
                    "drawer-side z-40 top-16 h-[calc(100dvh-4rem)] overflow-hidden"
                } else {
                    "drawer-side z-40 top-16 h-[calc(100dvh-4rem)] overflow-hidden lg:hidden"
                }>
                    <button type="button" tabindex="-1" aria-label="Close navigation menu" class="drawer-overlay lg:hidden" on:click=move |_| close_sidebar()></button>
                    // Colours come from the --lekton-sidebar-* theming tokens.
                    <nav id="portal-navigation" tabindex="-1" aria-label="Section navigation" data-lekton-sidebar on:click=move |ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            use wasm_bindgen::JsCast;
                            if ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).and_then(|e| e.closest("a").ok().flatten()).is_some() {
                                sidebar_open.set(false);
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = ev;
                    } class="menu bg-base-200 min-h-0 h-full w-[var(--lekton-sidebar-width)] p-4 text-base-content border-r border-base-300 pt-6 max-lg:pt-3 overflow-y-auto block">
                        <div class="flex items-center justify-between gap-2 mb-2 pl-3 lg:hidden">
                            <span class="font-semibold">"Menu"</span>
                            <button type="button" class="btn btn-square btn-ghost size-11" aria-label="Close navigation" on:click=move |_| close_sidebar()>
                                <svg aria-hidden="true" class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"></path></svg>
                            </button>
                        </div>
                        <ul class="lg:hidden">
                            <Suspense fallback=|| ()>
                                {move || {
                                    let items = use_navigation().get()?.ok()?;
                                    let groups = use_navbar_groups().get()?.ok()?;
                                    let include_docs = !path.get().starts_with("/docs");
                                    Some(view! { <SectionMenuItems items=items groups=groups include_docs=include_docs /> })
                                }}
                            </Suspense>
                        </ul>
                        <Show when=move || has_context_sidebar.get()>
                            <div class="divider my-2 lg:hidden"></div>
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
                        </Show>
                    </nav>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(slug: &str, children: usize) -> NavItem {
        NavItem {
            slug: slug.into(),
            title: slug.into(),
            parent_slug: None,
            order: 0,
            children: (0..children)
                .map(|i| NavItem {
                    slug: format!("{slug}/{i}"),
                    title: format!("{i}"),
                    parent_slug: Some(slug.into()),
                    order: 0,
                    children: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn administration_navigation_is_only_for_administrators() {
        assert!(has_context_sidebar("/admin/users", true));
        assert!(!has_context_sidebar("/admin/users", false));
        assert!(has_context_sidebar("/docs/handbook", false));
        assert!(!has_context_sidebar("/learn", true));
    }

    #[test]
    fn sections_come_before_single_documents_in_navigation_order() {
        let ordered = sections_first(vec![
            item("policy", 0),
            item("handbook", 2),
            item("faq", 0),
            item("orders", 1),
        ]);
        let slugs: Vec<_> = ordered.iter().map(|i| i.slug.as_str()).collect();
        assert_eq!(slugs, ["handbook", "orders", "policy", "faq"]);
    }
}
