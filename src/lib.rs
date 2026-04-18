#![recursion_limit = "256"]

pub mod api;
pub mod app;
pub mod auth;
pub mod components;
#[cfg(feature = "ssr")]
pub mod config;
pub mod db;
pub mod editor;
pub mod error;
#[cfg(feature = "ssr")]
pub mod mcp;
pub mod pages;
pub mod rag;
pub mod rendering;
pub mod schema;
pub mod search;
pub mod storage;
#[cfg(test)]
pub mod test_utils;

/// Client-side hydration entry point.
///
/// This function is called by the WASM bundle to hydrate the server-rendered HTML.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
