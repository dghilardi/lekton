use leptos::prelude::*;

/// Renders pre-built markdown HTML and triggers Mermaid diagram rendering after mount.
///
/// On the client (hydrate), calls `window.renderMermaid()` once after the component
/// mounts so that any `<pre class="mermaid">` elements emitted by the markdown renderer
/// are processed. The call is idempotent: the loader skips nodes that already carry
/// `data-processed` or `data-mermaid-queued`, so duplicate calls are harmless.
#[component]
pub fn MarkdownContent(html: String) -> impl IntoView {
    #[cfg(all(feature = "hydrate", feature = "mermaid"))]
    {
        // Effect reads no reactive source → runs exactly once on mount per component instance.
        Effect::new(move |_| {
            let _ = js_sys::eval("window.renderMermaid && window.renderMermaid()");
        });
    }

    view! { <div inner_html=html /> }
}
