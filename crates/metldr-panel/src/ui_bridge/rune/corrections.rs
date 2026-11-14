use super::{Core, FlowError, State, Tx};
use crate::model::{RunId, UserText};
use crate::ui_bridge::corrections::correction_key;
use ::rune::{El, col};
use std::rc::{Rc, Weak};
use wasm_bindgen_futures::spawn_local;
#[cfg(target_arch = "wasm32")]
use {crate::ui_bridge::activation::SubmissionActivation, std::cell::OnceCell};

const RETRY_MESSAGE: &str = "Correction not sent. Press Enter again.";

struct Submitted {
    core: Weak<Core>,
    request: String,
    text: String,
    draft_version: u64,
}

#[derive(Default)]
pub(super) struct Corrections {
    submitted: Option<Submitted>,
    #[cfg(target_arch = "wasm32")]
    activation: OnceCell<Option<Rc<SubmissionActivation>>>,
}

pub(super) fn sync(state: &mut State) {
    let Some(core) = state.core().cloned() else {
        state.corrections.submitted = None;
        return;
    };
    let Some(submitted) = &state.corrections.submitted else {
        return;
    };
    if !submitted
        .core
        .upgrade()
        .is_some_and(|owner| Rc::ptr_eq(&owner, &core))
    {
        state.corrections.submitted = None;
        return;
    }
    let progress = core.rune_correction_progress();
    match progress {
        Some((request, true, _)) if request == submitted.request => {
            if state.draft_version == submitted.draft_version
                && state.draft.preedit.is_none()
                && state.draft.text == submitted.text
            {
                state.clear_draft();
            }
            state.corrections.submitted = None;
        }
        None => state.corrections.submitted = None,
        _ => {}
    }
}

pub(super) fn submit(
    state: &mut State,
    expected_core: &Rc<Core>,
    run: &RunId,
    draft_version: u64,
    draft: &str,
    tx: &mut Tx,
) -> Result<bool, FlowError> {
    if state.draft_version != draft_version
        || state.draft.preedit.is_some()
        || state.draft.text != draft
        || state.corrections.submitted.is_some()
    {
        return Ok(false);
    }
    let Some(core) = state.core().cloned() else {
        return Ok(false);
    };
    let (current_run, available) = core.rune_correction_offer();
    if !Rc::ptr_eq(&core, expected_core) || !available || current_run.as_ref() != Some(run) {
        return Ok(false);
    }
    let Some(updates) = state.updates.clone() else {
        return Ok(false);
    };
    let (request, work) = core.rune_correction(run, draft_version, draft.to_owned(), tx)?;
    if updates.presentation.borrow().error.as_deref() == Some(RETRY_MESSAGE) {
        updates.presentation.borrow_mut().error = None;
    }
    state.corrections.submitted = Some(Submitted {
        core: Rc::downgrade(&core),
        request,
        text: draft.to_owned(),
        draft_version,
    });
    state.menu = None;
    spawn_local(async move {
        updates.finish(work.await);
    });
    state.sound(crate::interaction_audio::Cue::Pulse);
    Ok(true)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn attach_field(field: El, state: &State) -> El {
    let Some(core) = state.core().cloned() else {
        return field;
    };
    let (Some(run), true) = core.rune_correction_offer() else {
        return field;
    };
    if state.corrections.submitted.is_some()
        || state.draft.preedit.is_some()
        || super::composer::source_menu_owns_enter(state)
        || UserText::try_from(state.draft.text.as_str()).is_err()
    {
        return field;
    }
    let Some(activation) = state
        .corrections
        .activation
        .get_or_init(|| SubmissionActivation::new().ok().map(Rc::new))
        .clone()
    else {
        return field;
    };
    let draft = state.draft.text.clone();
    let draft_version = state.draft_version;
    field.on_native_submit(
        correction_key(&run, draft_version),
        move |state: &mut State, event, tx| {
            if activation.require(event.as_ref()).is_err()
                || super::composer::source_menu_owns_enter(state)
            {
                return false;
            }
            matches!(
                submit(state, &core, &run, draft_version, &draft, tx),
                Ok(true)
            )
        },
    )
}

/// A correction requires the original trusted event; keep the draft if it was not admitted.
pub(super) fn focus(state: &State, _: &mut Tx) -> bool {
    let Some(core) = state.core() else {
        return false;
    };
    let (run, available) = core.rune_correction_offer();
    if run.is_none()
        || !available
        || state.corrections.submitted.is_some()
        || state.draft.preedit.is_some()
        || UserText::try_from(state.draft.text.as_str()).is_err()
    {
        return false;
    }
    state.fail(FlowError::InvalidInput(RETRY_MESSAGE.into()));
    true
}

/// Present next to Stop when an active run exposes the correction target.
pub(super) fn control(state: &State) -> Option<El> {
    let core = state.core()?.clone();
    let (run, available) = core.rune_correction_offer();
    let run = run?;
    let draft = state.draft.text.clone();
    let draft_version = state.draft_version;
    let enabled = available
        && state.corrections.submitted.is_none()
        && state.draft.preedit.is_none()
        && UserText::try_from(draft.as_str()).is_ok();
    let control = state
        .preferences
        .typography()
        .text("Send correction")
        .key(correction_key(&run, draft_version))
        .min_h(28.0)
        .role("button")
        .focusable(enabled)
        .disabled(!enabled)
        .label("Send correction");
    Some(if enabled {
        control
            .native_activation()
            .press(state.preferences.palette().press)
            .press_scale(super::theme::PRESS_SCALE)
            .on_press_tx(move |state: &mut State, tx| {
                if let Err(error) = submit(state, &core, &run, draft_version, &draft, tx) {
                    state.fail(error);
                }
            })
    } else {
        control
    })
}

pub(super) fn view(state: &State, core: &Core) -> Vec<El> {
    let typography = state.preferences.typography();
    let snapshot = core.rune_correction_snapshot();
    let run = snapshot.run_id();
    let mut result = Vec::new();
    let receipts = snapshot.receipts();
    if let Some(run) = &run {
        for receipt in receipts.iter().filter(|receipt| receipt.run_id() == *run) {
            let mut entry = col()
                .w_full()
                .gap(4.0)
                .child(typography.text(receipt.label()).wrap())
                .child(typography.text(receipt.text()).wrap());
            if receipt.truncated() {
                entry = entry.child(typography.text("Correction preview truncated").wrap());
            }
            if let Some(detail) = receipt.detail() {
                entry = entry.child(typography.text(detail).wrap());
            }
            result.push(entry);
        }
    }
    if snapshot.pending() {
        result.push(typography.text("Correction pending").wrap());
    }
    if snapshot.uncertain() {
        result.push(typography.text("Correction status is unconfirmed.").wrap());
        result.push(
            typography
                .text("Check correction")
                .key("correction-refresh")
                .min_h(28.0)
                .role("button")
                .focusable(true)
                .label("Check correction")
                .native_activation()
                .press(state.preferences.palette().press)
                .press_scale(super::theme::PRESS_SCALE)
                .on_press_tx(|state: &mut State, _| {
                    let Some(core) = state.core().cloned() else {
                        return;
                    };
                    let Some(updates) = state.updates.clone() else {
                        return;
                    };
                    state.sound(crate::interaction_audio::Cue::Press);
                    spawn_local(async move {
                        updates.finish(core.refresh_correction().await);
                    });
                }),
        );
    }
    result
}
