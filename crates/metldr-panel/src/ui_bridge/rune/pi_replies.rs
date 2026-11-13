use super::{Core, FlowError, State, Tx};
use crate::model::{PiUiDialog, RunId};
use crate::ui_bridge::pi_ui::{Answer, pi_reply_key, pi_suggestion_key};
use ::rune::{El, TextEdit, TextStyle, col, oklch, text};
use std::collections::{HashMap, HashSet};
use wasm_bindgen_futures::spawn_local;

#[derive(Default)]
pub(super) struct PiReplies {
    edits: HashMap<String, TextEdit>,
    selected: HashMap<String, String>,
}

fn edit_key(run: &RunId, request: &RunId) -> String {
    serde_json::to_string(&(run.as_str(), request.as_str())).expect("string key")
}

pub(super) fn sync(state: &mut State) {
    let Some(core) = state.core().cloned() else {
        state.pi_replies = PiReplies::default();
        return;
    };
    let (ui, _, _, _, _) = core.rune_pi_snapshot();
    let mut active = HashSet::new();
    if let Some(ui) = ui {
        for pending in ui.pending {
            let key = edit_key(&ui.run, &pending.request);
            if matches!(&pending.dialog, PiUiDialog::Select { .. }) {
                active.insert(key.clone());
            }
            let prefill = match pending.dialog {
                PiUiDialog::Input { .. } => Some(String::new()),
                PiUiDialog::Editor { prefill, .. } => Some(prefill.unwrap_or_default()),
                _ => None,
            };
            if let Some(prefill) = prefill {
                active.insert(key.clone());
                state.pi_replies.edits.entry(key).or_insert_with(|| {
                    let mut edit = TextEdit::default();
                    edit.replace_text(prefill);
                    edit
                });
            }
        }
    }
    state.pi_replies.edits.retain(|key, _| active.contains(key));
    state
        .pi_replies
        .selected
        .retain(|key, _| active.contains(key));
}

fn button(
    typography: super::preferences::Typography,
    label: String,
    key: String,
    enabled: bool,
    handler: impl Fn(&mut State, &mut Tx) + 'static,
) -> El {
    let control = typography
        .text(label.clone())
        .key(key)
        .min_h(28.0)
        .role("button")
        .focusable(enabled)
        .disabled(!enabled)
        .wrap()
        .label(label);
    if enabled {
        control
            .native_activation()
            .press(oklch(0.6, 0.0, 0.0).alpha(0.2))
            .on_press_tx(move |state: &mut State, tx| {
                state.sound(crate::interaction_audio::Cue::Press);
                handler(state, tx)
            })
    } else {
        control
    }
}

fn reply(state: &mut State, run: RunId, request: RunId, answer: Answer, tx: &mut Tx) {
    let Some(core) = state.core().cloned() else {
        return;
    };
    let Some(updates) = state.updates.clone() else {
        return;
    };
    match core.rune_pi_reply(run, request, answer, tx) {
        Ok(work) => spawn_local(async move {
            updates.finish(work.await);
        }),
        Err(error) => state.fail(error),
    }
}

fn action(
    typography: super::preferences::Typography,
    run: &RunId,
    request: &RunId,
    answer: Answer,
    label: String,
    enabled: bool,
) -> El {
    let key = pi_reply_key(run, request, &answer);
    let run = run.clone();
    let request = request.clone();
    button(typography, label, key, enabled, move |state, tx| {
        let answer = match &answer {
            Answer::Select(value) => Answer::Select(value.clone()),
            Answer::Confirm(value) => Answer::Confirm(*value),
            Answer::Cancel => Answer::Cancel,
            Answer::Text(_) => return,
        };
        reply(state, run.clone(), request.clone(), answer, tx);
    })
}

pub(super) fn view(state: &State, core: &Core) -> Vec<El> {
    let typography = state.preferences.typography();
    let palette = state.preferences.palette();
    let (ui, enabled, suggestion_enabled, notice, workflow) = core.rune_pi_snapshot();
    let mut result = Vec::new();
    if let Some(notice) = notice {
        result.push(typography.text(notice).wrap());
    }
    let Some(ui) = ui else { return result };
    if let Some(title) = &ui.title {
        result.push(typography.text(crate::pi_ui::display_plain(title)).wrap());
    }
    for status in &ui.statuses {
        if let Some(value) = &status.text {
            result.push(typography.text(crate::pi_ui::display_plain(value)).wrap());
        }
    }
    for widget in ui
        .widgets
        .iter()
        .filter(|widget| widget.placement == "aboveEditor")
    {
        if let Some(lines) = &widget.lines {
            for line in lines {
                result.push(typography.text(crate::pi_ui::display_plain(line)).wrap());
            }
        }
    }
    for notice in &ui.notices {
        result.push(
            typography
                .text(format!(
                    "{}: {}",
                    crate::pi_ui::display_plain(&notice.level),
                    crate::pi_ui::display_plain(&notice.message)
                ))
                .wrap(),
        );
    }
    if ui.notices_omitted > 0 {
        result.push(
            typography
                .text(format!("{} earlier notices omitted", ui.notices_omitted))
                .wrap(),
        );
    }
    for pending in &ui.pending {
        let request = &pending.request;
        let title = match &pending.dialog {
            PiUiDialog::Select { title, .. }
            | PiUiDialog::Confirm { title, .. }
            | PiUiDialog::Input { title, .. }
            | PiUiDialog::Editor { title, .. } => crate::pi_ui::display_plain(title),
        };
        let title = if title.trim().is_empty() {
            "Pi request".to_owned()
        } else {
            title
        };
        let mut panel = col().w_full().gap(8.0);
        match &pending.dialog {
            PiUiDialog::Select { options, .. } => {
                panel = panel.child(typography.text(title.clone()).wrap());
                let entry = edit_key(&ui.run, request);
                let selected = state.pi_replies.selected.get(&entry);
                let mut seen = HashSet::new();
                for (index, option) in options.iter().enumerate() {
                    if !seen.insert(option) {
                        continue;
                    }
                    let label = crate::pi_ui::display_plain(option);
                    let choice = if label.trim().is_empty() {
                        format!("Option {}", index + 1)
                    } else {
                        format!("Option {}: {label}", index + 1)
                    };
                    let label = format!(
                        "{} {choice}",
                        if selected == Some(option) {
                            "Selected:"
                        } else {
                            "Select:"
                        }
                    );
                    let key = serde_json::to_string(&(
                        "pi-choice",
                        ui.run.as_str(),
                        request.as_str(),
                        option,
                    ))
                    .expect("string key");
                    let entry = entry.clone();
                    let value = option.clone();
                    panel = panel.child(
                        button(typography, label.clone(), key, enabled, move |state, _| {
                            state
                                .pi_replies
                                .selected
                                .insert(entry.clone(), value.clone());
                        })
                        .label(format!("{label} for {title}")),
                    );
                }
                if let Some(value) = selected {
                    panel = panel.child(
                        action(
                            typography,
                            &ui.run,
                            request,
                            Answer::Select(value.clone()),
                            "Send".into(),
                            enabled,
                        )
                        .label(format!("Send selection for {title}")),
                    );
                } else {
                    let key =
                        serde_json::to_string(&("pi-no-choice", ui.run.as_str(), request.as_str()))
                            .expect("string key");
                    panel = panel.child(
                        button(typography, "Send".into(), key, false, |_, _| {})
                            .label(format!("Send selection for {title}")),
                    );
                }
            }
            PiUiDialog::Confirm { message, .. } => {
                panel = panel
                    .child(typography.text(title.clone()).wrap())
                    .child(typography.text(crate::pi_ui::display_plain(message)).wrap());
                panel = panel
                    .child(
                        action(
                            typography,
                            &ui.run,
                            request,
                            Answer::Confirm(true),
                            "Yes".into(),
                            enabled,
                        )
                        .label(format!("Yes for {title}")),
                    )
                    .child(
                        action(
                            typography,
                            &ui.run,
                            request,
                            Answer::Confirm(false),
                            "No".into(),
                            enabled,
                        )
                        .label(format!("No for {title}")),
                    );
            }
            PiUiDialog::Input { placeholder, .. }
            | PiUiDialog::Editor {
                prefill: placeholder,
                ..
            } => {
                let label = title.clone();
                panel = panel.child(typography.text(label.clone()).wrap());
                let key = edit_key(&ui.run, request);
                if let Some(edit) = state.pi_replies.edits.get(&key) {
                    let entry = key.clone();
                    let ink = palette.ink;
                    let mut field = ::rune::el::text_input(
                        edit,
                        TextStyle::new((16.0 * typography.scale).max(16.0), 400.0),
                    )
                    .key(format!("pi-input:{key}"))
                    .w_full()
                    .min_h(36.0)
                    .padding(8.0)
                    .radius(5.0)
                    .border(1.0, palette.line)
                    .fill(palette.field)
                    .color(ink)
                    .caret(ink, ink.alpha(0.25))
                    .label(label)
                    .disabled(!enabled)
                    .on_edit(::rune::edit_access(move |state: &mut State| {
                        state
                            .pi_replies
                            .edits
                            .get_mut(&entry)
                            .expect("synced Pi field")
                    }));
                    if matches!(&pending.dialog, PiUiDialog::Input { .. }) {
                        if let Some(placeholder) = placeholder {
                            field =
                                field.placeholder(crate::pi_ui::display_plain(placeholder), ink);
                        }
                    } else {
                        field = field.min_h(80.0);
                    }
                    panel = panel.child(field);
                }
                let action_key = pi_reply_key(&ui.run, request, &Answer::Text(String::new()));
                let run = ui.run.clone();
                let request = request.clone();
                panel = panel.child(
                    button(
                        typography,
                        "Submit".into(),
                        action_key,
                        enabled,
                        move |state, tx| {
                            let key = edit_key(&run, &request);
                            let Some(edit) = state.pi_replies.edits.get(&key) else {
                                return;
                            };
                            if edit.preedit.is_some() {
                                state.fail(FlowError::InvalidInput(
                                    "Finish composing your answer before submitting.".into(),
                                ));
                                return;
                            }
                            let value = edit.text.clone();
                            reply(state, run.clone(), request.clone(), Answer::Text(value), tx);
                        },
                    )
                    .label(format!("Submit for {title}")),
                );
            }
        }
        panel = panel.child(
            action(
                typography,
                &ui.run,
                request,
                Answer::Cancel,
                "Cancel".into(),
                enabled,
            )
            .label(format!("Cancel {title}")),
        );
        result.push(panel);
    }
    for widget in ui
        .widgets
        .iter()
        .filter(|widget| widget.placement == "belowEditor")
    {
        if let Some(lines) = &widget.lines {
            for line in lines {
                result.push(typography.text(crate::pi_ui::display_plain(line)).wrap());
            }
        }
    }
    if let Some(editor) = &ui.editor {
        let destination = state.selection.clone();
        if workflow
            .as_deref()
            .is_some_and(|workflow| destination == "pi" || destination == workflow)
        {
            result.push(typography.text("Suggested text").wrap());
            result.push(
                typography
                    .text(crate::pi_ui::display_plain(&editor.text))
                    .wrap(),
            );
            let key = pi_suggestion_key(&ui.run, editor.revision, &destination);
            let run = ui.run.clone();
            let revision = editor.revision;
            result.push(button(
                typography,
                if state.draft.text.is_empty() {
                    "Use suggestion"
                } else {
                    "Replace draft"
                }
                .into(),
                key,
                suggestion_enabled,
                move |state, tx| {
                    if state.selection != destination {
                        return;
                    }
                    if state.draft.preedit.is_some() {
                        state.fail(FlowError::InvalidInput(
                            "Finish composing your draft before replacing it.".into(),
                        ));
                        return;
                    }
                    let Some(core) = state.core().cloned() else {
                        return;
                    };
                    match core.rune_pi_suggestion(&run, revision, &destination, tx) {
                        Ok(value) => {
                            state.replace_draft(value);
                            state.menu = None;
                            tx.focus("rune-composer-field");
                        }
                        Err(error) => state.fail(error),
                    }
                },
            ));
        }
    }
    result
}
