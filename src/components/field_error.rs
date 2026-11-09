//! Inline form validation shared by admin forms.
use leptos::prelude::*;

/// The message for an invalid field, rendered right after it. The field points
/// at it with `aria-describedby` (see [`described_by`]) and sets `aria-invalid`.
#[component]
pub fn FieldError(id: &'static str, #[prop(into)] error: Signal<Option<String>>) -> impl IntoView {
    move || {
        error
            .get()
            .map(|message| view! { <p id=id class="text-error text-sm mt-1">{message}</p> })
    }
}

/// `aria-describedby` for a field: its hint, if any, plus its error while shown.
pub fn described_by(
    hint_id: Option<&'static str>,
    error_id: &'static str,
    has_error: bool,
) -> Option<String> {
    let ids: Vec<&str> = hint_id
        .into_iter()
        .chain(has_error.then_some(error_id))
        .collect();
    (!ids.is_empty()).then(|| ids.join(" "))
}

/// Move keyboard focus to the element with `id`, e.g. the first invalid field.
pub fn focus_element(id: &str) {
    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::JsCast;
        if let Some(element) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(id))
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = element.focus();
        }
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = id;
}

#[cfg(test)]
mod tests {
    use super::described_by;

    #[test]
    fn descriptions_include_the_error_only_while_it_is_shown() {
        assert_eq!(described_by(None, "e", false), None);
        assert_eq!(described_by(None, "e", true).as_deref(), Some("e"));
        assert_eq!(described_by(Some("h"), "e", true).as_deref(), Some("h e"));
        assert_eq!(described_by(Some("h"), "e", false).as_deref(), Some("h"));
    }
}
