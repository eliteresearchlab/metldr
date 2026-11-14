//! The consent card for one secret entry (decision 09, B3.5).
//!
//! The user types the secret into a masked field that never echoes, copies or saves it. Enter
//! sends it for this entry only and wipes the field; any change of the pending entry wipes it too.

use super::{State, Tx, browser_actions, preferences};
use crate::browser_action_display::origin;
use crate::model::{BrowserActionId, BrowserActionIntent, BrowserActionPhase};
use ::rune::{El, TextEdit, TextStyle, col};
use wasm_bindgen_futures::spawn_local;

pub(super) struct SecretEntry {
    edit: TextEdit,
    target: Option<(BrowserActionId, u64)>,
}

impl Default for SecretEntry {
    fn default() -> Self {
        Self {
            edit: TextEdit::default().secure(),
            target: None,
        }
    }
}

/// Wipes the typed secret once its entry is no longer the one pending.
pub(super) fn sync(state: &mut State) {
    let pending = state.core().and_then(|core| {
        core.rune_browser_action().and_then(|(action, ..)| {
            matches!(action.intent, BrowserActionIntent::EnterSecret { .. })
                .then_some((action.id, action.revision))
        })
    });
    if pending != state.secret.target {
        state.secret.edit.clear();
        state.secret.target = pending;
    }
}

pub(super) fn pending(row: &browser_actions::UiBrowserAction) -> bool {
    row.action.phase == BrowserActionPhase::Pending
        && matches!(row.action.intent, BrowserActionIntent::EnterSecret { .. })
}

fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_millis() as f64)
    }
}

fn enter(state: &mut State, decision: browser_actions::DecisionKey, tx: &mut Tx) {
    let Some(core) = state.core().cloned() else {
        return;
    };
    let Some(updates) = state.updates.clone() else {
        return;
    };
    if state.secret.target.as_ref() != Some(&(decision.id.clone(), decision.revision)) {
        state.secret.edit.clear();
        return;
    }
    let secret = std::mem::take(&mut state.secret.edit.text);
    state.secret.edit.clear();
    match core.enter_secret_from_rune(decision, secret, tx) {
        Ok(work) => {
            state.sound(crate::interaction_audio::Cue::Press);
            spawn_local(async move {
                updates.finish(work.await);
            });
        }
        Err(error) => state.fail(error),
    }
}

fn decline(state: &mut State, decision: browser_actions::DecisionKey, tx: &mut Tx) {
    state.secret.edit.clear();
    state.decide_browser_action(decision, tx);
}

pub(super) fn card(
    row: &browser_actions::UiBrowserAction,
    state: &State,
    typography: preferences::Typography,
) -> El {
    let action = &row.action;
    let BrowserActionIntent::EnterSecret {
        field_label,
        secret,
        expires_at_ms,
        ..
    } = &action.intent
    else {
        return col();
    };
    let palette = state.preferences.palette();
    let ink = palette.ink;
    let site = origin(&action.url);
    let proposer = state
        .core()
        .and_then(|core| {
            core.page
                .view
                .borrow()
                .task_plan
                .as_ref()
                .filter(|plan| plan.run == action.run)
                .map(|plan| plan.proposer.clone())
        })
        .unwrap_or_else(|| "Agent".into());
    let remaining_s = ((*expires_at_ms as f64 - now_ms()) / 1000.0).max(0.0) as u64;
    let expiry = if remaining_s == 0 {
        "Expired".to_owned()
    } else {
        format!("Expires in {} min", remaining_s.div_ceil(60))
    };
    let typed = state.secret.target.as_ref() == Some(&(action.id.clone(), action.revision))
        && !state.secret.edit.text.is_empty();
    let field = ::rune::el::text_input(
        &state.secret.edit,
        TextStyle::new((16.0 * typography.scale).max(16.0), 400.0),
    )
    .key("secret-entry-field")
    .label(format!("{} for {site}", secret.label()))
    .w_full()
    .min_h(36.0)
    .padding(6.0)
    .single_line()
    .secure()
    .fill(palette.field)
    .border(1.0, palette.line)
    .color(ink)
    .caret(ink, palette.selection)
    .disabled(!row.approve)
    .on_edit(|state: &mut State| &mut state.secret.edit);
    let mut card = col()
        .key(format!("browser-action:{}", action.id.as_str()))
        .role("group")
        .label("Secret entry")
        .w_full()
        .gap(8.0)
        .child(typography.text(row.display.title).wrap())
        .child(typography.text(format!("{proposer} asks for a {} on {site}", secret.label().to_lowercase())).wrap())
        .child(typography.text(format!("Field: {field_label}")).wrap())
        .child(typography.text(&expiry).role("status"))
        .child(field)
        .child(
            typography
                .text("Only this site gets it. The agent never sees it.")
                .wrap(),
        );
    for (approve, enabled, label) in [
        (true, row.approve && typed && remaining_s > 0, "Enter"),
        (false, row.decline, "Decline"),
    ] {
        let decision = browser_actions::DecisionKey {
            id: action.id.clone(),
            revision: action.revision,
            approve,
        };
        card = card.child(
            typography
                .text(label)
                .key(decision.element_key())
                .role("button")
                .focusable(true)
                .disabled(!enabled)
                .label(format!("{label} {} for {site}", secret.label().to_lowercase()))
                .native_activation()
                .on_press_tx(move |state: &mut State, tx| {
                    if approve {
                        enter(state, decision.clone(), tx)
                    } else {
                        decline(state, decision.clone(), tx)
                    }
                }),
        );
    }
    card
}
