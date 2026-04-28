use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::app::AppState;

#[server(GetCustomCss, "/api")]
pub async fn get_custom_css() -> Result<String, ServerFnError> {
    let state = expect_context::<AppState>();
    let settings = state
        .settings_repo
        .get_settings()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(settings.custom_css)
}

#[server(SaveCustomCss, "/api")]
pub async fn save_custom_css(css: String) -> Result<String, ServerFnError> {
    let state = expect_context::<AppState>();
    state
        .settings_repo
        .set_custom_css(&css)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok("Custom CSS saved successfully".to_string())
}
