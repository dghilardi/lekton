pub mod app;
pub mod models {
    pub mod document;
    pub mod link_validator;
    pub mod search;
}
#[cfg(feature = "ssr")]
pub mod auth;
#[cfg(feature = "ssr")]
pub mod state;
pub mod components {
    pub mod document_view;
    pub mod editor;
    pub mod search_bar;
}
#[cfg(feature = "ssr")]
pub mod api {
    pub mod ingest;
    pub mod search;
}
#[cfg(feature = "ssr")]
pub mod demo_seeder;

#[cfg(feature = "ssr")]
pub mod ssr_utils {
    use axum::response::IntoResponse;
    use http::StatusCode;
    use leptos::prelude::ServerFnError;

    pub fn server_fn_error_to_response(e: ServerFnError) -> impl IntoResponse {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}
