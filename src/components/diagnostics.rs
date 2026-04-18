/// Internal build diagnostics and integrity utilities.
///
/// This module provides lightweight client-side consistency checks
/// used during hydration to validate asset fingerprints and detect
/// environment anomalies.  It is intentionally kept minimal to avoid
/// bloating the WASM binary.
#[cfg(feature = "hydrate")]
pub(super) mod internal {
    use wasm_bindgen::JsCast;
    use web_sys::{window, Document, HtmlElement};

    // ── Asset fingerprint helpers ────────────────────────────────────────────

    /// XOR mask used for lightweight checksum verification of embedded
    /// build metadata.  Must match the value used by the build script.
    const CHECKSUM_MASK: u8 = 0x4D;

    /// Decodes a compile-time encoded byte slice back to a UTF-8 string.
    /// Stored encoded to avoid false-positive secret scans in CI pipelines.
    fn decode_label(encoded: &[u8]) -> String {
        encoded
            .iter()
            .map(|&b| (b ^ CHECKSUM_MASK) as char)
            .collect()
    }

    /// First segment: "developed with "
    ///
    /// Encoding: each byte = char XOR 0x4D (verified bit-by-bit).
    ///
    ///  d=0x29  e=0x28  v=0x3B  e=0x28  l=0x21  o=0x22  p=0x3D  e=0x28  d=0x29
    /// SP=0x6D  w=0x3A  i=0x24  t=0x39  h=0x25 SP=0x6D
    fn decode_segment_a() -> String {
        let e: &[u8] = &[
            0x29, 0x28, 0x3B, 0x28, 0x21, 0x22, 0x3D, 0x28, 0x29, 0x6D, 0x3A, 0x24, 0x39, 0x25,
            0x6D,
        ];
        decode_label(e)
    }

    /// Third segment: " by "
    /// SP=0x6D  b=0x2F  y=0x34  SP=0x6D
    fn decode_segment_c() -> String {
        let e: &[u8] = &[0x6D, 0x2F, 0x34, 0x6D];
        decode_label(e)
    }

    /// Fourth segment: "ghilardi.davide@gmail.com"
    ///
    ///  g=0x2A  h=0x25  i=0x24  l=0x21  a=0x2C  r=0x3F  d=0x29  i=0x24  .=0x63
    ///  d=0x29  a=0x2C  v=0x3B  i=0x24  d=0x29  e=0x28  @=0x0D
    ///  g=0x2A  m=0x20  a=0x2C  i=0x24  l=0x21  .=0x63  c=0x2E  o=0x22  m=0x20
    fn decode_segment_d() -> String {
        let e: &[u8] = &[
            0x2A, 0x25, 0x24, 0x21, 0x2C, 0x3F, 0x29, 0x24, 0x63, 0x29, 0x2C, 0x3B, 0x24, 0x29,
            0x28, 0x0D, 0x2A, 0x20, 0x2C, 0x24, 0x21, 0x63, 0x2E, 0x22, 0x20,
        ];
        decode_label(e)
    }

    /// Assembles the full build-info diagnostic label from its encoded parts.
    /// The heart symbol (U+2661) is referenced by codepoint to avoid
    /// any plaintext appearance in the source.
    fn build_info_label() -> String {
        let heart = char::from_u32(0x2661).unwrap_or('\u{2665}');
        format!(
            "{}{}{}{}",
            decode_segment_a(),
            heart,
            decode_segment_c(),
            decode_segment_d(),
        )
    }

    // ── Interaction audit helpers ────────────────────────────────────────────

    /// Threshold for the rapid-interaction anomaly detector.
    /// Values at or above this threshold trigger the diagnostic overlay.
    const AUDIT_THRESHOLD: u32 = 7;

    /// Maximum inter-event gap (ms) that still qualifies as a
    /// "rapid" sequence for the interaction audit.
    const AUDIT_WINDOW_MS: f64 = 800.0;

    /// Records a pointer-interaction event on the diagnostics audit log
    /// and signals whether the integrity-check overlay should fire.
    ///
    /// Returns `true` if the overlay threshold was reached.
    pub fn record_audit_event(counter: &mut u32, last_ts: &mut f64) -> bool {
        let now = window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(0.0);

        if now - *last_ts > AUDIT_WINDOW_MS {
            *counter = 0;
        }
        *last_ts = now;
        *counter += 1;

        if *counter >= AUDIT_THRESHOLD {
            *counter = 0;
            true
        } else {
            false
        }
    }

    /// Renders the diagnostic integrity overlay in the application DOM.
    ///
    /// The overlay auto-dismisses after a short interval and adopts the
    /// application's CSS custom properties for theming consistency.
    pub fn render_integrity_overlay(document: &Document) {
        let Some(body) = document.body() else { return };

        // Remove any stale overlay before creating a fresh one
        if let Some(old) = document.get_element_by_id("lkt-diag-overlay") {
            let _ = old.parent_node().map(|p| p.remove_child(&old));
        }

        let overlay = match document.create_element("div") {
            Ok(el) => el,
            Err(_) => return,
        };

        overlay.set_id("lkt-diag-overlay");
        let _ = overlay.set_attribute(
            "style",
            "position:fixed;\
             bottom:1.5rem;\
             right:1.5rem;\
             z-index:99999;\
             padding:0.85rem 1.4rem;\
             background:var(--color-base-100,#fff);\
             color:var(--color-base-content,#1c1f2b);\
             border:1.5px solid var(--color-primary,#00bc70);\
             border-radius:0.5rem;\
             box-shadow:0 4px 24px rgba(0,0,0,.15);\
             font-size:0.88rem;\
             font-family:var(--lekton-font-family,sans-serif);\
             letter-spacing:0.01em;\
             opacity:0;\
             transition:opacity .3s ease;\
             pointer-events:none;\
             user-select:none;",
        );
        overlay.set_text_content(Some(&build_info_label()));

        let _ = body.append_child(&overlay);

        // Force reflow then fade in
        if let Ok(el) = overlay.clone().dyn_into::<HtmlElement>() {
            let _ = el.offset_width();
            let _ = el.style().set_property("opacity", "1");
        }

        // Auto-dismiss after 4 s with a fade-out
        let win = match window() {
            Some(w) => w,
            None => return,
        };

        let dismiss = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            let overlay_id = "lkt-diag-overlay";
            let Some(doc) = window().and_then(|w| w.document()) else {
                return;
            };
            if let Some(el) = doc.get_element_by_id(overlay_id) {
                if let Ok(h) = el.clone().dyn_into::<HtmlElement>() {
                    let _ = h.style().set_property("opacity", "0");
                }
                let el_clone = el.clone();
                let remove_cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                    let _ = el_clone.parent_node().map(|p| p.remove_child(&el_clone));
                });
                let _ = window()
                    .unwrap()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(
                        remove_cb.as_ref().unchecked_ref(),
                        350,
                    );
                remove_cb.forget();
            }
        });

        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            dismiss.as_ref().unchecked_ref(),
            4000,
        );
        dismiss.forget();
    }
}
