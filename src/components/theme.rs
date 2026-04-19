use leptos::prelude::*;

/// Theme toggle component — cycles through system/light/dark themes.
///
/// Persists choice in localStorage and applies it to the `<html>` element's `data-theme`.
/// Uses three states: "system" (follows OS preference), "light", and "dark".
#[component]
pub fn ThemeToggle() -> impl IntoView {
    let (theme, set_theme) = signal("system".to_string());

    #[cfg(feature = "hydrate")]
    {
        let saved = js_sys::eval("localStorage.getItem('lekton-theme') || 'system'")
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| "system".to_string());
        set_theme.set(saved);
    }

    let cycle_theme = move |_| {
        let next = match theme.get().as_str() {
            "system" => "light",
            "light" => "dark",
            "dark" => "system",
            _ => "system",
        };
        set_theme.set(next.to_string());

        #[cfg(feature = "hydrate")]
        {
            let js_code = format!(
                r#"(function(){{
                    var theme = '{}';
                    if (theme === 'system') {{
                        localStorage.removeItem('lekton-theme');
                        var actual = window.matchMedia('(prefers-color-scheme:dark)').matches ? 'dark' : 'light';
                        document.documentElement.setAttribute('data-theme', actual);
                    }} else {{
                        localStorage.setItem('lekton-theme', theme);
                        document.documentElement.setAttribute('data-theme', theme);
                    }}
                }})()"#,
                next
            );
            let _ = js_sys::eval(&js_code);
        }
    };

    view! {
        <div class="tooltip tooltip-bottom" data-tip=move || {
            match theme.get().as_str() {
                "light" => "Light mode (click for dark)",
                "dark" => "Dark mode (click for system)",
                _ => "System theme (click for light)",
            }
        }>
            <button
                class="btn btn-ghost btn-sm btn-square"
                on:click=cycle_theme
                aria-label="Toggle theme"
            >
                {move || match theme.get().as_str() {
                    "light" => view! {
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z">
                            </path>
                        </svg>
                    }.into_any(),
                    "dark" => view! {
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M20.354 15.354A9 9 0 018.646 3.646 9.003 9.003 0 0012 21a9.003 9.003 0 008.354-5.646z">
                            </path>
                        </svg>
                    }.into_any(),
                    _ => view! {
                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z">
                            </path>
                        </svg>
                    }.into_any(),
                }}
            </button>
        </div>
    }
}
