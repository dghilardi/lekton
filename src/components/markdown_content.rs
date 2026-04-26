use leptos::prelude::*;

/// Renders pre-built markdown HTML and triggers Mermaid diagram rendering after mount.
///
/// On the client (hydrate), calls `window.renderMermaid()` after each render so that
/// any `<pre class="mermaid">` elements emitted by the markdown renderer are processed.
#[component]
pub fn MarkdownContent(html: String) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    {
        Effect::new(move |_| {
            let _ = js_sys::eval("window.renderMermaid && window.renderMermaid()");
        });
    }

    view! { <div inner_html=html /> }
}
