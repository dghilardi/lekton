pub mod api;
pub mod app;
pub mod auth;
pub mod db;
pub mod editor;
pub mod error;
pub mod rendering;
pub mod schema;
pub mod search;
pub mod storage;

/// Client-side hydration entry point.
///
/// This function is called by the WASM bundle to hydrate the server-rendered HTML.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
