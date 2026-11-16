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

impl TextSystem {
    /// Whether fonts were added since this text system last registered them.
    pub(crate) fn fonts_behind(&self) -> bool {
        ADDED.with_borrow(|added| added.len() != self.added)
    }

    /// Registers the fonts added since it last looked as fallbacks and drops what it shaped
    /// without them. True when there were any.
    pub(crate) fn follow_added(&mut self) -> bool {
        let fresh: Vec<&'static [u8]> = ADDED.with_borrow(|added| added.get(self.added..).unwrap_or_default().to_vec());
        if fresh.is_empty() {
            return false;
        }
        self.added += fresh.len();
        let collection = &mut self.font_cx.collection;
        let primary = [collection.family_id(&self.families.0), collection.family_id(&self.families.1)];
        let mut fallbacks: Vec<FamilyId> = Vec::new();
        for data in fresh {
            let mut ids: Vec<FamilyId> = collection.register_fonts(Blob::new(Arc::new(data)), None).into_iter().map(|(id, _)| id).collect();
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
