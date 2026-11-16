//! A tooltip: a short text about its trigger.
//!
//! - It opens once the pointer rests on the trigger for 600 ms, and at once on keyboard focus.
//!   While another tooltip is open, or closed less than 400 ms ago, the next opens at once.
//! - It closes when the pointer leaves both the trigger and the tooltip, when focus leaves, on
//!   Escape, a press outside or on the trigger, and when the window loses focus.
//! - The pointer can move onto the tooltip without closing it, so its text can be read zoomed.
//! - It never takes focus. It names a trigger that has no name of its own, such as an icon
//!   button (`Type::Label`), and describes any other (`Type::Description`).
//! - Only one tooltip is open at a time; the app keeps which one, as one value.

use crate::popup::{self, Kind};
use rune::{Align, El, Side, Tx, time::Instant};
use std::{cell::Cell, time::Duration};

/// How long the pointer rests on a trigger before its tooltip opens.
pub const DELAY: Duration = Duration::from_millis(600);
/// How long after a tooltip closes the next one still opens at once.
pub const WARM_FOR: Duration = Duration::from_millis(400);
/// How long the pointer may take to cross from the trigger onto the tooltip.
const CROSS: Duration = Duration::from_millis(150);

thread_local! {
    /// Whether a tooltip is open, and when the last one closed.
    static WARM: Cell<(bool, Option<Instant>)> = const { Cell::new((false, None)) };
    /// The pointer is over an open tooltip, so leaving its trigger keeps it open.
    static OVER: Cell<bool> = const { Cell::new(false) };
}

/// Whether the next tooltip opens without its delay.
pub fn warm() -> bool {
    WARM.with(|w| {
        let (open, closed) = w.get();
        open || closed.is_some_and(|at| at.elapsed() < WARM_FOR)
    })
}

fn note(open: bool) {
    WARM.with(|w| w.set(if open { (true, w.get().1) } else { (false, Some(Instant::now())) }));
    if !open {
        OVER.with(|o| o.set(false));
    }
}

/// Whether the tooltip names its trigger or describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Label,
    Description,
}

/// Makes `el` the trigger of the tooltip `text`, open now when `open`. `on_open_change` opens
/// or closes it; the app keeps which tooltip is open.
pub fn trigger<S: 'static>(
    el: El,
    text: &str,
    kind: Type,
    open: bool,
    on_open_change: impl Fn(&mut S, bool, &mut Tx) + Clone + 'static,
) -> El {
    let el = match kind {
        Type::Label => el.label(text),
        Type::Description => el.description(text),
    };
    let hover = on_open_change.clone();
    el.hover_delays(if warm() { Duration::ZERO } else { DELAY }, CROSS)
        .on_hover_change(move |s: &mut S, inside, tx| {
            if inside != open && (inside || !OVER.with(Cell::get)) {
                note(inside);
                hover(s, inside, tx);
            }
        })
        .on_focus_change(move |s: &mut S, focused, keyboard, tx| {
            let to = focused && keyboard;
            if to != open {
                note(to);
                on_open_change(s, to, tx);
            }
        })
}

/// The tooltip `el` for the keyed `trigger`, on `side` of it. `on_close` closes it.
pub fn tooltip<S: 'static>(el: El, trigger: &str, side: Side, on_close: impl Fn(&mut S, &mut Tx) + 'static) -> El {
    let on_close = std::rc::Rc::new(on_close);
    let leave = std::rc::Rc::clone(&on_close);
    let el = el
        .role("tooltip")
        .a11y_hidden()
        .anchored(trigger, side, Align::Center, 6.0)
        .on_hover_change(move |s: &mut S, inside, tx| {
            OVER.with(|o| o.set(inside));
            if !inside {
                note(false);
                leave(s, tx);
            }
        });
    popup::layer(
        el,
        Kind::Tooltip,
        trigger,
        true,
        move |s: &mut S, _, tx| {
            note(false);
            on_close(s, tx);
        },
        |_: &mut S, _| {},
    )
}

#[cfg(test)]
mod tests {
    //! The tooltip's cases. Not ported: touch (Rune hovers only with a pointer), shortcut hints
    //! (no tooltip shows one in metldr yet), payloads shared by many triggers, and the
    //! announcement when a press opens a toggleable tooltip.
    use super::*;
    use rune::{Cx, Env, Key, KeyEvent, Mods, Runtime, col, row, text};

    #[derive(Default)]
    struct App {
        open: Option<&'static str>,
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let mut root = col().w_full().h_full().gap(40.0).padding(40.0);
        for key in ["save", "share"] {
            let trigger_el = row().key(key).size(30.0, 30.0).focusable(true).role("button").on_press(|_: &mut App| {});
            root = root.child(trigger(trigger_el, key, Type::Label, app.open == Some(key), move |a: &mut App, open, _| {
                a.open = open.then_some(key);
            }));
        }
        root = root.child(row().key("away").size(30.0, 30.0).absolute(250.0, 250.0).focusable(true).role("button").on_press(|_: &mut App| {}));
        if let Some(key) = app.open {
            root = root.child(tooltip(row().key("tip").padding(4.0).child(text(key)), key, Side::Top, |a: &mut App, _| a.open = None));
        }
        root
    }

    fn runtime() -> Runtime<App> {
        note(false);
        WARM.with(|w| w.set((false, None)));
        let mut rt = Runtime::new(App::default(), view, (300.0, 300.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt
    }

    fn center(rt: &Runtime<App>, key: &str) -> (f32, f32) {
        let e = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap();
        (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)
    }

    /// Moves the pointer onto `key`, then lets `seconds` pass.
    fn rest_on(rt: &mut Runtime<App>, key: &str, seconds: f64) {
        let (x, y) = center(rt, key);
        rt.pointer_move(x, y);
        rt.frame(0.0);
        rt.frame(seconds);
    }

    #[test]
    fn it_opens_after_the_pointer_rests_and_not_before() {
        let mut rt = runtime();
        rest_on(&mut rt, "save", 0.3);
        assert_eq!(rt.state.open, None, "a pass shorter than the delay opens nothing");
        rt.frame(0.4);
        assert_eq!(rt.state.open, Some("save"));
    }

    #[test]
    fn keyboard_focus_opens_it_at_once_and_a_click_does_not() {
        let mut rt = runtime();
        rt.key(KeyEvent { key: Key::Tab, mods: Mods::default() });
        rt.frame(0.0);
        rt.frame(0.0);
        assert_eq!(rt.state.open, Some("save"));
        rt.key(KeyEvent { key: Key::Escape, mods: Mods::default() });
        rt.frame(0.0);
        assert_eq!(rt.state.open, None, "Escape closes it");
        let (x, y) = center(&rt, "away");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        rt.frame(0.0);
        assert_eq!(rt.state.open, None, "focus by a click opens nothing");
    }

    #[test]
    fn a_press_on_the_trigger_closes_it() {
        let mut rt = runtime();
        rest_on(&mut rt, "save", 0.7);
        assert_eq!(rt.state.open, Some("save"));
        let (x, y) = center(&rt, "save");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert_eq!(rt.state.open, None);
    }

    #[test]
    fn the_next_tooltip_opens_at_once_while_one_is_warm() {
        let mut rt = runtime();
        rest_on(&mut rt, "save", 0.7);
        assert_eq!(rt.state.open, Some("save"));
        rest_on(&mut rt, "share", 0.05);
        assert_eq!(rt.state.open, Some("share"), "warm: no delay");
    }

    #[test]
    fn it_names_its_trigger() {
        let rt = runtime();
        let json: serde_json::Value = serde_json::from_str(&rt.a11y_json()).unwrap();
        assert!(json.as_array().unwrap().iter().any(|i| i["t"] == "save"), "{json}");
    }
}
