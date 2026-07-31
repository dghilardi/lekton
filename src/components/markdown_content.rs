use leptos::prelude::*;

/// Renders pre-built markdown HTML and triggers Mermaid diagram rendering after mount.
///
/// On the client (hydrate), calls `window.renderMermaid()` once after the component
/// mounts so that any `<pre class="mermaid">` elements emitted by the markdown renderer
/// are processed. The call is idempotent: the loader skips nodes that already carry
/// `data-processed` or `data-mermaid-queued`, so duplicate calls are harmless.
#[component]
pub fn MarkdownContent(html: String) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    {
        // Scroll to the hash anchor after the HTML is injected into the DOM.
        Effect::new(move |_| {
            let _ = js_sys::eval("let h=window.location.hash;if(h){let el=document.getElementById(h.slice(1));if(el)el.scrollIntoView({behavior:'smooth'})}");
        });
    }

    #[cfg(all(feature = "hydrate", feature = "mermaid"))]
    {
        // Effect reads no reactive source → runs exactly once on mount per component instance.
        Effect::new(move |_| {
            let _ = js_sys::eval("window.renderMermaid && window.renderMermaid()");
        });
    }

    #[cfg(feature = "hydrate")]
    {
        Effect::new(move |_| {
            let _ = js_sys::eval("window.initCodeBlocks && window.initCodeBlocks()");
        });

        // Rendered markdown arrives as HTML, outside Leptos' typed link
        // components. Keep its internal documentation links in the same pinned
        // release view as the surrounding navigation shell.
        Effect::new(move |_| {
            let _ = js_sys::eval(
                "(()=>{const pins=new URLSearchParams(location.search).getAll('v');\
                 document.querySelectorAll('[data-lekton-markdown] a[href]').forEach(a=>{\
                 const u=new URL(a.getAttribute('href'),location.href);\
                 if(u.origin!==location.origin||!u.pathname.startsWith('/docs/'))return;\
                 u.searchParams.delete('v');pins.forEach(v=>u.searchParams.append('v',v));\
                 a.setAttribute('href',u.pathname+u.search+u.hash)})})()",
            );
        });
    }

    view! { <div data-lekton-markdown inner_html=html /> }
}
