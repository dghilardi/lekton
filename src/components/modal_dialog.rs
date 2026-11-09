//! Modal dialogs on the native `<dialog>` element.
use leptos::prelude::*;

/// A modal built on `<dialog>.showModal()`: the rest of the page becomes inert,
/// so focus stays inside; Escape closes it; and closing returns focus to the
/// control that opened it. `labelled_by` names the dialog after its heading.
#[component]
pub fn ModalDialog(
    #[prop(into)] open: Signal<bool>,
    on_close: Callback<()>,
    #[prop(into)] labelled_by: String,
    /// Extra classes for the dialog box.
    #[prop(optional)]
    class: &'static str,
    children: ChildrenFn,
) -> impl IntoView {
    let dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    Effect::new(move || {
        let open = open.get();
        if let Some(dialog) = dialog_ref.get() {
            if open && !dialog.open() {
                let _ = dialog.show_modal();
            } else if !open && dialog.open() {
                dialog.close();
            }
        }
    });
    view! {
        <dialog
            node_ref=dialog_ref
            class="modal"
            aria-labelledby=labelled_by
            on:cancel=move |ev: leptos::ev::Event| {
                ev.prevent_default();
                on_close.run(());
            }
            on:close=move |_: leptos::ev::Event| on_close.run(())
            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                // A modal dialog makes the page inert, but Tab past its last
                // control still reaches the browser's own UI: wrap instead.
                #[cfg(feature = "hydrate")]
                if ev.key() == "Tab" {
                    if let Some(dialog) = dialog_ref.get_untracked() {
                        if wrap_focus(&dialog, ev.shift_key()) {
                            ev.prevent_default();
                        }
                    }
                }
                #[cfg(not(feature = "hydrate"))]
                let _ = ev;
            }
        >
            <div class=format!("modal-box {class}")>
                <Show when=move || open.get()>{children()}</Show>
            </div>
            <form method="dialog" class="modal-backdrop">
                <button type="submit" tabindex="-1">"Close"</button>
            </form>
        </dialog>
    }
}

/// Asks before a destructive action, in place of the browser's `confirm()`:
/// same look as the rest of the portal, Cancel focused first, Escape cancels.
/// `id` must be unique on the page; it names the dialog after its heading.
#[component]
pub fn ConfirmDialog(
    #[prop(into)] open: Signal<bool>,
    on_cancel: Callback<()>,
    on_confirm: Callback<()>,
    #[prop(into)] id: String,
    #[prop(into)] title: String,
    #[prop(into)] message: String,
    confirm_label: &'static str,
) -> impl IntoView {
    let title_id = format!("{id}-title");
    let heading_id = title_id.clone();
    view! {
        <ModalDialog open=open on_close=on_cancel labelled_by=title_id>
            <h3 id=heading_id.clone() class="font-bold text-lg">{title.clone()}</h3>
            <p class="py-4 text-sm text-base-content/75">{message.clone()}</p>
            <div class="modal-action">
                <button type="button" class="btn btn-ghost" on:click=move |_| on_cancel.run(())>"Cancel"</button>
                <button type="button" class="btn btn-error" on:click=move |_| on_confirm.run(())>{confirm_label}</button>
            </div>
        </ModalDialog>
    }
}

/// Move focus from the last control to the first (or back from the first to
/// the last) inside `dialog`. Returns whether it moved focus.
#[cfg(feature = "hydrate")]
fn wrap_focus(dialog: &web_sys::HtmlDialogElement, backwards: bool) -> bool {
    let wrap = js_sys::Function::new_with_args(
        "dialog, backwards",
        r#"const items = [...dialog.querySelectorAll('a[href], button, input, select, textarea, [tabindex]')]
            .filter(el => el.tabIndex >= 0 && !el.disabled && el.getClientRects().length > 0);
        if (!items.length) return false;
        const first = items[0], last = items[items.length - 1], active = document.activeElement;
        const outside = !dialog.contains(active);
        if (backwards && (active === first || outside)) { last.focus(); return true; }
        if (!backwards && (active === last || outside)) { first.focus(); return true; }
        return false;"#,
    );
    wrap.call2(
        &wasm_bindgen::JsValue::NULL,
        dialog,
        &wasm_bindgen::JsValue::from_bool(backwards),
    )
    .ok()
    .and_then(|moved| moved.as_bool())
    .unwrap_or(false)
}
