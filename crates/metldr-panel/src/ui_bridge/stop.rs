use super::*;

pub(super) enum Stopped {
    Pending,
    Native(StopReceipt),
}

impl Stopped {
    pub(super) fn name(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Native(StopReceipt::Requested) => "requested",
            Self::Native(StopReceipt::AlreadyTerminal) => "already_terminal",
        }
    }
}

impl Core {
    pub(super) fn stop(
        self: &Rc<Self>,
        id: &str,
    ) -> Result<impl Future<Output = Result<Stopped, FlowError>> + 'static, FlowError> {
        self.ensure_current()?;
        if self.page.recovery.borrow().active() {
            return Err(FlowError::HistoryBusy);
        }
        let stop = self.owner.borrow_mut().stop(id)?;
        if matches!(stop, Stop::Pending) {
            let waker = self.preparing.borrow_mut().take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
        let core = Rc::clone(self);
        Ok(async move {
            match stop {
                Stop::Pending => Ok(Stopped::Pending),
                Stop::Native(native) => core
                    .flow()
                    .request_stop(core.scope.clone(), native)
                    .await
                    .map(Stopped::Native),
            }
        })
    }
}
