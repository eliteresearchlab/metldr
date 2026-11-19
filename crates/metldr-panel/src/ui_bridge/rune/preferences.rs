pub(super) use super::theme::{Palette, Typography};
use super::*;
use crate::preferences::{
    UiPreferenceSnapshot, UiPreferenceStatus, UiPreferenceSubscription, UiPreferences,
};
use ::rune::{El, col};
use wasm_bindgen_futures::JsFuture;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Theme(&'static str),
    Size(&'static str),
    Sound(bool),
    /// A molded theme's key colour.
    KeyColor(crate::preferences::KeyColor),
    RetryAppearance,
    RetrySound,
    RetrySetup,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Control {
    #[default]
    Theme,
    Size,
    Sound,
}

impl Control {
    fn status_key(self) -> &'static str {
        if self == Self::Sound {
            "preferences-sound-wait"
        } else {
            "preferences-appearance-wait"
        }
    }

    fn return_key(self) -> &'static str {
        match self {
            Self::Theme => "preferences-theme",
            Self::Size => "preferences-text-size",
            Self::Sound => "preferences-sound",
        }
    }
}

#[derive(Default)]
pub(super) struct Preferences {
    preferences: Option<UiPreferences>,
    subscription: Option<UiPreferenceSubscription>,
    pending: Option<Rc<Cell<Option<UiPreferenceSnapshot>>>>,
    snapshot: Option<UiPreferenceSnapshot>,
    failed: bool,
    parking: Option<Control>,
    last_appearance: Control,
    /// Palette and text scale of `snapshot`, computed once per change and lent by reference
    /// while drawing.
    look: Look,
}

struct Look {
    palette: Palette,
    scale: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            palette: Palette::for_theme("dark"),
            scale: 1.0,
        }
    }
}

impl Preferences {
    pub(super) fn setup(&mut self, wake: ::rune::Waker) {
        self.close();
        let result = (|| {
            let preferences = UiPreferences::new()?;
            let pending = Rc::new(Cell::new(None));
            let changed = Rc::clone(&pending);
            let subscription = preferences.subscribe_rust(move |snapshot| {
                changed.set(Some(snapshot));
                wake();
            })?;
            Ok::<_, JsValue>((preferences, subscription, pending))
        })();
        match result {
            Ok((preferences, subscription, pending)) => {
                self.preferences = Some(preferences);
                self.subscription = Some(subscription);
                self.pending = Some(pending);
                self.failed = false;
            }
            Err(_) => self.failed = true,
        }
    }

    pub(super) fn poll(&mut self) -> bool {
        let next = self.pending.as_ref().and_then(|pending| pending.take());
        let changed = next.is_some_and(|snapshot| self.snapshot != Some(snapshot));
        let snapshot = next.or(self.snapshot);
        let completed = self.parking.is_some_and(|control| {
            snapshot.is_some_and(|snapshot| {
                !busy(if control == Control::Sound {
                    snapshot.sound_status()
                } else {
                    snapshot.appearance_status()
                })
            })
        });
        if completed {
            self.parking = None;
        }
        if !changed {
            return completed;
        }
        let Some(snapshot) = snapshot else {
            return completed;
        };
        #[cfg(target_arch = "wasm32")]
        if let Some(root) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.document_element())
        {
            if self
                .snapshot
                .is_none_or(|old| old.theme_key() != snapshot.theme_key())
            {
                let _ = root.set_attribute("data-theme", snapshot.theme_key());
                if let Some(root) = root.dyn_ref::<web_sys::HtmlElement>() {
                    let _ = root.style().set_property(
                        "color-scheme",
                        if snapshot.theme_key() == "light" {
                            "light"
                        } else {
                            "dark"
                        },
                    );
                }
            }
            if self
                .snapshot
                .is_none_or(|old| old.size_key() != snapshot.size_key())
            {
                if snapshot.size_key() == "medium" {
                    let _ = root.remove_attribute("data-font-size");
                } else {
                    let _ = root.set_attribute("data-font-size", snapshot.size_key());
                }
            }
        }
        self.snapshot = Some(snapshot);
        self.look = Look {
            palette: Palette::with_key(snapshot.theme_key(), Some(super::theme::in_srgb(snapshot.key_color().lch()))),
            scale: scale(snapshot.size_key()),
        };
        true
    }

    pub(super) fn close(&mut self) {
        self.subscription.take();
        self.preferences.take();
        self.pending.take();
        self.parking = None;
    }

    pub(super) fn snapshot(&self) -> Option<UiPreferenceSnapshot> {
        self.snapshot
    }

    pub(super) fn retry_setup(&mut self, wake: ::rune::Waker, tx: &mut Tx) {
        self.setup(wake);
        if !self.failed {
            self.park(Control::Theme, tx);
        }
    }

    fn park(&mut self, control: Control, tx: &mut Tx) {
        self.parking = Some(control);
        tx.focus(control.status_key());
    }

    pub(super) fn apply(&mut self, action: Action, tx: &mut Tx) {
        if let Action::KeyColor(color) = action {
            if let Some(preferences) = &self.preferences {
                let _ = preferences.set_key_color(color.key().into());
            }
            return;
        }
        let control = match action {
            Action::Theme(_) => {
                self.last_appearance = Control::Theme;
                Control::Theme
            }
            Action::Size(_) => {
                self.last_appearance = Control::Size;
                Control::Size
            }
            Action::RetryAppearance => self.last_appearance,
            Action::Sound(_) | Action::RetrySound => Control::Sound,
            Action::RetrySetup | Action::KeyColor(_) => return,
        };
        self.park(control, tx);
        let Some(preferences) = &self.preferences else {
            return;
        };
        let promise = match action {
            Action::Theme(value) => preferences.set_theme(value.into()),
            Action::Size(value) => preferences.set_font_size(value.into()),
            Action::RetryAppearance => preferences.retry_appearance(),
            Action::Sound(value) => {
                let _ = preferences.set_sounds_enabled(value);
                return;
            }
            Action::RetrySound => {
                let _ = preferences.retry_sound();
                return;
            }
            Action::RetrySetup | Action::KeyColor(_) => return,
        };
        if let Ok(promise) = promise {
            spawn_local(async move {
                let _ = JsFuture::from(promise).await;
            });
        }
    }

    pub(super) fn palette(&self) -> &Palette {
        &self.look.palette
    }


    pub(super) fn typography(&self) -> Typography {
        Typography::new(self.look.scale, &self.look.palette)
    }

    fn notice(
        &self,
        control: Control,
        phase: UiPreferenceStatus,
        message: impl Into<String>,
    ) -> El {
        let parked = self
            .parking
            .is_some_and(|pending| pending.status_key() == control.status_key());
        let notice = self
            .typography()
            .text(message)
            .role(if failed(phase) { "alert" } else { "status" })
            .wrap();
        if parked {
            // Removing this scope restores focus only while focus still belongs to it.
            notice
                .key(control.status_key())
                .focusable(true)
                .tab_index(-1)
                .focus_return(self.parking.unwrap().return_key())
        } else {
            notice.key(if control == Control::Sound {
                "preferences-sound-status"
            } else {
                "preferences-appearance-status"
            })
        }
    }

    pub(super) fn view(&self, action: impl Fn(&mut State, Action, &mut Tx) + 'static) -> El {
        let action = Rc::new(action);
        let typography = self.typography();
        let mut result = col().w_full().gap(6.0);
        if self.failed {
            return result
                .child(
                    typography
                        .text("Preferences could not be initialized.")
                        .wrap(),
                )
                .child(button(
                    typography,
                    "Retry preferences",
                    "preferences-retry",
                    true,
                    Action::RetrySetup,
                    &action,
                ));
        }
        let Some(snapshot) = self.snapshot else {
            return result.child(self.notice(
                Control::Theme,
                UiPreferenceStatus::Loading,
                "Loading preferences.",
            ));
        };
        let appearance = snapshot.appearance_status();
        let enabled = !busy(appearance);
        result = result.child(typography.text("Theme").role("heading"));
        let palette = self.palette();
        let group = |axis| metldr_ui::choice::GroupProps {
            axis,
            disabled: !enabled,
            ..Default::default()
        };
        let themes = [
            ("catppuccin", "Catppuccin", "preferences-theme-catppuccin"),
            ("gruvbox", "Gruvbox", "preferences-theme-gruvbox"),
            ("cyberpunk", "Cyberpunk", "preferences-theme-cyberpunk"),
            ("light", "Light", "preferences-theme-light"),
            ("hardware", "Hardware", "preferences-theme-hardware"),
        ]
        .into_iter()
        .map(|(key, label, id)| option(typography, label, id, snapshot.theme_key() == key, enabled, Action::Theme(key), &action));
        result = result.child(
            metldr_ui::choice::radio_group(col().key("preferences-theme").w_full().gap(6.0), "Theme", group(::rune::FocusAxis::Vertical))
                .children(themes),
        );
        if palette.molded() {
            result = result.child(typography.text("Key colour").role("heading"));
            let current = snapshot.key_color();
            let keys = crate::preferences::KeyColor::ALL
                .into_iter()
                .map(|color| swatch(palette, color, color == current, enabled, &action));
            result = result.child(
                metldr_ui::choice::radio_group(row().key("preferences-key-colour").w_full().gap(8.0).flex_wrap(), "Key colour", group(::rune::FocusAxis::Horizontal))
                    .children(keys),
            );
        }
        result = result.child(typography.text("Text size").role("heading"));
        let sizes = [
            ("small", "Small", "preferences-size-small"),
            ("medium", "Medium", "preferences-size-medium"),
            ("large", "Large", "preferences-size-large"),
        ]
        .into_iter()
        .map(|(key, label, id)| segment(typography, palette, label, id, snapshot.size_key() == key, enabled, Action::Size(key), &action));
        let sizes = metldr_ui::choice::radio_group(
            ::rune::row().key("preferences-text-size").w_full().padding(2.0).gap(2.0).radius(10.0).fill(typography.field),
            "Text size",
            group(::rune::FocusAxis::Both),
        )
        .children(sizes);
        result = result.child(sizes).child(self.notice(
            Control::Theme,
            appearance,
            status(true, appearance),
        ));
        if failed(appearance) {
            result = result.child(button(
                typography,
                "Retry appearance settings",
                "preferences-retry-appearance",
                true,
                Action::RetryAppearance,
                &action,
            ));
        }
        let sound = snapshot.sound_status();
        result = result
            .child(sound_switch(typography, palette, snapshot.sounds_enabled(), !busy(sound), &action))
            .child(self.notice(Control::Sound, sound, status(false, sound)));
        if failed(sound) {
            result = result.child(button(
                typography,
                "Retry sound settings",
                "preferences-retry-sound",
                true,
                Action::RetrySound,
                &action,
            ));
        }
        result
    }
}

fn busy(status: UiPreferenceStatus) -> bool {
    matches!(
        status,
        UiPreferenceStatus::Loading | UiPreferenceStatus::Saving
    )
}
fn failed(status: UiPreferenceStatus) -> bool {
    matches!(
        status,
        UiPreferenceStatus::LoadFailed | UiPreferenceStatus::SaveFailed
    )
}
/// A preference domain's status line, spelled out so a frame formats nothing.
fn status(appearance: bool, status: UiPreferenceStatus) -> &'static str {
    match (appearance, status) {
        (true, UiPreferenceStatus::Loading) => "Loading Appearance preferences.",
        (true, UiPreferenceStatus::Saving) => "Saving Appearance preferences.",
        (true, UiPreferenceStatus::Saved) => "Appearance preferences saved.",
        (true, UiPreferenceStatus::LoadFailed) => {
            "Appearance preferences could not be loaded. Choose a setting or retry."
        }
        (true, UiPreferenceStatus::SaveFailed) => {
            "Appearance preferences could not be saved. The current setting remains active. Retry to save it."
        }
        (false, UiPreferenceStatus::Loading) => "Loading Sound preferences.",
        (false, UiPreferenceStatus::Saving) => "Saving Sound preferences.",
        (false, UiPreferenceStatus::Saved) => "Sound preferences saved.",
        (false, UiPreferenceStatus::LoadFailed) => {
            "Sound preferences could not be loaded. Choose a setting or retry."
        }
        (false, UiPreferenceStatus::SaveFailed) => {
            "Sound preferences could not be saved. The current setting remains active. Retry to save it."
        }
    }
}

pub(super) fn outlined(typography: Typography, label: impl Into<String>, key: impl Into<String>) -> El {
    let label = label.into();
    keyed(typography, typography.legend(&label).label(label).key(key.into()))
}

/// `el` as an outlined control on the theme's material: a key on its lip, a neon key, or a field.
pub(super) fn keyed(typography: Typography, el: El) -> El {
    let el = el.px(10.0).py(6.0).radius(8.0).fill(typography.field);
    match (typography.lip, typography.neon) {
        (Some(lip), _) => super::theme::on_lip(el, lip, false),
        (None, Some(accent)) => super::theme::neon_key(el, accent),
        (None, None) => el,
    }
}

fn button(
    typography: Typography,
    label: impl Into<String>,
    key: impl Into<String>,
    enabled: bool,
    value: Action,
    action: &Rc<impl Fn(&mut State, Action, &mut Tx) + 'static>,
) -> El {
    let press = Rc::clone(action);
    let el = metldr_ui::button::button(
        outlined(typography, label, key).border(1.0, typography.line),
        metldr_ui::button::Props {
            disabled: !enabled,
            ..Default::default()
        },
        move |state: &mut State, tx| {
            if matches!(value, Action::RetrySetup) {
                state.sound(crate::interaction_audio::Cue::Press);
            }
            press(state, value, tx)
        },
    );
    if !enabled {
        el.opacity(0.5)
    } else if typography.lip.is_some() {
        el.press(typography.press)
    } else {
        el.press(typography.press).press_scale(super::theme::PRESS_SCALE)
    }
}

/// One option of a setting's radio group, outlined more strongly while checked.
fn option(
    typography: Typography,
    label: impl Into<String>,
    key: impl Into<String>,
    checked: bool,
    enabled: bool,
    value: Action,
    action: &Rc<impl Fn(&mut State, Action, &mut Tx) + 'static>,
) -> El {
    let press = Rc::clone(action);
    let el = outlined(typography, label, key).border(if checked { 1.5 } else { 1.0 }, if checked { typography.ink } else { typography.line });
    let el = metldr_ui::choice::radio(el, !enabled, checked, move |state: &mut State, tx| press(state, value, tx));
    if enabled { el.press(typography.press) } else { el.opacity(0.5) }
}

/// One key colour: a key cap in that colour, ringed in ink while chosen.
fn swatch(
    palette: &Palette,
    color: crate::preferences::KeyColor,
    checked: bool,
    enabled: bool,
    action: &Rc<impl Fn(&mut State, Action, &mut Tx) + 'static>,
) -> El {
    let press = Rc::clone(action);
    let swatch = super::theme::swatch(color.lch());
    let key = ::rune::el()
        .key(format!("preferences-key-{}", color.key()))
        .label(color.label())
        .size(28.0, 28.0)
        .radius(6.0)
        .fill(swatch)
        .border(if checked { 2.0 } else { 1.0 }, if checked { palette.ink } else { palette.line });
    let key = super::theme::on_lip(key, super::theme::key_lip(swatch), false);
    let key = metldr_ui::choice::radio(key, !enabled, checked, move |state: &mut State, tx| press(state, Action::KeyColor(color), tx));
    if enabled { key } else { key.opacity(0.5) }
}

/// One segment of a segmented setting: its label over the indicator while checked.
#[allow(clippy::too_many_arguments)]
fn segment(
    typography: Typography,
    palette: &Palette,
    label: &'static str,
    key: &'static str,
    checked: bool,
    enabled: bool,
    value: Action,
    action: &Rc<impl Fn(&mut State, Action, &mut Tx) + 'static>,
) -> El {
    let press = Rc::clone(action);
    let mut el = ::rune::row().key(key).grow(1.0).min_h(28.0).items_center().justify_center().radius(8.0).label(label);
    if checked {
        let indicator = metldr_ui::choice::indicator("preferences-text-size")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(8.0)
            .fill(palette.surface)
            .border(1.0, palette.line);
        // A molded indicator slides to its segment in 260 ms, as the theme's detents do.
        el = el.child(if palette.molded() { indicator.motion(::rune::Spring::from_motion(0.26, 0.0)) } else { indicator });
    }
    let el = el.child(typography.legend(label).color(if checked { palette.ink } else { palette.ink_2 }));
    let el = metldr_ui::choice::radio(el, !enabled, checked, move |state: &mut State, tx| press(state, value, tx));
    if enabled { el.press(typography.press) } else { el.opacity(0.5) }
}

/// The interaction sounds setting as a switch.
fn sound_switch(
    typography: Typography,
    palette: &Palette,
    on: bool,
    enabled: bool,
    action: &Rc<impl Fn(&mut State, Action, &mut Tx) + 'static>,
) -> El {
    let press = Rc::clone(action);
    switch(typography, palette, "preferences-sound", "Interaction sounds", on, enabled, move |state: &mut State, on, tx| {
        press(state, Action::Sound(on), tx)
    })
}

/// A setting as a switch: its label, then a track whose thumb sits right while on.
pub(super) fn switch(
    typography: Typography,
    palette: &Palette,
    key: &str,
    label: &str,
    on: bool,
    enabled: bool,
    on_change: impl Fn(&mut State, bool, &mut Tx) + 'static,
) -> El {
    // The thumb slides by an offset, so a toggle repaints the switch without laying out its row.
    // A molded switch is a slide switch: a key on its lip that snaps across on a detent.
    let (track_w, thumb_w, thumb_h, travel) = if palette.molded() { (44.0, 20.0, 16.0, 20.0) } else { (28.0, 12.0, 12.0, 12.0) };
    let thumb = ::rune::el()
        .size(thumb_w, thumb_h)
        .radius(if palette.molded() { 2.0 } else { 6.0 })
        .fill(if on || palette.molded() { palette.surface } else { palette.ink_3 })
        .offset(if on { travel } else { 0.0 }, 0.0);
    let thumb = if palette.molded() { super::theme::detent(thumb.shadow(2.0, 0.0, palette.lip)) } else { thumb };
    let track = ::rune::row()
        .size(track_w, thumb_h + 4.0)
        .padding(2.0)
        .radius(if palette.molded() { 3.0 } else { 8.0 })
        .fill(if on { palette.accent } else { palette.field })
        .border(1.0, if on { palette.accent } else { palette.line })
        .child(thumb);
    let el = ::rune::row()
        .key(key.to_owned())
        .label(label.to_owned())
        .w_full()
        .min_h(32.0)
        .gap(8.0)
        .items_center()
        .child(typography.legend(label).grow(1.0))
        .child(track);
    let el = metldr_ui::choice::switch(el, !enabled, on, on_change);
    if enabled { el } else { el.opacity(0.5) }
}

fn scale(font_size: &str) -> f32 {
    match font_size {
        "small" => 0.9375,
        "large" => 1.125,
        _ => 1.0,
    }
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn live_messages_preserve_roles_across_updates() {
        fn view(phase: &usize, _: &mut Cx<usize>) -> El {
            ::rune::text(["Saving", "Save failed", "Saved"][*phase])
                .key("notice")
                .role(["status", "alert", "text"][*phase])
        }
        let mut rt = ::rune::Runtime::new(0, view, (480.0, 320.0), ::rune::Env::default());
        for (phase, role) in ["status", "alert", "text"].into_iter().enumerate() {
            rt.update(|state, _| *state = phase);
            rt.frame(0.0);
            let items: Vec<serde_json::Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0]["k"], role);
            assert_eq!(items[0]["t"], ["Saving", "Save failed", "Saved"][phase]);
        }
    }

    #[test]
    fn preference_status_returns_focus_without_overriding_another_control() {
        fn view(state: &State, _: &mut Cx<State>) -> El {
            let waiting = state.preferences.parking.is_some();
            col()
                .w_full()
                .child(
                    col().key("preferences-theme").child(
                        text("Selected theme")
                            .key("theme-choice")
                            .role("button")
                            .focusable(!waiting)
                            .disabled(waiting)
                            .focus_selected(true),
                    ),
                )
                .child(state.preferences.notice(
                    Control::Theme,
                    if waiting {
                        UiPreferenceStatus::Saving
                    } else {
                        UiPreferenceStatus::Saved
                    },
                    "Appearance status",
                ))
                .child(
                    text("Other control")
                        .key("other")
                        .role("button")
                        .focusable(true),
                )
        }
        fn focused(runtime: &::rune::Runtime<State>) -> Option<String> {
            runtime
                .elements()
                .into_iter()
                .find(|element| element.focused)
                .and_then(|element| element.key)
        }
        for moved in [false, true] {
            let mut runtime = ::rune::Runtime::new(
                State::default(),
                view,
                (400.0, 400.0),
                ::rune::Env::default(),
            );
            runtime.frame(0.0);
            runtime.update(|state, tx| state.preferences.park(Control::Theme, tx));
            runtime.frame(0.0);
            assert_eq!(
                focused(&runtime).as_deref(),
                Some(Control::Theme.status_key())
            );
            if moved {
                runtime.update(|_, tx| tx.focus("other"));
                runtime.frame(0.0);
            }
            runtime.update(|state, _| state.preferences.parking = None);
            runtime.frame(0.0);
            assert_eq!(
                focused(&runtime).as_deref(),
                Some(if moved { "other" } else { "theme-choice" })
            );
        }
    }
}
