//! Release selector for a document page.
//!
//! Lets a reader switch which release of the owning source they are reading.
//! Hidden unless the source published at least two releases — with one there is
//! nothing to choose.

use leptos::prelude::*;

use crate::versioning::{encode_pin_param, ReleasePins, PIN_PARAM};

/// Build the URL for reading `slug` at `release`, preserving any pin on other
/// sources.
///
/// Selecting the release that currently carries `latest` produces the *unpinned*
/// URL, so the canonical address stays clean and the reader follows the alias as
/// it moves rather than freezing on today's latest.
fn href_for(
    pathname: &str,
    existing: &[String],
    source_id: &str,
    release: &str,
    latest_release: Option<&str>,
) -> String {
    let mut pins = ReleasePins::from_param_values(existing);

    if latest_release == Some(release) {
        pins.remove(source_id);
    } else {
        pins.set(source_id, release);
    }

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

#[component]
pub fn ReleaseSelector(
    /// The source owning the document; absent for documents with no source.
    source_id: Option<String>,
    /// The release being read, `None` for an unversioned document.
    current_release: Option<String>,
    /// Releases published by this source, newest-published first.
    releases: Vec<String>,
    /// The release currently aliased `latest`.
    latest_release: Option<String>,
) -> impl IntoView {
    let Some(source_id) = source_id else {
        return ().into_any();
    };

    // One release is not a choice, so the control stays out of the way entirely.
    if releases.len() < 2 {
        return ().into_any();
    }

    let location = leptos_router::hooks::use_location();
    let query = leptos_router::hooks::use_query_map();

    let label = current_release
        .clone()
        .unwrap_or_else(|| "latest".to_string());
    let is_pinned_to_older =
        current_release.is_some() && current_release.as_deref() != latest_release.as_deref();

    view! {
        <div class="dropdown dropdown-end flex-shrink-0">
            <div
                tabindex="0"
                role="button"
                aria-haspopup="menu"
                aria-label="Select documentation release"
                class="btn btn-ghost btn-sm gap-1.5 font-normal text-base-content/70 hover:text-base-content"
            >
                <svg class="w-3.5 h-3.5 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M7 7h.01M7 3h5a1.99 1.99 0 011.414.586l7 7a2 2 0 010 2.828l-7 7a2 2 0 01-2.828 0l-7-7A1.99 1.99 0 013 12V7a4 4 0 014-4z" />
                </svg>
                <span class="font-mono text-xs">{label}</span>
                {move || if is_pinned_to_older {
                    view! { <span class="badge badge-xs border-primary/30 text-primary">"pinned"</span> }.into_any()
                } else {
                    ().into_any()
                }}
                <svg class="w-3 h-3 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="m6 9 6 6 6-6" />
                </svg>
            </div>
            <ul
                tabindex="0"
                class="dropdown-content z-[1] menu p-2 shadow bg-base-100 rounded-box w-56 border border-base-200"
            >
                <li class="menu-title text-[10px] uppercase tracking-wider">"Release"</li>
                {releases.into_iter().map(|release| {
                    let source_id = source_id.clone();
                    let latest = latest_release.clone();
                    let is_current = current_release.as_deref() == Some(release.as_str())
                        || (current_release.is_none() && latest.as_deref() == Some(release.as_str()));
                    let is_latest = latest.as_deref() == Some(release.as_str());
                    let shown = release.clone();

                    let href = {
                        let release = release.clone();
                        Signal::derive(move || {
                            let existing = query
                                .read()
                                .get_all(PIN_PARAM)
                                .unwrap_or_default();
                            href_for(
                                &location.pathname.get(),
                                &existing,
                                &source_id,
                                &release,
                                latest.as_deref(),
                            )
                        })
                    };

                    view! {
                        <li>
                            <a
                                href=move || href.get()
                                aria-current=move || if is_current { Some("true") } else { None }
                                class:font-semibold=move || is_current
                                class="flex items-center justify-between gap-2 text-sm"
                            >
                                <span class="font-mono text-xs">{shown}</span>
                                {if is_latest {
                                    view! {
                                        <span class="badge badge-xs badge-ghost">"latest"</span>
                                    }.into_any()
                                } else {
                                    ().into_any()
                                }}
                            </a>
                        </li>
                    }
                }).collect::<Vec<_>>()}
            </ul>
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecting_latest_produces_the_unpinned_url() {
        let href = href_for("/docs/a", &[], "svc", "1.2.0", Some("1.2.0"));
        assert_eq!(
            href, "/docs/a",
            "the canonical address for latest carries no pin, so readers follow the alias"
        );
    }

    #[test]
    fn selecting_an_older_release_pins_it() {
        let href = href_for("/docs/a", &[], "svc", "1.0.0", Some("1.2.0"));
        assert_eq!(href, "/docs/a?v=svc:1.0.0");
    }

    #[test]
    fn other_sources_keep_their_pins() {
        let existing = vec!["other:9.9.9".to_string()];
        let href = href_for("/docs/a", &existing, "svc", "1.0.0", Some("1.2.0"));

        assert!(
            href.contains("other:9.9.9"),
            "an unrelated pin must survive the switch, got: {href}"
        );
        assert!(href.contains("svc:1.0.0"), "got: {href}");
    }

    #[test]
    fn switching_back_to_latest_drops_only_this_sources_pin() {
        let existing = vec!["svc:1.0.0".to_string(), "other:9.9.9".to_string()];
        let href = href_for("/docs/a", &existing, "svc", "1.2.0", Some("1.2.0"));

        assert!(!href.contains("svc:"), "got: {href}");
        assert!(
            href.contains("other:9.9.9"),
            "the other pin must remain, got: {href}"
        );
    }

    #[test]
    fn re_selecting_a_pinned_release_is_idempotent() {
        let existing = vec!["svc:1.0.0".to_string()];
        let href = href_for("/docs/a", &existing, "svc", "1.0.0", Some("1.2.0"));
        assert_eq!(href, "/docs/a?v=svc:1.0.0");
    }

    #[test]
    fn only_query_breaking_characters_are_encoded() {
        assert_eq!(
            encode_pin_param("svc:1.0.0"),
            "svc:1.0.0",
            "a colon is legal in a query value, so a pinned URL stays readable"
        );
        assert_eq!(encode_pin_param("a&b=c"), "a%26b%3Dc");
        assert_eq!(encode_pin_param("a b"), "a%20b");
        assert_eq!(encode_pin_param("100%"), "100%25");
    }
}
