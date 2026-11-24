//! Gmail reply suggestions: a card beside a Gmail reply, in an in-page frame
//! (`inpage/replies.html`, see [`super::inpage`]). The frame's page asks the extension for the
//! thread's replies; this draws what it reports, and a pressed reply goes back to the page, which
//! puts it in the reply as plain text.

use super::State;
use super::conversations::CLOSE;
use super::inpage::{self, MARGIN};
use metldr_ui::theme::{Palette, Typography, card_shadow, pop_in};
use ::rune::{Cx, El, Key, KeyEvent, Tx, col, row};
use serde::Deserialize;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

/// The card's width in points.
const WIDTH: f32 = 360.0;

#[derive(Deserialize)]
struct Suggestion {
    #[serde(default)]
    label: String,
    #[serde(default)]
    tone: String,
    body: String,
}

/// What the frame's page reports, as `{phase, ...}` JSON.
#[derive(Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
enum Phase {
    Writing,
    Ready { suggestions: Vec<Suggestion> },
    Empty,
    Failed { error: String },
}

/// The reply card's own part of the state.
pub(super) struct Replies {
    palette: Palette,
    typography: Typography,
    phase: Phase,
}

impl Replies {
    pub(super) fn retheme(&mut self, palette: Palette, typography: Typography) {
        (self.palette, self.typography) = (palette, typography);
    }
}

thread_local! {
    /// The frame page's `act(name)`: "retry", or "insert:<n>" for the n-th reply.
    static ACT: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

fn act(name: &str) {
    ACT.with_borrow(|act| act.as_ref().map(|act| act.call1(&JsValue::NULL, &name.into())));
}

fn init() -> State {
    let (palette, typography) = inpage::palette();
    let mut state = State::default();
    state.gmail_replies = Some(Box::new(Replies { palette, typography, phase: Phase::Writing }));
    state
}

fn note(t: Typography, text: &str, color: ::rune::Color) -> El {
    t.text(text.to_owned()).font(t.font(13.0, 500.0)).color(color).w_full().wrap()
}

fn retry(t: Typography, p: &Palette) -> El {
    let el = row()
        .key("replies-retry")
        .h(32.0)
        .px(12.0)
        .center_y()
        .radius(8.0)
        .border(1.0, p.line)
        .hover(p.hover)
        .press(p.line_strong)
        .press_scale(metldr_ui::theme::PRESS_SCALE)
        .child(t.text("Try again").font(t.font(13.0, 500.0)).color(p.ink));
    metldr_ui::button::button(el, Default::default(), |_: &mut State, _| act("retry"))
}

/// One reply: its tone over its text, pressed to put it in the reply.
fn suggestion(t: Typography, p: &Palette, index: usize, reply: &Suggestion) -> El {
    let tone = if reply.label.is_empty() { &reply.tone } else { &reply.label };
    let mut card = col().key(format!("replies-{index}")).w_full().gap(4.0).px(12.0).py(10.0).radius(10.0).border(1.0, p.line);
    if !tone.is_empty() {
        card = card.child(t.legend(tone.to_uppercase()).font(t.font(10.0, 600.0).tracking(0.5)).color(p.orange));
    }
    let card = card
        .child(t.text(reply.body.clone()).font(t.font(13.0, 400.0).lh(19.5)).color(p.ink).w_full().wrap())
        .hover(p.hover)
        .press(p.line_strong)
        .label(format!("Use this reply: {}", reply.body))
        .press_scale(metldr_ui::theme::PRESS_SCALE);
    metldr_ui::button::button(card, Default::default(), move |_: &mut State, _| act(&format!("insert:{index}")))
}

fn card(s: &Replies) -> El {
    let (p, t) = (&s.palette, s.typography);
    let body = match &s.phase {
        Phase::Writing => note(t, "Writing replies\u{2026}", p.ink_3).role("status"),
        Phase::Empty => col().w_full().gap(10.0).child(note(t, "No replies yet.", p.ink_2)).child(retry(t, p)),
        Phase::Failed { error } => col().w_full().gap(10.0).child(note(t, error, p.red).role("alert")).child(retry(t, p)),
        Phase::Ready { suggestions } => {
            let list = col().key("replies-list").w_full().max_h(360.0).gap(8.0);
            let list = list.children(suggestions.iter().enumerate().map(|(i, reply)| suggestion(t, p, i, reply)));
            metldr_ui::scroll_area::viewport(list, "Reply suggestions", false)
        }
    };
    let close = row()
        .key("replies-close")
        .size(28.0, 28.0)
        .center()
        .radius(8.0)
        .child(::rune::el::icon(&CLOSE, 14.0).color(p.ink_2))
        .hover(p.hover)
        .label("Close");
    let close = metldr_ui::button::button(close, Default::default(), |_: &mut State, _| inpage::close());
    let head = row()
        .w_full()
        .center_y()
        .child(t.text("Reply suggestions").font(t.font(14.0, 650.0)).color(p.ink).role("heading").grow(1.0))
        .child(close);
    let card = col()
        .key("replies-card")
        .w(WIDTH)
        .gap(10.0)
        .px(16.0)
        .py(12.0)
        .radius(12.0)
        .fill(p.surface)
        .border(1.0, p.line)
        .label("Reply suggestions")
        .child(head)
        .child(body)
        .transition(pop_in(p));
    card_shadow(card, p)
}

fn view(state: &State, _: &mut Cx<State>) -> El {
    let Some(s) = state.gmail_replies.as_deref() else { return col() };
    col().padding(MARGIN).child(card(s))
}

fn key(_: &mut State, event: &KeyEvent, _: &mut Tx) -> bool {
    if event.key != Key::Escape {
        return false;
    }
    inpage::close();
    true
}

/// Starts the reply card in `theme` with the interface `fonts`; `sized(width, height)` hears the
/// frame size it needs, `close()` that the reader dismissed it and `act(name)` each action.
#[wasm_bindgen]
pub fn start_replies(
    fonts: js_sys::Array,
    theme: String,
    sized: js_sys::Function,
    close: js_sys::Function,
    act: js_sys::Function,
) -> Result<(), JsValue> {
    ACT.with_borrow_mut(|stored| *stored = Some(act));
    let frame = inpage::Frame {
        title: "metldr reply suggestions",
        size: (WIDTH + 2.0 * MARGIN, 120.0),
        fits: "replies-card",
        init,
        view,
        on_key: key,
    };
    inpage::start(frame, fonts, theme, sized, Some(close))
}

/// What the frame's page reports, `{phase: "writing" | "empty"}`, `{phase: "ready",
/// suggestions}` or `{phase: "failed", error}`, as JSON.
#[wasm_bindgen]
pub fn replies_phase(report: String) {
    let phase = serde_json::from_str(&report).unwrap_or(Phase::Failed { error: "The reply suggestions failed.".into() });
    inpage::update(move |state| {
        if let Some(replies) = state.gmail_replies.as_deref_mut() {
            replies.phase = phase;
        }
    });
}
