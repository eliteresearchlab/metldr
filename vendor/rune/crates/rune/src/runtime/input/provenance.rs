//! Admission of web host input into the runtime.
//!
//! The web host mirrors controls as assistive-technology nodes. A press on a mirrored node is
//! applied to the runtime node at its path, so the press must come from the browser and must name
//! the mirror the page holds. A synthetic click dispatched by page script, or a press made against
//! a mirror the runtime has since replaced, is refused before any handler runs.

/// Whether a message from the web host may reach the runtime. `trusted` is the browser's
/// `isTrusted` for the event that produced the message. `revision` is the mirror revision the page
/// rendered, and `current` is the revision the runtime holds now.
///
/// Other web input is not checked here. `platform/web/bridge.js` stops untrusted input before any
/// listener sees it.
pub(crate) fn admits(kind: &str, trusted: bool, revision: Option<u64>, current: u64) -> bool {
    match kind {
        "a11y_focus" | "a11y_press" | "a11y_link" => trusted && revision == Some(current),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::admits;

    #[test]
    fn mirrored_controls_refuse_synthetic_input() {
        assert!(admits("a11y_press", true, Some(4), 4));
        assert!(!admits("a11y_press", false, Some(4), 4));
        assert!(!admits("a11y_link", false, Some(4), 4));
    }

    #[test]
    fn mirrored_controls_refuse_presses_from_a_replaced_mirror() {
        assert!(!admits("a11y_press", true, Some(3), 4));
        assert!(!admits("a11y_press", true, None, 4));
    }
}
