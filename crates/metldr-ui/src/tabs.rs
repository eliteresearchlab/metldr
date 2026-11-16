//! Tabs: a row of tabs that picks which panel shows.
//!
//! - The tablist is one Tab stop that Tab enters on the selected tab. The arrows along its axis
//!   move between tabs, wrapping at the ends unless told not to; Home and End jump to the ends.
//! - Automatic activation selects a tab as soon as the keyboard focuses it. Manual activation
//!   waits for Enter or Space, for tabs whose switch costs something (loading a conversation).
//!   A press always selects.
//! - A disabled tab stays focusable, so it can be found, and is never selected.
//! - The selected tab names the panel it shows, and the panel carries the tab's name.
//! - Every change goes through the app, which may refuse it.

use rune::{El, FocusAxis, Key, KeyEvent, Tx};
use std::rc::Rc;

/// When a focused tab is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    Automatic,
    Manual,
}

/// The options a tablist takes each render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListProps {
    pub axis: FocusAxis,
    pub wrap: bool,
    pub activation: Activation,
}

impl Default for ListProps {
    fn default() -> Self {
        Self { axis: FocusAxis::Horizontal, wrap: true, activation: Activation::Automatic }
    }
}

/// Makes `el` a tablist named `label`. Put its tabs inside, each made with [`tab`].
pub fn list(el: El, label: impl Into<String>, props: ListProps) -> El {
    let el = el.role("tablist").label(label).focus_group().focus_axis(props.axis).focus_wrap(props.wrap);
    // Automatic: the arrows press the tab they reach, as they do a radio.
    if props.activation == Activation::Manual { el.focus_manual() } else { el }
}

/// Makes `el` one tab, `selected` now, showing the panel keyed `panel` when selected. Selecting
/// it asks `on_select`; its list's activation decides whether the arrows do.
pub fn tab<S: 'static>(
    el: El,
    selected: bool,
    disabled: bool,
    panel: Option<&str>,
    on_select: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let el = el.role("tab").focus_selected(selected).focusable(true).unavailable(disabled);
    let el = match panel {
        Some(panel) if selected => el.controls(panel),
        _ => el,
    };
    if disabled || selected {
        return el;
    }
    let on_select = Rc::new(on_select);
    let key = Rc::clone(&on_select);
    el.on_press_tx(move |s: &mut S, tx| on_select(s, tx))
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let activates = matches!(ev.key, Key::Enter) || matches!(ev.key, Key::Text(ref t) if t == " ");
            if activates && !tx.key_repeat() {
                key(s, tx);
            }
            activates
        })
}

/// Makes `el` the panel a tab shows, named `label` as its tab is.
pub fn panel(el: El, label: impl Into<String>) -> El {
    el.role("tabpanel").label(label)
}

#[cfg(test)]
mod tests {
    //! The tabs cases for what metldr uses. Not ported: links as tabs, find in page in hidden
    //! panels (`hidden="until-found"`), right to left, the sliding indicator (a segmented
    //! control's covers that), and controllers.
    use super::*;
    use rune::{Cx, Env, Mods, Runtime, col, row};

    struct App {
        selected: usize,
        activation: Activation,
        picks: usize,
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let tabs = (0..4).map(|i| {
            tab(
                row().key(format!("tab{i}")).size(40.0, 20.0).label(format!("Tab {i}")),
                app.selected == i,
                i == 2,
                Some("panel"),
                move |a: &mut App, _| {
                    a.selected = i;
                    a.picks += 1;
                },
            )
        });
        col()
            .child(row().key("before").size(20.0, 20.0).focusable(true).role("button").on_press(|_: &mut App| {}))
            .child(list(row().key("list").gap(4.0), "Chats", ListProps { activation: app.activation, ..ListProps::default() }).children(tabs))
            .child(panel(col().key("panel").size(100.0, 40.0), format!("Tab {}", app.selected)))
    }

    fn runtime(activation: Activation) -> Runtime<App> {
        let mut rt = Runtime::new(App { selected: 1, activation, picks: 0 }, view, (300.0, 200.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt
    }

    fn key(rt: &mut Runtime<App>, key: Key, shift: bool) {
        rt.key(KeyEvent { key, mods: Mods { shift, ..Mods::default() } });
        rt.frame(0.0);
        rt.frame(0.0);
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    #[test]
    fn tab_enters_on_the_selected_tab_and_leaves_in_one_stop() {
        let mut rt = runtime(Activation::Manual);
        key(&mut rt, Key::Tab, false);
        key(&mut rt, Key::Tab, false);
        assert_eq!(focused(&rt).as_deref(), Some("tab1"));
        key(&mut rt, Key::Tab, false);
        assert_ne!(focused(&rt).as_deref().map(|k| k.starts_with("tab")), Some(true), "one Tab stop");
    }

    #[test]
    fn arrows_move_wrap_and_reach_the_ends_and_a_disabled_tab_is_focusable() {
        let mut rt = runtime(Activation::Manual);
        rt.update(|_, tx| tx.focus("tab1"));
        rt.frame(0.0);
        key(&mut rt, Key::Right, false);
        assert_eq!(focused(&rt).as_deref(), Some("tab2"), "the disabled tab is still reachable");
        key(&mut rt, Key::Right, false);
        key(&mut rt, Key::Right, false);
        assert_eq!(focused(&rt).as_deref(), Some("tab0"), "wraps");
        key(&mut rt, Key::End, false);
        assert_eq!(focused(&rt).as_deref(), Some("tab3"));
        key(&mut rt, Key::Home, false);
        assert_eq!(focused(&rt).as_deref(), Some("tab0"));
    }

    #[test]
    fn manual_activation_waits_for_enter_and_automatic_selects_on_focus() {
        let mut rt = runtime(Activation::Manual);
        rt.update(|_, tx| tx.focus("tab1"));
        rt.frame(0.0);
        key(&mut rt, Key::Left, false);
        assert_eq!(rt.state.selected, 1, "manual: focus moved, selection stayed");
        key(&mut rt, Key::Enter, false);
        assert_eq!(rt.state.selected, 0);

        let mut rt = runtime(Activation::Automatic);
        rt.update(|_, tx| tx.focus("tab1"));
        rt.frame(0.0);
        key(&mut rt, Key::Right, false);
        key(&mut rt, Key::Right, false);
        assert_eq!(rt.state.selected, 3, "the disabled tab was passed over for selection");
        assert_eq!(rt.state.picks, 1);
    }

    #[test]
    fn the_mirror_has_a_tablist_with_the_selected_tab_and_a_named_panel() {
        let rt = runtime(Activation::Manual);
        let items: Vec<serde_json::Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        assert!(items.iter().any(|i| i["k"] == "tablist" && i["t"] == "Chats"), "{items:?}");
        assert!(items.iter().any(|i| i["k"] == "tabpanel" && i["t"] == "Tab 1"));
        let tab = items.iter().find(|i| i["t"] == "Tab 1" && i["role"] == "tab").unwrap();
        assert_eq!(tab["sel"].as_bool(), Some(true));
    }
}
