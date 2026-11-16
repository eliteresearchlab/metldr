//! A toolbar: a row of controls that is one Tab stop.
//!
//! - Tab enters the toolbar on the item it left from, or its first item; the arrows along its
//!   axis move between items, wrapping at the ends unless told not to; Home and End jump.
//! - A disabled toolbar or item stays focusable and says it is unavailable, so the arrows still
//!   reach it and a tooltip can say why.
//! - Its items are ordinary controls: buttons, toggles, menus. Give its buttons
//!   `focusable_when_disabled`, so a disabled one stays on the arrows' path.

use rune::{El, FocusAxis};

/// The options a toolbar takes each render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Props {
    pub axis: FocusAxis,
    pub wrap: bool,
}

impl Default for Props {
    fn default() -> Self {
        Self { axis: FocusAxis::Horizontal, wrap: true }
    }
}

/// Makes `el` a toolbar named `label`. Put its controls inside.
pub fn toolbar(el: El, label: impl Into<String>, props: Props) -> El {
    el.role("toolbar").label(label).focus_group().focus_axis(props.axis).focus_wrap(props.wrap)
}

#[cfg(test)]
mod tests {
    //! The toolbar cases for what metldr uses. Not ported: inputs inside a toolbar (their arrows
    //! stay the input's), links, right to left, and controllers.
    use super::*;
    use crate::button::{self, Props as ButtonProps};
    use rune::{Cx, Env, Key, KeyEvent, Mods, Runtime, col, row};

    #[derive(Default)]
    struct App {
        pressed: Vec<usize>,
    }

    fn view(_: &App, _: &mut Cx<App>) -> El {
        let items = (0..3).map(|i| {
            button::button(
                row().key(format!("item{i}")).size(40.0, 20.0).label(format!("Item {i}")),
                ButtonProps { disabled: i == 1, focusable_when_disabled: true },
                move |a: &mut App, _| a.pressed.push(i),
            )
        });
        col()
            .child(row().key("before").size(20.0, 20.0).focusable(true).role("button").on_press(|_: &mut App| {}))
            .child(toolbar(row().key("bar").gap(4.0), "Chat actions", Props::default()).children(items))
            .child(row().key("after").size(20.0, 20.0).focusable(true).role("button").on_press(|_: &mut App| {}))
    }

    fn runtime() -> Runtime<App> {
        let mut rt = Runtime::new(App::default(), view, (300.0, 200.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt
    }

    fn key(rt: &mut Runtime<App>, key: Key) {
        rt.key(KeyEvent { key, mods: Mods::default() });
        rt.frame(0.0);
        rt.frame(0.0);
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    #[test]
    fn it_is_one_tab_stop_that_remembers_its_item() {
        let mut rt = runtime();
        key(&mut rt, Key::Tab);
        key(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("item0"));
        key(&mut rt, Key::Right);
        assert_eq!(focused(&rt).as_deref(), Some("item1"), "a disabled item is reachable");
        key(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("after"), "one stop");
    }

    #[test]
    fn arrows_wrap_and_reach_the_ends() {
        let mut rt = runtime();
        rt.update(|_, tx| tx.focus("item2"));
        rt.frame(0.0);
        key(&mut rt, Key::Right);
        assert_eq!(focused(&rt).as_deref(), Some("item0"));
        key(&mut rt, Key::End);
        assert_eq!(focused(&rt).as_deref(), Some("item2"));
        key(&mut rt, Key::Home);
        assert_eq!(focused(&rt).as_deref(), Some("item0"));
    }

    #[test]
    fn the_mirror_names_the_toolbar() {
        let rt = runtime();
        let items: Vec<serde_json::Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        assert!(items.iter().any(|i| i["k"] == "toolbar" && i["t"] == "Chat actions"), "{items:?}");
    }
}
