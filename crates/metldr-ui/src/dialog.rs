//! A dialog: a window over the page, opened by a trigger.
//!
//! - The trigger says it opens a dialog. Opening moves focus to the first control inside.
//! - The dialog is named by its title and described by its description.
//! - Modal (the default): Tab stays inside, the page behind is inert and hidden from assistive
//!   technology, and a press on the backdrop closes it unless `close_on_outside_press` is off.
//! - Escape, the backdrop and a close button close it, and focus returns to the trigger.
//! - A nested dialog closes on its own: Escape and its backdrop close only the top one.
//! - An alert dialog keeps an outside press from closing it and names itself an alert.

use rune::{El, Tx};

/// The options a dialog takes each render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Props {
    pub close_on_outside_press: bool,
    pub alert: bool,
}

impl Default for Props {
    fn default() -> Self {
        Self { close_on_outside_press: true, alert: false }
    }
}

/// Makes `el` a trigger that opens a dialog with `on_open`.
pub fn trigger<S: 'static>(el: El, open: bool, on_open: impl Fn(&mut S, &mut Tx) + 'static) -> El {
    close(el.has_popup("dialog").expanded(open), on_open)
}

/// Makes `el` the dialog titled `title`, described by `description`, opened from the trigger
/// keyed `trigger`. Escape or a press outside asks `on_close`.
pub fn dialog<S: 'static>(
    el: El,
    title: impl Into<String>,
    description: Option<&str>,
    trigger: &str,
    props: Props,
    on_close: impl Fn(&mut S) + 'static,
) -> El {
    let outside = props.close_on_outside_press && !props.alert;
    let el = el
        .role(if props.alert { "alertdialog" } else { "dialog" })
        .label(title)
        .focus_return(trigger)
        .modal(outside, on_close);
    match description {
        Some(text) => el.description(text),
        None => el,
    }
}

/// The dimmed layer behind a modal dialog: it fills the window and holds the dialog centred.
pub fn backdrop(dialog: El, dim: rune::Color) -> El {
    rune::col()
        .key("dialog-backdrop")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(dim)
        .center()
        .padding(10.0)
        .child(dialog)
}

/// A button inside the dialog that closes it with `on_close`; a press, Enter or Space runs it.
pub fn close<S: 'static>(el: El, on_close: impl Fn(&mut S, &mut Tx) + 'static) -> El {
    crate::press::activate(el.role("button").focusable(true), on_close)
}

#[cfg(test)]
mod tests {
    //! The dialog spec's cases for what metldr uses. Not ported: positive tab indices, shadow
    //! roots and custom elements, virtual clicks and touch focus (the canvas has one input
    //! path), page scroll locking (Rune scrolls only what is under the pointer), forms with
    //! method dialog, and the confirm helper.
    use super::*;
    use rune::{Cx, Env, Key, KeyEvent, Mods, Runtime, col, oklch, row, text};
    use serde_json::Value;

    #[derive(Default)]
    struct App {
        open: bool,
        nested: bool,
        alert: bool,
        closed: usize,
    }

    fn field(key: &'static str) -> El {
        row().key(key).label(key).size(120.0, 24.0).focusable(true).on_press(|_: &mut App| {})
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let mut page = col()
            .w_full()
            .h_full()
            .child(trigger(row().key("open").size(80.0, 24.0).child(text("Open")), app.open, |a: &mut App, _| a.open = true))
            .child(field("page-field"));
        if app.open {
            let mut body = col()
                .key("dialog")
                .w(240.0)
                .child(text("Saved chats"))
                .child(field("first"))
                .child(trigger(row().key("more").size(80.0, 24.0).child(text("More")), app.nested, |a: &mut App, _| a.nested = true))
                .child(close(row().key("close").size(80.0, 24.0).child(text("Close")), |a: &mut App, _| a.open = false));
            if app.nested {
                body = body.child(backdrop(
                    dialog(col().key("inner").w(160.0).child(field("inner-field")), "More", None, "more", Props::default(), |a: &mut App| a.nested = false),
                    oklch(0.0, 0.0, 0.0).alpha(0.3),
                ));
            }
            let props = Props { alert: app.alert, ..Props::default() };
            page = page.child(backdrop(
                dialog(body, "Saved chats", Some("Reopen a chat"), "open", props, |a: &mut App| {
                    a.open = false;
                    a.closed += 1;
                }),
                oklch(0.0, 0.0, 0.0).alpha(0.3),
            ));
        }
        page
    }

    fn rig(app: App) -> Runtime<App> {
        let mut rt = Runtime::new(app, view, (400.0, 400.0), Env::default());
        rt.frame(0.0);
        rt
    }

    fn settle(rt: &mut Runtime<App>) {
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    }

    fn click_at(rt: &mut Runtime<App>, x: f32, y: f32) {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(rt);
    }

    fn click(rt: &mut Runtime<App>, key: &str) {
        let r = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap().rect;
        click_at(rt, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    }

    fn press(rt: &mut Runtime<App>, key: Key, shift: bool) {
        rt.key(KeyEvent { key, mods: Mods { shift, ..Mods::default() } });
        settle(rt);
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    fn open(rt: &mut Runtime<App>) {
        press(rt, Key::Tab, false);
        assert_eq!(focused(rt).as_deref(), Some("open"));
        press(rt, Key::Enter, false);
        assert!(rt.state.open);
    }

    fn shown(rt: &Runtime<App>, key: &str) -> bool {
        rt.elements().iter().any(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    #[test]
    fn the_trigger_says_it_opens_a_dialog_and_opening_moves_focus_in() {
        let mut rt = rig(App::default());
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        assert_eq!(items.iter().find(|i| i["t"] == "Open").unwrap()["popup"], "dialog");
        open(&mut rt);
        assert_eq!(focused(&rt).as_deref(), Some("first"));
    }

    #[test]
    fn tab_moves_through_the_dialog_and_wraps_both_ways() {
        let mut rt = rig(App::default());
        open(&mut rt);
        press(&mut rt, Key::Tab, false);
        assert_eq!(focused(&rt).as_deref(), Some("more"));
        press(&mut rt, Key::Tab, false);
        assert_eq!(focused(&rt).as_deref(), Some("close"));
        press(&mut rt, Key::Tab, false);
        assert_eq!(focused(&rt).as_deref(), Some("first"), "never leaves the dialog");
        press(&mut rt, Key::Tab, true);
        assert_eq!(focused(&rt).as_deref(), Some("close"));
    }

    #[test]
    fn escape_closes_and_returns_focus_to_the_trigger() {
        let mut rt = rig(App::default());
        open(&mut rt);
        press(&mut rt, Key::Escape, false);
        assert!(!rt.state.open);
        assert_eq!(focused(&rt).as_deref(), Some("open"));
    }

    #[test]
    fn the_close_button_closes_and_returns_focus() {
        let mut rt = rig(App::default());
        open(&mut rt);
        click(&mut rt, "close");
        assert!(!shown(&rt, "dialog"));
        assert_eq!(focused(&rt).as_deref(), Some("open"));
    }

    #[test]
    fn pressing_the_backdrop_closes_and_pressing_inside_keeps_it_open() {
        let mut rt = rig(App::default());
        open(&mut rt);
        click(&mut rt, "first");
        assert!(rt.state.open, "a press inside keeps it open");
        click_at(&mut rt, 395.0, 395.0);
        assert!(!rt.state.open);
        assert_eq!(rt.state.closed, 1);
    }

    #[test]
    fn the_page_behind_is_inert_and_hidden_from_assistive_technology() {
        let mut rt = rig(App::default());
        open(&mut rt);
        let json = rt.a11y_json();
        assert!(!json.contains("page-field"));
        let items: Vec<Value> = serde_json::from_str(&json).unwrap();
        assert!(items.iter().any(|i| i["t"] == "first"));
    }

    #[test]
    fn a_nested_dialog_closes_on_its_own_and_returns_focus_to_its_trigger() {
        let mut rt = rig(App::default());
        open(&mut rt);
        click(&mut rt, "more");
        assert!(rt.state.nested);
        assert_eq!(focused(&rt).as_deref(), Some("inner-field"));
        press(&mut rt, Key::Escape, false);
        assert!((rt.state.nested, rt.state.open) == (false, true));
        assert_eq!(focused(&rt).as_deref(), Some("more"));
        press(&mut rt, Key::Escape, false);
        assert!(!rt.state.open, "a second Escape closes the outer one");
    }

    #[test]
    fn an_alert_dialog_keeps_an_outside_press_from_closing_it() {
        let mut rt = rig(App { alert: true, ..App::default() });
        open(&mut rt);
        click_at(&mut rt, 395.0, 395.0);
        assert!(rt.state.open);
        press(&mut rt, Key::Escape, false);
        assert!(!rt.state.open);
    }
}
