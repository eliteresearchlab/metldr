//! Fonts added while the app runs: fallbacks a web app fetches after its first frame, so a
//! cold start loads and parses only the faces its interface draws with.

use std::cell::RefCell;

use super::{Blob, FamilyId, TextSystem, measure, system_fonts};
use std::sync::Arc;

thread_local! {
    /// Every font added on this thread, in order. Each text system registers the ones past its
    /// own count, so windows, overlays and the measurer each follow once.
    static ADDED: RefCell<Vec<&'static [u8]>> = const { RefCell::new(Vec::new()) };
}

/// Registers `fonts` as fallbacks for every window, overlay and [`measure`](super::measure) on
/// this thread, as if the app had passed them in `AppSpec::fonts` after its sans and mono:
/// supported scripts and CJK draw with them instead of missing glyphs. Each window lays out
/// again on its next frame; wake the app (`AppHandle::wake`) when the call comes from outside
/// a handler. [`font_epoch`](super::font_epoch) changes, so caches of measured sizes follow.
pub fn add_fonts(fonts: &[&'static [u8]]) {
    if fonts.is_empty() {
        return;
    }
    ADDED.with_borrow_mut(|added| added.extend_from_slice(fonts));
    measure::note_added();
}

thread_local! {
    /// Whether text laid out on this thread held CJK or emoji, so an app that adds those fonts
    /// on demand knows when to.
    static CJK_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static EMOJI_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SCRIPTS_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether any text laid out on this thread held Han, kana, Hangul or Bopomofo. An app that
/// ships a large CJK font adds it (`add_fonts`) once this turns true instead of at start.
pub fn cjk_seen() -> bool {
    CJK_SEEN.get()
}

/// Whether any text laid out on this thread held an emoji: a pictograph (U+1F000 to U+1FAFF),
/// the emoji presentation selector, a star or a circle. Symbols such as a check mark or a plain
/// star stay text and do not count. An app that ships a colour emoji font adds it once this
/// turns true.
pub fn emoji_seen() -> bool {
    EMOJI_SEEN.get()
}

/// Whether any text laid out on this thread held Hebrew, Arabic, Devanagari, Bengali or Thai,
/// or a symbol, arrow, box drawing or math character an interface face seldom carries. An app
/// that ships fonts for these adds them once this turns true.
pub fn scripts_seen() -> bool {
    SCRIPTS_SEEN.get()
}

/// Notes `text` for [`cjk_seen`], [`emoji_seen`] and [`scripts_seen`]. ASCII text, and every
/// call once all three were seen, costs one check.
pub(super) fn note_text(text: &str) {
    if text.is_ascii() || (CJK_SEEN.get() && EMOJI_SEEN.get() && SCRIPTS_SEEN.get()) {
        return;
    }
    let cjk = |c: u32| matches!(c,
        0x1100..=0x11FF | 0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3040..=0x30FF | 0x3100..=0x312F
        | 0x3130..=0x318F | 0x31A0..=0x31FF | 0x3200..=0x4DBF | 0x4E00..=0x9FFF | 0xA960..=0xA97F
        | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF);
    let emoji = |c: u32| matches!(c, 0x1F000..=0x1FAFF | 0xFE0F | 0x2B50 | 0x2B55);
    let script = |c: u32| matches!(c,
        0x0590..=0x08FF | 0x0900..=0x09FF | 0x0E00..=0x0E7F | 0xA8E0..=0xA8FF | 0xFB1D..=0xFDFF
        | 0xFE70..=0xFEFF | 0x2100..=0x214F | 0x2190..=0x2BFF | 0x1D400..=0x1D7FF);
    for c in text.chars().map(u32::from) {
        if cjk(c) {
            CJK_SEEN.set(true);
        } else if emoji(c) {
            EMOJI_SEEN.set(true);
        } else if script(c) {
            SCRIPTS_SEEN.set(true);
        }
    }
}

impl TextSystem {
    /// Whether fonts were added since this text system last registered them.
    pub(crate) fn fonts_behind(&self) -> bool {
        ADDED.with_borrow(|added| added.len() != self.added)
    }

    /// Registers the fonts added since it last looked as fallbacks and drops what it shaped
    /// without them. True when there were any.
    pub(crate) fn follow_added(&mut self) -> bool {
        let fresh: Vec<&'static [u8]> =
            ADDED.with_borrow(|added| added[self.added.min(added.len())..].to_vec());
        if fresh.is_empty() {
            return false;
        }
        self.added += fresh.len();
        let collection = &mut self.font_cx.collection;
        let primary = [collection.family_id(&self.families.0), collection.family_id(&self.families.1)];
        let mut fallbacks: Vec<FamilyId> = Vec::new();
        for data in fresh {
            let registered = collection.register_fonts(Blob::new(Arc::new(data)), None);
            let mut ids: Vec<FamilyId> = registered.into_iter().map(|(id, _)| id).collect();
            ids.sort_unstable();
            for id in ids {
                if !primary.contains(&Some(id)) && !fallbacks.contains(&id) {
                    fallbacks.push(id);
                }
            }
        }
        system_fonts::append_supplied(collection, &fallbacks);
        self.drop_shaped();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Family, TextStyle};

    const INTER: &[u8] = include_bytes!("../../assets/Inter-Regular.ttf");
    const PLEX: &[u8] = include_bytes!("../../assets/IBMPlexMono-Regular.ttf");

    /// CJK text marks the thread; ASCII and other scripts never do.
    #[test]
    fn cjk_text_is_noted_once_laid_out() {
        let mut ts = TextSystem::new(&[INTER], ("Inter", "Inter"));
        let style = TextStyle::new(13.0, 400.0);
        ts.layout_rich("plain text, العربية, ★", &[], style, 1.0, None);
        assert!(!cjk_seen());
        ts.layout_rich("日本語", &[], style, 1.0, None);
        assert!(cjk_seen());
    }

    /// Emoji text marks the thread; check marks, plain stars and other symbols never do.
    #[test]
    fn emoji_text_is_noted_once_laid_out() {
        let mut ts = TextSystem::new(&[INTER], ("Inter", "Inter"));
        let style = TextStyle::new(13.0, 400.0);
        ts.layout_rich("done ✓ ★ → €", &[], style, 1.0, None);
        assert!(!emoji_seen());
        ts.layout_rich("ship it 🚀", &[], style, 1.0, None);
        assert!(emoji_seen());
    }

    /// Hebrew, Arabic, Indic, Thai, symbol and math text marks the thread; extended Latin,
    /// Greek and Cyrillic, which the interface fallbacks cover, never do.
    #[test]
    fn other_scripts_are_noted_once_laid_out() {
        let mut ts = TextSystem::new(&[INTER], ("Inter", "Inter"));
        let style = TextStyle::new(13.0, 400.0);
        ts.layout_rich("café, Ελληνικά, Русский • …", &[], style, 1.0, None);
        assert!(!scripts_seen());
        ts.layout_rich("שלום", &[], style, 1.0, None);
        assert!(scripts_seen());
    }

    /// A running text system and one built later both draw with an added font, and the
    /// measurer's epoch moves so cached sizes follow.
    #[test]
    fn added_fonts_reach_running_and_new_text_systems() {
        let style = TextStyle::new(13.0, 400.0).family(Family::Named("IBM Plex Mono"));
        let width = |ts: &mut TextSystem| ts.layout_rich("iiii", &[], style, 2.0, None).full_width();
        let mut running = TextSystem::new(&[INTER], ("Inter", "Inter"));
        let before = width(&mut running);
        let epoch = crate::text::font_epoch();
        add_fonts(&[PLEX]);
        assert_ne!(crate::text::font_epoch(), epoch);
        assert!(running.fonts_behind());
        assert!(running.follow_added());
        assert!(!running.follow_added(), "a font registers once");
        let after = width(&mut running);
        assert!(after > before * 1.5, "monospace i is wider than Inter's: {before} to {after}");
        let mut later = TextSystem::new(&[INTER], ("Inter", "Inter"));
        assert!(!later.fonts_behind());
        assert_eq!(width(&mut later), after);
    }
}
