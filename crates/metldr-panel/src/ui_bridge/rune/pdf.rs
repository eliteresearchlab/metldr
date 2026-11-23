/*!
 * SVG icon paths from Lucide, copyright (c) for portions of Lucide are held by Cole Bemis
 * 2013-2022 as part of Feather (MIT); all other copyright (c) for Lucide are held by Lucide
 * Contributors 2022.
 *
 * Permission to use, copy, modify, and/or distribute this software for any purpose with or
 * without fee is hereby granted, provided that the above copyright notice and this permission
 * notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
 * SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE
 * AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT,
 * NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
 * PERFORMANCE OF THIS SOFTWARE.
 */

//! The PDF tools: a small bar over a PDF tab, in an in-page frame (`inpage/pdf.html`, see
//! [`super::inpage`]), that summarizes the PDF or opens the chat on it. The frame's page reads the
//! PDF and talks to the extension; this draws what it reports and names the action pressed.

use super::State;
use super::composer::CHECK;
use super::conversations::{CLOSE, SUMMARY};
use super::inpage::{self, MARGIN};
use metldr_ui::theme::{Palette, Typography, card_shadow, pop_in};
use ::rune::{Cx, El, Icon, Key, KeyEvent, Tx, col, row};
use serde::Deserialize;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

/// The summary card's width in points.
const WIDTH: f32 = 340.0;

/// Chat: a speech bubble (Lucide message-circle).
const MESSAGE: Icon = Icon {
    name: "metldr-message-circle",
    view: 24.0,
    body: "<path d=\"M7.9 20A9 9 0 1 0 4 16.1L2 22Z\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.9\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
};
/// Copy: two sheets (Lucide copy).
const COPY: Icon = Icon {
    name: "metldr-copy",
    view: 24.0,
    body: "<rect width=\"14\" height=\"14\" x=\"8\" y=\"8\" rx=\"2\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.9\"/><path d=\"M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.9\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
};

/// What the frame's page reports, as `{phase, ...}` JSON.
#[derive(Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
enum Phase {
    Idle,
    Reading,
    Summarizing,
    /// A PDF on this computer, which only the reader can hand over.
    Choose,
    Summary { bullets: Vec<String> },
    Failed { error: String },
}

/// The PDF tools' own part of the state.
pub(super) struct Tools {
    palette: Palette,
    typography: Typography,
    phase: Phase,
    copied: bool,
}

impl Tools {
    pub(super) fn retheme(&mut self, palette: Palette, typography: Typography) {
        (self.palette, self.typography) = (palette, typography);
    }

    fn busy(&self) -> bool {
        matches!(self.phase, Phase::Reading | Phase::Summarizing)
    }
}

thread_local! {
    /// The frame page's `act(name)`: "summarize", "chat", "choose" or "copy".
    static ACT: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

fn act(name: &str) {
    ACT.with_borrow(|act| act.as_ref().map(|act| act.call1(&JsValue::NULL, &name.into())));
}

fn init() -> State {
    let (palette, typography) = inpage::palette();
    let mut state = State::default();
    state.pdf_tools = Some(Box::new(Tools { palette, typography, phase: Phase::Idle, copied: false }));
    state
}

fn tools(state: &mut State) -> Option<&mut Tools> {
    state.pdf_tools.as_deref_mut()
}

/// A labelled key of the bar, or an icon-only one when `text` is empty.
fn key_el(t: Typography, p: &Palette, key: &'static str, icon: &'static Icon, text: &'static str, label: &'static str) -> El {
    let mut out = row()
        .key(key)
        .h(32.0)
        .px(if text.is_empty() { 8.0 } else { 10.0 })
        .gap(6.0)
        .center_y()
        .radius(8.0)
        .child(::rune::el::icon(icon, 15.0).color(p.ink_2))
        .hover(p.hover)
        .press(p.line_strong)
        .press_scale(metldr_ui::theme::PRESS_SCALE)
        .label(label);
    if !text.is_empty() {
        out = out.child(t.text(text).font(t.font(13.0, 500.0)).color(p.ink));
    }
    out
}

fn bar(s: &Tools) -> El {
    let (p, t) = (&s.palette, s.typography);
    let props = metldr_ui::button::Props { disabled: s.busy(), focusable_when_disabled: true };
    let summarize = metldr_ui::button::button(
        key_el(t, p, "pdf-summarize", &SUMMARY, "Summarize", "Summarize this PDF").busy(s.busy()),
        props,
        |_: &mut State, _| act("summarize"),
    );
    let chat = metldr_ui::button::button(
        key_el(t, p, "pdf-chat", &MESSAGE, "Chat", "Chat about this PDF"),
        Default::default(),
        |_: &mut State, _| act("chat"),
    );
    let bar = row().gap(2.0).padding(4.0).radius(12.0).fill(p.surface).border(1.0, p.line).child(summarize).child(chat);
    let bar = metldr_ui::toolbar::toolbar(bar, "PDF tools", Default::default());
    card_shadow(bar, p)
}

fn note(t: Typography, text: &str, color: ::rune::Color) -> El {
    t.text(text.to_owned()).font(t.font(13.0, 500.0)).color(color).w_full().wrap()
}

/// The card above the bar: progress, the summary, the file to choose or what went wrong.
fn card(s: &Tools) -> Option<El> {
    let (p, t) = (&s.palette, s.typography);
    let body = match &s.phase {
        Phase::Idle => return None,
        Phase::Reading => note(t, "Reading the PDF\u{2026}", p.ink_3).role("status"),
        Phase::Summarizing => note(t, "Summarizing\u{2026}", p.ink_3).role("status"),
        Phase::Failed { error } => note(t, error, p.red).role("alert"),
        Phase::Choose => {
            let choose = row()
                .key("pdf-choose")
                .h(32.0)
                .px(12.0)
                .center_y()
                .radius(8.0)
                .fill(p.ink)
                .press(p.press_solid)
                .press_scale(metldr_ui::theme::PRESS_SCALE)
                .child(t.text("Choose the file").font(t.font(13.0, 600.0)).color(p.surface));
            let choose = metldr_ui::button::button(choose, Default::default(), |_: &mut State, _| act("choose"));
            col()
                .gap(10.0)
                .w_full()
                .child(note(t, "This PDF is on your computer. Choose it to read it here.", p.ink_2))
                .child(choose)
        }
        Phase::Summary { bullets } => {
            let point = |text: &String| {
                row()
                    .w_full()
                    .gap(8.0)
                    .child(t.text("\u{2022}").font(t.font(13.0, 600.0)).color(p.accent))
                    .child(t.text(text.clone()).font(t.font(13.0, 400.0).lh(19.5)).grow(1.0).shrink().wrap())
            };
            let list = col().key("pdf-points").w_full().max_h(320.0).gap(8.0).children(bullets.iter().map(point));
            metldr_ui::scroll_area::viewport(list, "Summary", false)
        }
    };
    let title = match s.phase {
        Phase::Summary { .. } => "Summary",
        Phase::Choose => "PDF on this computer",
        _ => "PDF",
    };
    let mut head = row()
        .w_full()
        .gap(4.0)
        .center_y()
        .child(t.text(title).font(t.font(14.0, 650.0)).color(p.ink).role("heading").grow(1.0));
    if matches!(s.phase, Phase::Summary { .. }) {
        let (icon, label) = if s.copied { (&CHECK, "Copied") } else { (&COPY, "Copy summary") };
        let copy = metldr_ui::button::button(key_el(t, p, "pdf-copy", icon, "", label), Default::default(), |state: &mut State, _| {
            act("copy");
            if let Some(tools) = tools(state) {
                tools.copied = true;
            }
        });
        head = head.child(copy);
    }
    let close = metldr_ui::button::button(
        key_el(t, p, "pdf-close", &CLOSE, "", "Close"),
        Default::default(),
        |state: &mut State, _| dismiss(state),
    );
    let card = col()
        .key("pdf-card")
        .w(WIDTH)
        .gap(10.0)
        .px(16.0)
        .py(12.0)
        .radius(12.0)
        .fill(p.surface)
        .border(1.0, p.line)
        .label(title)
        .child(head.child(close))
        .child(body)
        .transition(pop_in(p));
    Some(card_shadow(card, p))
}

fn dismiss(state: &mut State) {
    if let Some(tools) = tools(state)
        && !tools.busy()
    {
        tools.phase = Phase::Idle;
    }
}

fn view(state: &State, _: &mut Cx<State>) -> El {
    let Some(s) = state.pdf_tools.as_deref() else { return col() };
    let tools = col().key("pdf-tools").gap(10.0).items_end();
    let tools = match card(s) {
        Some(card) => tools.child(card),
        None => tools,
    };
    col().padding(MARGIN).child(tools.child(bar(s)))
}

fn key(state: &mut State, event: &KeyEvent, _: &mut Tx) -> bool {
    let open = state.pdf_tools.as_deref().is_some_and(|tools| !matches!(tools.phase, Phase::Idle));
    if event.key != Key::Escape || !open {
        return false;
    }
    dismiss(state);
    true
}

/// Starts the PDF tools in `theme` with the interface `fonts`; `sized(width, height)` hears the
/// frame size they need and `act(name)` each action pressed.
#[wasm_bindgen]
pub fn start_pdf(fonts: js_sys::Array, theme: String, sized: js_sys::Function, act: js_sys::Function) -> Result<(), JsValue> {
    ACT.with_borrow_mut(|stored| *stored = Some(act));
    let frame = inpage::Frame {
        title: "metldr PDF tools",
        size: (WIDTH + 2.0 * MARGIN, 72.0),
        fits: "pdf-tools",
        init,
        view,
        on_key: key,
    };
    inpage::start(frame, fonts, theme, sized, None)
}

/// What the frame's page reports, `{phase: "reading" | "summarizing" | "choose" | "idle"}`,
/// `{phase: "summary", bullets}` or `{phase: "failed", error}`, as JSON.
#[wasm_bindgen]
pub fn pdf_phase(report: String) {
    let phase = serde_json::from_str(&report).unwrap_or_else(|_| Phase::Failed { error: "The PDF tools failed.".into() });
    inpage::update(move |state| {
        if let Some(tools) = tools(state) {
            tools.phase = phase;
            tools.copied = false;
        }
    });
}
