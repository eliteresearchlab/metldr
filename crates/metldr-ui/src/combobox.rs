//! A combobox: a text input that keeps focus while it shows a list of suggestions.
//!
//! - The input is a combobox that says it opens a listbox, whether it is open and, while open,
//!   which list it controls and which option is highlighted. Focus stays in the input; the
//!   highlight is virtual.
//! - Down and Up move the highlight, wrapping at the ends; on a closed list they open it on the
//!   first or last option. Page Down and Page Up move ten. Home and End stay the caret's.
//!   Alt+Down opens without highlighting and Alt+Up closes.
//! - Enter chooses the highlighted option. Escape and Tab close the list, and so do a press
//!   outside and the window losing focus.

use crate::popup::{self, Kind};
use rune::{Align, El, Key, KeyEvent, Side, Transition, Tx};

/// How far Page Up and Page Down move the highlight.
const PAGE: isize = 10;

/// What a key in the input asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Move the highlight this many options, opening the list first when it is closed.
    Move(isize),
    /// Open the list without a highlight.
    Open,
    Choose,
    Close,
}

/// What `key` asks of a combobox whose list is `open` now, or None when the input keeps it.
pub fn action(key: &KeyEvent, open: bool) -> Option<Action> {
    let alt = key.mods.alt;
    Some(match key.key {
        Key::Down if alt => (!open).then_some(Action::Open)?,
        Key::Up if alt => open.then_some(Action::Close)?,
        Key::Down => Action::Move(1),
        Key::Up => Action::Move(-1),
        Key::PageDown if open => Action::Move(PAGE),
        Key::PageUp if open => Action::Move(-PAGE),
        Key::Enter if open && !key.mods.shift => Action::Choose,
        Key::Escape | Key::Tab if open => Action::Close,
        _ => return None,
    })
}

/// The option the highlight lands on after moving `by` from `from` among `len` options: the
/// first or last from none, wrapping at the ends for a single step and stopping there for a page.
pub fn moved(from: Option<usize>, len: usize, by: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len as isize - 1;
    let to = match from {
        None if by > 0 => 0,
        None => last,
        Some(i) if by.abs() == 1 => (i as isize + by).rem_euclid(len as isize),
        Some(i) => (i as isize + by).clamp(0, last),
    };
    Some(to as usize)
}

/// Makes `el` (a text input) a combobox whose list is keyed `list` and is `open` now, with the
/// option keyed `active` highlighted.
pub fn input(el: El, list: &str, open: bool, active: Option<&str>) -> El {
    let el = el.role("combobox").has_popup("listbox").expanded(open);
    if !open {
        return el;
    }
    let el = el.controls(list);
    match active {
        Some(active) => el.active_descendant(active),
        None => el,
    }
}

/// The list named `label`, on `side` of the element keyed `anchor`, grown with `motion`; its
/// closes return to the keyed `trigger`. `on_close` closes it.
pub fn list<S: 'static>(
    el: El,
    label: impl Into<String>,
    anchor: &str,
    side: Side,
    trigger: &str,
    motion: Transition,
    on_close: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let list = el
        .role("listbox")
        .label(label)
        .focus_group()
        .anchored_fit(anchor, side, Align::Start, 8.0)
        .transition(motion);
    popup::layer(
        list,
        Kind::Combobox,
        trigger,
        true,
        move |s: &mut S, _, tx: &mut Tx| on_close(s, tx),
        |_: &mut S, _| {},
    )
}

/// An option, `highlighted` now, that a press chooses with `on_choose`. A disabled one stays an
/// option and does nothing.
pub fn option<S: 'static>(
    el: El,
    disabled: bool,
    highlighted: bool,
    on_choose: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let el = el
        .role("option")
        .focus_selected(highlighted)
        .focusable(true)
        .unavailable(disabled);
    if disabled {
        el
    } else {
        crate::press::activate(el, on_choose)
    }
}

#[cfg(test)]
mod tests {
    //! The combobox spec's cases for what metldr uses. Not ported: chips, creating and custom
    //! values, clearing, async items, virtual lists, the screen reader's real focus in the list,
    //! Ctrl+N and Ctrl+P, and right to left.
    use super::*;
    use rune::{Cx, Env, Mods, Runtime, TextEdit, TextStyle, col, row, text, text_input};
    use serde_json::Value;

    const FRUITS: [&str; 3] = ["Apple", "Banana", "Cherry"];

    #[derive(Default)]
    struct App {
        edit: TextEdit,
        open: bool,
        highlighted: Option<usize>,
        chosen: Option<usize>,
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let active = app.highlighted.map(|i| FRUITS[i]);
        let field = text_input(&app.edit, TextStyle::new(14.0, 400.0))
            .key("field")
            .label("Fruit")
            .size(200.0, 24.0)
            .single_line()
            .on_edit(|a: &mut App| &mut a.edit);
        let field = input(field, "fruits", app.open, active).on_input_key(|a: &mut App, key, _| {
            let Some(act) = action(key, a.open) else {
                return false;
            };
            match act {
                Action::Move(by) => {
                    a.open = true;
                    a.highlighted = moved(a.highlighted, FRUITS.len(), by);
                }
                Action::Open => a.open = true,
                Action::Choose => {
                    a.chosen = a.highlighted;
                    a.open = false;
                }
                Action::Close => a.open = false,
            }
            true
        });
        let mut out = col().child(field).child(
            row()
                .key("outside")
                .size(200.0, 30.0)
                .focusable(true)
                .on_press(|_: &mut App| {}),
        );
        if app.open {
            let options = FRUITS.iter().enumerate().map(|(i, &name)| {
                option(
                    row()
                        .key(name)
                        .label(name)
                        .size(140.0, 24.0)
                        .child(text(name)),
                    false,
                    app.highlighted == Some(i),
                    move |a: &mut App, _| {
                        a.chosen = Some(i);
                        a.open = false;
                    },
                )
            });
            out = out.child(
                list(
                    col().key("fruits").w(160.0),
                    "Fruits",
                    "field",
                    Side::Bottom,
                    "field",
                    Transition::opacity(),
                    |a: &mut App, _| {
                        a.open = false;
                        a.highlighted = None;
                    },
                )
                .children(options),
            );
        }
        out
    }

    fn rig() -> Runtime<App> {
        let mut rt = Runtime::new(App::default(), view, (400.0, 400.0), Env::default());
        rt.frame(0.0);
        rt.update(|_, tx| tx.focus("field"));
        rt.frame(0.0);
        rt
    }

    fn press(rt: &mut Runtime<App>, key: Key, alt: bool) {
        rt.key(KeyEvent {
            key,
            mods: Mods {
                alt,
                ..Mods::default()
            },
        });
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    }

    fn field(rt: &Runtime<App>) -> (Value, Vec<Value>) {
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        (
            items.iter().find(|i| i["k"] == "input").unwrap().clone(),
            items,
        )
    }

    #[test]
    fn names_list_while_open() {
        let mut rt = rig();
        let (input, _) = field(&rt);
        assert_eq!(
            (
                input["role"].as_str(),
                input["popup"].as_str(),
                input["x"].as_bool()
            ),
            (Some("combobox"), Some("listbox"), Some(false))
        );
        assert!(input.get("ctl").is_none() && input.get("ad").is_none());
        press(&mut rt, Key::Down, false);
        let (input, items) = field(&rt);
        let path = |name: &str| items.iter().find(|i| i["t"] == name).unwrap()["p"].clone();
        assert_eq!(input["x"], true);
        assert_eq!(
            input["ctl"],
            items.iter().find(|i| i["k"] == "listbox").unwrap()["p"]
        );
        assert_eq!(input["ad"], path("Apple"));
        assert_eq!(
            items.iter().find(|i| i["t"] == "Apple").unwrap()["sel"],
            true
        );
    }

    #[test]
    fn arrows_wrap_highlight() {
        let mut rt = rig();
        press(&mut rt, Key::Up, false);
        assert_eq!(
            (rt.state.open, rt.state.highlighted),
            (true, Some(2)),
            "Up opens on the last option"
        );
        press(&mut rt, Key::Down, false);
        assert_eq!(rt.state.highlighted, Some(0), "wraps");
        press(&mut rt, Key::PageDown, false);
        assert_eq!(rt.state.highlighted, Some(2), "a page stops at the end");
        let focused = rt
            .elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key);
        assert_eq!(focused.as_deref(), Some("field"));
    }

    #[test]
    fn enter_chooses_escape_closes() {
        let mut rt = rig();
        press(&mut rt, Key::Down, false);
        press(&mut rt, Key::Down, false);
        press(&mut rt, Key::Enter, false);
        assert_eq!((rt.state.open, rt.state.chosen), (false, Some(1)));
        press(&mut rt, Key::Down, true);
        assert_eq!(
            (rt.state.open, rt.state.highlighted),
            (true, Some(1)),
            "Alt+Down opens without moving"
        );
        press(&mut rt, Key::Up, true);
        assert!(!rt.state.open);
        press(&mut rt, Key::Down, false);
        press(&mut rt, Key::Escape, false);
        assert!(!rt.state.open);
        press(&mut rt, Key::Down, false);
        let r = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("outside"))
            .unwrap()
            .rect;
        rt.pointer_down(r[0] + 5.0, r[1] + 5.0);
        rt.pointer_up(r[0] + 5.0, r[1] + 5.0);
        rt.frame(0.0);
        assert!(!rt.state.open);
    }

    /// Run with `--ignored --release`: the mirror's JSON for a long chat whose controls name what
    /// they open and highlight (tabs, triggers, this prompt), as each sync builds it.
    #[test]
    #[ignore]
    fn mirror_relations_cost() {
        struct Big;
        fn view(_: &Big, _: &mut Cx<Big>) -> El {
            col()
                .key("root")
                .w_full()
                .children((0..2000).map(|i| {
                    text(format!("Message {i} with a few words in it"))
                        .key(format!("m{i}"))
                        .size(300.0, 12.3)
                }))
                .children((0..12).map(|i| {
                    row()
                        .key(format!("t{i}"))
                        .role("tab")
                        .label(format!("Chat {i}"))
                        .size(40.0, 20.0)
                        .controls(format!("m{}", 1980 + i))
                        .active_descendant(format!("m{}", 1000 + i))
                        .on_press(|_: &mut Big| {})
                }))
        }
        let mut rt = Runtime::new(Big, view, (400.0, 30000.0), Env::default());
        rt.frame(0.0);
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        assert_eq!(
            items
                .iter()
                .filter(|i| i.get("ctl").is_some() && i.get("ad").is_some())
                .count(),
            12
        );
        let mut best = f64::MAX;
        for _ in 0..7 {
            let t = std::time::Instant::now();
            for _ in 0..50 {
                std::hint::black_box(rt.a11y_json());
            }
            best = best.min(t.elapsed().as_secs_f64() / 50.0);
        }
        eprintln!("mirror relations json: {:.1} us", best * 1e6);
    }

    #[test]
    fn a_highlight_moves_by_steps_and_pages() {
        assert_eq!(moved(None, 3, 1), Some(0));
        assert_eq!(moved(None, 3, -1), Some(2));
        assert_eq!(moved(Some(2), 3, 1), Some(0));
        assert_eq!(moved(Some(0), 30, PAGE), Some(10));
        assert_eq!(moved(Some(25), 30, PAGE), Some(29));
        assert_eq!(moved(None, 0, 1), None);
    }
}
