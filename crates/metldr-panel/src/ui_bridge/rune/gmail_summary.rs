//! A Gmail thread's summary: a card in the thread, in an in-page frame (`inpage/summary.html`,
//! see [`super::inpage`]). The frame's page asks the extension for the thread's kept summary or a
//! new one; this draws what it reports and names the action pressed.

use super::State;
use super::conversations::SUMMARY;
use super::inpage::{self, MARGIN};
use metldr_ui::theme::{Palette, Typography, card_shadow, pop_in};
use ::rune::{Cx, El, KeyEvent, Tx, col, row};
use serde::Deserialize;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

#[derive(Deserialize)]
struct Summary {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    action_items: Vec<String>,
    #[serde(default)]
    dates: Vec<String>,
    #[serde(default)]
    confidence: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    intent: Option<String>,
}

/// What the frame's page reports, as `{phase, ...}` JSON.
#[derive(Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
enum Phase {
    /// No summary kept: the reader may ask for one.
    Offer,
    Summarizing,
    Ready { summary: Summary },
    Failed { error: String },
}

/// The summary card's own part of the state.
pub(super) struct Card {
    palette: Palette,
    typography: Typography,
    phase: Phase,
}

impl Card {
    pub(super) fn retheme(&mut self, palette: Palette, typography: Typography) {
        (self.palette, self.typography) = (palette, typography);
    }
}

thread_local! {
    /// The frame page's `act(name)`: "summarize" or "again".
    static ACT: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

fn act(name: &str) {
    ACT.with_borrow(|act| act.as_ref().map(|act| act.call1(&JsValue::NULL, &name.into())));
}

fn init() -> State {
    let (palette, typography) = inpage::palette();
    let mut state = State::default();
    state.gmail_summary = Some(Box::new(Card { palette, typography, phase: Phase::Summarizing }));
    state
}

fn action(t: Typography, p: &Palette, key: &'static str, text: &'static str, label: &'static str, name: &'static str) -> El {
    let el = row()
        .key(key)
        .h(30.0)
        .px(12.0)
        .gap(6.0)
        .center_y()
        .radius(8.0)
        .border(1.0, p.line)
        .label(label)
        .hover(p.hover)
        .press(p.line_strong)
        .press_scale(metldr_ui::theme::PRESS_SCALE)
        .child(::rune::el::icon(&SUMMARY, 14.0).color(p.ink_2))
        .child(t.text(text).font(t.font(13.0, 500.0)).color(p.ink));
    metldr_ui::button::button(el, Default::default(), move |_: &mut State, _| act(name))
}

fn note(t: Typography, text: &str, color: ::rune::Color) -> El {
    t.text(text.to_owned()).font(t.font(13.0, 500.0)).color(color).w_full().wrap()
}

fn points(t: Typography, p: &Palette, title: &'static str, items: &[String]) -> Option<El> {
    if items.is_empty() {
        return None;
    }
    let point = |text: &String| {
        row()
            .w_full()
            .gap(8.0)
            .child(t.text("\u{2022}").font(t.font(13.0, 600.0)).color(p.accent))
            .child(t.text(text.clone()).font(t.font(13.0, 400.0).lh(19.5)).grow(1.0).shrink().wrap())
    };
    let head = t.legend(title.to_uppercase()).font(t.font(10.0, 600.0).tracking(0.5)).color(p.ink_3).role("heading");
    Some(col().w_full().gap(4.0).label(title).child(head).children(items.iter().map(point)))
}

fn body(s: &Card) -> El {
    let (p, t) = (&s.palette, s.typography);
    match &s.phase {
        Phase::Offer => row().w_full().child(action(t, p, "summary-ask", "Summarize this email", "Summarize this email", "summarize")),
        Phase::Summarizing => note(t, "Summarizing\u{2026}", p.ink_3).role("status"),
        Phase::Failed { error } => col()
            .w_full()
            .gap(10.0)
            .child(note(t, error, p.red).role("alert"))
            .child(row().child(action(t, p, "summary-retry", "Try again", "Summarize this email again", "again"))),
        Phase::Ready { summary } => {
            let mut head = row().w_full().gap(8.0).center_y().child(t.text("Summary").font(t.font(14.0, 650.0)).color(p.ink).role("heading").grow(1.0));
            if let Some(intent) = summary.intent.as_deref().filter(|i| !i.is_empty()) {
                head = head.child(t.legend(intent.to_uppercase()).font(t.font(10.0, 600.0).tracking(0.5)).color(p.orange));
            }
            let mut out = col().w_full().gap(10.0).child(head);
            if !summary.summary.is_empty() {
                out = out.child(t.text(summary.summary.clone()).font(t.font(13.0, 400.0).lh(19.5)).color(p.ink).w_full().wrap());
            }
            for section in [points(t, p, "Action items", &summary.action_items), points(t, p, "Dates", &summary.dates)].into_iter().flatten() {
                out = out.child(section);
            }
            let about = match (summary.model.as_str(), summary.confidence.as_str()) {
                ("", "") => String::new(),
                (model, "") => model.to_owned(),
                ("", confidence) => format!("{confidence} confidence"),
                (model, confidence) => format!("{model} \u{00b7} {confidence} confidence"),
            };
            let foot = row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(t.text(about).font(t.font(11.0, 500.0)).color(p.ink_3).grow(1.0).shrink().wrap())
                .child(action(t, p, "summary-again", "Summarize again", "Summarize this email again", "again"));
            out.child(foot)
        }
    }
}

fn view(state: &State, _: &mut Cx<State>) -> El {
    let Some(s) = state.gmail_summary.as_deref() else { return col() };
    let p = &s.palette;
    let card = col()
        .key("summary-card")
        .w_full()
        .px(16.0)
        .py(12.0)
        .radius(12.0)
        .fill(p.surface)
        .border(1.0, p.line)
        .label("Email summary")
        .child(body(s))
        .transition(pop_in(p));
    col().w_full().padding(MARGIN).child(card_shadow(card, p))
}

fn key(_: &mut State, _: &KeyEvent, _: &mut Tx) -> bool {
    false
}

/// Starts the summary card in `theme` with the interface `fonts`; `sized(width, height)` hears
/// the frame size it needs and `act(name)` each action pressed.
#[wasm_bindgen]
pub fn start_summary(
    fonts: js_sys::Array,
    theme: String,
    sized: js_sys::Function,
    act: js_sys::Function,
) -> Result<(), JsValue> {
    ACT.with_borrow_mut(|stored| *stored = Some(act));
    let frame = inpage::Frame {
        title: "metldr email summary",
        size: (600.0, 80.0),
        fits: "summary-card",
        init,
        view,
        on_key: key,
    };
    inpage::start(frame, fonts, theme, sized, None)
}

/// What the frame's page reports, `{phase: "offer" | "summarizing"}`, `{phase: "ready",
/// summary}` or `{phase: "failed", error}`, as JSON.
#[wasm_bindgen]
pub fn summary_phase(report: String) {
    let phase = serde_json::from_str(&report).unwrap_or(Phase::Failed { error: "The summary failed.".into() });
    inpage::update(move |state| {
        if let Some(card) = state.gmail_summary.as_deref_mut() {
            card.phase = phase;
        }
    });
}
