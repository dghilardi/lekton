#[cfg(feature = "hydrate")]
use crate::components::diagnostics::internal::{record_audit_event, render_integrity_overlay};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogoConfig {
    /// Effective logo URL for light theme (None = use inline SVG fallback)
    pub light_url: Option<String>,
    /// Effective logo URL for dark theme (None = use inline SVG fallback)
    pub dark_url: Option<String>,
}

/// Server-side check: resolves which logo files are present in public/ and
/// returns the effective URL for each theme with the following priority:
///   logo-{theme}.svg  >  logo.svg  >  None (inline SVG)
#[server]
pub async fn check_logo_availability() -> Result<LogoConfig, ServerFnError> {
    let base = std::path::Path::new("public/logo.svg").exists();
    let light_file = std::path::Path::new("public/logo-light.svg").exists();
    let dark_file = std::path::Path::new("public/logo-dark.svg").exists();

    let light_url = if light_file {
        Some("/logo-light.svg".to_string())
    } else if base {
        Some("/logo.svg".to_string())
    } else {
        None
    };

    let dark_url = if dark_file {
        Some("/logo-dark.svg".to_string())
    } else if base {
        Some("/logo.svg".to_string())
    } else {
        None
    };

    Ok(LogoConfig {
        light_url,
        dark_url,
    })
}

#[component]
pub fn BrandedLogo() -> impl IntoView {
    let logo_config = Resource::new(|| (), |_| check_logo_availability());

    // Diagnostics audit state — tracks rapid-click sequences on the brand mark
    #[cfg(feature = "hydrate")]
    let audit_counter = store_value(0u32);
    #[cfg(feature = "hydrate")]
    let audit_ts = store_value(0f64);

    #[cfg(feature = "hydrate")]
    let on_logo_click = move |_ev: leptos::ev::MouseEvent| {
        let mut counter = audit_counter.get_value();
        let mut ts = audit_ts.get_value();
        let triggered = record_audit_event(&mut counter, &mut ts);
        audit_counter.set_value(counter);
        audit_ts.set_value(ts);
        if triggered {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                render_integrity_overlay(&doc);
            }
        }
    };

    #[cfg(not(feature = "hydrate"))]
    let on_logo_click = move |_ev: leptos::ev::MouseEvent| {};

    view! {
        <a class="flex items-center gap-3 text-xl font-bold tracking-tight hover:opacity-80 transition-opacity" href="/" on:click=on_logo_click>

            <div class="relative w-8 h-8 flex items-center justify-center">
                <Suspense fallback=move || view! {
                    <svg class="w-7 h-7 text-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M12 2L2 7l10 5 10-5-10-5Z"/>
                        <path d="M2 17l10 5 10-5"/>
                        <path d="M2 12l10 5 10-5"/>
                    </svg>
                }>
                    {move || {
                        if let Some(Ok(config)) = logo_config.get() {
                            let light_url = config.light_url;
                            let dark_url = config.dark_url;

                            if light_url == dark_url {
                                // Same source for both themes (or both absent): single element, no theme classes needed
                                match light_url {
                                    Some(url) => view! {
                                        <img src=url alt="Logo" class="w-full h-full object-contain" />
                                    }.into_any(),
                                    None => view! {
                                        <svg class="w-7 h-7 text-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                                            <path d="M12 2L2 7l10 5 10-5-10-5Z"/>
                                            <path d="M2 17l10 5 10-5"/>
                                            <path d="M2 12l10 5 10-5"/>
                                        </svg>
                                    }.into_any(),
                                }
                            } else {
                                // Different source per theme: render both overlaid, CSS controls visibility
                                view! {
                                    {match light_url {
                                        Some(url) => view! {
                                            <img src=url alt="Logo" class="absolute inset-0 w-full h-full object-contain lekton-logo-light" />
                                        }.into_any(),
                                        None => view! {
                                            <svg class="absolute inset-0 w-7 h-7 m-auto text-primary lekton-logo-light" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                                                <path d="M12 2L2 7l10 5 10-5-10-5Z"/>
                                                <path d="M2 17l10 5 10-5"/>
                                                <path d="M2 12l10 5 10-5"/>
                                            </svg>
                                        }.into_any(),
                                    }}
                                    {match dark_url {
                                        Some(url) => view! {
                                            <img src=url alt="Logo" class="absolute inset-0 w-full h-full object-contain lekton-logo-dark" />
                                        }.into_any(),
                                        None => view! {
                                            <svg class="absolute inset-0 w-7 h-7 m-auto text-primary lekton-logo-dark" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                                                <path d="M12 2L2 7l10 5 10-5-10-5Z"/>
                                                <path d="M2 17l10 5 10-5"/>
                                                <path d="M2 12l10 5 10-5"/>
                                            </svg>
                                        }.into_any(),
                                    }}
                                }.into_any()
                            }
                        } else {
                            // Resource not yet resolved or server error — show inline SVG
                            view! {
                                <svg class="w-7 h-7 text-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M12 2L2 7l10 5 10-5-10-5Z"/>
                                    <path d="M2 17l10 5 10-5"/>
                                    <path d="M2 12l10 5 10-5"/>
                                </svg>
                            }.into_any()
                        }
                    }}
                </Suspense>
            </div>

            // Brand name
            <span class="hidden sm:inline truncate max-w-[150px] text-base-content">
                "Lekton"
            </span>
        </a>
    }
}
