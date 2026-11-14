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

use super::{Menu, Mode, State};
use ::rune::{Align, Cx, El, Icon, Key, KeyEvent, Side, TextStyle, Tx, col, oklch, row};

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

#[derive(Clone, Copy)]
struct Palette {
    surface: ::rune::Color,
    hover: ::rune::Color,
    ink: ::rune::Color,
    secondary: ::rune::Color,
    muted: ::rune::Color,
    line: ::rune::Color,
    strong: ::rune::Color,
    accent: ::rune::Color,
    press: ::rune::Color,
    press_solid: ::rune::Color,
}

fn palette(state: &State) -> Palette {
    let p = state.preferences.palette();
    Palette {
        surface: p.surface,
        hover: p.hover,
        ink: p.ink,
        secondary: p.ink_2,
        muted: p.ink_3,
        line: p.line,
        strong: p.line_strong,
        accent: p.accent,
        press: p.press,
        press_solid: p.press_solid,
    }
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

fn models(state: &mut State, _: &mut Tx) {
    state.sound(crate::interaction_audio::Cue::Press);
    state.source_engaged = false;
    state.menu = if state.menu == Some(Menu::Models) {
        None
    } else {
        Some(Menu::Models)
    };
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

pub(super) fn view(state: &State, cx: &mut Cx<State>) -> El {
    let p = palette(state);
    let typography = state.preferences.typography();
    let model_label = state.selected_choice().map_or_else(
        || "Choose model".to_owned(),
        |choice| {
            if supports_plan(&choice.value) && state.mode == Mode::Plan {
                format!("Plan mode, {}", choice.label)
            } else {
                choice.label.to_string()
            }
        },
    );
    let mut field = ::rune::el::text_input(
        &state.draft,
        TextStyle::new((16.0 * typography.scale).max(16.0), 400.0),
    )
    .key("rune-composer-field")
    .w_full()
    .min_h(68.0)
    .max_h(100.0)
    .padding(8.0)
    .label("Prompt")
    .placeholder("Write a message…", p.muted)
    .caret(p.ink, p.accent.alpha(0.28))
    .on_edit(|state: &mut State| &mut state.draft)
    .on_input_change(|state: &mut State, _| {
        state.draft_changed();
        state.menu = token(&state.draft.text).map(|_| Menu::Sources);
        state.source_engaged = false;
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
        p.secondary,
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
                .text(model_label)
                .font(typography.font(12.0, 500.0))
                .color(p.secondary)
                .truncate(),
        )
        .child(::rune::el::icon(&CHEVRON, 11.0).color(p.muted))
        .hover(p.hover)
        .press(p.strong)
        .press_scale(super::theme::PRESS_SCALE)
        .role("button")
        .focusable(true)
        .label("Choose model")
        .expanded(state.menu == Some(Menu::Models))
        .on_press_tx(models)
        .on_key(|state: &mut State, key, tx| {
            if !matches!(&key.key, Key::Down | Key::Up) {
                return enter(state, key, tx, models);
            }
            state.menu = Some(Menu::Models);
            state.source_engaged = false;
            let choices = state.choices();
            let choice = if key.key == Key::Down {
                choices
                    .iter()
                    .find(|choice| choice.enabled && !state.running())
            } else {
                choices
                    .iter()
                    .rev()
                    .find(|choice| choice.enabled && !state.running())
            };
            let Some(choice) = choice else { return true };
            let focus_key = format!("model:{}", choice.value);
            state.menu = Some(Menu::Models);
            tx.focus(focus_key);
            true
        });
    let running = state.running();
    let dictate = button("rune-dictate", "Dictation unavailable", &MIC, p.muted)
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
        let mut send = button(
            "rune-send",
            "Send",
            &SEND,
            if can_send { p.surface } else { p.secondary },
        )
        .fill(if can_send { p.ink } else { p.strong })
        .disabled(!can_send);
        if can_send {
            send = send
                .press(p.press_solid)
                .press_scale(super::theme::PRESS_SCALE)
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
                p.strong
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
                            .child(::rune::el::icon(&GLOBE, 15.0).color(p.secondary)),
                    )
                    .child(
                        typography
                            .text("Current page")
                            .font(typography.font(12.5, 500.0))
                            .color(p.ink)
                            .truncate(),
                    )
                    .child(
                        typography
                            .text("Use the page you are viewing")
                            .grow(1.0)
                            .font(typography.font(12.0, 400.0))
                            .color(p.muted)
                            .truncate(),
                    )
                    .focusable(true)
                    .disabled(disabled)
                    .role("button")
                    .label("Current page, Use the page you are viewing");
                if !disabled {
                    page = page
                        .press(p.strong)
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
                        .child(::rune::el::icon(&PAPERCLIP, 16.0).color(p.muted))
                        .child(
                            typography
                                .text("Photos and files")
                                .grow(1.0)
                                .font(typography.font(12.5, 500.0))
                                .color(p.ink),
                        )
                        .child(
                            typography
                                .text("Unavailable")
                                .font(typography.font(12.0, 400.0))
                                .color(p.muted),
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
                        .color(p.muted)
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
                            .color(p.muted),
                    )
                    .anchored_fit("rune-composer", Side::Top, Align::Start, 8.0)
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
            let choices = state.choices();
            let rows =
                choices
                    .into_iter()
                    .fold(col().key("rune-model-rows").w_full(), |rows, choice| {
                        let active = choice.value == state.selection;
                        let disabled = !choice.enabled || state.running();
                        let mut model = row()
                            .key(format!("model:{}", choice.value))
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
                                    .text(choice.label.clone())
                                    .grow(1.0)
                                    .font(typography.font(12.5, 500.0))
                                    .color(if choice.enabled { p.ink } else { p.muted })
                                    .truncate(),
                            )
                            .child(if choice.enabled {
                                typography.text("")
                            } else {
                                typography
                                    .text("Unavailable")
                                    .font(typography.font(11.0, 400.0))
                                    .color(p.muted)
                            })
                            .child(if active {
                                ::rune::el::icon(&CHECK, 13.0).color(p.ink)
                            } else {
                                typography.text("").size(13.0, 13.0)
                            })
                            .focusable(true)
                            .disabled(disabled)
                            .checked(active)
                            .role("menuitemradio")
                            .label(choice.label.clone());
                        if !disabled {
                            model =
                                model
                                    .press(p.strong)
                                    .on_press_tx(move |state: &mut State, tx| {
                                        if !state.running() {
                                            state.select(choice.value.to_string(), tx);
                                        }
                                    });
                        }
                        rows.child(model)
                    });
            let mut menu = col()
                .key("rune-model-menu")
                .role("menu")
                .focus_group()
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
                })
                .anchored_fit("rune-model-trigger", Side::Top, Align::Start, 8.0)
                .focus_return("rune-model-trigger")
                .on_key(|state: &mut State, key, _| {
                    if key.key == Key::Escape {
                        state.menu = None;
                        state.source_engaged = false;
                        true
                    } else {
                        false
                    }
                });
            if supports_plan(&state.selection) {
                let modes = [
                    (Mode::Configured, "Codex configured mode"),
                    (Mode::Plan, "Codex Plan mode"),
                ];
                let mode_rows = modes.into_iter().fold(
                    col().key("rune-mode-rows").w_full(),
                    |rows, (mode, label)| {
                        let disabled = state.capture_disabled();
                        let mut mode_row = row()
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
                                    .text(label)
                                    .grow(1.0)
                                    .font(typography.font(12.5, 500.0))
                                    .color(p.ink),
                            )
                            .child(if state.mode == mode {
                                ::rune::el::icon(&CHECK, 13.0).color(p.ink)
                            } else {
                                typography.text("").size(13.0, 13.0)
                            })
                            .focusable(true)
                            .disabled(disabled)
                            .checked(state.mode == mode)
                            .role("menuitemradio")
                            .label(label);
                        if !disabled {
                            mode_row = mode_row.press(p.strong).on_press_tx(
                                move |state: &mut State, _| {
                                    if state.capture_disabled() || !supports_plan(&state.selection)
                                    {
                                        return;
                                    }
                                    state.mode = mode;
                                    state.menu = None;
                                    state.source_engaged = false;
                                    state.draft.focused = true;
                                },
                            );
                        }
                        rows.child(mode_row)
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
            (Key::Enter, "rune-model-trigger"),
            (Key::Escape, "rune-model-trigger"),
            (Key::Tab, "rune-source-trigger"),
        ] {
            rt.key(KeyEvent {
                key: key.clone(),
                mods: Mods::default(),
            });
            rt.frame(0.0);
            assert_eq!(focused(&rt).as_deref(), Some(target));
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
