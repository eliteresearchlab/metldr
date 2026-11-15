//! Choosing one value: a radio group, a segmented control (a radio group laid out as one bar
//! with a sliding indicator), and a switch.
//!
//! - A radio group is one tab stop that Tab enters on the checked item. Arrows on both axes
//!   check the next item, skip disabled ones and wrap; the checked item is the only one pressed.
//! - A segmented control is a horizontal radio group unless it says otherwise; its indicator
//!   covers the checked item and glides to the next one (a short fade under Reduce Motion).
//! - A disabled group checks nothing and none of its items take focus.
//! - A switch is on or off: a press or Space flips it, and its label is part of it.
//! - Every change goes through the app, which may refuse it.

use rune::{El, FocusAxis, Key, KeyEvent, Tx};
use std::rc::Rc;

/// The options a radio group or segmented control takes each render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupProps {
    pub disabled: bool,
    pub axis: FocusAxis,
    pub wrap: bool,
}

impl Default for GroupProps {
    fn default() -> Self {
        Self { disabled: false, axis: FocusAxis::Both, wrap: true }
    }
}

/// Makes `el` a radio group named `label`. Put its items inside, each made with [`radio`].
pub fn radio_group(el: El, label: impl Into<String>, props: GroupProps) -> El {
    el.role("radiogroup").label(label).focus_group().focus_axis(props.axis).focus_wrap(props.wrap)
}

/// Makes `el` one item of a radio group, `checked` now. Checking it, by a press or by arrows
/// landing on it, asks `on_check`; the app applies the value, or refuses by doing nothing.
pub fn radio<S: 'static>(
    el: El,
    disabled: bool,
    checked: bool,
    on_check: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let el = el.role("radio").focus_selected(checked);
    if disabled {
        return el.disabled(true);
    }
    el.focusable(true).on_press_tx(move |s: &mut S, tx| {
        if !checked {
            on_check(s, tx);
        }
    })
}

/// The indicator of the segmented control keyed `group`: put it inside the checked item, under
/// its label, and it glides there from the item checked before.
pub fn indicator(group: &str) -> El {
    let id = format!("{group}-indicator");
    rune::el().key(id.clone()).matched(id).a11y_hidden()
}

/// Makes `el` a switch that is `on` now. A press or Space asks `on_change` for the other state.
pub fn switch<S: 'static>(
    el: El,
    disabled: bool,
    on: bool,
    on_change: impl Fn(&mut S, bool, &mut Tx) + 'static,
) -> El {
    let el = el.role("switch").checked(on).focusable(true).unavailable(disabled);
    let change = Rc::new(move |s: &mut S, tx: &mut Tx| {
        if !disabled {
            on_change(s, !on, tx);
        }
    });
    let key_change = Rc::clone(&change);
    el.on_press_tx(move |s: &mut S, tx| change(s, tx)).on_key(move |s: &mut S, key: &KeyEvent, tx: &mut Tx| {
        if key.key != Key::Text(" ".into()) {
            return false;
        }
        if !tx.key_repeat() {
            key_change(s, tx);
        }
        true
    })
}

#[cfg(test)]
mod tests {
    //! The segmented control and switch specs' cases. Not ported: form submit and reset (the
    //! canvas has no forms), fractional and scaled indicator geometry (Rune lays the indicator
    //! inside the item, so it matches by construction), and the input-source reports.
    use super::*;
    use rune::{Cx, Env, Mods, Runtime, col, row, text};
    use serde_json::Value;

    const SIZES: &[&str] = &["small", "medium", "large"];

    struct App {
        size: &'static str,
        disabled_item: Option<&'static str>,
        disabled: bool,
        refuse: bool,
        sound: bool,
        why: Vec<&'static str>,
    }

    impl Default for App {
        fn default() -> Self {
            Self { size: "medium", disabled_item: None, disabled: false, refuse: false, sound: false, why: Vec::new() }
        }
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let props = GroupProps { disabled: app.disabled, ..GroupProps::default() };
        let items = SIZES.iter().map(|&size| {
            let checked = app.size == size;
            let mut item = row().key(size).size(60.0, 24.0);
            if checked {
                item = item.child(indicator("size").size(60.0, 24.0).absolute(0.0, 0.0));
            }
            radio(item.child(text(size)), app.disabled || app.disabled_item == Some(size), checked, move |a: &mut App, _| {
                if !a.refuse {
                    a.size = size;
                    a.why.push("press");
                }
            })
        });
        col()
            .child(row().key("before").size(60.0, 24.0).focusable(true).on_press(|_: &mut App| {}))
            .child(radio_group(row().key("sizes"), "Text size", props).children(items))
            .child(switch(row().key("sound").size(120.0, 24.0).child(text("Sound")), false, app.sound, |a: &mut App, on, _| {
                if !a.refuse {
                    a.sound = on;
                }
            }))
            .child(row().key("after").size(60.0, 24.0).focusable(true).on_press(|_: &mut App| {}))
    }

    fn rig(app: App) -> Runtime<App> {
        let mut rt = Runtime::new(app, view, (400.0, 300.0), Env::default());
        rt.frame(0.0);
        rt
    }

    fn click(rt: &mut Runtime<App>, key: &str) {
        let r = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap().rect;
        rt.pointer_down(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_up(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.frame(0.0);
    }

    fn press(rt: &mut Runtime<App>, key: Key) {
        rt.key(KeyEvent { key, mods: Mods::default() });
        rt.frame(0.0);
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    fn tab_to(rt: &mut Runtime<App>, key: &str) {
        for _ in 0..8 {
            if focused(rt).as_deref() == Some(key) {
                return;
            }
            press(rt, Key::Tab);
        }
        panic!("Tab never reached {key}");
    }

    fn mirror(rt: &Runtime<App>, name: &str) -> Value {
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        items.into_iter().find(|i| i["t"] == name).unwrap_or(Value::Null)
    }

    #[test]
    fn the_default_item_is_checked_and_only_that_one() {
        let rt = rig(App::default());
        assert_eq!((mirror(&rt, "medium")["role"].as_str(), mirror(&rt, "medium")["c"].as_str()), (Some("radio"), Some("true")));
        assert_eq!(mirror(&rt, "small")["c"], "false");
        assert_eq!(mirror(&rt, "large")["c"], "false");
    }

    #[test]
    fn clicking_an_item_checks_it() {
        let mut rt = rig(App::default());
        click(&mut rt, "large");
        assert_eq!(rt.state.size, "large");
        assert_eq!(rt.state.why, ["press"]);
    }

    #[test]
    fn clicking_a_disabled_item_does_nothing() {
        let mut rt = rig(App { disabled_item: Some("large"), ..App::default() });
        click(&mut rt, "large");
        assert_eq!(rt.state.size, "medium");
    }

    #[test]
    fn a_change_can_be_refused() {
        let mut rt = rig(App { refuse: true, ..App::default() });
        click(&mut rt, "small");
        assert_eq!(rt.state.size, "medium");
    }

    #[test]
    fn tab_lands_on_the_checked_item_and_the_next_tab_leaves() {
        let mut rt = rig(App::default());
        tab_to(&mut rt, "before");
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("medium"));
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("sound"));
    }

    #[test]
    fn arrow_keys_on_both_axes_check_the_next_item_skip_disabled_ones_and_wrap() {
        let mut rt = rig(App { disabled_item: Some("large"), ..App::default() });
        tab_to(&mut rt, "medium");
        press(&mut rt, Key::Right);
        assert_eq!((focused(&rt).as_deref(), rt.state.size), (Some("small"), "small"), "skips the disabled item and wraps");
        press(&mut rt, Key::Down);
        assert_eq!(rt.state.size, "medium");
        press(&mut rt, Key::Up);
        press(&mut rt, Key::Left);
        assert_eq!(rt.state.size, "medium");
    }

    #[test]
    fn the_indicator_covers_the_checked_item_and_follows_it() {
        let mut rt = rig(App::default());
        let rect = |rt: &Runtime<App>, key: &str| rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap().target;
        let under = |rt: &Runtime<App>| rt.elements().into_iter().find(|e| e.key.as_deref() == Some("size-indicator")).map(|e| e.target);
        assert_eq!(under(&rt), Some(rect(&rt, "medium")));
        click(&mut rt, "large");
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(under(&rt), Some(rect(&rt, "large")));
    }

    #[test]
    fn a_disabled_group_checks_nothing_and_its_items_take_no_focus() {
        let mut rt = rig(App { disabled: true, ..App::default() });
        click(&mut rt, "small");
        assert_eq!(rt.state.size, "medium");
        tab_to(&mut rt, "before");
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("sound"));
    }

    #[test]
    fn a_switch_starts_off_and_a_click_or_space_flips_it() {
        let mut rt = rig(App::default());
        assert_eq!((mirror(&rt, "Sound")["role"].as_str(), mirror(&rt, "Sound")["c"].as_str()), (Some("switch"), Some("false")));
        click(&mut rt, "sound");
        assert!(rt.state.sound);
        assert_eq!(mirror(&rt, "Sound")["c"], "true");
        tab_to(&mut rt, "sound");
        press(&mut rt, Key::Text(" ".into()));
        assert!(!rt.state.sound);
    }
}
