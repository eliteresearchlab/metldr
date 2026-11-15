//! A collapsible: a trigger that shows and hides one panel.
//!
//! - The trigger is a button that says whether the panel is expanded.
//! - A closed panel leaves the page, so Tab skips it; with `keep_mounted` it stays, hidden from
//!   sight, input and assistive technology.
//! - A change carries its reason, and the app may refuse it.
//! - Disabled, the trigger stays focusable, reads as unavailable and opens nothing.
//! - Closing while focus is inside the panel returns focus to the trigger.
//! - The panel enters and leaves with the element transition the app gives it; one open from
//!   the first frame does not animate, and Reduce Motion turns it into a short fade.

use rune::{El, Tx};

/// Why the open state changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The trigger was pressed.
    TriggerPress,
    /// The app changed it.
    Imperative,
}

/// The options a collapsible takes each render.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Props {
    pub disabled: bool,
    /// Keep the closed panel in the page, hidden.
    pub keep_mounted: bool,
}

/// An uncontrolled collapsible's state, for an app that keeps no open state of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Collapsible {
    open: bool,
}

impl Collapsible {
    /// Starts open or closed.
    pub fn new(open: bool) -> Self {
        Self { open }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Asks to open or close for `reason`. `allow` sees the change and may refuse it. Returns
    /// whether the state changed.
    pub fn set_open(&mut self, open: bool, reason: Reason, allow: impl FnOnce(bool, Reason) -> bool) -> bool {
        if self.open == open || !allow(open, reason) {
            return false;
        }
        self.open = open;
        true
    }

    pub fn toggle(&mut self, reason: Reason, allow: impl FnOnce(bool, Reason) -> bool) -> bool {
        self.set_open(!self.open, reason, allow)
    }
}

/// Makes `el` the trigger of a panel that is `open` now. A press asks `on_open_change` for the
/// other state with its reason; the app applies it (or refuses by doing nothing).
pub fn trigger<S: 'static>(
    el: El,
    props: Props,
    open: bool,
    on_open_change: impl Fn(&mut S, bool, Reason) + 'static,
) -> El {
    let el = el.role("button").expanded(open).focusable(true).unavailable(props.disabled);
    crate::press::activate(el, move |s: &mut S, _: &mut Tx| {
        if !props.disabled {
            on_open_change(s, !open, Reason::TriggerPress);
        }
    })
}

/// The panel keyed `key` for the trigger keyed `trigger`, or `None` while it is closed and not
/// kept mounted.
pub fn panel(el: El, key: &str, trigger: &str, props: Props, open: bool) -> Option<El> {
    if !open && !props.keep_mounted {
        return None;
    }
    let el = el.key(key).focus_return(trigger);
    Some(if open { el } else { el.h(0.0).clip_rounded(0.0).inert().a11y_hidden() })
}

#[cfg(test)]
mod tests {
    //! The collapsible spec's cases. Not ported: generated ids (Rune keys parts by key),
    //! the trigger pointing at its panel (the mirror has no element ids yet; tracked in parity),
    //! find in page and hidden until found (the canvas has no page text to find; the mirror
    //! holds only what is open), a class setting display, and the open-change-complete
    //! callback (Rune transitions settle on their own; nothing in metldr waits on one).
    use super::*;
    use rune::{Cx, Env, Key, KeyEvent, Mods, Runtime, col, row, text};
    use serde_json::Value;

    #[derive(Default)]
    struct App {
        section: Collapsible,
        refuse: bool,
        disabled: bool,
        keep: bool,
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let props = Props { disabled: app.disabled, keep_mounted: app.keep };
        let mut out = col().child(row().key("before").size(80.0, 24.0).focusable(true).on_press(|_: &mut App| {})).child(
            trigger(row().key("trigger").size(120.0, 24.0).child(text("Details")), props, app.section.is_open(), |a: &mut App, open, reason| {
                let refuse = a.refuse;
                a.section.set_open(open, reason, |_, _| !refuse);
            }),
        );
        if let Some(p) = panel(
            col().child(row().key("inside").size(80.0, 24.0).focusable(true).on_press(|_: &mut App| {}).child(text("Inside"))),
            "panel",
            "trigger",
            props,
            app.section.is_open(),
        ) {
            out = out.child(p);
        }
        out.child(row().key("after").size(80.0, 24.0).focusable(true).on_press(|_: &mut App| {}))
    }

    fn rig(app: App) -> Runtime<App> {
        let mut rt = Runtime::new(app, view, (300.0, 300.0), Env::default());
        rt.frame(0.0);
        rt
    }

    fn has(rt: &Runtime<App>, key: &str) -> bool {
        rt.elements().iter().any(|e| e.key.as_deref() == Some(key) && !e.moving)
    }

    fn click(rt: &mut Runtime<App>, key: &str) {
        let r = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap().rect;
        rt.pointer_down(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_up(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        settle(rt);
    }

    fn settle(rt: &mut Runtime<App>) {
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
    }

    fn press(rt: &mut Runtime<App>, key: Key) {
        rt.key(KeyEvent { key, mods: Mods::default() });
        settle(rt);
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    fn trigger_item(rt: &Runtime<App>) -> Value {
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        items.into_iter().find(|i| i["t"] == "Details").unwrap()
    }

    #[test]
    fn starts_closed_with_the_panel_out_of_the_page() {
        let rt = rig(App::default());
        assert!(!has(&rt, "inside"));
        assert_eq!(trigger_item(&rt)["x"], false);
    }

    #[test]
    fn clicking_the_trigger_opens_the_panel_and_clicking_again_closes_it() {
        let mut rt = rig(App::default());
        click(&mut rt, "trigger");
        assert!(has(&rt, "inside"));
        assert_eq!(trigger_item(&rt)["x"], true);
        click(&mut rt, "trigger");
        assert!(!has(&rt, "inside"));
        assert_eq!(trigger_item(&rt)["x"], false);
    }

    #[test]
    fn enter_and_space_toggle_a_focused_trigger() {
        for key in [Key::Enter, Key::Text(" ".into())] {
            let mut rt = rig(App::default());
            press(&mut rt, Key::Tab);
            press(&mut rt, Key::Tab);
            assert_eq!(focused(&rt).as_deref(), Some("trigger"));
            press(&mut rt, key);
            assert!(rt.state.section.is_open());
        }
    }

    #[test]
    fn it_can_start_open() {
        let rt = rig(App { section: Collapsible::new(true), ..App::default() });
        assert!(has(&rt, "inside"));
    }

    #[test]
    fn an_opening_can_be_refused() {
        let mut rt = rig(App { refuse: true, ..App::default() });
        click(&mut rt, "trigger");
        assert!(!rt.state.section.is_open());
        assert!(!has(&rt, "inside"));
    }

    #[test]
    fn a_disabled_trigger_stays_focusable_says_so_and_opens_nothing() {
        let mut rt = rig(App { disabled: true, ..App::default() });
        assert_eq!(trigger_item(&rt)["d"], true);
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("trigger"));
        press(&mut rt, Key::Enter);
        click(&mut rt, "trigger");
        assert!(!rt.state.section.is_open());
    }

    #[test]
    fn the_app_opens_and_closes_it_from_anywhere() {
        let mut rt = rig(App::default());
        rt.update(|a, _| {
            a.section.set_open(true, Reason::Imperative, |_, _| true);
        });
        settle(&mut rt);
        assert!(has(&rt, "inside"));
        rt.update(|a, _| {
            a.section.toggle(Reason::Imperative, |_, _| true);
        });
        settle(&mut rt);
        assert!(!has(&rt, "inside"));
    }

    #[test]
    fn tab_moves_from_the_open_trigger_into_the_panel_and_skips_a_closed_one() {
        let mut rt = rig(App::default());
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("after"), "a closed panel is skipped");
        let mut rt = rig(App { section: Collapsible::new(true), ..App::default() });
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("inside"));
    }

    #[test]
    fn with_keep_mounted_the_closed_panel_stays_hidden_from_input_and_assistive_technology() {
        let mut rt = rig(App { keep: true, ..App::default() });
        assert!(rt.elements().iter().any(|e| e.key.as_deref() == Some("panel")));
        assert!(!rt.a11y_json().contains("Inside"));
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("after"));
    }

    #[test]
    fn closing_the_panel_while_focus_is_inside_moves_focus_to_the_trigger() {
        let mut rt = rig(App { section: Collapsible::new(true), ..App::default() });
        for _ in 0..3 {
            press(&mut rt, Key::Tab);
        }
        assert_eq!(focused(&rt).as_deref(), Some("inside"));
        rt.update(|a, _| {
            a.section.set_open(false, Reason::Imperative, |_, _| true);
        });
        settle(&mut rt);
        assert_eq!(focused(&rt).as_deref(), Some("trigger"));
    }
}
