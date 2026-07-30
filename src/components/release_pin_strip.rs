//! Persistent strip declaring which sources are pinned to a non-`latest` release.
//!
//! Present only while at least one pin is active, so it is self-limiting: remove
//! the last chip and it disappears entirely.
//!
//! The treatment is deliberately *evident by position and typography, neutral by
//! colour*: a state toolbar under the header rather than an alert. A warning
//! yellow would fight the amber primary and would read as "something is broken",
//! when pinning is a thing the reader asked for.

use leptos::prelude::*;

use crate::versioning::{encode_pin_param, ReleasePins, PIN_PARAM};

/// Rebuild the current URL with `pins` as its pin set.
fn url_with_pins(pathname: &str, pins: &ReleasePins) -> String {
    let values = pins.to_param_values();
    if values.is_empty() {
        return pathname.to_string();
    }
    let query = values
        .iter()
        .map(|v| format!("{PIN_PARAM}={}", encode_pin_param(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{pathname}?{query}")
}

/// The URL that results from dropping `source_id`'s pin.
fn url_without(pathname: &str, existing: &[String], source_id: &str) -> String {
    let mut pins = ReleasePins::from_param_values(existing);
    pins.remove(source_id);
    url_with_pins(pathname, &pins)
}

#[component]
pub fn ReleasePinStrip() -> impl IntoView {
    let location = leptos_router::hooks::use_location();
    let query = leptos_router::hooks::use_query_map();

    let pins = Signal::derive(move || {
        ReleasePins::from_param_values(query.read().get_all(PIN_PARAM).unwrap_or_default())
    });
    let raw = Signal::derive(move || query.read().get_all(PIN_PARAM).unwrap_or_default());
    let has_pins = Signal::derive(move || !pins.get().is_empty());
    let count = Signal::derive(move || pins.get().len());

    // No enter/exit animation on purpose: pins only change by navigating, so the
    // strip appears alongside the page transition that is already running.
    view! {
        <Show when=move || has_pins.get()>
            {
                let reset_href = Signal::derive(move || location.pathname.get());
                view! {
                    <div
                        role="region"
                        aria-label="Active release pins"
                        class="sticky top-16 z-30 border-b border-base-300 bg-base-200/95 backdrop-blur-sm"
                    >
                        <div class="flex items-center gap-2 px-4 py-2 sm:px-6 lg:px-10">
                            <svg
                                class="h-3.5 w-3.5 shrink-0 opacity-60"
                                fill="none"
                                stroke="currentColor"
                                viewBox="0 0 24 24"
                                aria-hidden="true"
                            >
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                    d="M7 7h.01M7 3h5a1.99 1.99 0 011.414.586l7 7a2 2 0 010 2.828l-7 7a2 2 0 01-2.828 0l-7-7A1.99 1.99 0 013 12V7a4 4 0 014-4z" />
                            </svg>
                            <span class="hidden shrink-0 text-xs font-medium text-base-content/70 sm:inline">
                                "Viewing"
                            </span>

                            // Chips keep their natural width and scroll, like browser
                            // tabs: shrinking them to illegibility would defeat the
                            // point. The counter button below is the reliable path, so
                            // nothing is ever hidden without an affordance.
                            <div class="flex min-w-0 flex-1 gap-1.5 overflow-x-auto lekton-pin-scroll">
                                {move || pins.get().iter().map(|pin| {
                                    let source_id = pin.source_id.clone();
                                    let release = pin.release.clone();
                                    let remove_href = {
                                        let source_id = source_id.clone();
                                        Signal::derive(move || url_without(
                                            &location.pathname.get(),
                                            &raw.get(),
                                            &source_id,
                                        ))
                                    };
                                    let aria = format!("Stop viewing {source_id} at {release}");
                                    view! {
                                        <span class="badge badge-sm shrink-0 gap-1 border-primary/30 bg-base-100 py-3 text-base-content">
                                            <span class="text-xs font-medium">{source_id.clone()}</span>
                                            <span class="font-mono text-xs text-primary">
                                                {format!("@{release}")}
                                            </span>
                                            <a
                                                href=move || remove_href.get()
                                                aria-label=aria
                                                title="Remove pin"
                                                class="ml-0.5 -mr-1 flex h-4 w-4 items-center justify-center rounded-full text-base-content/50 hover:bg-base-300 hover:text-base-content"
                                            >
                                                <svg class="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M6 18L18 6M6 6l12 12" />
                                                </svg>
                                            </a>
                                        </span>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>

                            <a
                                href=move || reset_href.get()
                                class="btn btn-ghost btn-xs shrink-0 font-normal text-base-content/60 hover:text-base-content"
                            >
                                {move || {
                                    let n = count.get();
                                    if n > 1 {
                                        format!("Reset all {n} to latest")
                                    } else {
                                        "Reset to latest".to_string()
                                    }
                                }}
                            </a>
                        </div>
                    </div>
                }
            }
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_the_only_pin_yields_the_bare_path() {
        let existing = vec!["svc:1.0.0".to_string()];
        assert_eq!(
            url_without("/docs/a", &existing, "svc"),
            "/docs/a",
            "dropping the last pin must leave a clean URL so the strip disappears"
        );
    }

    #[test]
    fn removing_one_pin_leaves_the_others() {
        let existing = vec!["a:1.0.0".to_string(), "b:2.0.0".to_string()];
        let url = url_without("/docs/x", &existing, "a");

        assert!(!url.contains("a:1.0.0"), "got: {url}");
        assert!(url.contains("b:2.0.0"), "got: {url}");
    }

    #[test]
    fn removing_an_unpinned_source_changes_nothing() {
        let existing = vec!["a:1.0.0".to_string()];
        assert_eq!(
            url_without("/docs/x", &existing, "zzz"),
            "/docs/x?v=a:1.0.0"
        );
    }
}
