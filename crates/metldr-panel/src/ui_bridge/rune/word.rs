//! The word popup: a looked-up word, its senses and synonyms, in an in-page frame
//! (`inpage/word.html`, see [`super::inpage`]) in the theme the reader chose.

use super::State;
use super::inpage::{self, MARGIN};
use metldr_ui::theme::{Palette, Typography, card_shadow, pop_in};
use ::rune::{Cx, El, Key, KeyEvent, Tx, col, row};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

/// The popup's width in points; its height follows its content.
const WIDTH: f32 = 320.0;
/// How many synonyms show before the rest fold into a count.
const SYNONYMS: usize = 4;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sense {
    #[serde(default)]
    definition: String,
    #[serde(default)]
    part_of_speech: String,
    #[serde(default)]
    example: Option<String>,
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    definitions: Vec<Sense>,
    #[serde(default)]
    synonyms: Vec<String>,
    #[serde(default)]
    source: String,
}

enum Phase {
    Looking,
    Found(Found),
    Failed(String),
}

/// The word popup's own part of the state.
pub(super) struct Popup {
    palette: Palette,
    typography: Typography,
    word: String,
    phase: Phase,
}

impl Popup {
    pub(super) fn retheme(&mut self, palette: Palette, typography: Typography) {
        (self.palette, self.typography) = (palette, typography);
    }
}

fn init() -> State {
    let (palette, typography) = inpage::palette();
    let popup = Popup { typography, palette, word: String::new(), phase: Phase::Looking };
    let mut state = State::default();
    state.word_popup = Some(Box::new(popup));
    state
}

/// `text` without the bold and italic marks a model's definition can carry.
fn plain(text: &str) -> String {
    text.replace("**", "").replace("__", "")
}

fn chip(t: Typography, p: &Palette, label: &str) -> El {
    t.legend(label)
        .font(t.font(11.0, 600.0))
        .color(p.ink_2)
        .px(7.0)
        .py(3.0)
        .radius(6.0)
        .fill(p.field)
        .border(1.0, p.line)
}

fn header(s: &Popup, found: Option<&Found>) -> El {
    let (p, t) = (&s.palette, s.typography);
    let word = t.text(s.word.as_str()).font(t.font(15.0, 650.0)).color(p.accent).role("heading");
    let mut out = row().gap(6.0).flex_wrap().child(word);
    let Some(found) = found else { return out };
    for synonym in found.synonyms.iter().take(SYNONYMS) {
        out = out.child(chip(t, p, synonym));
    }
    if found.synonyms.len() > SYNONYMS {
        let rest = &found.synonyms[SYNONYMS..];
        let more = chip(t, p, &format!("+{}", rest.len())).label(format!("More synonyms: {}", rest.join(", ")));
        out = out.child(more);
    }
    // A definition a model wrote, rather than one from a dictionary.
    if !["api", "local", "cache"].contains(&found.source.as_str()) {
        out = out.child(chip(t, p, "ai").label("Written by a model"));
    }
    out
}

fn sense(t: Typography, p: &Palette, sense: &Sense) -> El {
    let pos = if sense.part_of_speech.is_empty() { "unknown" } else { &sense.part_of_speech };
    let mut out = col().w_full().gap(3.0).child(
        t.legend(pos.to_uppercase()).font(t.font(10.0, 600.0).tracking(0.5)).color(p.orange),
    );
    out = out.child(t.text(plain(&sense.definition)).font(t.font(13.0, 400.0).lh(19.5)).w_full().wrap());
    if let Some(example) = sense.example.as_deref().filter(|e| !e.is_empty()) {
        let quoted = format!("\u{201c}{example}\u{201d}");
        out = out.child(t.text(quoted).font(t.font(12.0, 400.0).italic().lh(17.5)).color(p.ink_2).w_full().wrap());
    }
    out
}

fn body(s: &Popup) -> El {
    let (p, t) = (&s.palette, s.typography);
    let note = |text: &str, color| t.text(text.to_owned()).font(t.font(12.0, 500.0)).color(color).w_full().wrap();
    match &s.phase {
        Phase::Looking => note("Looking up\u{2026}", p.ink_3).role("status"),
        Phase::Failed(message) => note(message, p.red).role("alert"),
        Phase::Found(found) if found.definitions.is_empty() => note("No definition found.", p.ink_3),
        Phase::Found(found) => {
            let senses = found.definitions.iter().map(|d| sense(t, p, d));
            let list = col().key("word-senses").w_full().max_h(280.0).gap(12.0).children(senses);
            metldr_ui::scroll_area::viewport(list, "Definitions", false)
        }
    }
}

fn view(state: &State, _: &mut Cx<State>) -> El {
    let Some(s) = state.word_popup.as_deref() else { return col() };
    let p = &s.palette;
    let found = match &s.phase {
        Phase::Found(found) => Some(found),
        _ => None,
    };
    let card = col()
        .key("word-card")
        .w(WIDTH)
        .gap(8.0)
        .px(16.0)
        .py(12.0)
        .radius(12.0)
        .fill(p.surface)
        .border(1.0, p.line)
        .label(format!("Definition of {}", s.word))
        .child(header(s, found))
        .child(body(s))
        .transition(pop_in(p));
    col().padding(MARGIN).child(card_shadow(card, p))
}

fn key(_: &mut State, event: &KeyEvent, _: &mut Tx) -> bool {
    let escape = event.key == Key::Escape;
    if escape {
        inpage::close();
    }
    escape
}

fn update(change: impl FnOnce(&mut Popup) + Send + 'static) {
    inpage::update(move |state| {
        if let Some(popup) = state.word_popup.as_deref_mut() {
            change(popup);
        }
    });
}

/// Starts the popup in `theme` with the interface `fonts`; `sized(width, height)` hears the
/// frame size the card needs, `close()` that the reader dismissed it.
#[wasm_bindgen]
pub fn start_word(fonts: js_sys::Array, theme: String, sized: js_sys::Function, close: js_sys::Function) -> Result<(), JsValue> {
    let frame = inpage::Frame {
        title: "metldr word",
        size: (WIDTH + 2.0 * MARGIN, 160.0),
        fits: "word-card",
        init,
        view,
        on_key: key,
    };
    inpage::start(frame, fonts, theme, sized, Some(close))
}

/// Shows `word` while its lookup runs.
#[wasm_bindgen]
pub fn word_show(word: String) {
    update(move |state| {
        state.word = word;
        state.phase = Phase::Looking;
    });
}

/// The extension's answer to the lookup, `{result}` or `{error}`, as JSON.
#[wasm_bindgen]
pub fn word_result(answer: String) {
    #[derive(Deserialize)]
    struct Answer {
        result: Option<Found>,
        error: Option<String>,
    }
    let phase = match serde_json::from_str::<Answer>(&answer) {
        Ok(Answer { result: Some(found), .. }) => Phase::Found(found),
        Ok(Answer { error: Some(error), .. }) => Phase::Failed(error),
        _ => Phase::Failed("The lookup failed.".into()),
    };
    update(move |state| state.phase = phase);
}
