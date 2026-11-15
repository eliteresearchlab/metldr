//! How every control takes a press: a pointer press, Enter or Space, a held key pressing once.

use rune::{El, Key, KeyEvent, Tx};
use std::rc::Rc;

/// Runs `run` when `el` is pressed or, while it has focus, on Enter or Space.
pub fn activate<S: 'static>(el: El, run: impl Fn(&mut S, &mut Tx) + 'static) -> El {
    let run = Rc::new(run);
    let key_run = Rc::clone(&run);
    el.on_press_tx(move |s: &mut S, tx| run(s, tx)).on_key(move |s: &mut S, key: &KeyEvent, tx: &mut Tx| {
        if key.key != Key::Enter && key.key != Key::Text(" ".into()) {
            return false;
        }
        if !tx.key_repeat() {
            key_run(s, tx);
        }
        true
    })
}
