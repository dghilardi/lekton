mod contextual_sidebars;
mod custom_css;
mod diagnostics;
mod layout;
mod logo;
mod markdown_content;
mod navigation;
mod reader_resource;
mod release_pin_strip;
mod release_selector;
mod search;
mod theme;
mod user_menu;

pub use contextual_sidebars::*;
pub use custom_css::*;
pub use layout::*;
pub use logo::*;
pub use markdown_content::*;
pub use navigation::*;
pub use reader_resource::*;
pub use release_pin_strip::*;
pub use release_selector::*;
pub use search::*;
pub use theme::*;
pub use user_menu::*;

/// The URL's release pins, shared by every link renderer.
///
/// Rule for SSR: a `Memo` read by more than one `Suspense` boundary must be
/// computed before the first boundary renders (as `Layout` does for this one),
/// because boundaries resolve concurrently on server worker threads and a
/// first computation racing another thread can expose an unset value.
#[derive(Clone, Copy)]
pub struct ActiveReleasePins(pub leptos::prelude::Memo<Vec<String>>);

/// Add the active URL release pins to an internal documentation link.
pub fn active_release_pin_values() -> Vec<String> {
    use leptos::prelude::*;

    use_context::<ActiveReleasePins>()
        .map(|active| active.0.get())
        .unwrap_or_default()
}

/// Add the active URL release pins to an internal documentation link.
pub fn pinned_doc_href(url: &str) -> String {
    crate::versioning::with_release_pins(url, &active_release_pin_values())
}
