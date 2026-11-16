//! Popups and layers: how each kind of popup closes and moves focus. Menu, select, combobox,
//! popover, dialog, tooltip, hover card and navigation menu share one behaviour table, so a new
//! popup picks its kind instead of wiring each close by hand.
//!
//! - A popup closes on Escape and on a press outside it and its trigger. A press on the trigger
//!   reaches the trigger, which toggles the popup itself.
//! - Focus leaving closes the kinds that live in the Tab order after their trigger (popover,
//!   hover card, navigation menu, menu, select, combobox). A tooltip never holds focus.
//! - The window losing focus closes menus, selects, comboboxes, tooltips and hover cards. A
//!   popover, dialog and navigation menu stay, so work in them can continue.
//! - A popup that is not `dismissible` (it holds unsaved work) refuses those closes. Escape and
//!   an outside press are attempts, which `on_attempt` hears so the app can ask; focus or the
//!   window leaving is refused without a word.
//! - A press on the trigger closes a tooltip; other kinds leave that press to the trigger.
//! - Only the topmost popup hears a close; a modal dialog above takes them all.
//! - A popup that moves focus returns it to its trigger when Escape closes it.

use rune::{El, Tx};
use std::rc::Rc;

pub use rune::Dismiss as Reason;

/// What Tab does inside the popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// Tab closes the popup, like a native menu, and focus returns to the trigger.
    Close,
    /// Tab moves on through the page; focus leaving closes the popup.
    Flow,
    /// Tab wraps inside the popup: a modal dialog.
    Trap,
    /// Focus never enters the popup, so Tab stays the page's.
    Native,
}

/// How a kind of popup behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Behavior {
    pub tab: Tab,
    /// The window losing focus closes it.
    pub closes_on_blur: bool,
    /// Focus moving outside it and its trigger closes it.
    pub closes_on_focus_loss: bool,
    /// It moves focus in when it opens and back to its trigger when it closes.
    pub moves_focus: bool,
    /// A press on its trigger closes it. Other kinds leave that press to the trigger.
    pub closes_on_trigger_press: bool,
}

/// The kinds of popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Menu,
    Select,
    Combobox,
    Popover,
    Dialog,
    Tooltip,
    HoverCard,
    NavigationMenu,
}

impl Kind {
    pub const fn behavior(self) -> Behavior {
        let plain = Behavior { tab: Tab::Close, closes_on_blur: true, closes_on_focus_loss: true, moves_focus: true, closes_on_trigger_press: false };
        match self {
            Kind::Menu | Kind::Select => plain,
            // Focus stays in the input while its list is open.
            Kind::Combobox => Behavior { tab: Tab::Native, moves_focus: false, ..plain },
            // Part of the page's Tab order; switching windows keeps the work in it.
            Kind::Popover => Behavior { tab: Tab::Flow, closes_on_blur: false, ..plain },
            // Holds focus until the user answers, whatever else happens.
            Kind::Dialog => Behavior { tab: Tab::Trap, closes_on_blur: false, closes_on_focus_loss: false, ..plain },
            // Never takes focus; focus stays on its trigger, and pressing the trigger closes it.
            Kind::Tooltip => Behavior { tab: Tab::Native, moves_focus: false, closes_on_trigger_press: true, ..plain },
            Kind::HoverCard => Behavior { tab: Tab::Flow, ..plain },
            Kind::NavigationMenu => Behavior { tab: Tab::Flow, closes_on_blur: false, ..plain },
        }
    }
}

impl Behavior {
    /// Whether `reason` closes a popup with this behaviour, before `dismissible` is asked.
    pub const fn closes(self, reason: Reason) -> bool {
        match reason {
            Reason::Escape | Reason::OutsidePress => true,
            Reason::AnchorPress => self.closes_on_trigger_press,
            Reason::FocusOut => self.closes_on_focus_loss,
            Reason::WindowBlur => self.closes_on_blur,
        }
    }
}

/// What a close request comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Close,
    /// Refused, and the user tried on purpose: `on_attempt` hears it.
    Attempt,
    /// Refused without a word.
    Ignore,
}

/// The outcome of `reason` for a popup with `behavior` that is `dismissible` now.
pub const fn outcome(behavior: Behavior, dismissible: bool, reason: Reason) -> Outcome {
    if !behavior.closes(reason) {
        Outcome::Ignore
    } else if dismissible {
        Outcome::Close
    } else if matches!(reason, Reason::Escape | Reason::OutsidePress) {
        Outcome::Attempt
    } else {
        Outcome::Ignore
    }
}

/// Makes `el` a non-modal popup of `kind` opened by the keyed `trigger`. `on_close` closes it;
/// `on_attempt` hears a refused attempt while it is not `dismissible`. A modal dialog uses
/// [`crate::dialog`] instead.
pub fn layer<S: 'static>(
    el: El,
    kind: Kind,
    trigger: &str,
    dismissible: bool,
    on_close: impl Fn(&mut S, Reason, &mut Tx) + 'static,
    on_attempt: impl Fn(&mut S, Reason) + 'static,
) -> El {
    let behavior = kind.behavior();
    let trigger = Rc::<str>::from(trigger);
    let on_close = Rc::new(on_close);
    let tab_close = Rc::clone(&on_close);
    let tab_trigger = Rc::clone(&trigger);
    let el = el.layer(move |s: &mut S, reason, tx: &mut Tx| match outcome(behavior, dismissible, reason) {
        Outcome::Close => {
            on_close(s, reason, tx);
            if behavior.moves_focus && reason == Reason::Escape {
                tx.focus(trigger.to_string());
            }
        }
        Outcome::Attempt => on_attempt(s, reason),
        Outcome::Ignore => {}
    });
    let el = if behavior.moves_focus { el.focus_return(tab_trigger.to_string()) } else { el };
    if behavior.tab != Tab::Close {
        return el;
    }
    el.on_key(move |s: &mut S, key: &rune::KeyEvent, tx: &mut Tx| {
        if key.key != rune::Key::Tab || !dismissible {
            return false;
        }
        tab_close(s, Reason::FocusOut, tx);
        tx.focus(tab_trigger.to_string());
        true
    })
}

#[cfg(test)]
mod tests {
    //! The popup presets' and dismissal cases. Not ported: the "click" outside press (a modal
    //! dialog's, which `dialog` covers through Rune's modal), Android's back gesture and the
    //! sheet swipe (no such input in Rune), iframes (a Rune window has none), and shortcut
    //! blocking (no global shortcuts register in metldr yet).
    use super::*;
    use rune::{Align, Cx, Env, Key, KeyEvent, Mods, Runtime, Side, col, row};

    #[test]
    fn each_kind_closes_for_its_reasons() {
        use Reason::*;
        let closes = |kind: Kind| [Escape, OutsidePress, FocusOut, WindowBlur].map(|r| kind.behavior().closes(r));
        assert_eq!(closes(Kind::Menu), [true, true, true, true]);
        assert_eq!(closes(Kind::Combobox), [true, true, true, true]);
        assert_eq!(closes(Kind::Popover), [true, true, true, false], "switching windows keeps a popover");
        assert_eq!(closes(Kind::Dialog), [true, true, false, false]);
        assert_eq!(closes(Kind::Tooltip), [true, true, true, true]);
        assert_eq!(closes(Kind::NavigationMenu), [true, true, true, false]);
        assert!(!Kind::Tooltip.behavior().moves_focus && !Kind::Combobox.behavior().moves_focus);
        assert!(Kind::Tooltip.behavior().closes(AnchorPress) && !Kind::Menu.behavior().closes(AnchorPress));
    }

    #[test]
    fn a_popup_holding_work_refuses_and_only_attempts_are_heard() {
        let menu = Kind::Menu.behavior();
        assert_eq!(outcome(menu, false, Reason::Escape), Outcome::Attempt);
        assert_eq!(outcome(menu, false, Reason::OutsidePress), Outcome::Attempt);
        assert_eq!(outcome(menu, false, Reason::FocusOut), Outcome::Ignore);
        assert_eq!(outcome(menu, false, Reason::WindowBlur), Outcome::Ignore);
        assert_eq!(outcome(Kind::Popover.behavior(), true, Reason::WindowBlur), Outcome::Ignore);
        assert_eq!(outcome(menu, true, Reason::WindowBlur), Outcome::Close);
    }

    #[derive(Default)]
    struct App {
        open: bool,
        dismissible: bool,
        closed: Vec<Reason>,
        attempts: Vec<Reason>,
        kind: Option<Kind>,
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        let mut root = col()
            .w_full()
            .h_full()
            .child(row().key("trigger").size(60.0, 20.0).focusable(true).on_press(|a: &mut App| a.open = !a.open))
            .child(row().key("away").size(60.0, 20.0).absolute(200.0, 200.0).focusable(true).on_press(|_: &mut App| {}));
        if app.open {
            let popup = col().key("popup").size(80.0, 40.0).anchored("trigger", Side::Bottom, Align::Start, 4.0)
                .child(row().key("inside").size(40.0, 20.0).focusable(true).on_press(|_: &mut App| {}));
            root = root.child(layer(
                popup,
                app.kind.unwrap_or(Kind::Menu),
                "trigger",
                app.dismissible,
                |a: &mut App, why, _| {
                    a.open = false;
                    a.closed.push(why);
                },
                |a: &mut App, why| a.attempts.push(why),
            ));
        }
        root
    }

    fn runtime(kind: Kind, dismissible: bool) -> Runtime<App> {
        let app = App { open: true, dismissible, kind: Some(kind), ..App::default() };
        let mut rt = Runtime::new(app, view, (320.0, 320.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt
    }

    fn key(rt: &mut Runtime<App>, key: Key) {
        rt.key(KeyEvent { key, mods: Mods::default() });
        rt.frame(0.0);
    }

    fn focused(rt: &Runtime<App>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    #[test]
    fn escape_closes_and_returns_focus_to_the_trigger() {
        let mut rt = runtime(Kind::Menu, true);
        rt.update(|_, tx| tx.focus("inside"));
        rt.frame(0.0);
        key(&mut rt, Key::Escape);
        assert_eq!(rt.state.closed, [Reason::Escape]);
        rt.frame(0.0);
        assert_eq!(focused(&rt).as_deref(), Some("trigger"));
    }

    #[test]
    fn tab_closes_a_menu_and_a_press_outside_closes_it() {
        let mut rt = runtime(Kind::Menu, true);
        rt.update(|_, tx| tx.focus("inside"));
        rt.frame(0.0);
        key(&mut rt, Key::Tab);
        assert_eq!(rt.state.closed, [Reason::FocusOut]);
        rt.update(|a, _| a.open = true);
        rt.frame(0.0);
        rt.pointer_down(230.0, 210.0);
        rt.pointer_up(230.0, 210.0);
        rt.frame(0.0);
        assert_eq!(rt.state.closed, [Reason::FocusOut, Reason::OutsidePress]);
    }

    #[test]
    fn a_popover_stays_open_when_the_window_loses_focus() {
        let mut rt = runtime(Kind::Popover, true);
        rt.window_focus(false);
        rt.frame(0.0);
        assert!(rt.state.open && rt.state.closed.is_empty());
        let mut rt = runtime(Kind::Combobox, true);
        rt.window_focus(false);
        rt.frame(0.0);
        assert_eq!(rt.state.closed, [Reason::WindowBlur]);
    }

    #[test]
    fn a_popup_holding_work_stays_and_hears_the_attempt() {
        let mut rt = runtime(Kind::Popover, false);
        key(&mut rt, Key::Escape);
        rt.pointer_down(230.0, 210.0);
        rt.pointer_up(230.0, 210.0);
        rt.frame(0.0);
        assert!(rt.state.open);
        assert_eq!(rt.state.attempts, [Reason::Escape, Reason::OutsidePress]);
    }
}
