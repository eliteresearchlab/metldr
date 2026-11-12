use super::*;
use crate::model::{PiUiDialog, PiUiView, RunRoute, RunStatus};
use crate::pi_ui::display_plain;

mod display;
mod observe;
pub use display::*;
pub(super) use observe::Observer;
pub use observe::UiPiSubscription;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Key {
    run: RunId,
    request: RunId,
}
impl Key {
    fn offered(&self, ui: Option<&PiUiView>) -> bool {
        ui.is_some_and(|ui| {
            ui.run == self.run && ui.pending.iter().any(|p| p.request == self.request)
        })
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) enum Operation {
    #[default]
    Idle,
    Sending(Key),
    Waiting(Key),
    Uncertain(Key),
}
impl Operation {
    fn observe(&mut self, ui: Option<&PiUiView>) {
        if let Self::Waiting(key) = self
            && !key.offered(ui)
        {
            *self = Self::Idle;
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct State {
    connected: bool,
    stale: bool,
    ready: bool,
    recovery_required: bool,
    running: bool,
    workflow: Option<String>,
    ui: Option<PiUiView>,
    operation: Operation,
}
impl State {
    fn read(core: &Core) -> Self {
        let view = core.page.view.borrow();
        let run = view
            .pi_ui
            .as_ref()
            .and_then(|ui| view.runs.iter().find(|r| r.id == ui.run));
        let workflow = run.and_then(|r| match &r.route {
            RunRoute::Pi { workflow, .. } => Some(workflow.as_str().to_owned()),
            _ => None,
        });
        Self {
            connected: core.flow().is_connected(),
            stale: core.page.invalidated.get()
                || view.selection != core.scope
                || !core
                    .page
                    .current
                    .borrow()
                    .upgrade()
                    .is_some_and(|p| std::ptr::eq(p.as_ref(), core)),
            ready: core.owner.borrow().readable().is_ok()
                && !core.page.recovery.borrow().active()
                && !matches!(*core.owner.borrow(), Owner::Uncertain),
            recovery_required: matches!(*core.owner.borrow(), Owner::Uncertain),
            running: workflow.is_some() && run.is_some_and(|r| r.status == RunStatus::Running),
            workflow,
            ui: view.pi_ui.clone(),
            operation: core.pi_operation.borrow().clone(),
        }
    }
    fn enabled(&self) -> bool {
        self.connected
            && !self.stale
            && self.ready
            && self.running
            && matches!(self.operation, Operation::Idle)
    }
}

pub(super) enum Answer {
    Select(String),
    Confirm(bool),
    Text(String),
    Cancel,
}
impl Answer {
    fn offered(&self, dialog: &PiUiDialog) -> bool {
        match (self, dialog) {
            (Self::Select(value), PiUiDialog::Select { options, .. }) => options.contains(value),
            (Self::Confirm(_), PiUiDialog::Confirm { .. }) => true,
            (Self::Text(value), PiUiDialog::Input { .. } | PiUiDialog::Editor { .. }) => {
                serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 32768 - 512)
            }
            (Self::Cancel, _) => true,
            _ => false,
        }
    }
}

#[wasm_bindgen]
pub struct UiPiUi {
    core: Rc<Core>,
    activation: super::activation::Activation,
}
#[wasm_bindgen]
impl UiBridge {
    pub fn pi_ui(&self) -> Result<UiPiUi, JsValue> {
        Ok(UiPiUi {
            core: Rc::clone(&self.core),
            activation: super::activation::Activation::new().map_err(js_error)?,
        })
    }
}
#[wasm_bindgen]
impl UiPiUi {
    pub fn subscribe(
        &self,
        #[wasm_bindgen(unchecked_param_type = "(snapshot: UiPiSnapshot) => void")]
        callback: Function,
    ) -> Result<UiPiSubscription, JsValue> {
        Observer::subscribe(Rc::clone(&self.core), callback)
    }
    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn select(
        &self,
        run: String,
        request: String,
        value: String,
        #[wasm_bindgen(unchecked_param_type = "Event")] event: JsValue,
    ) -> Result<Promise, JsValue> {
        self.send(run, request, Answer::Select(value), event)
    }
    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn confirm(
        &self,
        run: String,
        request: String,
        value: bool,
        #[wasm_bindgen(unchecked_param_type = "Event")] event: JsValue,
    ) -> Result<Promise, JsValue> {
        self.send(run, request, Answer::Confirm(value), event)
    }
    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn text(
        &self,
        run: String,
        request: String,
        value: String,
        #[wasm_bindgen(unchecked_param_type = "Event")] event: JsValue,
    ) -> Result<Promise, JsValue> {
        self.send(run, request, Answer::Text(value), event)
    }
    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn cancel(
        &self,
        run: String,
        request: String,
        #[wasm_bindgen(unchecked_param_type = "Event")] event: JsValue,
    ) -> Result<Promise, JsValue> {
        self.send(run, request, Answer::Cancel, event)
    }
    pub fn editor_text(
        &self,
        run: String,
        revision: String,
        destination: String,
        #[wasm_bindgen(unchecked_param_type = "Event")] event: JsValue,
    ) -> Result<String, JsValue> {
        self.activation.require(&event).map_err(js_error)?;
        self.core
            .pi_editor_text(&run, &revision, &destination)
            .map_err(js_error)
    }
}

impl UiPiUi {
    fn send(
        &self,
        run: String,
        request: String,
        answer: Answer,
        event: JsValue,
    ) -> Result<Promise, JsValue> {
        self.activation.require(&event).map_err(js_error)?;
        let parse = |id| {
            serde_json::from_value(serde_json::Value::String(id))
                .map_err(|_| crate::err("Invalid Pi request identity."))
        };
        let key = Key {
            run: parse(run)?,
            request: parse(request)?,
        };
        let work = self.core.reserve_pi_reply(key, answer).map_err(js_error)?;
        Ok(future_to_promise(async move {
            work.await.map(|()| JsValue::UNDEFINED).map_err(js_error)
        }))
    }
}
impl Core {
    fn pi_editor_text(
        &self,
        run: &str,
        revision: &str,
        destination: &str,
    ) -> Result<String, FlowError> {
        let state = State::read(self);
        let editor = state
            .ui
            .as_ref()
            .filter(|ui| ui.run.as_str() == run)
            .and_then(|ui| ui.editor.as_ref());
        if state.connected
            && !state.stale
            && state.ready
            && !matches!(state.operation, Operation::Uncertain(_))
            && state
                .workflow
                .as_deref()
                .is_some_and(|workflow| destination == "pi" || destination == workflow)
        {
            if let Some(editor) = editor
                && editor.revision.to_string() == revision
            {
                return Ok(editor.text.clone());
            }
        }
        Err(FlowError::InvalidInput(
            "This Pi suggestion is no longer available.".into(),
        ))
    }

    #[cfg(feature = "rune-ui")]
    pub(super) fn rune_pi_snapshot(
        &self,
    ) -> (Option<PiUiView>, bool, bool, Option<String>, Option<String>) {
        let state = State::read(self);
        let enabled = state.enabled();
        let snapshot = UiPiSnapshot {
            state: state.clone(),
        };
        let suggestion_enabled = snapshot.editor_enabled() && !snapshot.busy();
        let notice = snapshot.notice();
        (
            state.ui,
            enabled,
            suggestion_enabled,
            notice,
            state.workflow,
        )
    }

    #[cfg(feature = "rune-ui")]
    pub(super) fn rune_pi_reply(
        self: &Rc<Self>,
        run: RunId,
        request: RunId,
        answer: Answer,
        tx: &mut ::rune::Tx,
    ) -> Result<impl Future<Output = Result<(), FlowError>> + 'static, FlowError> {
        if !tx.consume_activation(&pi_reply_key(&run, &request, &answer)) {
            return Err(FlowError::InvalidInput(
                "Use a current action to answer the request.".into(),
            ));
        }
        self.reserve_pi_reply(Key { run, request }, answer)
    }

    #[cfg(feature = "rune-ui")]
    pub(super) fn rune_pi_suggestion(
        &self,
        run: &RunId,
        revision: u64,
        destination: &str,
        tx: &mut ::rune::Tx,
    ) -> Result<String, FlowError> {
        if !tx.consume_activation(&pi_suggestion_key(run, revision, destination)) {
            return Err(FlowError::InvalidInput(
                "Use the current suggestion control.".into(),
            ));
        }
        let state = State::read(self);
        if !matches!(state.operation, Operation::Idle) {
            return Err(FlowError::InvalidInput(
                "This Pi suggestion is no longer available.".into(),
            ));
        }
        self.pi_editor_text(run.as_str(), &revision.to_string(), destination)
    }

    fn reserve_pi_reply(
        self: &Rc<Self>,
        key: Key,
        answer: Answer,
    ) -> Result<impl Future<Output = Result<(), FlowError>> + 'static, FlowError> {
        let state = State::read(self);
        let offered = state
            .ui
            .as_ref()
            .filter(|ui| ui.run == key.run)
            .and_then(|ui| ui.pending.iter().find(|p| p.request == key.request));
        if !state.enabled() || !offered.is_some_and(|p| answer.offered(&p.dialog)) {
            return Err(FlowError::InvalidInput(
                "This Pi reply is no longer available or was not offered.".into(),
            ));
        }
        *self.pi_operation.borrow_mut() = Operation::Sending(key.clone());
        self.notify_pi_ui();
        let core = Rc::clone(self);
        Ok(async move {
            let (kind, text, confirm) = match answer {
                Answer::Select(value) => ("select", Some(value), None),
                Answer::Confirm(value) => ("confirm", None, Some(value)),
                Answer::Text(value) => ("text", Some(value), None),
                Answer::Cancel => ("cancel", None, None),
            };
            let result = core
                .flow()
                .pi_ui_reply(
                    core.scope.clone(),
                    key.run.clone(),
                    key.request.clone(),
                    kind,
                    text,
                    confirm,
                )
                .await;
            let result = match result {
                Ok(view) => match core.accept(view) {
                    Ok(_) => {
                        *core.pi_operation.borrow_mut() = Operation::Waiting(key);
                        Ok(())
                    }
                    Err(error) => {
                        *core.pi_operation.borrow_mut() = Operation::Uncertain(key);
                        Err(error)
                    }
                },
                Err(error) => {
                    *core.pi_operation.borrow_mut() = Operation::Uncertain(key);
                    Err(error)
                }
            };
            core.notify_pi_ui();
            result
        })
    }

    pub(super) fn notify_pi_ui(&self) {
        self.page.notify_connection();
        {
            let view = self.page.view.borrow();
            if view.selection == self.scope {
                self.pi_operation.borrow_mut().observe(view.pi_ui.as_ref());
            }
        }
        let observers: Vec<_> = {
            let mut list = self.pi_subscriptions.borrow_mut();
            list.retain(|weak| weak.upgrade().is_some_and(|o| !o.closed()));
            list.iter().filter_map(std::rc::Weak::upgrade).collect()
        };
        for observer in observers {
            if observer.emit().is_err() {
                observer.close();
            }
            observer.wake();
        }
        let changed = self.page.changed.borrow().clone();
        if let Some(changed) = changed {
            changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "rune-ui")]
    #[test]
    fn activation_keys_bind_exact_choice_and_operation() {
        let id = |n| {
            serde_json::from_value(serde_json::json!(format!(
                "0123456789abcdef0123456789abcdef:{n}"
            )))
            .unwrap()
        };
        let (run, request, other) = (id(1), id(2), id(3));
        let selected = pi_reply_key(&run, &request, &Answer::Select("a\nb".into()));
        assert_ne!(
            selected,
            pi_reply_key(&run, &request, &Answer::Select("a b".into()))
        );
        assert_ne!(selected, pi_reply_key(&run, &request, &Answer::Cancel));
        assert_ne!(
            selected,
            pi_reply_key(&run, &other, &Answer::Select("a\nb".into()))
        );
        assert_ne!(
            selected,
            pi_reply_key(&other, &request, &Answer::Select("a\nb".into()))
        );
        assert_ne!(
            pi_reply_key(&run, &request, &Answer::Confirm(true)),
            pi_reply_key(&run, &request, &Answer::Confirm(false))
        );
        assert_ne!(
            pi_suggestion_key(&run, 1, "pi"),
            pi_suggestion_key(&run, 2, "pi")
        );
        assert_ne!(
            pi_suggestion_key(&run, 1, "pi"),
            pi_suggestion_key(&run, 1, "other")
        );
    }
    #[test]
    fn exact_answers_keep_raw_values_and_unknown_receipts_stay_locked() {
        let raw = "\u{1b}[31m<b>choice</b>\u{1b}[0m";
        let dialog = PiUiDialog::Select {
            title: "Choose".into(),
            options: vec![raw.into()],
        };
        assert!(Answer::Select(raw.into()).offered(&dialog));
        assert!(!Answer::Select(display_plain(raw)).offered(&dialog));
        assert!(!Answer::Text(raw.into()).offered(&dialog));
        let input = PiUiDialog::Input {
            title: "Name".into(),
            placeholder: None,
        };
        assert!(Answer::Text("مرحبا 🌱".into()).offered(&input));
        assert!(!Answer::Text("\n".repeat(16384)).offered(&input));
        let id = |n| {
            serde_json::from_value(serde_json::json!(format!(
                "0123456789abcdef0123456789abcdef:{n}"
            )))
            .unwrap()
        };
        let key = Key {
            run: id(1),
            request: id(2),
        };
        let mut waiting = Operation::Waiting(key.clone());
        waiting.observe(None);
        assert!(matches!(waiting, Operation::Idle));
        let mut sending = Operation::Sending(key.clone());
        sending.observe(None);
        assert!(matches!(sending, Operation::Sending(_)));
        let mut unknown = Operation::Uncertain(key);
        unknown.observe(None);
        assert!(matches!(unknown, Operation::Uncertain(_)));
    }
}

#[cfg(feature = "rune-ui")]
pub(super) fn pi_reply_key(run: &RunId, request: &RunId, answer: &Answer) -> String {
    let (kind, value) = match answer {
        Answer::Select(value) => ("select", Some(value.as_str())),
        Answer::Confirm(true) => ("confirm-yes", None),
        Answer::Confirm(false) => ("confirm-no", None),
        Answer::Text(_) => ("text", None),
        Answer::Cancel => ("cancel", None),
    };
    serde_json::to_string(&("pi", run.as_str(), request.as_str(), kind, value)).expect("string key")
}
#[cfg(feature = "rune-ui")]
pub(super) fn pi_suggestion_key(run: &RunId, revision: u64, destination: &str) -> String {
    serde_json::to_string(&("pi-suggestion", run.as_str(), revision, destination))
        .expect("string key")
}
