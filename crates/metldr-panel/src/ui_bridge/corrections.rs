use super::*;
use crate::model::{CorrectionHistory, CorrectionReceipt, CorrectionState, RunRoute, RunStatus};

mod observe;
pub(super) use observe::Observer;
pub use observe::UiNativeCorrectionSubscription;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Pending {
    run: RunId,
    request: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) enum Operation {
    #[default]
    Idle,
    Sending(Pending),
    Waiting(Pending),
    Uncertain(Pending),
    Recorded(Pending),
}
impl Operation {
    pub(super) fn blocked(&self) -> bool {
        !matches!(self, Self::Idle | Self::Recorded(_))
    }
    pub(super) fn sending(&self) -> bool {
        matches!(self, Self::Sending(_))
    }
    pub(super) fn uncertain(&self) -> bool {
        matches!(self, Self::Uncertain(_))
    }
    fn pending(&self) -> Option<&Pending> {
        match self {
            Self::Idle | Self::Recorded(_) => None,
            Self::Sending(p) | Self::Waiting(p) | Self::Uncertain(p) => Some(p),
        }
    }
    fn owns(&self, pending: &Pending) -> bool {
        self.pending() == Some(pending)
    }
    fn recorded(&self, pending: &Pending) -> bool {
        matches!(self, Self::Recorded(saved) if saved == pending)
    }
    pub(super) fn observe(&mut self, view: &PageView, scope: &Scope) {
        if &view.selection == scope
            && self.pending().is_some_and(|p| {
                view.runs.iter().any(|run| {
                    run.id == p.run
                        && run
                            .corrections
                            .entries
                            .iter()
                            .any(|r| r.request == p.request)
                })
            })
        {
            if let Some(pending) = self.pending().cloned() {
                *self = Self::Recorded(pending);
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct State {
    connected: bool,
    stale: bool,
    available: bool,
    operation: Operation,
    run: Option<RunId>,
    receipts: Vec<UiCorrectionReceipt>,
}
impl State {
    fn candidate(&self, expected: Option<&RunId>) -> Option<RunId> {
        self.run.clone().filter(|run| {
            self.available
                && !self.operation.blocked()
                && expected.is_none_or(|expected| expected == run)
        })
    }
    fn read(core: &Core) -> Self {
        let view = core.page.view.borrow();
        let operation = core.correction_operation.borrow().clone();
        let stale = core.ensure_current().is_err();
        let connected = core.flow().is_connected();
        let target = view.native_correction.as_ref().filter(|target| {
            view.runs.iter().any(|run| {
                run.id == target.run
                    && run.status == RunStatus::Running
                    && matches!(&run.route, RunRoute::Codex { .. } | RunRoute::Pi { .. })
            })
        });
        let available = connected
            && !stale
            && target.is_some_and(|target| target.available)
            && !operation.blocked()
            && core.owner.borrow().readable().is_ok()
            && !core.page.recovery.borrow().active()
            && !core.page.settings.borrow().blocked()
            && !core.page.navigating.get()
            && !matches!(*core.owner.borrow(), Owner::Uncertain);
        Self {
            connected,
            stale,
            available,
            operation,
            run: target.map(|target| target.run.clone()),
            receipts: view
                .runs
                .iter()
                .flat_map(|run| receipts(&run.id, &run.corrections))
                .collect(),
        }
    }
}

#[wasm_bindgen]
pub struct UiNativeCorrectionSnapshot {
    state: State,
}
#[wasm_bindgen]
impl UiNativeCorrectionSnapshot {
    #[wasm_bindgen(getter)]
    pub fn connected(&self) -> bool {
        self.state.connected
    }
    #[wasm_bindgen(getter)]
    pub fn stale(&self) -> bool {
        self.state.stale
    }
    #[wasm_bindgen(getter)]
    pub fn available(&self) -> bool {
        self.state.available
    }
    #[wasm_bindgen(getter)]
    pub fn pending(&self) -> bool {
        matches!(
            self.state.operation,
            Operation::Sending(_) | Operation::Waiting(_)
        )
    }
    #[wasm_bindgen(getter)]
    pub fn uncertain(&self) -> bool {
        self.state.operation.uncertain()
    }
    #[wasm_bindgen(getter)]
    pub fn run_id(&self) -> Option<String> {
        self.state.run.as_ref().map(|run| run.as_str().to_owned())
    }
    #[wasm_bindgen(getter)]
    pub fn admitted_request_id(&self) -> Option<String> {
        match &self.state.operation {
            Operation::Recorded(pending) => Some(pending.request.clone()),
            _ => None,
        }
    }
    pub fn receipts(&self) -> Vec<UiCorrectionReceipt> {
        self.state.receipts.clone()
    }
}

#[wasm_bindgen]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UiCorrectionStatus {
    Pending,
    Dispatching,
    CodexAccepted,
    PiQueued,
    PiHandled,
    Rejected,
    NotSent,
    Uncertain,
}
#[wasm_bindgen]
#[derive(Clone, PartialEq, Eq)]
pub struct UiCorrectionReceipt {
    run: RunId,
    receipt: CorrectionReceipt,
}
pub(super) fn receipts(run: &RunId, history: &CorrectionHistory) -> Vec<UiCorrectionReceipt> {
    history
        .entries
        .iter()
        .map(|receipt| UiCorrectionReceipt {
            run: run.clone(),
            receipt: receipt.clone(),
        })
        .collect()
}
#[wasm_bindgen]
impl UiCorrectionReceipt {
    #[wasm_bindgen(getter)]
    pub fn run_id(&self) -> String {
        self.run.as_str().into()
    }
    #[wasm_bindgen(getter)]
    pub fn request_id(&self) -> String {
        self.receipt.request.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> String {
        self.receipt.text.text.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn truncated(&self) -> bool {
        self.receipt.text.truncated
    }
    #[wasm_bindgen(getter)]
    pub fn label(&self) -> String {
        self.receipt.state.label()
    }
    #[wasm_bindgen(getter)]
    pub fn detail(&self) -> Option<String> {
        match &self.receipt.state {
            CorrectionState::Rejected { message }
            | CorrectionState::NotSent { message }
            | CorrectionState::Uncertain { message } => Some(message.clone()),
            _ => None,
        }
    }
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> UiCorrectionStatus {
        match &self.receipt.state {
            CorrectionState::Pending => UiCorrectionStatus::Pending,
            CorrectionState::Dispatching => UiCorrectionStatus::Dispatching,
            CorrectionState::CodexAccepted => UiCorrectionStatus::CodexAccepted,
            CorrectionState::PiQueued => UiCorrectionStatus::PiQueued,
            CorrectionState::PiHandled => UiCorrectionStatus::PiHandled,
            CorrectionState::Rejected { .. } => UiCorrectionStatus::Rejected,
            CorrectionState::NotSent { .. } => UiCorrectionStatus::NotSent,
            CorrectionState::Uncertain { .. } => UiCorrectionStatus::Uncertain,
        }
    }
}

#[wasm_bindgen]
pub struct UiNativeCorrections {
    core: Rc<Core>,
    activation: activation::SubmissionActivation,
    last_request: RefCell<Option<String>>,
}
#[wasm_bindgen]
impl UiBridge {
    pub fn native_corrections(&self) -> Result<UiNativeCorrections, JsValue> {
        Ok(UiNativeCorrections {
            core: Rc::clone(&self.core),
            activation: activation::SubmissionActivation::new().map_err(js_error)?,
            last_request: RefCell::new(None),
        })
    }
}
#[wasm_bindgen]
impl UiNativeCorrections {
    #[wasm_bindgen(getter)]
    pub fn request_id(&self) -> Result<String, JsValue> {
        self.last_request
            .borrow()
            .clone()
            .ok_or_else(|| crate::err("No correction has been submitted by this handle."))
    }

    pub fn subscribe(
        &self,
        #[wasm_bindgen(unchecked_param_type = "(snapshot: UiNativeCorrectionSnapshot) => void")]
        callback: Function,
    ) -> Result<UiNativeCorrectionSubscription, JsValue> {
        Observer::subscribe(Rc::clone(&self.core), callback)
    }
    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn send(
        &self,
        text: String,
        #[wasm_bindgen(unchecked_param_type = "Event")] event: JsValue,
    ) -> Result<Promise, JsValue> {
        self.activation.require(&event).map_err(js_error)?;
        let (pending, work) = self.core.reserve_correction(None, text).map_err(js_error)?;
        self.last_request.replace(Some(pending.request.clone()));
        self.core.page.notify_settings();
        Ok(future_to_promise(async move {
            work.await.map(|()| JsValue::UNDEFINED).map_err(js_error)
        }))
    }
    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn refresh(&self) -> Promise {
        let core = Rc::clone(&self.core);
        future_to_promise(async move {
            core.refresh_correction()
                .await
                .map(|()| JsValue::UNDEFINED)
                .map_err(js_error)
        })
    }
}

impl Core {
    fn reserve_correction(
        self: &Rc<Self>,
        expected_run: Option<&RunId>,
        text: String,
    ) -> Result<
        (
            Pending,
            impl Future<Output = Result<(), FlowError>> + 'static,
        ),
        FlowError,
    > {
        let state = State::read(self);
        let run = state.candidate(expected_run).ok_or_else(|| {
            FlowError::InvalidInput("A correction is not available for this active run.".into())
        })?;
        let text = UserText::try_from(text.as_str())?;
        let flow = self.flow();
        let pending = Pending {
            run: run.clone(),
            request: flow.correction_request_id()?,
        };
        *self.correction_operation.borrow_mut() = Operation::Sending(pending.clone());
        let core = Rc::clone(self);
        let receipt = pending.clone();
        Ok((receipt, async move {
            let result = flow
                .correct(pending.request.clone(), core.scope.clone(), run, text)
                .await;
            match result {
                Ok(()) => {
                    if core.correction_operation.borrow().owns(&pending) {
                        *core.correction_operation.borrow_mut() = Operation::Waiting(pending);
                    }
                    core.page.notify_settings();
                    let _ = core.current().await;
                    Ok(())
                }
                Err(error) => {
                    let reconciled = reconcile(&core, &flow, &pending).await;
                    if reconciled == Reconciled::Admitted {
                        Ok(())
                    } else {
                        if core.correction_operation.borrow().owns(&pending) {
                            *core.correction_operation.borrow_mut() =
                                if matches!(error, FlowError::Remote(_))
                                    && reconciled == Reconciled::Absent
                                {
                                    Operation::Idle
                                } else {
                                    Operation::Uncertain(pending)
                                };
                        }
                        core.page.notify_settings();
                        Err(error)
                    }
                }
            }
        }))
    }

    pub(super) async fn refresh_correction(&self) -> Result<(), FlowError> {
        self.ensure_current()?;
        if self.correction_operation.borrow().sending() {
            return Err(FlowError::HistoryBusy);
        }
        let pending = self.correction_operation.borrow().pending().cloned();
        if let Some(pending) = pending {
            let flow = self.flow();
            if reconcile(self, &flow, &pending).await != Reconciled::Admitted {
                return Err(FlowError::InvalidInput(
                    "Correction status is unresolved. Check saved work before sending again."
                        .into(),
                ));
            }
        } else {
            self.current().await?;
        }
        Ok(())
    }

    #[cfg(feature = "rune-ui")]
    pub(super) fn rune_correction_snapshot(&self) -> UiNativeCorrectionSnapshot {
        UiNativeCorrectionSnapshot {
            state: State::read(self),
        }
    }

    #[cfg(feature = "rune-ui")]
    pub(super) fn rune_correction_progress(&self) -> Option<(String, bool, bool)> {
        match &*self.correction_operation.borrow() {
            Operation::Sending(p) => Some((p.request.clone(), false, false)),
            Operation::Waiting(p) | Operation::Recorded(p) => {
                Some((p.request.clone(), true, false))
            }
            Operation::Uncertain(p) => Some((p.request.clone(), false, true)),
            Operation::Idle => None,
        }
    }

    #[cfg(feature = "rune-ui")]
    pub(super) fn rune_correction_offer(&self) -> (Option<RunId>, bool) {
        let state = State::read(self);
        (state.run.clone(), state.candidate(None).is_some())
    }

    #[cfg(feature = "rune-ui")]
    pub(super) fn rune_correction(
        self: &Rc<Self>,
        run: &RunId,
        draft_version: u64,
        text: String,
        tx: &mut ::rune::Tx,
    ) -> Result<
        (
            String,
            impl Future<Output = Result<(), FlowError>> + 'static,
        ),
        FlowError,
    > {
        if !tx.consume_activation(&correction_key(run, draft_version)) {
            return Err(FlowError::InvalidInput(
                "Use the current correction button.".into(),
            ));
        }
        let (pending, work) = self.reserve_correction(Some(run), text)?;
        self.page.notify_settings();
        Ok((pending.request, work))
    }
}

#[cfg(feature = "rune-ui")]
pub(super) fn correction_key(run: &RunId, draft_version: u64) -> String {
    serde_json::to_string(&("correction", run.as_str(), draft_version)).expect("string key")
}

#[derive(Debug, PartialEq, Eq)]
enum Reconciled {
    Admitted,
    Absent,
    Unknown,
}
fn receipt_proof(history: &CorrectionHistory, request: &str) -> Reconciled {
    if history
        .entries
        .iter()
        .any(|receipt| receipt.request == request)
    {
        Reconciled::Admitted
    } else if history.entries.len() == usize::from(history.total) {
        Reconciled::Absent
    } else {
        Reconciled::Unknown
    }
}
async fn reconcile(core: &Core, flow: &FlowClient, pending: &Pending) -> Reconciled {
    if core.correction_operation.borrow().recorded(pending) {
        return Reconciled::Admitted;
    }
    let Some((view, run)) = flow
        .correction_history(&core.scope, &pending.run, &pending.request)
        .await
    else {
        return Reconciled::Unknown;
    };
    if !flow.same_connection(&core.flow()) {
        return Reconciled::Unknown;
    }
    let observed_sequence = view.sequence.clone();
    let Ok(accepted) = core.accept(view) else {
        return Reconciled::Unknown;
    };
    if core.correction_operation.borrow().recorded(pending) {
        return Reconciled::Admitted;
    }
    let proof = receipt_proof(&run.corrections, &pending.request);
    if proof == Reconciled::Admitted {
        if core.correction_operation.borrow().owns(pending) {
            *core.correction_operation.borrow_mut() = Operation::Recorded(pending.clone());
        }
        core.page.notify_settings();
        Reconciled::Admitted
    } else if accepted.sequence != observed_sequence {
        Reconciled::Unknown
    } else {
        proof
    }
}

#[cfg(test)]
mod checks {
    use super::*;

    #[cfg(feature = "rune-ui")]
    #[test]
    fn correction_activation_binds_the_run_and_draft_version() {
        let run: RunId =
            serde_json::from_value(serde_json::json!("0123456789abcdef0123456789abcdef:1"))
                .unwrap();
        let other_run: RunId =
            serde_json::from_value(serde_json::json!("0123456789abcdef0123456789abcdef:2"))
                .unwrap();
        assert_ne!(correction_key(&run, 1), correction_key(&run, 2));
        assert_ne!(correction_key(&run, 1), correction_key(&other_run, 1));
    }

    #[test]
    fn reservation_rejects_pending_and_uncertain() {
        let id: RunId =
            serde_json::from_value(serde_json::json!("0123456789abcdef0123456789abcdef:1"))
                .unwrap();
        let pending = Pending {
            run: id.clone(),
            request: "request".into(),
        };
        let mut state = State {
            connected: true,
            stale: false,
            available: true,
            operation: Operation::Idle,
            run: Some(id.clone()),
            receipts: vec![],
        };
        assert_eq!(state.candidate(Some(&id)), Some(id.clone()));
        for operation in [
            Operation::Sending(pending.clone()),
            Operation::Waiting(pending.clone()),
            Operation::Uncertain(pending),
        ] {
            state.operation = operation;
            assert!(state.candidate(Some(&id)).is_none());
        }
    }

    #[test]
    fn partial_history_cannot_prove_a_correction_was_absent() {
        let mut history = CorrectionHistory {
            total: 2,
            entries: vec![CorrectionReceipt {
                ordinal: 1,
                request: "owned-request".into(),
                text: crate::history::Preview {
                    text: "Keep the source".into(),
                    truncated: false,
                },
                state: CorrectionState::Pending,
            }],
        };
        assert_eq!(
            receipt_proof(&history, "owned-request"),
            Reconciled::Admitted
        );
        assert_eq!(
            receipt_proof(&history, "other-request"),
            Reconciled::Unknown
        );
        history.total = 1;
        assert_eq!(receipt_proof(&history, "other-request"), Reconciled::Absent);
        assert_eq!(
            history.entries[0].state.label(),
            "Saved and waiting to send"
        );
    }
}
impl Core {
    pub(super) fn notify_corrections(&self) {
        let observers: Vec<_> = {
            let mut list = self.correction_subscriptions.borrow_mut();
            list.retain(|weak| weak.upgrade().is_some_and(|o| !o.closed()));
            list.iter().filter_map(std::rc::Weak::upgrade).collect()
        };
        for observer in observers {
            if observer.emit().is_err() {
                observer.close();
            }
            observer.wake();
        }
    }
}
