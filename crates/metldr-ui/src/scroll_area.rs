//! Scroll area: a scroll view the keyboard can reach. While its content overflows and nothing
//! inside it can take focus, it is a Tab stop named `label`, and the arrow, page, Home and End
//! keys scroll it; it reads as a region either way. A view that fits, or one holding a focusable
//! control, is passed over, as that control is the way in.

use rune::{El, ScrollSpec};

/// Makes `el` a scroll area named `label` that follows its end when `stick_to_end`.
pub fn viewport(el: El, label: &str, stick_to_end: bool) -> El {
    el.scroll(ScrollSpec { stick_bottom: stick_to_end }).scroll_stop(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Cx, Env, Key, KeyEvent, Mods, Runtime, col, el, text};

    fn view(state: &(u32, bool), _: &mut Cx<(u32, bool)>) -> El {
        let mut body = col();
        for i in 0..state.0 {
            body = body.child(text(format!("Message {i}")).h(30.0));
        }
        if state.1 {
            body = body.child(el().key("copy").size(20.0, 20.0).focusable(true).on_press(|_: &mut (u32, bool)| {}));
        }
        col().child(el().key("before").size(40.0, 20.0).focusable(true).on_press(|_: &mut (u32, bool)| {}))
            .child(viewport(col().key("thread").w_full().h(100.0), "Messages", false).child(body))
    }

    fn focused(rt: &Runtime<(u32, bool)>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    fn tab(rt: &mut Runtime<(u32, bool)>) {
        rt.key(KeyEvent { key: Key::Tab, mods: Mods::default() });
        rt.frame(0.016);
    }

    #[test]
    fn an_overflowing_area_without_controls_is_a_tab_stop_the_arrows_scroll() {
        let mut rt = Runtime::new((20, false), view, (200.0, 300.0), Env::new(false, 1.0));
        rt.frame(0.0);
        rt.frame(0.016);
        tab(&mut rt);
        assert_eq!(focused(&rt).as_deref(), Some("before"));
        tab(&mut rt);
        assert_eq!(focused(&rt).as_deref(), Some("thread"), "overflowing with nothing focusable inside");
        let top = |rt: &Runtime<(u32, bool)>| rt.elements().into_iter().find(|e| e.text.as_deref() == Some("Message 0")).unwrap().rect[1];
        let before = top(&rt);
        rt.key(KeyEvent { key: Key::Down, mods: Mods::default() });
        rt.frame(0.016);
        rt.frame(1.0);
        assert!((before - top(&rt) - 40.0).abs() < 0.5, "Down scrolls a 40 point line: {before} to {}", top(&rt));
    }

    #[test]
    fn a_fitting_area_or_one_with_a_control_is_passed_over() {
        for state in [(2, false), (20, true)] {
            let mut rt = Runtime::new(state, view, (200.0, 300.0), Env::new(false, 1.0));
            rt.frame(0.0);
            rt.frame(0.016);
            tab(&mut rt);
            tab(&mut rt);
            assert_ne!(focused(&rt).as_deref(), Some("thread"), "{state:?}");
        }
    }
}
