//! A toggle and a toggle group.
//!
//! - A toggle is a button that stays pressed or not, and says so; its name does not change.
//! - A group holds several toggles as one tab stop: arrows move focus without pressing, Home
//!   and End jump to the ends, focus wraps unless `wrap` is off, and the arrows of the other
//!   axis go on to the app. Tab enters on the pressed toggle, else the first.
//! - One toggle pressed at a time, or several with `multiple`; pressing the pressed one
//!   unpresses it unless `deselectable` is off. Values are reported in the order the toggles
//!   appear.
//! - A disabled toggle stays reachable by arrows and says it is unavailable; a disabled group
//!   keeps its tab stop and presses nothing.
//! - Every change goes through the app, which may refuse it.

use rune::{El, FocusAxis, Key, KeyEvent, Tx};
use std::rc::Rc;

/// The options a toggle takes each render.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Props {
    pub disabled: bool,
}

/// Makes `el` a toggle that is `pressed` now. A press asks `on_pressed_change` for the other
/// state; the app applies it, or refuses by doing nothing.
pub fn toggle<S: 'static>(
    el: El,
    props: Props,
    pressed: bool,
    on_pressed_change: impl Fn(&mut S, bool, &mut Tx) + 'static,
) -> El {
    let el = el
        .role("toggle")
        .checked(pressed)
        .focus_selected(pressed)
        .focusable(true)
        .unavailable(props.disabled);
    let press = Rc::new(move |s: &mut S, tx: &mut Tx| {
        if !props.disabled {
            on_pressed_change(s, !pressed, tx);
        }
    });
    let key_press = Rc::clone(&press);
    el.on_press_tx(move |s: &mut S, tx| press(s, tx)).on_key(move |s: &mut S, key: &KeyEvent, tx: &mut Tx| {
        if key.key != Key::Enter && key.key != Key::Text(" ".into()) {
            return false;
        }
        if !tx.key_repeat() {
            key_press(s, tx);
        }
        true
    })
}

/// The options a toggle group takes each render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupProps {
    pub disabled: bool,
    pub multiple: bool,
    pub deselectable: bool,
    pub axis: FocusAxis,
    pub wrap: bool,
}

impl Default for GroupProps {
    fn default() -> Self {
        Self { disabled: false, multiple: false, deselectable: true, axis: FocusAxis::Horizontal, wrap: true }
    }
}

/// Makes `el` a toggle group named `label`. Put its toggles inside, each made with
/// [`group_item`].
pub fn group(el: El, label: impl Into<String>, props: GroupProps) -> El {
    el.role("group").label(label).focus_group().focus_axis(props.axis).focus_wrap(props.wrap)
}

/// The value a group would hold after `item` is pressed, from `value` in toggle order; `None`
/// when the press changes nothing.
pub fn pressed_value(value: &[String], item: &str, order: &[&str], props: GroupProps) -> Option<Vec<String>> {
    if props.disabled {
        return None;
    }
    let on = value.iter().any(|v| v == item);
    if on && !props.deselectable {
        return None;
    }
    let mut next: Vec<String> = if props.multiple {
        value.iter().filter(|v| v.as_str() != item).cloned().collect()
    } else {
        Vec::new()
    };
    if !on {
        next.push(item.to_owned());
    }
    next.sort_by_key(|v| order.iter().position(|o| o == v).unwrap_or(usize::MAX));
    Some(next)
}

/// One toggle of a group whose value is `value`, in toggle order `order`. A press asks
/// `on_value_change` for the group's next value.
pub fn group_item<S: 'static>(
    el: El,
    item: &'static str,
    item_props: Props,
    value: &[String],
    order: &'static [&'static str],
    props: GroupProps,
    on_value_change: impl Fn(&mut S, Vec<String>, &mut Tx) + 'static,
) -> El {
    let pressed = value.iter().any(|v| v == item);
    let current = value.to_vec();
    let disabled = props.disabled || item_props.disabled;
    toggle(el, Props { disabled }, pressed, move |s: &mut S, _, tx| {
        if let Some(next) = pressed_value(&current, item, order, props) {
            on_value_change(s, next, tx);
        }
    })
}

#[cfg(test)]
mod tests {
    //! The toggle and toggle group specs' cases. Not ported: right to left (metldr's interface
    //! has no right-to-left locale yet), rendering as a link and form submit (a toggle is a
    //! control on the canvas), and the pending cases, which the button module covers.
    use super::*;
    use rune::{Cx, Env, Mods, Runtime, col, row, text};
    use serde_json::Value;

    const ORDER: &[&str] = &["left", "center", "right"];

    struct App {
        bold: bool,
        value: Vec<String>,
        props: GroupProps,
        disabled_item: Option<&'static str>,
        refuse: bool,
        presses: usize,
    }

    impl Default for App {
        fn default() -> Self {
            Self { bold: false, value: vec!["center".into()], props: GroupProps::default(), disabled_item: None, refuse: false, presses: 0 }
        }
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let items = ORDER.iter().map(|&item| {
            group_item(
                row().key(item).size(60.0, 24.0).child(text(item)),
                item,
                Props { disabled: app.disabled_item == Some(item) },
                &app.value,
                ORDER,
                app.props,
                |a: &mut App, next, _| {
                    a.presses += 1;
                    if !a.refuse {
                        a.value = next;
                    }
                },
            )
        });
        col()
            .child(row().key("before").size(60.0, 24.0).focusable(true).on_press(|_: &mut App| {}))
            .child(toggle(row().key("bold").size(60.0, 24.0).child(text("Bold")), Props::default(), app.bold, |a: &mut App, on, _| {
                if !a.refuse {
                    a.bold = on;
                }
            }))
            .child(group(row().key("align"), "Alignment", app.props).children(items))
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

    fn value(rt: &Runtime<App>) -> Vec<&str> {
        rt.state.value.iter().map(String::as_str).collect()
    }

    #[test]
    fn a_toggle_starts_unpressed_presses_on_click_and_keeps_its_name() {
        let mut rt = rig(App::default());
        assert_eq!(mirror(&rt, "Bold")["c"], "false");
        click(&mut rt, "bold");
        assert!(rt.state.bold);
        assert_eq!(mirror(&rt, "Bold")["c"], "true", "pressed, under the same name");
        click(&mut rt, "bold");
        assert!(!rt.state.bold);
    }

    #[test]
    fn enter_and_space_press_a_focused_toggle() {
        for key in [Key::Enter, Key::Text(" ".into())] {
            let mut rt = rig(App::default());
            tab_to(&mut rt, "bold");
            press(&mut rt, key);
            assert!(rt.state.bold);
        }
    }

    #[test]
    fn a_toggle_can_start_pressed_and_a_change_can_be_refused() {
        let mut rt = rig(App { bold: true, refuse: true, ..App::default() });
        assert_eq!(mirror(&rt, "Bold")["c"], "true");
        click(&mut rt, "bold");
        assert!(rt.state.bold);
    }

    #[test]
    fn the_group_is_named_and_its_default_toggle_is_pressed() {
        let rt = rig(App::default());
        assert!(rt.a11y_json().contains("\"Alignment\""));
        assert_eq!(mirror(&rt, "center")["c"], "true");
        assert_eq!(mirror(&rt, "left")["c"], "false");
    }

    #[test]
    fn pressing_a_toggle_unpresses_the_others_and_pressing_the_pressed_one_unpresses_it() {
        let mut rt = rig(App::default());
        click(&mut rt, "left");
        assert_eq!(value(&rt), ["left"]);
        click(&mut rt, "left");
        assert!(value(&rt).is_empty());
    }

    #[test]
    fn a_group_change_can_be_refused() {
        let mut rt = rig(App { refuse: true, ..App::default() });
        click(&mut rt, "left");
        assert_eq!(value(&rt), ["center"]);
    }

    #[test]
    fn with_multiple_presses_add_to_the_value_in_toggle_order() {
        let mut rt = rig(App { props: GroupProps { multiple: true, ..GroupProps::default() }, ..App::default() });
        click(&mut rt, "right");
        click(&mut rt, "left");
        assert_eq!(value(&rt), ["left", "center", "right"]);
    }

    #[test]
    fn with_deselectable_off_the_pressed_toggle_stays_pressed() {
        let mut rt = rig(App { props: GroupProps { deselectable: false, ..GroupProps::default() }, ..App::default() });
        click(&mut rt, "center");
        assert_eq!(value(&rt), ["center"]);
    }

    #[test]
    fn a_disabled_toggle_cant_be_pressed_but_arrows_reach_it_and_it_says_so() {
        let mut rt = rig(App { disabled_item: Some("right"), ..App::default() });
        click(&mut rt, "right");
        assert_eq!(value(&rt), ["center"]);
        assert_eq!(mirror(&rt, "right")["d"], true);
        let mut rt = rig(App { disabled_item: Some("right"), ..App::default() });
        tab_to(&mut rt, "center");
        press(&mut rt, Key::Right);
        assert_eq!(focused(&rt).as_deref(), Some("right"));
    }

    #[test]
    fn a_disabled_group_presses_nothing_and_keeps_its_tab_stop() {
        let disabled = || App { props: GroupProps { disabled: true, ..GroupProps::default() }, ..App::default() };
        let mut rt = rig(disabled());
        click(&mut rt, "left");
        assert_eq!(rt.state.presses, 0);
        let mut rt = rig(disabled());
        tab_to(&mut rt, "center");
    }

    #[test]
    fn tab_moves_into_the_group_once_on_the_pressed_toggle_and_out_of_it() {
        let mut rt = rig(App::default());
        tab_to(&mut rt, "bold");
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("center"), "Tab lands on the pressed toggle");
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("after"), "only one toggle is a tab stop");
    }

    #[test]
    fn arrows_move_focus_without_pressing_wrap_and_home_and_end_jump() {
        let mut rt = rig(App::default());
        tab_to(&mut rt, "center");
        press(&mut rt, Key::Right);
        assert_eq!(focused(&rt).as_deref(), Some("right"));
        assert_eq!(value(&rt), ["center"], "moving focus presses nothing");
        press(&mut rt, Key::Right);
        assert_eq!(focused(&rt).as_deref(), Some("left"), "focus wraps");
        press(&mut rt, Key::Left);
        assert_eq!(focused(&rt).as_deref(), Some("right"));
        press(&mut rt, Key::Home);
        assert_eq!(focused(&rt).as_deref(), Some("left"));
        press(&mut rt, Key::End);
        assert_eq!(focused(&rt).as_deref(), Some("right"));
    }

    #[test]
    fn up_and_down_do_nothing_in_a_horizontal_group() {
        let mut rt = rig(App::default());
        tab_to(&mut rt, "center");
        press(&mut rt, Key::Down);
        press(&mut rt, Key::Up);
        assert_eq!(focused(&rt).as_deref(), Some("center"));
    }

    #[test]
    fn enter_and_space_press_the_focused_toggle_in_a_group() {
        for key in [Key::Enter, Key::Text(" ".into())] {
            let mut rt = rig(App::default());
            tab_to(&mut rt, "center");
            press(&mut rt, Key::Right);
            press(&mut rt, key);
            assert_eq!(value(&rt), ["right"]);
        }
    }

    #[test]
    fn tab_returns_to_the_toggle_focused_last() {
        let mut rt = rig(App::default());
        tab_to(&mut rt, "center");
        press(&mut rt, Key::Right);
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("after"));
        rt.key(KeyEvent { key: Key::Tab, mods: Mods { shift: true, ..Mods::default() } });
        rt.frame(0.0);
        assert_eq!(focused(&rt).as_deref(), Some("right"));
    }

    #[test]
    fn arrows_with_a_modifier_are_left_to_the_app() {
        let mut rt = rig(App::default());
        tab_to(&mut rt, "center");
        rt.key(KeyEvent { key: Key::Right, mods: Mods { alt: true, ..Mods::default() } });
        rt.frame(0.0);
        assert_eq!(focused(&rt).as_deref(), Some("center"));
    }

    #[test]
    fn with_wrap_off_arrows_stop_at_the_ends() {
        let mut rt = rig(App { props: GroupProps { wrap: false, ..GroupProps::default() }, ..App::default() });
        tab_to(&mut rt, "center");
        press(&mut rt, Key::Right);
        press(&mut rt, Key::Right);
        assert_eq!(focused(&rt).as_deref(), Some("right"));
    }

    #[test]
    fn a_vertical_group_moves_with_down_and_up() {
        let mut rt = rig(App { props: GroupProps { axis: FocusAxis::Vertical, ..GroupProps::default() }, ..App::default() });
        tab_to(&mut rt, "center");
        press(&mut rt, Key::Right);
        assert_eq!(focused(&rt).as_deref(), Some("center"));
        press(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("right"));
        press(&mut rt, Key::Up);
        assert_eq!(focused(&rt).as_deref(), Some("center"));
    }
}
