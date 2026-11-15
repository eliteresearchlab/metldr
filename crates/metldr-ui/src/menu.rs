//! A menu: a trigger button and the popup of items it opens.
//!
//! - The trigger is a button that says it opens a menu and whether it is open. A press,
//!   Enter, Space or Down opens the menu on its first item; Up opens it on its last; a press on
//!   the open trigger closes it.
//! - Inside, Up and Down move between items, skipping separators and labels, wrapping at the
//!   ends; Home and End jump; typing moves to the next item whose label starts with what was
//!   typed, so give each item a `label`.
//! - While open, the page behind is inert, so focus and assistive technology stay in the menu.
//!   Disabled items are focusable and do nothing.
//! - Enter or a press on an item runs it, closes the menu and returns focus to the trigger.
//!   Checkbox and radio items change their value and keep the menu open, unless they close on
//!   select.
//! - Escape, a press outside and Tab close the menu; Escape and Tab return focus to the trigger.
//! - The popup grows from its trigger and leaves with the same motion, shorter.

use rune::{Align, El, Key, KeyEvent, Side, Transition, Tx};
use std::rc::Rc;

/// Which item the menu focuses when it opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    First,
    Last,
}

/// Makes `el` the trigger of a menu that is `open` now. `ends` names the keys of its first and
/// last items, read only when a key opens the menu. `on_open_change` asks to open (with the item
/// it focuses) or to close; the app keeps the open state.
pub fn trigger<S: 'static>(
    el: El,
    popup: &str,
    open: bool,
    ends: impl Fn(&S) -> (String, String) + 'static,
    on_open_change: impl Fn(&mut S, Option<Open>, &mut Tx) + 'static,
) -> El {
    let on_open_change = Rc::new(on_open_change);
    let on_key = Rc::clone(&on_open_change);
    let el = if open { el.controls(popup) } else { el };
    el.role("button")
        .has_popup("menu")
        .expanded(open)
        .focusable(true)
        .on_press_tx(move |s: &mut S, tx| on_open_change(s, (!open).then_some(Open::First), tx))
        .on_key(move |s: &mut S, key: &KeyEvent, tx: &mut Tx| {
            let to = match key.key {
                Key::Enter | Key::Down => Open::First,
                Key::Text(ref t) if t == " " => Open::First,
                Key::Up => Open::Last,
                _ => return false,
            };
            if !tx.key_repeat() {
                let (first, last) = ends(s);
                on_key(s, Some(to), tx);
                tx.focus(if to == Open::First { first } else { last });
            }
            true
        })
}

/// The menu popup named `label` for the trigger keyed `trigger`, placed on `side` of it and
/// grown from it with `motion`. `on_close` closes it.
pub fn popup<S: 'static>(
    el: El,
    label: impl Into<String>,
    trigger: &str,
    side: Side,
    motion: Transition,
    on_close: impl Fn(&mut S) + 'static,
) -> El {
    let on_close = Rc::new(on_close);
    let outside = Rc::clone(&on_close);
    let trigger_key = trigger.to_owned();
    el.role("menu")
        .label(label)
        .focus_group()
        .anchored_fit(trigger, side, Align::Start, 8.0)
        .transition(motion)
        .focus_return(trigger)
        .modal(true, move |s: &mut S| outside(s))
        .on_key(move |s: &mut S, key: &KeyEvent, tx: &mut Tx| {
            if !matches!(key.key, Key::Escape | Key::Tab) {
                return false;
            }
            on_close(s);
            tx.focus(trigger_key.clone());
            true
        })
}

/// An item that runs `on_select` and closes the menu with `on_close`.
pub fn item<S: 'static>(
    el: El,
    disabled: bool,
    on_select: impl Fn(&mut S, &mut Tx) + 'static,
    on_close: impl Fn(&mut S) + 'static,
) -> El {
    let run = Rc::new(move |s: &mut S, tx: &mut Tx| {
        if disabled {
            return;
        }
        on_select(s, tx);
        on_close(s);
    });
    with_activation(el.role("menuitem").focusable(true).unavailable(disabled), run)
}

/// Closes the menu after a checkbox or radio item is chosen (`closeOnSelect`); without it they
/// keep the menu open.
pub type CloseOnSelect<S> = Option<Rc<dyn Fn(&mut S)>>;

/// A radio item, `checked` now: choosing it runs `on_select`, then closes the menu only with
/// `close_on_select`.
pub fn radio_item<S: 'static>(
    el: El,
    disabled: bool,
    checked: bool,
    close_on_select: CloseOnSelect<S>,
    on_select: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let run = Rc::new(move |s: &mut S, tx: &mut Tx| {
        if disabled {
            return;
        }
        if !checked {
            on_select(s, tx);
        }
        if let Some(close) = &close_on_select {
            close(s);
        }
    });
    with_activation(el.role("menuitemradio").checked(checked).focusable(true).unavailable(disabled), run)
}

/// A checkbox item, `checked` now: choosing it asks `on_change` for the other state, then
/// closes the menu only with `close_on_select`.
pub fn checkbox_item<S: 'static>(
    el: El,
    disabled: bool,
    checked: bool,
    close_on_select: CloseOnSelect<S>,
    on_change: impl Fn(&mut S, bool, &mut Tx) + 'static,
) -> El {
    let run = Rc::new(move |s: &mut S, tx: &mut Tx| {
        if disabled {
            return;
        }
        on_change(s, !checked, tx);
        if let Some(close) = &close_on_select {
            close(s);
        }
    });
    with_activation(el.role("menuitemcheckbox").checked(checked).focusable(true).unavailable(disabled), run)
}

/// A rule between groups of items: not focusable, hidden from assistive technology.
pub fn separator(el: El) -> El {
    el.a11y_hidden()
}

fn with_activation<S: 'static>(el: El, run: Rc<dyn Fn(&mut S, &mut Tx)>) -> El {
    crate::press::activate(el, move |s: &mut S, tx: &mut Tx| run(s, tx))
}

#[cfg(test)]
mod tests {
    //! The menu spec's cases for what metldr uses. Not ported: submenus and the pointer safe
    //! area (metldr's menus have none), shadow roots, scrollbars outside the menu, and the
    //! starting-style animation (Rune's transition covers enter and exit). The trigger's open
    //! state is read before and after: while open the page behind is inert, as the menu holds
    //! focus.
    use super::*;
    use rune::{Cx, Env, Mods, Runtime, col, row, text};
    use serde_json::Value;

    #[derive(Default)]
    struct App {
        open: bool,
        log: Vec<&'static str>,
        wrap: bool,
        mode: &'static str,
    }

    fn close(a: &mut App) {
        a.open = false;
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let mut out = col()
            .child(row().key("outside").size(200.0, 30.0).focusable(true).on_press(|_: &mut App| {}))
            .child(trigger(row().key("file").size(80.0, 24.0).child(text("File")), "menu", app.open, |_: &App| ("New file".into(), "Delete".into()), |a: &mut App, to, _| {
                a.open = to.is_some();
                if let Some(to) = to {
                    a.log.push(if to == Open::First { "first" } else { "last" });
                }
            }));
        if app.open {
            let entry = |name: &'static str, disabled: bool| {
                item(row().key(name).label(name).size(140.0, 24.0).focus_highlight(rune::oklch(0.9, 0.0, 0.0)).child(text(name)), disabled, move |a: &mut App, _| a.log.push(name), close)
            };
            out = out.child(
                popup(col().key("menu").w(160.0), "File", "file", Side::Bottom, Transition::opacity(), close)
                    .child(entry("New file", false))
                    .child(entry("Open", false))
                    .child(separator(row().h(1.0).w(140.0)))
                    .child(entry("Rename", true))
                    .child(checkbox_item(row().key("Wrap").size(140.0, 24.0).child(text("Wrap")), false, app.wrap, None, |a: &mut App, on, _| a.wrap = on))
                    .child(radio_item(row().key("Edit").size(140.0, 24.0).child(text("Edit")), false, app.mode == "edit", None, |a: &mut App, _| a.mode = "edit"))
                    .child(entry("Delete", false)),
            );
        }
        out
    }

    fn rig() -> Runtime<App> {
        let mut rt = Runtime::new(App::default(), view, (400.0, 400.0), Env::default());
        rt.frame(0.0);
        rt
    }

    fn settle(rt: &mut Runtime<App>) {
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    }

    fn click(rt: &mut Runtime<App>, key: &str) {
        let r = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap().rect;
        rt.pointer_move(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_down(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_up(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        settle(rt);
    }

    fn press(rt: &mut Runtime<App>, key: Key) {
        rt.key(KeyEvent { key, mods: Mods::default() });
        settle(rt);
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    fn open_with(key: Key) -> Runtime<App> {
        let mut rt = rig();
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("file"));
        press(&mut rt, key);
        rt
    }

    fn menu_shown(rt: &Runtime<App>) -> bool {
        rt.elements().iter().any(|e| e.key.as_deref() == Some("menu") && !e.exiting)
    }

    #[test]
    fn the_trigger_says_it_opens_a_menu_and_whether_it_is_open() {
        let mut rt = rig();
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        let file = items.iter().find(|i| i["t"] == "File").unwrap();
        assert_eq!((file["x"].as_bool(), file["popup"].as_str()), (Some(false), Some("menu")));
        click(&mut rt, "file");
        assert!(menu_shown(&rt), "while open, the page behind is inert, so the menu holds focus");
        press(&mut rt, Key::Escape);
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        assert_eq!(items.iter().find(|i| i["t"] == "File").unwrap()["x"], false);
    }

    #[test]
    fn a_click_opens_the_menu_and_a_click_on_the_open_trigger_closes_it() {
        let mut rt = rig();
        click(&mut rt, "file");
        assert!(menu_shown(&rt));
        click(&mut rt, "file");
        assert!(!menu_shown(&rt));
    }

    #[test]
    fn enter_space_and_down_open_on_the_first_item_and_up_on_the_last() {
        for key in [Key::Enter, Key::Text(" ".into()), Key::Down] {
            let rt = open_with(key);
            assert_eq!(focused(&rt).as_deref(), Some("New file"));
        }
        let rt = open_with(Key::Up);
        assert_eq!(focused(&rt).as_deref(), Some("Delete"));
    }

    #[test]
    fn arrows_move_between_items_skip_separators_reach_disabled_items_and_wrap() {
        let mut rt = open_with(Key::Enter);
        press(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("Open"));
        press(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("Rename"), "skips the separator, reaches the disabled item");
        press(&mut rt, Key::End);
        assert_eq!(focused(&rt).as_deref(), Some("Delete"));
        press(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("New file"), "wraps");
        press(&mut rt, Key::Home);
        assert_eq!(focused(&rt).as_deref(), Some("New file"));
    }

    #[test]
    fn typing_moves_to_the_next_item_that_starts_with_it() {
        let mut rt = open_with(Key::Enter);
        press(&mut rt, Key::Text("d".into()));
        assert_eq!(focused(&rt).as_deref(), Some("Delete"));
    }

    #[test]
    fn enter_on_an_item_runs_it_closes_the_menu_and_returns_focus() {
        let mut rt = open_with(Key::Enter);
        press(&mut rt, Key::Down);
        press(&mut rt, Key::Enter);
        assert_eq!(rt.state.log.last(), Some(&"Open"));
        assert!(!menu_shown(&rt));
        assert_eq!(focused(&rt).as_deref(), Some("file"));
    }

    #[test]
    fn clicking_an_item_runs_it_and_disabled_items_do_nothing() {
        let mut rt = rig();
        click(&mut rt, "file");
        click(&mut rt, "Rename");
        assert!(menu_shown(&rt));
        assert!(!rt.state.log.contains(&"Rename"));
        click(&mut rt, "Open");
        assert_eq!(rt.state.log.last(), Some(&"Open"));
        assert!(!menu_shown(&rt));
    }

    #[test]
    fn checkbox_and_radio_items_change_their_value_and_keep_the_menu_open() {
        let mut rt = rig();
        click(&mut rt, "file");
        click(&mut rt, "Wrap");
        click(&mut rt, "Edit");
        assert!((rt.state.wrap, rt.state.mode) == (true, "edit"));
        assert!(menu_shown(&rt));
    }

    #[test]
    fn escape_closes_and_returns_focus_to_the_trigger() {
        let mut rt = open_with(Key::Enter);
        press(&mut rt, Key::Escape);
        assert!(!menu_shown(&rt));
        assert_eq!(focused(&rt).as_deref(), Some("file"));
    }

    #[test]
    fn clicking_outside_closes_the_menu() {
        let mut rt = rig();
        click(&mut rt, "file");
        click(&mut rt, "outside");
        assert!(!menu_shown(&rt));
    }

    #[test]
    fn tab_closes_the_menu() {
        let mut rt = open_with(Key::Enter);
        press(&mut rt, Key::Tab);
        assert!(!menu_shown(&rt));
        assert_ne!(focused(&rt).as_deref(), Some("New file"));
    }

    #[test]
    fn the_menu_opens_below_the_trigger() {
        let mut rt = rig();
        click(&mut rt, "file");
        let rect = |key: &str| rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap().target;
        assert!(rect("menu")[1] >= rect("file")[1] + rect("file")[3]);
    }
}
