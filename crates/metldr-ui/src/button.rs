//! A button: presses run an action, refused while the button is disabled or still busy.
//!
//! - Disabled leaves the tab order. `focusable_when_disabled` keeps it focusable, reading as
//!   unavailable, so it can hold focus while it becomes disabled and host a tooltip saying why.
//! - An action that keeps working (a request, a save) makes the button pending: it reads as busy
//!   and unavailable, keeps focus, refuses presses until it settles, and after a second the
//!   announcer says "Loading" so a slow action doesn't pass in silence.
//! - A failed action leaves its error until the next press.

use rune::{El, Tx};

/// How long a press may stay pending before screen readers hear it is loading.
pub const PENDING_ANNOUNCE_AFTER: f64 = 1.0;

/// What screen readers hear for a long press, by default.
pub const LOADING: &str = "Loading";

/// The options a button takes each render.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Props {
    pub disabled: bool,
    pub focusable_when_disabled: bool,
}

/// What an action did with a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It finished during the press.
    Done,
    /// It keeps working; call [`Button::settle`] when it ends.
    Pending,
}

/// A button's own state, kept in the app's state between renders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Button {
    pending_since: Option<f64>,
    error: Option<String>,
}

impl Button {
    /// Whether the last press is still working.
    pub fn pending(&self) -> bool {
        self.pending_since.is_some()
    }

    /// Why the last press failed, until the next press.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// A press: whether it may run. A disabled or pending button refuses; a press that runs
    /// clears the last error.
    pub fn press(&mut self, props: Props) -> bool {
        if props.disabled || self.pending() {
            return false;
        }
        self.error = None;
        true
    }

    /// The action started at `now` (seconds) keeps working.
    pub fn begin(&mut self, now: f64) {
        self.pending_since = Some(now);
    }

    /// The pending action ended, with its error if it failed.
    pub fn settle(&mut self, result: Result<(), String>) {
        self.pending_since = None;
        self.error = result.err();
    }

    /// What the announcer should say at `now`: [`LOADING`] once the press has been pending for
    /// [`PENDING_ANNOUNCE_AFTER`] seconds.
    pub fn announcement(&self, now: f64) -> Option<&'static str> {
        self.pending_since
            .filter(|since| now - since >= PENDING_ANNOUNCE_AFTER)
            .map(|_| LOADING)
    }
}

/// Makes `el` a button whose press finishes at once. Key and name it as any element; its
/// content or `label` is its accessible name.
pub fn button<S: 'static>(
    el: El,
    props: Props,
    action: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    control(el, props, false, move |s: &mut S, tx: &mut Tx| {
        if !props.disabled {
            action(s, tx);
        }
    })
}

/// Makes `el` a button whose action may keep working. `current` is its state as rendered,
/// `state` reaches it in the app's state, `now` reads the clock in seconds, and `action` runs a
/// press and says whether it keeps working.
#[allow(clippy::too_many_arguments)]
pub fn pending_button<S: 'static>(
    el: El,
    props: Props,
    current: &Button,
    state: fn(&mut S) -> &mut Button,
    now: fn(&S) -> f64,
    action: impl Fn(&mut S, &mut Tx) -> Outcome + 'static,
) -> El {
    control(el, props, current.pending(), move |s: &mut S, tx: &mut Tx| {
        if !state(s).press(props) {
            return;
        }
        if action(s, tx) == Outcome::Pending {
            let at = now(s);
            state(s).begin(at);
        }
    })
}

/// The role, focus rule, accessible state and press and key handling every button shares.
fn control<S: 'static>(
    el: El,
    props: Props,
    pending: bool,
    press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let el = el.role("button").busy(pending);
    let el = if props.disabled && !props.focusable_when_disabled {
        el.disabled(true)
    } else {
        el.focusable(true).unavailable(props.disabled || pending)
    };
    crate::press::activate(el, press)
}

#[cfg(test)]
mod tests {
    //! The button spec's cases. Not ported: form submit and reset (the canvas has no forms) and
    //! rendering as a link (a link is its own element).
    use super::*;
    use rune::{Cx, Env, Key, KeyEvent, Mods, Runtime, col, row, text};
    use serde_json::Value;

    #[derive(Default)]
    struct App {
        publish: Button,
        log: Vec<&'static str>,
        clock: f64,
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let plain = |key: &'static str, props: Props| {
            button(row().key(key).size(80.0, 24.0).child(text(key)), props, move |a: &mut App, _| a.log.push(key))
        };
        col()
            .child(plain("Save", Props::default()))
            .child(plain("Archive", Props { disabled: true, focusable_when_disabled: true }))
            .child(plain("Delete", Props { disabled: true, ..Props::default() }))
            .child(pending_button(row().key("Publish").size(80.0, 24.0).child(text("Publish")), Props::default(), &app.publish, |a: &mut App| &mut a.publish, |a: &App| a.clock, |a: &mut App, _| {
                a.log.push("Publish");
                Outcome::Pending
            }))
            .child(crate::announcer::announcer(app.publish.announcement(app.clock)))
    }

    fn rig() -> Runtime<App> {
        let mut rt = Runtime::new(App::default(), view, (300.0, 300.0), Env::default());
        rt.frame(0.0);
        rt
    }

    fn click(rt: &mut Runtime<App>, key: &str) {
        let el = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap();
        let (x, y) = (el.rect[0] + el.rect[2] / 2.0, el.rect[1] + el.rect[3] / 2.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    }

    fn press(rt: &mut Runtime<App>, key: Key) {
        rt.key(KeyEvent { key, mods: Mods::default() });
        rt.frame(0.0);
    }

    fn focus(rt: &mut Runtime<App>, key: &str) {
        for _ in 0..8 {
            if focused(rt).as_deref() == Some(key) {
                return;
            }
            press(rt, Key::Tab);
        }
        panic!("Tab never reached {key}");
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    fn mirror(rt: &Runtime<App>, name: &str) -> Value {
        let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
        items.into_iter().find(|i| i["t"] == name).unwrap_or(Value::Null)
    }

    #[test]
    fn clicking_a_button_runs_its_handler() {
        let mut rt = rig();
        click(&mut rt, "Save");
        assert_eq!(rt.state.log, ["Save"]);
    }

    #[test]
    fn space_and_enter_press_a_focused_button() {
        for key in [Key::Text(" ".into()), Key::Enter] {
            let mut rt = rig();
            focus(&mut rt, "Save");
            press(&mut rt, key);
            assert_eq!(rt.state.log, ["Save"]);
        }
    }

    #[test]
    fn a_disabled_button_cant_be_focused_or_pressed() {
        let mut rt = rig();
        assert_eq!(mirror(&rt, "Delete")["d"], true);
        click(&mut rt, "Delete");
        for _ in 0..8 {
            press(&mut rt, Key::Tab);
            assert_ne!(focused(&rt).as_deref(), Some("Delete"));
        }
        assert!(rt.state.log.is_empty());
    }

    #[test]
    fn focusable_when_disabled_stays_focusable_says_so_and_runs_nothing() {
        let mut rt = rig();
        assert_eq!(mirror(&rt, "Archive")["d"], true);
        focus(&mut rt, "Save");
        press(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("Archive"), "Tab reaches it");
        click(&mut rt, "Archive");
        for key in [Key::Text(" ".into()), Key::Enter] {
            press(&mut rt, key);
        }
        assert!(rt.state.log.is_empty());
    }

    #[test]
    fn a_pending_press_is_busy_unavailable_keeps_focus_and_refuses_presses() {
        let mut rt = rig();
        assert!(mirror(&rt, "Publish").get("d").is_none());
        focus(&mut rt, "Publish");
        press(&mut rt, Key::Enter);
        assert_eq!(rt.state.log, ["Publish"]);
        let publish = mirror(&rt, "Publish");
        assert_eq!((publish["b"].as_bool(), publish["d"].as_bool()), (Some(true), Some(true)));
        assert_eq!(focused(&rt).as_deref(), Some("Publish"));
        press(&mut rt, Key::Enter);
        click(&mut rt, "Publish");
        assert_eq!(rt.state.log, ["Publish"], "presses do nothing while pending");
        rt.update(|a, _| a.publish.settle(Ok(())));
        rt.frame(0.0);
        assert!(mirror(&rt, "Publish").get("b").is_none());
        assert!(mirror(&rt, "Publish").get("d").is_none());
    }

    #[test]
    fn still_pending_after_a_second_it_says_loading() {
        let mut rt = rig();
        click(&mut rt, "Publish");
        assert_ne!(mirror(&rt, LOADING)["k"], "status");
        rt.update(|a, _| a.clock = PENDING_ANNOUNCE_AFTER);
        rt.frame(0.0);
        assert_eq!(mirror(&rt, LOADING)["k"], "status");
    }

    #[test]
    fn a_failed_action_ends_pending_and_keeps_its_error_until_the_next_press() {
        let mut rt = rig();
        click(&mut rt, "Publish");
        rt.update(|a, _| a.publish.settle(Err("offline".into())));
        rt.frame(0.0);
        assert!(!rt.state.publish.pending());
        assert_eq!(rt.state.publish.error(), Some("offline"));
        click(&mut rt, "Publish");
        assert_eq!(rt.state.publish.error(), None);
        assert_eq!(rt.state.log, ["Publish", "Publish"]);
    }
}
