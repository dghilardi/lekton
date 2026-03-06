use leptos::prelude::*;

use crate::app::get_custom_css;

/// Runtime custom CSS component — injects user-defined CSS from settings.
#[component]
pub fn RuntimeCustomCss() -> impl IntoView {
    let css_resource = Resource::new(|| (), |_| get_custom_css());

    view! {
        <Suspense fallback=|| ()>
            {move || {
                css_resource.get().map(|result| match result {
                    Ok(css) if !css.is_empty() => {
                        view! {
                            <style>{css}</style>
                        }.into_any()
                    }
                    _ => view! { <span /> }.into_any(),
                })
            }}
        </Suspense>
    }
}
