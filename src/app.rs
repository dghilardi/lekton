use leptos::prelude::*;
use leptos_meta::{Stylesheet, Title, provide_meta_context};
use leptos_router::components::*;
use leptos_router::hooks::use_params_map;
use leptos_router::path;

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/lekton.css"/>
        <Title text="Lekton - Documentation Portal"/>

        <Router>
            <nav class="top-nav">
                <div class="logo">"Lekton"</div>
                <crate::components::search_bar::SearchBar />
            </nav>
            <main>
                <Routes fallback=|| view! { "Page not found." }.into_view()>
                    <Route path=path!("/") view=HomePage/>
                    <Route path=path!("/doc/:slug") view=DocumentPage/>
                    <Route path=path!("/edit/:slug") view=EditPage/>
                </Routes>
            </main>
        </Router>
    }
}

#[component]
fn HomePage() -> impl IntoView {
    view! {
        <h1>"Welcome to Lekton"</h1>
        <p>"The dynamic documentation portal."</p>
    }
}

#[component]
fn DocumentPage() -> impl IntoView {
    let params = use_params_map();
    let slug = move || params.get().get("slug").unwrap_or_default();

    view! {
        <h1>"Document: " {slug}</h1>
        <crate::components::document_view::DocumentView slug=slug() />
    }
}

#[component]
fn EditPage() -> impl IntoView {
    let params = use_params_map();
    let slug_memo = Memo::new(move |_| params.get().get("slug").unwrap_or_default());

    let doc_content = Resource::new(
        move || slug_memo.get(),
        |s| async move { crate::components::document_view::get_document_content(s).await },
    );

    view! {
        <div class="edit-page">
            <h1>"Editing: " {move || slug_memo.get()}</h1>
            <Suspense fallback=|| view! { <p>"Loading content..."</p> }>
                {move || doc_content.get().and_then(|res| match res {
                    Ok(content) => Some(view! {
                        <crate::components::editor::Editor slug=slug_memo.get() initial_content=content />
                    }.into_any()),
                    Err(e) => Some(view! { <p class="error">"Error loading content: " {e.to_string()}</p> }.into_any()),
                })}
            </Suspense>
        </div>
    }
}
