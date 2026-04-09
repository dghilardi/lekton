use leptos::prelude::*;

use crate::app::{logout_user, IsDemoMode};

/// User menu in the navbar: shows login link for anonymous users, or a
/// dropdown with the user's email and a logout button when authenticated.
#[component]
pub fn UserMenu() -> impl IntoView {
    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>()
        .expect("UserMenu must be inside App");
    let is_demo_mode = use_context::<IsDemoMode>()
        .expect("UserMenu must be inside App").0;

    let logout_action = Action::new(|_: &()| async move {
        let _ = logout_user().await;
        #[cfg(feature = "hydrate")]
        {
            use leptos::web_sys::window;
            if let Some(w) = window() {
                let _ = w.location().assign("/");
            }
        }
    });

    view! {
        {move || {
            match current_user.get() {
                Some(user) => {
                    let display = user.name.clone().unwrap_or_else(|| user.email.clone());
                    let is_admin = user.is_admin;
                    view! {
                        <div class="dropdown dropdown-end">
                            <div tabindex="0" role="button" class="btn btn-ghost btn-sm gap-2 font-medium">
                                // Icon on small screens, name on sm+
                                <svg class="sm:hidden w-5 h-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="8" r="4"/><path d="M20 21a8 8 0 1 0-16 0"/></svg>
                                <span class="hidden sm:inline truncate max-w-[120px]">{display}</span>
                                {if is_admin {
                                    view! { <span class="hidden sm:inline badge badge-primary badge-xs">"Admin"</span> }.into_any()
                                } else {
                                    view! { <span /> }.into_any()
                                }}
                            </div>
                            <ul tabindex="0" class="dropdown-content menu bg-base-100 rounded-box z-[1] w-52 p-2 shadow border border-base-200 mt-2">
                                <li class="menu-title text-xs opacity-60 px-2 pb-1 truncate">{user.email.clone()}</li>
                                <div class="divider my-1"></div>
                                <li>
                                    <a href="/profile">
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 7a4 4 0 11-8 0 4 4 0 018 0zM21 21a9 9 0 10-18 0" />
                                        </svg>
                                        "Profile & Tokens"
                                    </a>
                                </li>
                                <li>
                                    <button
                                        class="text-error"
                                        on:click=move |_| { logout_action.dispatch(()); }
                                    >
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1" />
                                        </svg>
                                        "Log Out"
                                    </button>
                                </li>
                            </ul>
                        </div>
                    }.into_any()
                }
                None => {
                    // In demo mode, link to the in-app login form.
                    // In OAuth mode, link directly to the auth redirect endpoint.
                    let is_demo = is_demo_mode.get();
                    let href = if is_demo { "/login" } else { "/auth/login" };
                    let rel = if is_demo { "" } else { "external" };
                    view! {
                        <a href=href rel=rel class="btn btn-ghost btn-sm font-medium whitespace-nowrap">
                            <svg class="sm:hidden w-5 h-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="8" r="4"/><path d="M20 21a8 8 0 1 0-16 0"/></svg>
                            <span class="hidden sm:inline">"Log In"</span>
                        </a>
                    }.into_any()
                }
            }
        }}
    }
}
