use super::{Core, FlowError, State, Tx};
use crate::model::{CodexInputPrompt, RunId};
use crate::ui_bridge::codex_input::{
    UiCodexAnswer, UiCodexFieldAnswer, UiCodexQuestionAnswer, rune_reply_key,
};
use ::rune::{El, TextEdit, TextStyle, col, oklch};
use std::collections::{HashMap, HashSet};
use wasm_bindgen_futures::spawn_local;

#[derive(Default)]
pub(super) struct Replies {
    edits: HashMap<String, TextEdit>,
    selected: HashMap<String, Vec<String>>,
}

fn field_key(run: &RunId, request: &RunId, kind: &str, name: &str) -> String {
    serde_json::to_string(&(run.as_str(), request.as_str(), kind, name)).expect("string key")
}

// Call after accepting page updates and before rendering the reply controls.
pub(super) fn sync(state: &mut State) {
    let Some(core) = state.core().cloned() else {
        state.replies = Replies::default();
        return;
    };
    let (requests, _) = core.rune_codex_requests();
    let mut active = HashSet::new();
    for (run, pending, _) in requests {
        match pending.prompt {
            CodexInputPrompt::Questions { questions, .. } => {
                for question in questions {
                    let key = field_key(&run, &pending.request, "question", &question.id);
                    active.insert(key.clone());
                    state.replies.selected.entry(key.clone()).or_default();
                    if question.other {
                        state.replies.edits.entry(key).or_default();
                    }
                }
            }
            CodexInputPrompt::McpForm { fields, .. } => {
                for field in fields {
                    let key = field_key(&run, &pending.request, "field", &field.name);
                    active.insert(key.clone());
                    state.replies.edits.entry(key).or_insert_with(|| {
                        let mut edit = TextEdit::default();
                        edit.replace_text(
                            field
                                .default
                                .or_else(|| {
                                    field.choices.and_then(|choices| choices.into_iter().next())
                                })
                                .unwrap_or_default(),
                        );
                        edit
                    });
                }
            }
            CodexInputPrompt::McpToolApproval { .. } => {}
        }
    }
    state.replies.edits.retain(|key, _| active.contains(key));
    state.replies.selected.retain(|key, _| active.contains(key));
}

fn button(
    typography: super::preferences::Typography,
    label: String,
    key: String,
    enabled: bool,
) -> El {
    typography
        .text(label.clone())
        .key(key)
        .min_h(28.0)
        .role("button")
        .focusable(enabled)
        .disabled(!enabled)
        .wrap()
        .label(label)
}

fn input(
    edit: &TextEdit,
    typography: super::preferences::Typography,
    palette: &super::preferences::Palette,
) -> El {
    let ink = palette.ink;
    ::rune::el::text_input(
        edit,
        TextStyle::new((16.0 * typography.scale).max(16.0), 400.0),
    )
    .w_full()
    .min_h(36.0)
    .padding(8.0)
    .radius(5.0)
    .border(1.0, palette.line)
    .fill(palette.field)
    .color(ink)
    .single_line()
    .caret(ink, ink.alpha(0.25))
}

fn activate(
    el: El,
    press: ::rune::Color,
    enabled: bool,
    handler: impl Fn(&mut State, &mut Tx) + 'static,
) -> El {
    if enabled {
        el.native_activation()
            .press(press)
            .on_press_tx(move |state: &mut State, tx| {
                state.sound(crate::interaction_audio::Cue::Press);
                handler(state, tx)
            })
    } else {
        el
    }
}

fn send(state: &mut State, run: RunId, request: RunId, answer: UiCodexAnswer, tx: &mut Tx) {
    let Some(core) = state.core().cloned() else {
        return;
    };
    let Some(updates) = state.updates.clone() else {
        return;
    };
    match core.rune_codex_reply(run, request, answer, tx) {
        Ok(work) => spawn_local(async move {
            updates.finish(work.await);
        }),
        Err(error) => state.fail(error),
    }
}

fn native(state: &mut State, run: RunId, request: RunId, option: Option<String>, tx: &mut Tx) {
    let Some(core) = state.core().cloned() else {
        return;
    };
    let Some(updates) = state.updates.clone() else {
        return;
    };
    match core.rune_native_reply(run, request, option, tx) {
        Ok(work) => spawn_local(async move {
            updates.finish(work.await);
        }),
        Err(error) => state.fail(error),
    }
}

fn codex_action(
    typography: super::preferences::Typography,
    run: &RunId,
    request: &RunId,
    action: &'static str,
    label: &str,
    enabled: bool,
) -> El {
    let key = rune_reply_key("codex", run, request, action);
    let run = run.clone();
    let request = request.clone();
    activate(
        button(typography, label.into(), key, enabled)
            .label(format!("{label} for request {}", request.as_str())),
        typography.press,
        enabled,
        move |state: &mut State, tx| {
            let answer = match action {
                "skip" => UiCodexAnswer::skip(),
                "allow" => UiCodexAnswer::allow(),
                "deny" => UiCodexAnswer::deny(),
                _ => UiCodexAnswer::cancel(),
            };
            send(state, run.clone(), request.clone(), answer, tx);
        },
    )
}

pub(super) fn view(state: &State, core: &Core) -> Vec<El> {
    let typography = state.preferences.typography();
    let palette = state.preferences.palette();
    let mut result = Vec::new();
    let (requests, notice) = core.rune_codex_requests();
    if let Some(notice) = notice {
        result.push(typography.text(notice).wrap());
    }
    for (run, pending, enabled) in requests {
        let request = pending.request;
        let mut panel = col().w_full().gap(8.0);
        match pending.prompt.clone() {
            CodexInputPrompt::Questions {
                item, questions, ..
            } => {
                panel = panel.child(typography.text(item).wrap());
                for question in &questions {
                    let heading = if question.header.is_empty() {
                        "Question"
                    } else {
                        &question.header
                    };
                    panel = panel.child(typography.text(heading).wrap());
                    if !question.text.is_empty() {
                        panel = panel.child(typography.text(&question.text).wrap());
                    }
                    let question_label = if question.text.is_empty() {
                        heading
                    } else {
                        &question.text
                    };
                    let key = field_key(&run, &request, "question", &question.id);
                    for option in &question.options {
                        let selected = state
                            .replies
                            .selected
                            .get(&key)
                            .is_some_and(|v| v.contains(&option.label));
                        let label = format!(
                            "{} {}",
                            if selected { "Selected:" } else { "Select:" },
                            option.label
                        );
                        let choice_key =
                            serde_json::to_string(&(key.as_str(), option.label.as_str()))
                                .expect("choice key");
                        let entry = key.clone();
                        let value = option.label.clone();
                        panel = panel.child(activate(
                            button(typography, label.clone(), choice_key, enabled)
                                .label(format!("{label} for {question_label}")),
                            typography.press,
                            enabled,
                            move |state: &mut State, _| {
                                let choices =
                                    state.replies.selected.entry(entry.clone()).or_default();
                                if let Some(index) = choices.iter().position(|item| item == &value)
                                {
                                    choices.remove(index);
                                } else if choices.len() < 16 {
                                    choices.push(value.clone());
                                }
                            },
                        ));
                        if !option.description.is_empty() {
                            panel = panel.child(typography.text(&option.description).wrap());
                        }
                    }
                    if question.other {
                        if let Some(edit) = state.replies.edits.get(&key) {
                            panel = panel.child(typography.text("Other").wrap());
                            let entry = key.clone();
                            panel = panel.child(
                                input(edit, typography, palette)
                                    .key(format!("other:{key}"))
                                    .w_full()
                                    .label(format!("Other answer for {question_label}"))
                                    .disabled(!enabled)
                                    .on_edit(::rune::edit_access(move |state: &mut State| {
                                        state
                                            .replies
                                            .edits
                                            .get_mut(&entry)
                                            .expect("synced reply field")
                                    })),
                            );
                        }
                    }
                }
                let ready = enabled
                    && questions.iter().all(|question| {
                        let key = field_key(&run, &request, "question", &question.id);
                        let selected = state.replies.selected.get(&key).map_or(0, Vec::len);
                        let other = state
                            .replies
                            .edits
                            .get(&key)
                            .map_or("", |edit| edit.text.as_str());
                        let length = other.chars().count();
                        length <= 4096
                            && selected + usize::from(length > 0) <= 16
                            && (selected > 0 || length > 0)
                    });
                let action_key = rune_reply_key("codex", &run, &request, "questions");
                let submit_run = run.clone();
                let submit_request = request.clone();
                panel = panel.child(activate(
                    button(typography, "Submit answers".into(), action_key, ready)
                        .label(format!("Submit answers for request {}", request.as_str())),
                    typography.press,
                    ready,
                    move |state: &mut State, tx| {
                        if questions.iter().any(|question| {
                            let key =
                                field_key(&submit_run, &submit_request, "question", &question.id);
                            state
                                .replies
                                .edits
                                .get(&key)
                                .is_some_and(|edit| edit.preedit.is_some())
                        }) {
                            state.fail(FlowError::InvalidInput(
                                "Finish composing your answer before submitting.".into(),
                            ));
                            return;
                        }
                        let rows = questions
                            .iter()
                            .map(|question| {
                                let key = field_key(
                                    &submit_run,
                                    &submit_request,
                                    "question",
                                    &question.id,
                                );
                                let mut values = state
                                    .replies
                                    .selected
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or_default();
                                if let Some(value) = state
                                    .replies
                                    .edits
                                    .get(&key)
                                    .map(|edit| edit.text.clone())
                                    .filter(|value| !value.is_empty())
                                {
                                    values.push(value);
                                }
                                UiCodexQuestionAnswer::new(question.id.clone(), values)
                            })
                            .collect();
                        send(
                            state,
                            submit_run.clone(),
                            submit_request.clone(),
                            UiCodexAnswer::questions(rows),
                            tx,
                        );
                    },
                ));
                panel = panel.child(codex_action(
                    typography, &run, &request, "skip", "Skip", enabled,
                ));
            }
            CodexInputPrompt::McpForm {
                server,
                message,
                fields,
            } => {
                panel = panel.child(typography.text(format!("{server}: {message}")).wrap());
                for field in &fields {
                    let title = if field.title.is_empty() {
                        &field.name
                    } else {
                        &field.title
                    };
                    panel = panel.child(typography.text(title).wrap());
                    panel = panel.child(
                        typography
                            .text(format!(
                                "{} to {} characters",
                                field.min_chars, field.max_chars
                            ))
                            .wrap(),
                    );
                    if let Some(description) = &field.description {
                        panel = panel.child(typography.text(description).wrap());
                    }
                    let key = field_key(&run, &request, "field", &field.name);
                    if let Some(choices) = &field.choices {
                        for choice in choices {
                            let selected = state
                                .replies
                                .edits
                                .get(&key)
                                .is_some_and(|edit| edit.text == *choice);
                            let label = format!(
                                "{} {}",
                                if selected { "Selected:" } else { "Select:" },
                                choice
                            );
                            let choice_key =
                                serde_json::to_string(&(key.as_str(), choice.as_str()))
                                    .expect("choice key");
                            let entry = key.clone();
                            let value = choice.clone();
                            panel = panel.child(activate(
                                button(typography, label.clone(), choice_key, enabled)
                                    .label(format!("{label} for {title}")),
                                typography.press,
                                enabled,
                                move |state: &mut State, _| {
                                    if let Some(edit) = state.replies.edits.get_mut(&entry) {
                                        edit.replace_text(value.clone());
                                    }
                                },
                            ));
                        }
                    } else if let Some(edit) = state.replies.edits.get(&key) {
                        let entry = key.clone();
                        panel = panel.child(
                            input(edit, typography, palette)
                                .key(format!("input:{key}"))
                                .w_full()
                                .label(title.clone())
                                .description(format!(
                                    "{} to {} characters",
                                    field.min_chars, field.max_chars
                                ))
                                .disabled(!enabled)
                                .on_edit(::rune::edit_access(move |state: &mut State| {
                                    state
                                        .replies
                                        .edits
                                        .get_mut(&entry)
                                        .expect("synced reply field")
                                })),
                        );
                    }
                }
                let action_key = rune_reply_key("codex", &run, &request, "form");
                let submit_run = run.clone();
                let submit_request = request.clone();
                panel = panel.child(activate(
                    button(typography, "Submit form".into(), action_key, enabled)
                        .label(format!("Submit form for request {}", request.as_str())),
                    typography.press,
                    enabled,
                    move |state: &mut State, tx| {
                        if fields.iter().any(|field| {
                            let key = field_key(&submit_run, &submit_request, "field", &field.name);
                            state
                                .replies
                                .edits
                                .get(&key)
                                .is_some_and(|edit| edit.preedit.is_some())
                        }) {
                            state.fail(FlowError::InvalidInput(
                                "Finish composing your answer before submitting.".into(),
                            ));
                            return;
                        }
                        let rows = fields
                            .iter()
                            .map(|field| {
                                let key =
                                    field_key(&submit_run, &submit_request, "field", &field.name);
                                UiCodexFieldAnswer::new(
                                    field.name.clone(),
                                    state
                                        .replies
                                        .edits
                                        .get(&key)
                                        .map(|edit| edit.text.clone())
                                        .unwrap_or_default(),
                                )
                            })
                            .collect();
                        send(
                            state,
                            submit_run.clone(),
                            submit_request.clone(),
                            UiCodexAnswer::form(rows),
                            tx,
                        );
                    },
                ));
                panel = panel
                    .child(codex_action(
                        typography, &run, &request, "deny", "Decline", enabled,
                    ))
                    .child(codex_action(
                        typography, &run, &request, "cancel", "Cancel", enabled,
                    ));
            }
            CodexInputPrompt::McpToolApproval {
                server,
                message,
                review,
            } => {
                panel = panel
                    .child(typography.text(format!("{server}: {message}")).wrap())
                    .child(typography.text(review).wrap());
                panel = panel
                    .child(codex_action(
                        typography, &run, &request, "allow", "Allow", enabled,
                    ))
                    .child(codex_action(
                        typography, &run, &request, "deny", "Decline", enabled,
                    ))
                    .child(codex_action(
                        typography, &run, &request, "cancel", "Cancel", enabled,
                    ));
            }
        }
        result.push(panel);
    }
    let (permissions, notice) = core.rune_native_requests();
    if let Some(notice) = notice {
        result.push(typography.text(notice).wrap());
    }
    for (permission, enabled) in permissions {
        let mut panel = col()
            .w_full()
            .gap(8.0)
            .child(typography.text(&permission.title).wrap())
            .child(typography.text(permission.agent.label()).wrap())
            .child(typography.text(&permission.tool).wrap())
            .child(typography.text(&permission.input).wrap());
        for option in permission.options {
            let action = serde_json::to_string(&Some(&option.id)).expect("option key");
            let key = rune_reply_key("native", &permission.run, &permission.request, &action);
            let run = permission.run.clone();
            let request = permission.request.clone();
            let value = option.id;
            panel = panel.child(activate(
                button(typography, option.name.clone(), key, enabled)
                    .label(format!("{} for {}", option.name, permission.title)),
                typography.press,
                enabled,
                move |state: &mut State, tx| {
                    native(state, run.clone(), request.clone(), Some(value.clone()), tx)
                },
            ));
        }
        let action = serde_json::to_string(&Option::<String>::None).expect("option key");
        let key = rune_reply_key("native", &permission.run, &permission.request, &action);
        let run = permission.run;
        let request = permission.request;
        panel = panel.child(activate(
            button(typography, "Cancel".into(), key, enabled)
                .label(format!("Cancel {}", permission.title)),
            typography.press,
            enabled,
            move |state: &mut State, tx| native(state, run.clone(), request.clone(), None, tx),
        ));
        result.push(panel);
    }
    result
}
