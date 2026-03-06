use leptos::prelude::*;

use crate::app::logout_user;

/// User menu in the navbar: shows login link for anonymous users, or a
/// dropdown with the user's email and a logout button when authenticated.
#[component]
pub fn UserMenu() -> impl IntoView {
    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>()
        .expect("UserMenu must be inside App");

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
                                <span class="truncate max-w-[120px]">{display}</span>
                                {if is_admin {
                                    view! { <span class="badge badge-error badge-xs">"Admin"</span> }.into_any()
                                } else {
                                    view! { <span /> }.into_any()
                                }}
                            </div>
                            <ul tabindex="0" class="dropdown-content menu bg-base-100 rounded-box z-[1] w-52 p-2 shadow border border-base-200 mt-2">
                                <li class="menu-title text-xs opacity-60 px-2 pb-1 truncate">{user.email.clone()}</li>
                                <div class="divider my-1"></div>
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
                None => view! {
                    <a href="/login" class="btn btn-ghost btn-sm font-medium whitespace-nowrap">"Log In"</a>
                }.into_any(),
            }
        }}
    }
}
