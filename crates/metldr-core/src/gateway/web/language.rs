//! Language detection: py3langid in compact form (99.1% on sentences, 93.6% on four words in
//! the papluca test set), loaded through the model store the first time it is needed. Until it is
//! ready, Chrome's built-in detector answers when its model is already on the device.

use super::*;
use std::cell::Cell;

thread_local! {
    static DETECTOR: RefCell<Option<Rc<py3langid::Identifier>>> = const { RefCell::new(None) };
    static LOADING: Cell<bool> = const { Cell::new(false) };
}

/// The language of `text` as an ISO 639-1 code, when the detector is ready; the first call starts
/// loading it in the background.
pub(super) fn identify(text: &str) -> Option<String> {
    let detector = DETECTOR.with_borrow(Clone::clone);
    let Some(detector) = detector else {
        if !LOADING.replace(true) {
            spawn_local(async {
                let bytes = model::fetch_pinned(&model::LANGUAGES).await;
                let detector = bytes.and_then(|b| {
                    py3langid::Identifier::from_bytes(&b, py3langid::Options::default()).ok()
                });
                DETECTOR.with_borrow_mut(|slot| *slot = detector.map(Rc::new));
                LOADING.set(false);
            });
        }
        return None;
    };
    Some(detector.identify(text).language)
}
