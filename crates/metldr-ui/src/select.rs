//! A select: a trigger that shows the chosen option and the list of options it opens.
//!
//! - The trigger is a combobox that says it opens a listbox, whether it is open and, while open,
//!   which list it controls. A press, Enter, Space or an arrow opens the list on the chosen
//!   option, or on the first one when none is chosen; Home and End open it on the first and last.
//! - Typing on the closed trigger chooses the next option whose label starts with the text, as a
//!   native select does, without opening the list.
//! - Inside, Up and Down move between options, wrapping at the ends; Home and End jump; typing
//!   moves to the next option whose label starts with the text. A disabled option stays on the
//!   arrows' path and does nothing.
//! - Enter, Space or a press on an option chooses it, closes the list and returns focus to the
//!   trigger. Escape, Tab, a press outside and the window losing focus close it without choosing.

use crate::popup::{self, Kind};
use rune::{Align, El, Key, KeyEvent, Side, Transition, Tx};
use std::rc::Rc;

/// One option, as the closed trigger reads it to open on it and to type to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub key: String,
    pub label: String,
    pub disabled: bool,
}

/// Where the list opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Chosen,
    First,
    Last,
}

/// The option the list opens on: the chosen one, else the first that is not disabled; or the
/// first or last that is not disabled.
pub fn start(items: &[Item], chosen: Option<usize>, at: Start) -> Option<usize> {
    let enabled = |i: &usize| !items[*i].disabled;
    match at {
        Start::Chosen => chosen
            .filter(|&i| i < items.len() && !items[i].disabled)
            .or_else(|| (0..items.len()).find(enabled)),
        Start::First => (0..items.len()).find(enabled),
        Start::Last => (0..items.len()).rev().find(enabled),
    }
}

/// The next option after `from`, round to the start, whose label starts with `typed` in any
/// case and that is not disabled; `from` itself comes last, so repeating a letter cycles.
pub fn typeahead(items: &[Item], from: Option<usize>, typed: &str) -> Option<usize> {
    let typed = typed.trim_start();
    if typed.is_empty() || items.is_empty() {
        return None;
    }
    let first = from.map_or(0, |i| i + 1);
    (0..items.len())
        .map(|n| (first + n) % items.len())
        .find(|&i| {
            let label = items[i].label.as_bytes();
            !items[i].disabled
                && label.len() >= typed.len()
                && label[..typed.len()].eq_ignore_ascii_case(typed.as_bytes())
        })
}

/// Makes `el` the trigger of a select whose list is keyed `list` and is `open` now, with
/// `items` and the `chosen` one. `on_open_change` opens or closes it; `on_choose` takes the
/// option typed on the closed trigger. The app keeps both states.
pub fn trigger<S: 'static>(
    el: El,
    list: &str,
    open: bool,
    items: Rc<[Item]>,
    chosen: Option<usize>,
    on_open_change: impl Fn(&mut S, bool, &mut Tx) + 'static,
    on_choose: impl Fn(&mut S, usize, &mut Tx) + 'static,
) -> El {
    let on_open_change = Rc::new(on_open_change);
    let on_key = Rc::clone(&on_open_change);
    let press_items = Rc::clone(&items);
    let el = if open { el.controls(list) } else { el };
    el.role("combobox")
        .has_popup("listbox")
        .expanded(open)
        .focusable(true)
        .on_press_tx(move |s: &mut S, tx| {
            on_open_change(s, !open, tx);
            if !open && let Some(i) = start(&press_items, chosen, Start::Chosen) {
                tx.focus(press_items[i].key.clone());
            }
        })
        .on_key(move |s: &mut S, key: &KeyEvent, tx: &mut Tx| {
            let at = match &key.key {
                Key::Enter | Key::Down | Key::Up => Start::Chosen,
                Key::Text(t) if t == " " => Start::Chosen,
                Key::Home => Start::First,
                Key::End => Start::Last,
                Key::Text(t) if !open && !key.mods.cmd && !key.mods.ctrl => {
                    if let Some(i) = typeahead(&items, chosen, t) {
                        on_choose(s, i, tx);
                    }
                    return true;
                }
                _ => return false,
            };
            if !open && !tx.key_repeat() {
                on_key(s, true, tx);
                if let Some(i) = start(&items, chosen, at) {
                    tx.focus(items[i].key.clone());
                }
            }
            true
        })
}

/// The list named `label` for the trigger keyed `trigger`, below it and grown from it with
/// `motion`. `on_close` closes it.
pub fn popup<S: 'static>(
    el: El,
    label: impl Into<String>,
    trigger: &str,
    motion: Transition,
    on_close: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let list = el
        .role("listbox")
        .label(label)
        .focus_group()
        .anchored_fit(trigger, Side::Bottom, Align::Start, 4.0)
        .transition(motion);
    popup::layer(
        list,
        Kind::Select,
        trigger,
        true,
        move |s: &mut S, _, tx: &mut Tx| on_close(s, tx),
        |_: &mut S, _| {},
    )
}

/// An option, `chosen` now: choosing it runs `on_choose`, then closes the list with `on_close`.
/// A disabled one does nothing.
pub fn option<S: 'static>(
    el: El,
    disabled: bool,
    chosen: bool,
    on_choose: impl Fn(&mut S, &mut Tx) + 'static,
    on_close: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let el = el
        .role("option")
        .focus_selected(chosen)
        .focusable(true)
        .unavailable(disabled);
    crate::press::activate(el, move |s: &mut S, tx: &mut Tx| {
        if !disabled {
            on_choose(s, tx);
            on_close(s, tx);
        }
    })
}

#[cfg(test)]
mod tests {
    //! The select spec's cases for what metldr uses. Not ported: arrows skipping disabled
    //! options (metldr keeps them on the arrows' path, as its toolbar, tabs and menus do),
    //! drag and hold to choose, multiple selection, groups, forms, read-only, shadow roots,
    //! alignment over the trigger, and right to left.
    use super::*;
    use rune::{Cx, Env, Mods, Runtime, col, row, text};
    use serde_json::Value;

    const FRUITS: [(&str, bool); 5] = [
        ("Apple", false),
        ("Banana", false),
        ("Blueberry", false),
        ("Grape", true),
        ("Orange", false),
    ];

    #[derive(Default)]
    struct App {
        open: bool,
        chosen: Option<usize>,
    }

    fn items() -> Rc<[Item]> {
        FRUITS
            .iter()
            .map(|&(label, disabled)| Item {
                key: label.into(),
                label: label.into(),
                disabled,
            })
            .collect()
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let shown = app.chosen.map_or("Pick a fruit", |i| FRUITS[i].0);
        let mut out = col()
            .child(
                row()
                    .key("outside")
                    .size(200.0, 30.0)
                    .focusable(true)
                    .on_press(|_: &mut App| {}),
            )
            .child(trigger(
                row()
                    .key("fruit")
                    .size(100.0, 24.0)
                    .label("Fruit")
                    .child(text(shown)),
                "fruits",
                app.open,
                items(),
                app.chosen,
                |a: &mut App, open, _| a.open = open,
                |a: &mut App, i, _| a.chosen = Some(i),
            ));
        if app.open {
            let options = FRUITS.iter().enumerate().map(|(i, &(label, disabled))| {
                option(
                    row()
                        .key(label)
                        .label(label)
                        .size(140.0, 24.0)
                        .child(text(label)),
                    disabled,
                    app.chosen == Some(i),
                    move |a: &mut App, _| a.chosen = Some(i),
                    |a: &mut App, _| a.open = false,
                )
            });
            out = out.child(
                popup(
                    col().key("fruits").w(160.0),
                    "Fruit",
                    "fruit",
                    Transition::opacity(),
                    |a: &mut App, _| a.open = false,
                )
                .children(options),
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
        let r = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap()
            .rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(rt);
    }

    fn press(rt: &mut Runtime<App>, key: Key) {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        settle(rt);
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    fn on_trigger() -> Runtime<App> {
        let mut rt = rig();
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("fruit"));
        rt
    }

    fn mirror(rt: &Runtime<App>, name: &str) -> Value {
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        items
            .into_iter()
            .find(|i| i["t"] == name)
            .unwrap_or_else(|| panic!("no {name}"))
    }

    #[test]
    fn trigger_names_list() {
        let mut rt = rig();
        let fruit = mirror(&rt, "Fruit");
        assert_eq!(
            (
                fruit["role"].as_str(),
                fruit["popup"].as_str(),
                fruit["x"].as_bool()
            ),
            (Some("combobox"), Some("listbox"), Some(false))
        );
        assert!(fruit.get("ctl").is_none());
        click(&mut rt, "fruit");
        let (fruit, list) = (mirror(&rt, "Fruit"), rt.a11y_json());
        assert_eq!(fruit["x"], true);
        let list: Vec<Value> = serde_json::from_str(&list).unwrap();
        let list = list.iter().find(|i| i["k"] == "listbox").unwrap();
        assert_eq!(fruit["ctl"], list["p"], "aria-controls names the open list");
        assert_eq!(mirror(&rt, "Apple")["role"], "option");
    }

    #[test]
    fn keys_open_list() {
        for key in [Key::Enter, Key::Text(" ".into()), Key::Down, Key::Up] {
            let mut rt = on_trigger();
            press(&mut rt, key);
            assert_eq!(focused(&rt).as_deref(), Some("Apple"));
        }
        let mut rt = on_trigger();
        press(&mut rt, Key::End);
        assert_eq!(focused(&rt).as_deref(), Some("Orange"));
    }

    #[test]
    fn keys_move_highlight() {
        let mut rt = on_trigger();
        press(&mut rt, Key::Enter);
        press(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("Banana"));
        press(&mut rt, Key::End);
        press(&mut rt, Key::Up);
        assert_eq!(
            focused(&rt).as_deref(),
            Some("Grape"),
            "a disabled option stays on the arrows' path"
        );
        press(&mut rt, Key::End);
        press(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("Apple"), "wraps");
        press(&mut rt, Key::Text("o".into()));
        assert_eq!(focused(&rt).as_deref(), Some("Orange"));
    }

    #[test]
    fn enter_chooses_and_refocuses() {
        let mut rt = on_trigger();
        press(&mut rt, Key::Enter);
        press(&mut rt, Key::End);
        press(&mut rt, Key::Enter);
        assert_eq!((rt.state.chosen, rt.state.open), (Some(4), false));
        assert_eq!(focused(&rt).as_deref(), Some("fruit"));
        press(&mut rt, Key::Enter);
        assert_eq!(focused(&rt).as_deref(), Some("Orange"));
        assert_eq!(
            (
                mirror(&rt, "Orange")["sel"].as_bool(),
                mirror(&rt, "Apple")["sel"].as_bool()
            ),
            (Some(true), Some(false))
        );
    }

    #[test]
    fn dismiss_keeps_choice() {
        for close in [Some(Key::Escape), Some(Key::Tab), None] {
            let mut rt = on_trigger();
            press(&mut rt, Key::Down);
            press(&mut rt, Key::Down);
            match close {
                Some(key) => press(&mut rt, key),
                None => click(&mut rt, "outside"),
            }
            assert_eq!((rt.state.open, rt.state.chosen), (false, None));
        }
        let mut rt = on_trigger();
        press(&mut rt, Key::Enter);
        press(&mut rt, Key::Escape);
        assert_eq!(focused(&rt).as_deref(), Some("fruit"));
    }

    #[test]
    fn press_skips_disabled() {
        let mut rt = rig();
        click(&mut rt, "fruit");
        click(&mut rt, "Grape");
        assert_eq!((rt.state.open, rt.state.chosen), (true, None));
        click(&mut rt, "Banana");
        assert_eq!((rt.state.open, rt.state.chosen), (false, Some(1)));
    }

    #[test]
    fn typeahead_when_closed() {
        let mut rt = on_trigger();
        press(&mut rt, Key::Text("b".into()));
        assert_eq!((rt.state.chosen, rt.state.open), (Some(1), false));
        press(&mut rt, Key::Text("b".into()));
        assert_eq!(rt.state.chosen, Some(2));
        press(&mut rt, Key::Text("g".into()));
        assert_eq!(
            rt.state.chosen,
            Some(2),
            "a disabled option is never typed to"
        );
    }
}
