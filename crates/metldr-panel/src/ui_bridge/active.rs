use super::*;
use crate::model::RunView;
use std::future::Future;

#[wasm_bindgen]
pub struct UiActiveRun {
    active: Rc<Active>,
}

pub(super) struct Active {
    core: Rc<Core>,
    native: RunId,
    question: Option<String>,
    observation: RefCell<Observation>,
    stopping: Cell<bool>,
}

enum Observation {
    Ready,
    Watching(Option<Waker>),
    Detached,
    Terminal,
    Failed,
}

struct Watching(Rc<Active>);

impl Drop for Watching {
    fn drop(&mut self) {
        self.0
            .core
            .owner
            .borrow_mut()
            .observation_finished(&self.0.native);
        self.0.core.page.notify_settings();
    }
}

struct Stopping(Rc<Active>);

impl Drop for Stopping {
    fn drop(&mut self) {
        self.0.stopping.set(false);
    }
}

impl UiActiveRun {
    pub(super) fn new(core: Rc<Core>, run: &RunView) -> Self {
        Self {
            active: Rc::new(Active::new(core, run)),
        }
    }
}

#[wasm_bindgen]
impl UiActiveRun {
    #[wasm_bindgen(getter)]
    pub fn thread_id(&self) -> String {
        self.active.core.scope.conversation.clone()
    }

    pub fn watch(&self, ui_run_id: String, emit: Function) -> Result<Promise, JsValue> {
        let observing = self
            .active
            .watch(ui_run_id, move |event| emit_event(event, &emit))
            .map_err(js_error)?;
        Ok(future_to_promise(async move {
            observing
                .await
                .map(|()| JsValue::UNDEFINED)
                .map_err(js_error)
        }))
    }

    pub fn detach(&self) {
        self.active.detach();
    }

    pub fn stop(&self) -> Result<Promise, JsValue> {
        let stopping = self.active.stop().map_err(js_error)?;
        Ok(future_to_promise(async move {
            match stopping.await.map_err(js_error)? {
                StopReceipt::Requested => Ok("requested".into()),
                StopReceipt::AlreadyTerminal => Ok("already_terminal".into()),
            }
        }))
    }
}

impl Active {
    pub(super) fn new(core: Rc<Core>, run: &RunView) -> Self {
        Self {
            core,
            native: run.id.clone(),
            question: run.question.clone(),
            observation: RefCell::new(Observation::Ready),
            stopping: Cell::new(false),
        }
    }

    pub(super) fn detach(&self) {
        let wake = {
            let mut state = self.observation.borrow_mut();
            match &mut *state {
                Observation::Ready | Observation::Watching(_) => {
                    match std::mem::replace(&mut *state, Observation::Detached) {
                        Observation::Watching(wake) => wake,
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }

    pub(super) fn watch(
        self: &Rc<Self>,
        ui_run_id: String,
        emit: impl FnMut(Event) -> Result<(), FlowError> + 'static,
    ) -> Result<impl Future<Output = Result<(), FlowError>> + 'static, FlowError> {
        self.core.ensure_current()?;
        if self.core.page.recovery.borrow().active() {
            return Err(FlowError::HistoryBusy);
        }
        if !matches!(*self.observation.borrow(), Observation::Ready) {
            return Err(FlowError::InvalidInput(
                "Reconnect to observe this run again.".into(),
            ));
        }
        let messages = self
            .question
            .as_ref()
            .map(|question| AgUiMessage::User {
                id: format!("{}:question", self.native),
                content: question.clone(),
            })
            .into_iter()
            .collect();
        let stream = RunStream::new(self.core.scope.conversation.clone(), ui_run_id, messages)
            .map_err(|error| FlowError::InvalidInput(error))?;
        self.core.owner.borrow_mut().observe(self.native.clone())?;
        *self.observation.borrow_mut() = Observation::Watching(None);
        self.core.page.notify_settings();

        let active = Rc::clone(self);
        let watching = Watching(Rc::clone(self));
        Ok(async move {
            let _watching = watching;
            let result = active.observe(stream, emit).await;
            if result.is_err() {
                *active.observation.borrow_mut() = Observation::Failed;
            }
            result
        })
    }

    pub(super) fn stop(
        self: &Rc<Self>,
    ) -> Result<impl Future<Output = Result<StopReceipt, FlowError>> + 'static, FlowError> {
        self.core.ensure_current()?;
        let flow = self.core.flow();
        if self.stopping.replace(true) {
            return Err(FlowError::HistoryBusy);
        }
        let stopping = Stopping(Rc::clone(self));
        let active = Rc::clone(self);
        Ok(async move {
            let _stopping = stopping;
            active.core.ensure_current()?;
            if active.core.page.recovery.borrow().active()
                || !flow.same_connection(&active.core.flow())
            {
                return Err(FlowError::HistoryStale);
            }
            flow.request_stop(active.core.scope.clone(), active.native.clone())
                .await
        })
    }

    async fn observe(
        &self,
        mut stream: RunStream,
        mut emit: impl FnMut(Event) -> Result<(), FlowError>,
    ) -> Result<(), FlowError> {
        self.core.ensure_current()?;
        let mut deliver = |event| emit(event);
        let mut observing =
            std::pin::pin!(self.core.stream(&self.native, &mut stream, &mut deliver));
        std::future::poll_fn(|context| {
            let mut state = self.observation.borrow_mut();
            match &mut *state {
                Observation::Watching(wake) => *wake = Some(context.waker().clone()),
                _ => return Poll::Ready(Ok(())),
            }
            drop(state);
            observing.as_mut().poll(context).map(|result| {
                if result.is_ok() {
                    *self.observation.borrow_mut() = Observation::Terminal;
                }
                result
            })
        })
        .await
    }
}
