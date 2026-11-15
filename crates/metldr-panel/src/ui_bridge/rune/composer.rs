/*!
 * SVG icon paths adapted from beautiful-ui PromptBar, copyright (c) 2026 Shane Levine.
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */

use super::theme::MoldedCaret;
use super::theme::Palette;
use super::{Menu, Mode, State};
use ::rune::{Align, Cx, El, Icon, Key, KeyEvent, Side, TextStyle, Tx, col, row};

const CONTROL: f32 = 28.0;
const BORDER: f32 = 1.0;
// Rune's border takes no layout space; include it with the reference's14px padding.
const INSET: f32 = 14.0 + BORDER;
const RADIUS: f32 = 22.0;

pub(super) const PLUS: Icon = Icon {
    name: "metldr-plus",
    view: 24.0,
    body: "<path d=\"M12 5v14M5 12h14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\"/>",
};
pub(super) const CHEVRON: Icon = Icon {
    name: "metldr-chevron",
    view: 24.0,
    body: "<path d=\"m6 9 6 6 6-6\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.4\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
};
const SEND: Icon = Icon {
    name: "metldr-send",
    view: 24.0,
    body: "<path d=\"M12 19V5M5 12l7-7 7 7\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.4\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
};
const STOP: Icon = Icon {
    name: "metldr-stop",
    view: 24.0,
    body: "<rect x=\"6\" y=\"6\" width=\"12\" height=\"12\" rx=\"2\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"/>",
};
const MIC: Icon = Icon {
    name: "metldr-mic",
    view: 24.0,
    body: "<path d=\"M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3zM19 10v2a7 7 0 0 1-14 0v-2M12 19v3\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
};
const CHECK: Icon = Icon {
    name: "metldr-check",
    view: 24.0,
    body: "<path d=\"M20 6 9 17l-5-5\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
};
const PAPERCLIP: Icon = Icon {
    name: "metldr-paperclip",
    view: 24.0,
    body: "<path d=\"m21.4 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
};
const GLOBE: Icon = Icon {
    name: "metldr-globe",
    view: 24.0,
    body: "<circle cx=\"12\" cy=\"12\" r=\"10\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\"/><path d=\"M2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\"/>",
};

fn palette(state: &State) -> &Palette {
    state.preferences.palette()
}

#[derive(Clone, Copy, Debug)]
struct Token {
    marker: char,
    start: usize,
}

fn token(text: &str) -> Option<Token> {
    let start = text
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(0, |(i, ch)| i + ch.len_utf8());
    let marker = text[start..].chars().next()?;
    let query = &text[start + marker.len_utf8()..];
    (matches!(marker, '@' | '/')
        && query
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
    .then_some(Token { marker, start })
}

fn page_matches(text: &str) -> bool {
    token(text).is_none_or(|token| {
        token.marker != '/' && "current page".contains(&text[token.start + 1..].to_lowercase())
    })
}

fn supports_plan(value: &str) -> bool {
    value == "codex" || value.starts_with("codex:")
}

fn close(state: &mut State, _: &mut Tx) {
    state.sound(crate::interaction_audio::Cue::Release);
    state.menu = None;
    state.source_engaged = false;
}

pub(super) fn enter<S>(
    state: &mut S,
    key: &KeyEvent,
    tx: &mut Tx,
    action: fn(&mut S, &mut Tx),
) -> bool {
    if key.key != Key::Enter {
        return false;
    }
    if !tx.key_repeat() {
        action(state, tx);
    }
    true
}

fn sources(state: &mut State, _: &mut Tx) {
    state.sound(crate::interaction_audio::Cue::Pulse);
    state.source_engaged = false;
    state.menu = if state.menu == Some(Menu::Sources) {
        None
    } else {
        Some(Menu::Sources)
    };
}

/// The keys of the model menu's first and last items, in the order the menu shows them.
fn model_ends(state: &State) -> (String, String) {
    let all = state.choices();
    let total = all.len();
    let mut choices: Vec<_> = all.into_iter().filter(|choice| choice.enabled).collect();
    choices.sort_by_key(|choice| choice.short().0);
    let key = |choice: Option<&crate::composer::choices::Choice>| {
        choice.map_or_else(|| "model:setup".to_owned(), |choice| format!("model:{}", choice.value))
    };
    let last = if choices.len() < total { "model:setup".to_owned() } else { key(choices.last()) };
    (key(choices.first()), last)
}

fn close_menu(state: &mut State) {
    state.menu = None;
    state.source_engaged = false;
}

pub(super) fn source_menu_owns_enter(state: &State) -> bool {
    state.menu == Some(Menu::Sources)
        && page_matches(&state.draft.text)
        && !state.capture_disabled()
}

fn capture(state: &mut State, tx: &mut Tx) {
    if state.capture_disabled() {
        return;
    }
    if let Some(token) = token(&state.draft.text) {
        let retained = state.draft.text[..token.start].to_owned();
        state.replace_draft(retained);
    }
    state.source_engaged = false;
    state.capture(tx);
}

fn button(key: &'static str, label: &'static str, icon: &'static Icon, color: ::rune::Color) -> El {
    row()
        .key(key)
        .size(CONTROL, CONTROL)
        .radius(8.0)
        .center()
        .child(::rune::el::icon(icon, 16.0).color(color))
        .role("button")
        .focusable(true)
        .label(label)
}

/// One 30 px menu row: name, muted tag, then the check.
/// The sound a key makes in the composer: a switch going down (deeper for the space bar,
/// heavier for Enter, Backspace, Delete and Tab), a lighter one coming up, and a held key's
/// repeats only ticking. Shortcuts with a modifier and keys that type nothing stay silent.
fn typed(input: &::rune::PhysicalInput) -> Option<crate::interaction_audio::Cue> {
    use crate::interaction_audio::{Cue, Typed};
    use ::rune::PhysicalKeyCode as Code;
    let ::rune::PhysicalInput::Key { key, mods, pressed, repeat } = *input else { return None };
    if mods.cmd || mods.ctrl || mods.alt || mods.logo {
        return None;
    }
    let kind = match key {
        Code::Space => Typed::Space,
        Code::Enter | Code::NumpadEnter | Code::Backspace | Code::Delete | Code::Tab => Typed::Big,
        Code::KeyA | Code::KeyB | Code::KeyC | Code::KeyD | Code::KeyE | Code::KeyF | Code::KeyG
        | Code::KeyH | Code::KeyI | Code::KeyJ | Code::KeyK | Code::KeyL | Code::KeyM | Code::KeyN
        | Code::KeyO | Code::KeyP | Code::KeyQ | Code::KeyR | Code::KeyS | Code::KeyT | Code::KeyU
        | Code::KeyV | Code::KeyW | Code::KeyX | Code::KeyY | Code::KeyZ | Code::Digit0 | Code::Digit1
        | Code::Digit2 | Code::Digit3 | Code::Digit4 | Code::Digit5 | Code::Digit6 | Code::Digit7
        | Code::Digit8 | Code::Digit9 | Code::Minus | Code::Equal | Code::BracketLeft
        | Code::BracketRight | Code::Backslash | Code::Semicolon | Code::Quote | Code::Backquote
        | Code::Comma | Code::Period | Code::Slash | Code::IntlBackslash => Typed::Key,
        _ => return None,
    };
    Some(match (pressed, repeat) {
        (false, _) => Cue::Type(None),
        (true, true) => Cue::Type(Some(Typed::Repeat)),
        (true, false) => Cue::Type(Some(kind)),
    })
}

fn menu_row(
    typography: super::preferences::Typography,
    p: &Palette,
    key: String,
    name: String,
    tag: &'static str,
    active: bool,
) -> El {
    row()
        .key(key)
        .w_full()
        .h(30.0)
        .gap(8.0)
        .px(8.0)
        .center_y()
        .radius(6.0)
        .hover(p.hover)
        .focus_highlight(p.hover)
        .child(
            typography
                .legend(name)
                .grow(1.0)
                .font(typography.font(12.5, 500.0))
                .color(p.ink)
                .truncate(),
        )
        .child(
            typography
                .legend(tag)
                .font(typography.font(11.0, 400.0))
                .color(p.ink_3),
        )
        .child(if active {
            ::rune::el::icon(&CHECK, 13.0).color(p.ink)
        } else {
            ::rune::el().size(13.0, 13.0)
        })
}

pub(super) fn view(state: &State, cx: &Cx<State>) -> El {
    let p = palette(state);
    let typography = state.preferences.typography();
    // The label moves into the text node: a route label built for this frame is never copied again.
    let model_label: std::borrow::Cow<'static, str> = match state.selected_choice() {
        None => "Choose model".into(),
        Some(choice) if supports_plan(&choice.value) && state.mode == Mode::Plan => {
            format!("Plan mode, {}", choice.label).into()
        }
        Some(choice) => choice.label,
    };
    let mut field = ::rune::el::text_input(
        &state.draft,
        if p.molded {
            TextStyle::new((16.0 * typography.scale).max(16.0), 400.0).mono()
        } else {
            TextStyle::new((16.0 * typography.scale).max(16.0), 400.0)
        },
    )
    .key("rune-composer-field")
    .w_full()
    .min_h(68.0)
    .max_h(100.0)
    .padding(8.0)
    .color(p.ink)
    .label("Prompt")
    .placeholder("Write a message…", p.ink_3)
    .caret(p.ink, p.accent.alpha(0.28))
    .molded_caret(p)
    .on_edit(|state: &mut State| &mut state.draft)
    .on_input_change(|state: &mut State, _| {
        state.draft_changed();
        let menu = token(&state.draft.text).map(|_| Menu::Sources);
        if menu.is_some() && state.menu.is_none() {
            state.sound(crate::interaction_audio::Cue::Open);
        }
        state.menu = menu;
        state.source_engaged = false;
    })
    .on_physical_input(|state: &mut State, input, _| {
        if let Some(cue) = typed(input) {
            state.sound(cue);
        }
        false
    })
    .on_input_key(|state: &mut State, key, tx| match &key.key {
        Key::Enter if !key.mods.shift && tx.key_repeat() => true,
        Key::Escape if state.menu.is_some() => {
            close(state, tx);
            state.source_engaged = false;
            true
        }
        Key::Enter if !key.mods.shift && source_menu_owns_enter(state) => {
            capture(state, tx);
            true
        }
        Key::Tab if key.mods == Default::default() && source_menu_owns_enter(state) => {
            capture(state, tx);
            true
        }
        Key::Down | Key::Up
            if state.menu == Some(Menu::Sources)
                && token(&state.draft.text).is_some_and(|t| t.marker == '@') =>
        {
            state.source_engaged = true;
            true
        }
        Key::Enter if !key.mods.shift => {
            if !super::corrections::focus(state, tx) {
                state.send(tx);
            }
            true
        }
        _ => false,
    })
    .on_submit(|state: &mut State, tx| {
        if !super::corrections::focus(state, tx) {
            state.send(tx);
        }
    });
    #[cfg(target_arch = "wasm32")]
    {
        field = super::corrections::attach_field(field, state);
    }
    if state.menu == Some(Menu::Sources) {
        field = field.preserve_focus_on_press(true);
    }
    let field = ::rune::auto_grow(field, 1, 5);

    let add = button(
        "rune-source-trigger",
        "Add attachments and sources",
        &PLUS,
        p.ink_2,
    )
    .hover(p.hover)
    .press(p.press)
    .press_scale(super::theme::PRESS_SCALE)
    .on_press_tx(sources)
    .on_key(|state: &mut State, key, tx| {
        if matches!(&key.key, Key::Down | Key::Up) {
            state.menu = Some(Menu::Sources);
            state.source_engaged = false;
            if page_matches(&state.draft.text) && !state.capture_disabled() {
                tx.focus("rune-current-page");
            }
            true
        } else {
            enter(state, key, tx, sources)
        }
    })
    .expanded(state.menu == Some(Menu::Sources));
    let model = row()
        .key("rune-model-trigger")
        .h(CONTROL)
        .gap(4.0)
        .center_y()
        .px(6.0)
        .max_w(160.0)
        .shrink()
        .radius(8.0)
        .child(
            typography
                .legend(model_label)
                .font(typography.font(12.0, 500.0))
                .color(p.ink_2)
                .truncate(),
        )
        .child(::rune::el::icon(&CHEVRON, 11.0).color(p.ink_3))
        .hover(p.hover)
        .press(p.line_strong)
        .press_scale(super::theme::PRESS_SCALE)
        .label("Choose model");
    let model = metldr_ui::menu::trigger(
        model,
        state.menu == Some(Menu::Models),
        model_ends,
        |state: &mut State, open, _| {
            state.sound(crate::interaction_audio::Cue::Press);
            state.source_engaged = false;
            state.menu = open.map(|_| Menu::Models);
        },
    );
    let running = state.running();
    let dictate = button("rune-dictate", "Dictation unavailable", &MIC, p.ink_3)
        .opacity(0.48)
        .disabled(true);
    let action = if running {
        let stop = button("rune-stop", "Stop", &STOP, p.surface)
            .fill(p.ink)
            .press(p.press_solid)
            .press_scale(super::theme::PRESS_SCALE)
            .on_press_tx(|state: &mut State, tx| state.stop(tx))
            .on_key(|state: &mut State, key, tx| enter(state, key, tx, State::stop));
        if let Some(correction) = super::corrections::control(state) {
            row().gap(4.0).center_y().child(correction).child(stop)
        } else {
            stop
        }
    } else {
        let can_send = state.can_send();
        // A molded send is the primary key, in the key colour with its legend in `on_accent`.
        let (ink, fill) = match (can_send, p.molded) {
            (true, true) => (p.on_accent, p.accent),
            (true, false) => (p.surface, p.ink),
            (false, _) => (p.ink_2, p.line_strong),
        };
        let mut send = button("rune-send", "Send", &SEND, ink).fill(fill).disabled(!can_send);
        // A molded theme's send is its primary key: the key colour on a darker lip of its own.
        if p.molded && can_send {
            send = super::theme::on_lip(send, p.accent.mix(::rune::Color::BLACK, 0.45), true);
        }
        if can_send {
            send = send.press(p.press_solid);
            if !p.molded {
                send = send.press_scale(super::theme::PRESS_SCALE);
            }
            send = send
                .on_press_tx(|state: &mut State, tx| state.send(tx))
                .on_key(|state: &mut State, key, tx| enter(state, key, tx, State::send));
        }
        send
    };
    col()
        .key("rune-composer")
        .w_full()
        .gap(6.0)
        .padding(INSET)
        .fill(p.surface)
        .border(
            BORDER,
            if state.draft.focused {
                p.line_strong
            } else {
                p.line
            },
        )
        .radius(RADIUS)
        .clip_rounded(RADIUS)
        .child(
            ::rune::el::el()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .effect(super::glimm::MARK)
                .effect_data([
                    0.0,
                    if cx.is_reduced_motion() { 1.0 } else { 0.0 },
                    0.0,
                    0.0,
                ]),
        )
        .child(add.pin(Some(INSET), None, None, Some(INSET)))
        .child(field)
        .child(
            row()
                .w_full()
                .gap(4.0)
                .center_y()
                .child(row().size(CONTROL, CONTROL))
                .child(model)
                .child(row().grow(1.0))
                .child(dictate)
                .child(action),
        )
}

pub(super) fn menu(state: &State) -> Option<El> {
    let p = palette(state);
    let typography = state.preferences.typography();
    match state.menu? {
        Menu::Sources => {
            let marker = token(&state.draft.text).map(|t| t.marker);
            let query = token(&state.draft.text)
                .map(|t| &state.draft.text[t.start + 1..])
                .unwrap_or("")
                .to_lowercase();
            let show_page = marker != Some('/') && "current page".contains(&query);
            let mut rows = col().key("rune-source-rows").w_full();
            if show_page {
                let disabled = state.capture_disabled();
                let mut page = row()
                    .key("rune-current-page")
                    .w_full()
                    .h(36.0)
                    .gap(10.0)
                    .px(8.0)
                    .center_y()
                    .radius(6.0)
                    .fill(if state.source_engaged {
                        p.hover
                    } else {
                        p.surface
                    })
                    .hover(p.hover)
                    .focus_highlight(p.hover)
                    .child(
                        row()
                            .size(22.0, 22.0)
                            .center()
                            .child(::rune::el::icon(&GLOBE, 15.0).color(p.ink_2)),
                    )
                    .child(
                        typography
                            .legend("Current page")
                            .font(typography.font(12.5, 500.0))
                            .color(p.ink)
                            .truncate(),
                    )
                    .child(
                        typography
                            .legend("Use the page you are viewing")
                            .grow(1.0)
                            .font(typography.font(12.0, 400.0))
                            .color(p.ink_3)
                            .truncate(),
                    )
                    .focusable(true)
                    .disabled(disabled)
                    .role("button")
                    .label("Current page, Use the page you are viewing");
                if !disabled {
                    page = page
                        .press(p.line_strong)
                        .on_press_tx(|state: &mut State, tx| capture(state, tx));
                }
                rows = rows.child(page);
            }
            if marker != Some('/') && query.is_empty() {
                rows = rows.child(
                    row()
                        .key("rune-files-unavailable")
                        .w_full()
                        .h(36.0)
                        .gap(10.0)
                        .px(8.0)
                        .center_y()
                        .radius(6.0)
                        .child(::rune::el::icon(&PAPERCLIP, 16.0).color(p.ink_3))
                        .child(
                            typography
                                .legend("Photos and files")
                                .grow(1.0)
                                .font(typography.font(12.5, 500.0))
                                .color(p.ink),
                        )
                        .child(
                            typography
                                .legend("Unavailable")
                                .font(typography.font(12.0, 400.0))
                                .color(p.ink_3),
                        )
                        .disabled(true)
                        .role("button")
                        .label("Photos and files, unavailable"),
                );
            }
            if marker == Some('/') || !show_page {
                rows = rows.child(
                    typography
                        .text(if marker == Some('/') {
                            "Commands are unavailable."
                        } else {
                            "No matching sources."
                        })
                        .w_full()
                        .padding(8.0)
                        .font(typography.font(12.0, 400.0))
                        .color(p.ink_3)
                        .wrap(),
                );
            }
            Some(
                col()
                    .key("rune-source-menu")
                    .role("menu")
                    .focus_group()
                    .w_full()
                    .gap(5.0)
                    .padding(4.0)
                    .fill(p.surface)
                    .border(1.0, p.line)
                    .radius(10.0)
                    .shadow(0.0, 8.0, p.line.alpha(0.28))
                    .child(rows)
                    .child(row().w_full().h(1.0).fill(p.line))
                    .child(
                        typography
                            .text(if marker == Some('/') {
                                "Saved commands"
                            } else {
                                "Type to search sources and files"
                            })
                            .w_full()
                            .padding(7.0)
                            .font(typography.font(11.0, 400.0))
                            .color(p.ink_3),
                    )
                    .anchored_fit("rune-composer", Side::Top, Align::Start, 8.0)
                    .transition(super::theme::pop_in(p))
                    .focus_return("rune-source-trigger")
                    .on_key(|state: &mut State, key, _| {
                        if key.key == Key::Escape {
                            state.menu = None;
                            state.source_engaged = false;
                            true
                        } else {
                            false
                        }
                    }),
            )
        }
        Menu::Models => {
            // Configured routes by section with short names; routes still to set up fold into one
            // row that opens Settings. Full labels stay the accessible names.
            let mut choices = state.choices();
            choices.sort_by_key(|choice| choice.short().0);
            let hidden = choices.iter().filter(|choice| !choice.enabled).count();
            let shown = choices.len() - hidden;
            let mut rows = col().key("rune-model-rows").w_full();
            let mut section = None;
            for choice in choices.into_iter().filter(|choice| choice.enabled) {
                let (heading, name, tag) = choice.short();
                let (name, tag) = (name.to_owned(), tag);
                if section != Some(heading) {
                    section = Some(heading);
                    rows = rows.child(
                        typography
                            .legend(heading.heading())
                            .font(typography.font(11.0, 500.0))
                            .color(p.ink_3)
                            .px(8.0)
                            .pt(6.0)
                            .pb(2.0)
                            .a11y_hidden(),
                    );
                }
                let active = choice.value == state.selection;
                let disabled = state.running();
                let el = menu_row(typography, p, format!("model:{}", choice.value), name, tag, active)
                    .label(choice.label.clone())
                    .press(p.line_strong);
                let close: metldr_ui::menu::CloseOnSelect<State> = Some(std::rc::Rc::new(close_menu));
                rows = rows.child(metldr_ui::menu::radio_item(el, disabled, active, close, move |state: &mut State, tx| {
                    if !state.running() {
                        state.select(choice.value.to_string(), tx);
                    }
                }));
            }
            if hidden > 0 {
                let name = if shown == 0 { "Set up a model" } else { "Set up more" };
                rows = rows.child(metldr_ui::menu::item(
                    menu_row(typography, p, "model:setup".into(), name.into(), "", false).label(name).press(p.line_strong),
                    false,
                    |state: &mut State, tx| state.open_settings(tx),
                    close_menu,
                ));
            }
            let menu = col()
                .key("rune-model-menu")
                .min_w(176.0)
                .max_w(416.0)
                .max_h(384.0)
                .padding(4.0)
                .fill(p.surface)
                .border(1.0, p.line)
                .radius(10.0)
                .shadow(0.0, 8.0, p.line.alpha(0.28))
                .child(rows)
                .scroll(::rune::ScrollSpec {
                    stick_bottom: false,
                });
            let mut menu = metldr_ui::menu::popup(menu, "Models", "rune-model-trigger", Side::Top, super::theme::pop_in(p), close_menu);
            if supports_plan(&state.selection) {
                let modes = [
                    (Mode::Configured, "Codex configured mode"),
                    (Mode::Plan, "Codex Plan mode"),
                ];
                let mode_rows = modes.into_iter().fold(
                    col().key("rune-mode-rows").w_full(),
                    |rows, (mode, label)| {
                        let disabled = state.capture_disabled();
                        let mode_row = row()
                            .key(label)
                            .w_full()
                            .h(30.0)
                            .px(8.0)
                            .center_y()
                            .radius(6.0)
                            .hover(p.hover)
                            .focus_highlight(p.hover)
                            .child(
                                typography
                                    .legend(label)
                                    .grow(1.0)
                                    .font(typography.font(12.5, 500.0))
                                    .color(p.ink),
                            )
                            .child(if state.mode == mode {
                                ::rune::el::icon(&CHECK, 13.0).color(p.ink)
                            } else {
                                ::rune::el().size(13.0, 13.0)
                            })
                            .label(label)
                            .press(p.line_strong);
                        let close: metldr_ui::menu::CloseOnSelect<State> = Some(std::rc::Rc::new(close_menu));
                        rows.child(metldr_ui::menu::radio_item(mode_row, disabled, state.mode == mode, close, move |state: &mut State, _| {
                            if state.capture_disabled() || !supports_plan(&state.selection) {
                                return;
                            }
                            state.mode = mode;
                            state.draft.focused = true;
                        }))
                    },
                );
                menu = menu.child(mode_rows.role("group").label("Codex mode"));
            }
            Some(menu)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaced_and_restored_drafts_have_distinct_submission_versions() {
        let mut state = State::default();
        state.replace_draft("שלום مرحبا 🦀");
        let presented_version = state.draft_version;
        let presented_text = state.draft.text.clone();
        state.replace_draft("Changed");
        state.replace_draft(presented_text.clone());
        assert_eq!(state.draft.text, presented_text);
        assert_ne!(state.draft_version, presented_version);
        let restored_version = state.draft_version;
        state.clear_draft();
        assert!(state.draft.text.is_empty());
        assert_ne!(state.draft_version, restored_version);
    }

    #[test]
    fn the_actual_composer_preserves_button_tab_order_and_enter_activation() {
        use ::rune::{Env, Mods, Runtime};
        fn tree(state: &State, cx: &mut Cx<State>) -> El {
            let mut tree = col()
                .w_full()
                .child(view(state, cx))
                .on_key(|state: &mut State, key, tx| super::super::key(state, key, tx));
            if let Some(menu) = menu(state) {
                tree = tree.child(menu);
            }
            tree
        }
        let mut rt = Runtime::new(State::default(), tree, (640.0, 300.0), Env::default());
        rt.set_tab_visits_controls(true);
        rt.frame(0.0);
        let send = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("rune-send"))
            .unwrap();
        let [x, y, w, h] = send.rect;
        rt.pointer_down(x + w / 2.0, y + h / 2.0);
        rt.frame(0.0);
        assert_eq!(
            rt.elements()
                .into_iter()
                .find(|e| e.key == send.key)
                .unwrap()
                .press,
            0.0
        );
        rt.pointer_up(x + w / 2.0, y + h / 2.0);
        rt.frame(0.0);
        let focused = |rt: &Runtime<State>| {
            rt.elements()
                .into_iter()
                .find(|e| e.focused)
                .and_then(|e| e.key)
        };
        for (key, target) in [
            (Key::Tab, "rune-source-trigger"),
            (Key::Enter, "rune-source-trigger"),
            (Key::Escape, "rune-source-trigger"),
            (Key::Tab, "rune-composer-field"),
            (Key::Tab, "rune-model-trigger"),
            (Key::Enter, "model:"),
            (Key::Escape, "rune-model-trigger"),
            (Key::Tab, "rune-source-trigger"),
        ] {
            rt.key(KeyEvent {
                key: key.clone(),
                mods: Mods::default(),
            });
            rt.frame(0.0);
            // The model menu opens with focus inside it: its first item, or the menu itself while
            // it has none.
            let at = focused(&rt).unwrap_or_default();
            let inside = at.starts_with("model:") || at == "rune-model-menu";
            assert!(at == target || (target == "model:" && inside), "{at} is not {target}");
            if key == Key::Enter {
                assert!(rt.state.menu.is_some());
            } else if key == Key::Escape {
                assert!(rt.state.menu.is_none());
            }
        }
        for target in [
            "rune-model-trigger",
            "rune-composer-field",
            "rune-source-trigger",
        ] {
            rt.key(KeyEvent {
                key: Key::Tab,
                mods: Mods {
                    shift: true,
                    ..Default::default()
                },
            });
            rt.frame(0.0);
            assert_eq!(focused(&rt).as_deref(), Some(target));
        }
    }

    #[test]
    fn source_tokens_preserve_unicode_text_and_only_match_a_suffix() {
        for value in ["مرحبا", "🌟", "hi/command", "@page ", "@wrong!"] {
            assert!(token(value).is_none(), "{value}");
        }
        let value = "مرحبا 🌟 @cur";
        let found = token(value).unwrap();
        assert_eq!(found.marker, '@');
        assert_eq!(&value[..found.start], "مرحبا 🌟 ");
        assert_eq!(&value[found.start + 1..], "cur");
        assert_eq!(token("/command").unwrap().marker, '/');
    }
    #[test]
    fn a_menu_can_take_focus_when_it_opens_without_focusing_unavailable_controls() {
        use ::rune::{Env, KeyEvent, Mods, Runtime, TextEdit};
        let focused = |rt: &Runtime<Form>| {
            rt.elements()
                .into_iter()
                .find(|e| e.focused)
                .and_then(|e| e.key)
        };
        struct Form {
            field: TextEdit,
            open: bool,
            pressed: bool,
        }
        fn view(s: &Form, _: &mut Cx<Form>) -> El {
            col()
                .child(
                    ::rune::text_input(&s.field, TextStyle::new(14.0, 400.0))
                        .key("field")
                        .size(200.0, 40.0)
                        .on_edit(|s: &mut Form| &mut s.field)
                        .on_input_key(|s: &mut Form, key, tx| {
                            if key.key != Key::Down {
                                return false;
                            }
                            s.open = true;
                            tx.focus("menu/last");
                            true
                        }),
                )
                .children(s.open.then(|| {
                    col().key("menu").role("menu").focus_group().children([
                        ::rune::el::el()
                            .key("disabled")
                            .size(100.0, 24.0)
                            .focusable(true)
                            .disabled(true),
                        ::rune::el::el()
                            .key("hidden")
                            .size(100.0, 24.0)
                            .focusable(true)
                            .opacity(0.0),
                        ::rune::el::el()
                            .key("inert")
                            .size(100.0, 24.0)
                            .focusable(true)
                            .inert(),
                        ::rune::el::el()
                            .key("last")
                            .size(100.0, 24.0)
                            .focusable(true)
                            .on_press(|s: &mut Form| s.pressed = true),
                    ])
                }))
        }
        let mut edit = TextEdit::default();
        edit.focused = true;
        let mut rt = Runtime::new(
            Form {
                field: edit,
                open: false,
                pressed: false,
            },
            view,
            (300.0, 240.0),
            Env::default(),
        );
        rt.frame(0.0);
        rt.key(KeyEvent {
            key: Key::Down,
            mods: Mods::default(),
        });
        rt.key(KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        });
        rt.frame(0.0);
        assert_eq!(focused(&rt).as_deref(), Some("last"));
        rt.update_if(|s, _| {
            assert!(s.pressed);
            assert!(!s.field.focused);
            false
        });
        assert!(rt.update_if(|_, tx| {
            tx.focus("field");
            false
        }));
        rt.frame(0.0);
        assert_eq!(focused(&rt).as_deref(), Some("field"));
        for key in ["menu/disabled", "menu/hidden", "menu/inert", "missing"] {
            rt.update_if(|_, tx| {
                tx.focus(key);
                false
            });
            rt.frame(0.0);
            assert_eq!(focused(&rt).as_deref(), Some("field"));
        }
        rt.update_if(|_, tx| {
            tx.focus("menu/last");
            tx.blur();
            false
        });
        rt.frame(0.0);
        assert!(focused(&rt).is_none());
        rt.update_if(|_, tx| {
            tx.blur();
            tx.focus("field");
            false
        });
        rt.frame(0.0);
        assert_eq!(focused(&rt).as_deref(), Some("field"));
    }
}
