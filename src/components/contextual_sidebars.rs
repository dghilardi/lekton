use leptos::prelude::*;
use crate::app::{get_navigation, NavItem};
use crate::schema::component::list_schemas;
use crate::api::schemas::SchemaListItem;
use crate::pages::chat::ChatContext;

/// Sidebar for Documentation section.
/// This is just a wrapper around NavigationTree (existing).
#[component]
pub fn DocsSidebar() -> impl IntoView {
    use super::navigation::NavigationTree;
    view! {
        <ul class="flex flex-col gap-1 mt-6">
            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"Navigation"</li>
            <NavigationTree />
        </ul>
    }
}

/// Sidebar for Schema Registry.
/// Shows a searchable list of all registered schemas.
#[component]
pub fn RegistrySidebar() -> impl IntoView {
    let schemas_resource = Resource::new(|| (), |_| list_schemas());

    view! {
        <ul class="flex flex-col gap-1 mt-6">
            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"Schemas"</li>
            <Suspense fallback=move || view! { <li><span class="loading loading-spinner loading-sm"></span></li> }>
                {move || schemas_resource.get().map(|result| match result {
                    Ok(schemas) => {
                        if schemas.is_empty() {
                            view! { <li class="px-3 py-2 text-xs italic opacity-50">"No schemas found"</li> }.into_any()
                        } else {
                            view! {
                                {schemas.into_iter().map(|schema| {
                                    let href = format!("/schemas/{}", schema.name);
                                    view! {
                                        <li>
                                            <a href=href class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                                                <svg class="w-4 h-4 opacity-70 flex-shrink-0" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 7h10"/><path d="M7 12h10"/><path d="M7 17h10"/></svg>
                                                <span class="truncate">{schema.name}</span>
                                            </a>
                                        </li>
                                    }
                                }).collect::<Vec<_>>()}
                            }.into_any()
                        }
                    }
                    Err(_) => view! { <li class="text-error italic text-xs px-3 py-2">"Error loading schemas"</li> }.into_any(),
                })}
            </Suspense>
        </ul>
    }
}

/// Sidebar for Admin Panel.
/// Shows navigation links to different admin sub-pages.
#[component]
pub fn AdminSidebar() -> impl IntoView {
    view! {
        <ul class="flex flex-col gap-1 mt-6">
            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"Administration"</li>
            <li>
                <a href="/admin/tokens" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                    <svg class="w-4 h-4 opacity-70" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"></path></svg>
                    "Service Tokens"
                </a>
            </li>
            <li>
                <a href="/admin/navigation" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                    <svg class="w-4 h-4 opacity-70" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h16M4 10h16M4 14h16M4 18h16"></path></svg>
                    "Nav Ordering"
                </a>
            </li>
            <li>
                <a href="/admin/css" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                    <svg class="w-4 h-4 opacity-70" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L2 7l10 5 10-5-10-5Z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>
                    "Custom CSS"
                </a>
            </li>
            <li>
                <a href="/admin/rag" class="gap-3 group data-[active]:bg-primary/10 data-[active]:text-primary data-[active]:font-medium transition-colors">
                    <svg class="w-4 h-4 opacity-70" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg>
                    "RAG Settings"
                </a>
            </li>
        </ul>
    }
}

/// Sidebar for AI Chat history.
#[component]
pub fn ChatSidebar() -> impl IntoView {
    let context = use_context::<ChatContext>();
    
    view! {
        <ul class="flex flex-col gap-1 mt-6">
            <li class="menu-title text-xs font-semibold tracking-wider text-base-content/60 uppercase mb-1">"AI Chat"</li>
            {move || match context {
                Some(ctx) => {
                    let sessions = ctx.sessions;
                    let session_id = ctx.session_id;
                    let messages = ctx.messages;
                    let error_msg = ctx.error_msg;

                    let start_new_session = move |_| {
                        session_id.set(None);
                        messages.set(Vec::new());
                        error_msg.set(None);
                    };

                    let delete_session = move |sid: String| {
                        #[cfg(feature = "hydrate")]
                        {
                            use crate::pages::chat::fetch_delete_session;
                            use leptos::task::spawn_local;
                            let current_sid = session_id.get_untracked();
                            spawn_local(async move {
                                if fetch_delete_session(&sid).await.is_ok() {
                                    sessions.update(|sessions| {
                                        sessions.retain(|s| s.id != sid);
                                    });
                                    if current_sid.as_deref() == Some(&sid) {
                                        session_id.set(None);
                                        messages.set(Vec::new());
                                    }
                                }
                            });
                        }
                    };

                    view! {
                        <li>
                            <button class="btn btn-primary btn-sm w-full gap-2 mb-4" on:click=start_new_session>
                                <svg class="w-4 h-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="M5 12h14"/></svg>
                                "New Chat"
                            </button>
                        </li>
                        <For
                            each=move || sessions.get()
                            key=|s| s.id.clone()
                            children=move |session| {
                                let sid_click = session.id.clone();
                                let sid_delete = session.id.clone();
                                let is_active = {
                                    let sid = session.id.clone();
                                    move || session_id.get().as_deref() == Some(&sid)
                                };
                                view! {
                                    <li class="flex items-center group gap-1">
                                        <button
                                            class=move || format!(
                                                "btn btn-ghost btn-sm flex-1 justify-start text-left truncate font-normal px-2 hover:bg-base-300/50 {}",
                                                if is_active() { "bg-primary/10 text-primary font-medium" } else { "text-base-content/70" }
                                            )
                                            on:click={
                                                let sid = sid_click.clone();
                                                move |_| {
                                                    session_id.set(Some(sid.clone()));
                                                    messages.set(Vec::new());
                                                    error_msg.set(None);
                                                }
                                            }
                                        >
                                            <svg class="w-4 h-4 opacity-50 mr-1 flex-shrink-0" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                                            <span class="truncate text-xs">{session.title.clone()}</span>
                                        </button>
                                        <button
                                            class="btn btn-ghost btn-xs btn-square opacity-0 group-hover:opacity-100 hover:text-error transition-opacity"
                                            on:click={
                                                let sid = sid_delete.clone();
                                                move |_| delete_session(sid.clone())
                                            }
                                        >
                                            <svg class="w-3 h-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/></svg>
                                        </button>
                                    </li>
                                }
                            }
                        />
                    }.into_any()
                }
                None => view! { <li class="px-3 py-2 text-xs italic opacity-50">"No chat session active"</li> }.into_any(),
            }}
        </ul>
    }
}
