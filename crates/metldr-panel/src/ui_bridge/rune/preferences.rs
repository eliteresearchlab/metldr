use super::*;
use crate::preferences::{
    UiPreferenceSnapshot, UiPreferenceStatus, UiPreferenceSubscription, UiPreferences,
};
use ::rune::{Color, El, TextStyle, col, oklch, text};
use wasm_bindgen_futures::JsFuture;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Theme(&'static str),
    Size(&'static str),
    Sound(bool),
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
                .is_none_or(|old| old.theme() != snapshot.theme())
            {
                let _ = root.set_attribute("data-theme", &snapshot.theme());
                if let Some(root) = root.dyn_ref::<web_sys::HtmlElement>() {
                    let _ = root.style().set_property(
                        "color-scheme",
                        if snapshot.theme() == "light" {
                            "light"
                        } else {
                            "dark"
                        },
                    );
                }
            }
            if self
                .snapshot
                .is_none_or(|old| old.font_size() != snapshot.font_size())
            {
                if snapshot.font_size() == "medium" {
                    let _ = root.remove_attribute("data-font-size");
                } else {
                    let _ = root.set_attribute("data-font-size", &snapshot.font_size());
                }
            }
        }
        self.snapshot = Some(snapshot);
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
            Action::RetrySetup => return,
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
            Action::RetrySetup => return,
        };
        if let Ok(promise) = promise {
            spawn_local(async move {
                let _ = JsFuture::from(promise).await;
            });
        }
    }

    pub(super) fn palette(&self) -> Palette {
        Palette::for_theme(
            self.snapshot
                .map(|s| s.theme())
                .as_deref()
                .unwrap_or("catppuccin"),
        )
    }

    pub(super) fn text_scale(&self) -> f32 {
        match self.snapshot.map(|s| s.font_size()).as_deref() {
            Some("small") => 0.9375,
            Some("large") => 1.125,
            _ => 1.0,
        }
    }

    pub(super) fn typography(&self) -> Typography {
        Typography {
            scale: self.text_scale(),
            ink: self.palette().ink,
        }
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
        let mut result = col()
            .w_full()
            .gap(6.0)
            .child(typography.text("Appearance and sound").role("heading"));
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
        let mut themes = col().key("preferences-theme").w_full().gap(6.0);
        for (key, label) in [
            ("catppuccin", "Catppuccin"),
            ("gruvbox", "Gruvbox"),
            ("cyberpunk", "Cyberpunk"),
            ("light", "Light"),
        ] {
            let label = format!(
                "{}{}",
                label,
                if snapshot.theme() == key {
                    " (selected)"
                } else {
                    ""
                }
            );
            themes = themes.child(
                button(
                    typography,
                    label,
                    format!("preferences-theme-{key}"),
                    enabled,
                    Action::Theme(key),
                    &action,
                )
                .focus_selected(snapshot.theme() == key),
            );
        }
        result = result.child(themes);
        result = result.child(typography.text("Text size").role("heading"));
        let mut sizes = col().key("preferences-text-size").w_full().gap(6.0);
        for (key, label) in [("small", "Small"), ("medium", "Medium"), ("large", "Large")] {
            let label = format!(
                "{}{}",
                label,
                if snapshot.font_size() == key {
                    " (selected)"
                } else {
                    ""
                }
            );
            sizes = sizes.child(
                button(
                    typography,
                    label,
                    format!("preferences-size-{key}"),
                    enabled,
                    Action::Size(key),
                    &action,
                )
                .focus_selected(snapshot.font_size() == key),
            );
        }
        result = result.child(sizes).child(self.notice(
            Control::Theme,
            appearance,
            status("Appearance", appearance),
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
            .child(button(
                typography,
                if snapshot.sounds_enabled() {
                    "Interaction sounds: on"
                } else {
                    "Interaction sounds: off"
                },
                "preferences-sound",
                !busy(sound),
                Action::Sound(!snapshot.sounds_enabled()),
                &action,
            ))
            .child(self.notice(Control::Sound, sound, status("Sound", sound)));
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
fn status(domain: &str, status: UiPreferenceStatus) -> String {
    match status {
        UiPreferenceStatus::Loading => format!("Loading {domain} preferences."),
        UiPreferenceStatus::Saving => format!("Saving {domain} preferences."),
        UiPreferenceStatus::Saved => format!("{domain} preferences saved."),
        UiPreferenceStatus::LoadFailed => {
            format!("{domain} preferences could not be loaded. Choose a setting or retry.")
        }
        UiPreferenceStatus::SaveFailed => format!(
            "{domain} preferences could not be saved. The current setting remains active. Retry to save it."
        ),
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
    let label = label.into();
    let element = typography
        .text(label.clone())
        .label(label)
        .key(key.into())
        .role("button")
        .focusable(true)
        .disabled(!enabled)
        .padding(6.0);
    if !enabled {
        return element;
    }
    let press = Rc::clone(action);
    let key_action = Rc::clone(action);
    element
        .press(oklch(0.7, 0.08, 250.0).alpha(0.2))
        .on_press_tx(move |state: &mut State, tx| {
            if matches!(value, Action::RetrySetup) {
                state.sound(crate::interaction_audio::Cue::Press);
            }
            press(state, value, tx)
        })
        .on_key(move |state: &mut State, key, tx| {
            if key.key != ::rune::Key::Enter {
                return false;
            }
            if !tx.key_repeat() {
                if matches!(value, Action::RetrySetup) {
                    state.sound(crate::interaction_audio::Cue::Press);
                }
                key_action(state, value, tx);
            }
            true
        })
}

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub(super) page: Color,
    pub(super) canvas: Color,
    pub(super) surface: Color,
    pub(super) field: Color,
    pub(super) hover: Color,
    pub(super) hover_2: Color,
    pub(super) ink: Color,
    pub(super) ink_2: Color,
    pub(super) ink_3: Color,
    pub(super) line: Color,
}

#[derive(Clone, Copy)]
pub(super) struct Typography {
    pub(super) scale: f32,
    pub(super) ink: Color,
}

impl Typography {
    pub(super) fn font(self, size: f32, weight: f32) -> TextStyle {
        TextStyle::new(size * self.scale, weight)
    }

    pub(super) fn text(self, value: impl Into<String>) -> El {
        text(value).font(self.font(13.0, 400.0)).color(self.ink)
    }
}
impl Palette {
    fn for_theme(theme: &str) -> Self {
        let (page, canvas, surface, field, hover, hover_2, ink, ink_2, ink_3, line) = match theme {
            "light" => (
                (0.985, 0.001, 286.376),
                (0.961, 0.002, 247.84),
                (1.0, 0.0, 0.0),
                (0.961, 0.001, 286.375),
                (0.97, 0.002, 247.839),
                (0.933, 0.003, 247.86),
                (0.247, 0.006, 258.361),
                (0.506, 0.01, 264.477),
                (0.695, 0.009, 264.505),
                (0.946, 0.003, 264.542),
            ),
            "gruvbox" => (
                (0.24, 0.025, 75.0),
                (0.26, 0.025, 75.0),
                (0.3, 0.03, 75.0),
                (0.33, 0.03, 75.0),
                (0.33, 0.03, 75.0),
                (0.36, 0.03, 75.0),
                (0.91, 0.04, 85.0),
                (0.74, 0.045, 80.0),
                (0.65, 0.045, 80.0),
                (0.38, 0.03, 75.0),
            ),
            "cyberpunk" => (
                (0.17, 0.045, 290.0),
                (0.19, 0.045, 290.0),
                (0.23, 0.06, 290.0),
                (0.26, 0.06, 290.0),
                (0.26, 0.06, 290.0),
                (0.29, 0.06, 290.0),
                (0.94, 0.04, 195.0),
                (0.75, 0.08, 205.0),
                (0.65, 0.08, 205.0),
                (0.31, 0.06, 290.0),
            ),
            _ => (
                (0.209, 0.004, 264.477),
                (0.231, 0.004, 264.487),
                (0.26, 0.006, 271.191),
                (0.293, 0.006, 271.223),
                (0.289, 0.006, 271.22),
                (0.318, 0.007, 274.747),
                (0.964, 0.002, 247.839),
                (0.731, 0.008, 260.731),
                (0.541, 0.01, 264.484),
                (0.308, 0.006, 258.354),
            ),
        };
        let color = |(l, c, h)| oklch(l, c, h);
        Self {
            page: color(page),
            canvas: color(canvas),
            surface: color(surface),
            field: color(field),
            hover: color(hover),
            hover_2: color(hover_2),
            ink: color(ink),
            ink_2: color(ink_2),
            ink_3: color(ink_3),
            line: color(line),
        }
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
