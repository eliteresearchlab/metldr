//! The page summary settings: when a page summarizes on its own, how long a page must be first,
//! and the sites always or never summarized. They live in `chrome.storage.local.summaryPrefs`,
//! which the service worker's summary gate reads (`metldr_core::decide::summary`).

use super::preferences::{Palette, Typography};
use super::theme::ThemeCaret;
use super::*;
use ::rune::{El, TextEdit, col, row};
use metldr_core::decide::summary::Prefs;
use serde_json::json;

const KEY: &str = "summaryPrefs";
/// Each mode as stored, and its label.
const MODES: [(&str, &str); 3] = [("manual", "Manual"), ("smart", "Smart"), ("auto", "Always")];

/// What a save did: nothing yet, saved, or why it could not.
#[derive(Clone, PartialEq)]
enum Saved {
    Idle,
    Done,
    Failed(String),
}

/// What arrived since the last frame: settings read or written, and a save's result.
#[derive(Default)]
struct Inbox {
    prefs: Option<Prefs>,
    saved: Option<bool>,
}

pub(super) struct SummarySettings {
    inbox: Rc<RefCell<Inbox>>,
    /// The settings as stored, once read.
    stored: Option<Prefs>,
    mode: String,
    min_words: TextEdit,
    allow: TextEdit,
    deny: TextEdit,
    saved: Saved,
}

impl Default for SummarySettings {
    fn default() -> Self {
        Self {
            inbox: Rc::default(),
            stored: None,
            mode: Prefs::default().mode,
            min_words: TextEdit::default(),
            allow: TextEdit::default(),
            deny: TextEdit::default(),
            saved: Saved::Idle,
        }
    }
}

/// A list typed one site per line or comma separated.
fn sites(text: &str) -> Vec<String> {
    text.split(['\n', ','])
        .map(str::trim)
        .filter(|site| !site.is_empty())
        .map(str::to_owned)
        .collect()
}

fn local_storage() -> Option<JsValue> {
    crate::chrome_api("storage").and_then(|s| crate::get(&s, "local")).ok()
}

/// Reads the stored settings into a form's inbox.
pub(super) struct Loader(Rc<RefCell<Inbox>>);

impl Loader {
    pub(super) fn load(&self, wake: Option<::rune::Waker>) {
        let inbox = Rc::clone(&self.0);
        spawn_local(async move {
            let Some(local) = local_storage() else { return };
            let stored = crate::await_call(&local, "get", &[KEY.into()]).await.ok();
            let value = stored
                .and_then(|items| crate::get(&items, KEY).ok())
                .and_then(|v| gateway::json_of(&v))
                .unwrap_or_default();
            inbox.borrow_mut().prefs = Some(Prefs::from_stored(&value));
            if let Some(wake) = &wake {
                wake();
            }
        });
    }
}

impl SummarySettings {
    /// Reads the stored settings; `wake` redraws once they arrive.
    pub(super) fn load(&self, wake: Option<::rune::Waker>) {
        self.loader().load(wake);
    }

    /// A handle that reads the stored settings into this form, for a listener to keep.
    pub(super) fn loader(&self) -> Loader {
        Loader(Rc::clone(&self.inbox))
    }

    /// Takes what arrived since the last frame into the form; true when anything did.
    pub(super) fn poll(&mut self) -> bool {
        let Inbox { prefs, saved } = std::mem::take(&mut *self.inbox.borrow_mut());
        match saved {
            Some(true) => self.saved = Saved::Done,
            Some(false) => self.saved = Saved::Failed("The settings could not be saved.".into()),
            None => {}
        }
        let Some(prefs) = prefs else { return saved.is_some() };
        self.mode.clone_from(&prefs.mode);
        let words = format!("{}", prefs.min_auto_words);
        for (edit, text) in [(&mut self.min_words, words), (&mut self.allow, prefs.allowlist.join("\n")), (&mut self.deny, prefs.denylist.join("\n"))] {
            if edit.text != text {
                edit.replace_text(&text);
            }
        }
        self.stored = Some(prefs);
        true
    }

    /// The form as settings to store, or why it cannot be.
    fn prefs(&self) -> Result<Prefs, String> {
        let words = self.min_words.text.trim();
        let min_auto_words = match words.parse::<f64>() {
            Ok(n) if n.is_finite() && n >= 0.0 => n.round(),
            _ => return Err("Enter a number of words.".into()),
        };
        let base = self.stored.clone().unwrap_or_default();
        Ok(Prefs {
            mode: self.mode.clone(),
            allowlist: sites(&self.allow.text),
            denylist: sites(&self.deny.text),
            min_auto_words,
            min_prompt_words: base.min_prompt_words,
        })
    }

    fn save(&mut self, prefs: Prefs, wake: Option<::rune::Waker>) {
        self.saved = Saved::Idle;
        let inbox = Rc::clone(&self.inbox);
        spawn_local(async move {
            let items = crate::flow::json_value(&json!({KEY: &prefs})).ok();
            let written = match (local_storage(), items) {
                (Some(local), Some(items)) => crate::await_call(&local, "set", &[items]).await.is_ok(),
                _ => false,
            };
            let mut inbox = inbox.borrow_mut();
            inbox.saved = Some(written);
            if written {
                inbox.prefs = Some(prefs);
            }
            drop(inbox);
            if let Some(wake) = &wake {
                wake();
            }
        });
    }

    pub(super) fn view(&self, typography: Typography, palette: &Palette) -> El {
        let modes = MODES.iter().map(|(value, label)| {
            let checked = self.mode == *value;
            let el = super::preferences::outlined(typography, *label, format!("summary-mode-{value}"))
                .border(if checked { 1.5 } else { 1.0 }, if checked { typography.ink } else { typography.line });
            metldr_ui::choice::radio(el, false, checked, move |state: &mut State, _: &mut Tx| {
                state.sound(crate::interaction_audio::Cue::Press);
                state.page_tools.summaries.mode = (*value).to_owned();
                state.page_tools.summaries.saved = Saved::Idle;
            })
            .press(typography.press)
        });
        let mode = metldr_ui::choice::radio_group(
            row().key("summary-mode").gap(8.0).flex_wrap().children(modes),
            "Summarize pages",
            Default::default(),
        );
        let explain = match self.mode.as_str() {
            "auto" => "Every page long enough is summarized when it opens.",
            "smart" => "Articles long enough are summarized when they open; other pages ask first.",
            _ => "Pages are summarized only when you ask.",
        };
        let field = |key: &str, label: &'static str, edit: &TextEdit, lines: bool, edit_of: fn(&mut State) -> &mut TextEdit| {
            let input = ::rune::el::text_input(edit, typography.input())
                .key(key.to_owned())
                .label(label)
                .w_full()
                .min_h(if lines { 96.0 } else { 36.0 })
                .padding(6.0)
                .color(palette.ink)
                .caret(palette.ink, palette.selection)
                .theme_caret(palette)
                .on_edit(edit_of)
                .on_input_change(|state: &mut State, _| state.page_tools.summaries.saved = Saved::Idle);
            // A list scrolls inside a box of its own height, so a long one never pushes Save away.
            let input = if lines {
                col()
                    .w_full()
                    .h(140.0)
                    .scroll(::rune::ScrollSpec { stick_bottom: false })
                    .fill(palette.field)
                    .border(1.0, palette.line)
                    .child(input)
            } else {
                input.single_line().fill(palette.field).border(1.0, palette.line)
            };
            metldr_ui::field::field(typography.text(label), input, label, Default::default(), |line| typography.text(line.to_owned()))
        };
        let words = field("summary-min-words", "Words a page needs to summarize on its own", &self.min_words, false, |state| {
            &mut state.page_tools.summaries.min_words
        });
        let allow = field("summary-allow", "Sites always summarized, one per line", &self.allow, true, |state| {
            &mut state.page_tools.summaries.allow
        });
        let deny = field("summary-deny", "Sites never summarized, one per line", &self.deny, true, |state| {
            &mut state.page_tools.summaries.deny
        });
        let save = super::preferences::outlined(typography, "Save", "summary-save").border(1.0, typography.line).label("Save summary settings");
        let save = metldr_ui::button::button(save, Default::default(), |state: &mut State, _| {
            state.sound(crate::interaction_audio::Cue::Pulse);
            let wake = state.page_tools.wake();
            let summaries = &mut state.page_tools.summaries;
            match summaries.prefs() {
                Ok(prefs) => summaries.save(prefs, wake),
                Err(error) => summaries.saved = Saved::Failed(error),
            }
        })
        .press(typography.press);
        let reset = super::preferences::outlined(typography, "Reset", "summary-reset").border(1.0, typography.line).label("Reset summary settings");
        let reset = metldr_ui::button::button(reset, Default::default(), |state: &mut State, _| {
            state.sound(crate::interaction_audio::Cue::Release);
            let wake = state.page_tools.wake();
            state.page_tools.summaries.save(Prefs::default(), wake);
        })
        .press(typography.press);
        let mut out = col()
            .key("summary-settings")
            .w_full()
            .gap(8.0)
            .child(typography.text("Page summaries").role("heading"))
            .child(mode)
            .child(typography.text(explain).wrap().color(palette.ink_2))
            .child(words)
            .child(allow)
            .child(deny)
            .child(row().gap(8.0).child(save).child(reset));
        match &self.saved {
            Saved::Idle => {}
            Saved::Done => out = out.child(typography.text("Summary settings saved.").key("summary-saved").role("status")),
            Saved::Failed(why) => out = out.child(typography.text(why.clone()).wrap().key("summary-saved").role("alert")),
        }
        out
    }
}
