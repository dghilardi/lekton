//! Unsaved-change protection for the editor: dirty tracking, a confirmation
//! before leaving the page and a local draft that survives leaving anyway.
use serde::{Deserialize, Serialize};

/// The editor's form fields besides the document body.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EditorFields {
    pub slug: String,
    pub title: String,
    pub access_level: String,
    pub parent_slug: String,
    pub order: u32,
}

/// Unsaved editor state kept in the browser's local storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorDraft {
    pub fields: EditorFields,
    pub content: String,
    /// Milliseconds since the Unix epoch.
    pub saved_at: f64,
}

impl EditorDraft {
    /// Whether restoring this draft would change what the editor shows.
    pub fn differs_from(&self, fields: &EditorFields, content: &str) -> bool {
        &self.fields != fields || self.content != content
    }
}

/// The local-storage key for the draft of the page at `route_slug`; creation
/// without a slug in the route shares one key.
pub fn draft_key(route_slug: &str) -> String {
    if route_slug.is_empty() {
        "lekton-editor-draft:(new)".to_string()
    } else {
        format!("lekton-editor-draft:{route_slug}")
    }
}

pub const LEAVE_MESSAGE: &str =
    "You have unsaved changes. Leave this page and keep them only as a local draft?";

#[cfg(feature = "hydrate")]
pub mod browser {
    use super::{EditorDraft, LEAVE_MESSAGE};
    use leptos::prelude::*;
    use wasm_bindgen::{closure::Closure, JsCast};

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    pub fn load_draft(key: &str) -> Option<EditorDraft> {
        let raw = storage()?.get_item(key).ok().flatten()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn store_draft(key: &str, draft: &EditorDraft) {
        if let (Some(storage), Ok(raw)) = (storage(), serde_json::to_string(draft)) {
            let _ = storage.set_item(key, &raw);
        }
    }

    pub fn remove_draft(key: &str) {
        if let Some(storage) = storage() {
            let _ = storage.remove_item(key);
        }
    }

    pub fn now() -> f64 {
        js_sys::Date::now()
    }

    /// While `dirty` holds, ask before leaving: on unload (reload, closing the
    /// tab, external links) and on in-app links, which the router would
    /// otherwise follow without unloading the page. The browser's back button
    /// is not intercepted; the local draft covers it.
    pub fn install_leave_guard(dirty: Signal<bool>) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let before_unload = Closure::<dyn Fn(web_sys::BeforeUnloadEvent)>::new(
            move |event: web_sys::BeforeUnloadEvent| {
                if dirty.try_get_untracked().unwrap_or(false) {
                    event.prevent_default();
                    event.set_return_value(LEAVE_MESSAGE);
                }
            },
        );
        let click =
            Closure::<dyn Fn(web_sys::MouseEvent)>::new(move |event: web_sys::MouseEvent| {
                if !dirty.try_get_untracked().unwrap_or(false) || !leaves_page(&event) {
                    return;
                }
                let confirmed = web_sys::window()
                    .and_then(|w| w.confirm_with_message(LEAVE_MESSAGE).ok())
                    .unwrap_or(true);
                if !confirmed {
                    event.prevent_default();
                    event.stop_immediate_propagation();
                }
            });
        let _ = window.add_event_listener_with_callback(
            "beforeunload",
            before_unload.as_ref().unchecked_ref(),
        );
        // Capture phase, so this runs before the router's link handler.
        let _ = window.add_event_listener_with_callback_and_bool(
            "click",
            click.as_ref().unchecked_ref(),
            true,
        );
        let listeners = send_wrapper::SendWrapper::new((before_unload, click));
        on_cleanup(move || {
            let (before_unload, click) = listeners.take();
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "beforeunload",
                    before_unload.as_ref().unchecked_ref(),
                );
                let _ = window.remove_event_listener_with_callback_and_bool(
                    "click",
                    click.as_ref().unchecked_ref(),
                    true,
                );
            }
        });
    }

    /// Whether a click follows a same-origin link to another page.
    fn leaves_page(event: &web_sys::MouseEvent) -> bool {
        if event.default_prevented()
            || event.button() != 0
            || event.meta_key()
            || event.ctrl_key()
            || event.shift_key()
            || event.alt_key()
        {
            return false;
        }
        let Some(anchor) = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|e| e.closest("a[href]").ok().flatten())
            .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok())
        else {
            return false;
        };
        if anchor.target() == "_blank" || anchor.has_attribute("download") {
            return false;
        }
        let Some(location) = web_sys::window().map(|w| w.location()) else {
            return false;
        };
        // External links unload the page, where `beforeunload` asks instead.
        anchor.origin() == location.origin().unwrap_or_default()
            && (anchor.pathname() != location.pathname().unwrap_or_default()
                || anchor.search() != location.search().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> EditorFields {
        EditorFields {
            slug: "guides/start".into(),
            title: "Start".into(),
            access_level: "internal".into(),
            parent_slug: String::new(),
            order: 2,
        }
    }

    #[test]
    fn drafts_round_trip_through_json() {
        let draft = EditorDraft {
            fields: fields(),
            content: "<p>Hi</p>".into(),
            saved_at: 1.0,
        };
        let raw = serde_json::to_string(&draft).unwrap();
        assert_eq!(serde_json::from_str::<EditorDraft>(&raw).unwrap(), draft);
    }

    #[test]
    fn a_draft_differs_only_when_fields_or_content_change() {
        let draft = EditorDraft {
            fields: fields(),
            content: "<p>Hi</p>".into(),
            saved_at: 1.0,
        };
        assert!(!draft.differs_from(&fields(), "<p>Hi</p>"));
        assert!(draft.differs_from(&fields(), "<p>Other</p>"));
        let mut renamed = fields();
        renamed.title = "Renamed".into();
        assert!(draft.differs_from(&renamed, "<p>Hi</p>"));
    }

    #[test]
    fn creation_without_a_route_slug_has_its_own_key() {
        assert_eq!(draft_key(""), "lekton-editor-draft:(new)");
        assert_eq!(
            draft_key("guides/start"),
            "lekton-editor-draft:guides/start"
        );
    }
}
