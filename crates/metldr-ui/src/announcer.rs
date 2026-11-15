//! One polite live region for the whole page: what a control says to screen readers when nothing
//! on screen changes, such as a press still loading after a second.

use rune::{El, LiveRegion, oklcha, text};

/// The page's announcer. Keep one in every view and pass it the newest message, or `None`.
/// Its ink is transparent, so it draws nothing visible; screen readers read each new message
/// once.
pub fn announcer(message: Option<&str>) -> El {
    text(message.unwrap_or_default())
        .key("ui-announcer")
        .role("status")
        .live_region(LiveRegion::Polite)
        .size(1.0, 1.0)
        .color(oklcha(0.0, 0.0, 0.0, 0.0))
}
