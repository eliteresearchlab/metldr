//! `El::layer`: a non-modal layer hears Escape, a press outside, focus leaving and the window
//! losing focus, with the reason; only the topmost one hears them, and a modal above takes them.
use super::*;
use crate::{Align, Cx, Dismiss, El, Env, Key, KeyEvent, Mods, Side, col, el};

#[derive(Default)]
struct Pop {
    open: bool,
    second: bool,
    modal: bool,
    heard: Vec<(&'static str, Dismiss)>,
}

fn button(key: &str) -> El {
    el().key(key).size(40.0, 20.0).focusable(true).role("button")
}

fn view(s: &Pop, _: &mut Cx<Pop>) -> El {
    let mut root = col().key("root").w_full().h_full()
        .child(button("trigger").on_press(|s: &mut Pop| s.open = !s.open))
        .child(button("outside").absolute(300.0, 300.0));
    if s.open {
        root = root.child(col().key("pop").size(80.0, 60.0).anchored("trigger", Side::Bottom, Align::Start, 4.0)
            .layer(|s: &mut Pop, why, _| s.heard.push(("pop", why)))
            .child(button("inner").on_press(|s: &mut Pop| s.second = true)));
    }
    if s.second {
        root = root.child(col().key("pop2").size(60.0, 40.0).anchored("inner", Side::Right, Align::Start, 4.0)
            .layer(|s: &mut Pop, why, _| s.heard.push(("pop2", why)))
            .child(button("deep")));
    }
    if s.modal {
        root = root.child(col().key("modal").absolute(200.0, 200.0).size(80.0, 80.0)
            .modal(true, |s: &mut Pop| s.modal = false).child(button("in-modal")));
    }
    root
}

fn runtime(open: bool) -> Runtime<Pop> {
    let mut rt = Runtime::new(Pop { open, ..Pop::default() }, view, (400.0, 400.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt
}

fn center(rt: &Runtime<Pop>, key: &str) -> (f32, f32) {
    let e = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap();
    (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)
}

fn press(rt: &mut Runtime<Pop>, key: &str) {
    let (x, y) = center(rt, key);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn escape(rt: &mut Runtime<Pop>) {
    rt.key(KeyEvent { key: Key::Escape, mods: Mods::default() });
    rt.frame(0.0);
}

fn heard(rt: &Runtime<Pop>) -> Vec<(&'static str, Dismiss)> {
    rt.state.heard.clone()
}

#[test]
fn escape_reaches_the_open_layer() {
    let mut rt = runtime(true);
    escape(&mut rt);
    assert_eq!(heard(&rt), [("pop", Dismiss::Escape)]);
}

#[test]
fn a_press_outside_dismisses_and_a_press_on_the_trigger_or_inside_does_not() {
    let mut rt = runtime(true);
    press(&mut rt, "inner");
    rt.update(|s, _| s.second = false);
    rt.frame(0.0);
    assert!(heard(&rt).is_empty(), "inside: {:?}", heard(&rt));
    press(&mut rt, "trigger");
    assert_eq!(heard(&rt), [("pop", Dismiss::AnchorPress)], "a press on the trigger says so");
    rt.update(|s, _| s.heard.clear());
    assert!(!rt.state.open, "the trigger toggled it closed");
    rt.update(|s, _| s.open = true);
    rt.frame(0.0);
    press(&mut rt, "outside");
    assert_eq!(heard(&rt), [("pop", Dismiss::OutsidePress)]);
}

#[test]
fn focus_moving_out_dismisses_and_focus_moving_in_does_not() {
    let mut rt = runtime(true);
    press(&mut rt, "inner");
    rt.update(|s, _| s.second = false);
    rt.frame(0.0);
    assert!(heard(&rt).is_empty());
    rt.update(|_, tx| tx.focus("outside"));
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(heard(&rt).contains(&("pop", Dismiss::FocusOut)), "{:?}", heard(&rt));
}

#[test]
fn the_window_losing_focus_reaches_the_layer() {
    let mut rt = runtime(true);
    rt.window_focus(false);
    rt.frame(0.0);
    assert_eq!(heard(&rt), [("pop", Dismiss::WindowBlur)]);
}

#[test]
fn only_the_topmost_layer_hears_and_its_portal_chain_counts_as_inside() {
    let mut rt = runtime(true);
    press(&mut rt, "inner");
    assert!(rt.state.second);
    press(&mut rt, "deep");
    assert!(heard(&rt).is_empty(), "a press in the nested layer is inside both: {:?}", heard(&rt));
    escape(&mut rt);
    assert_eq!(heard(&rt), [("pop2", Dismiss::Escape)]);
}

#[test]
fn a_modal_above_takes_escape_and_the_layer_beneath_hears_nothing() {
    let mut rt = runtime(true);
    rt.update(|s, _| s.modal = true);
    rt.frame(0.0);
    escape(&mut rt);
    assert!(!rt.state.modal, "the modal closed");
    assert!(heard(&rt).is_empty(), "{:?}", heard(&rt));
}

#[test]
fn a_closed_layer_hears_nothing() {
    let mut rt = runtime(false);
    escape(&mut rt);
    press(&mut rt, "outside");
    rt.window_focus(false);
    rt.frame(0.0);
    assert!(heard(&rt).is_empty());
}
