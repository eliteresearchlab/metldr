use super::*;
impl Actor {
    pub(super) fn result(&mut self, id: CallId, result: runtime::Result<Value>) -> Result<()> {
        let pending = self.pending.remove(&id).ok_or("unknown actor response")?;
        if let Work::Page(job) = pending.work {
            let _ = job.reply.send(pending.prevented.map_or(result, Err));
            return Ok(());
        }
        let context = pending.context;
        let guarded = (|| -> Result<Value> {
            let value = result.map_err(|_| "browser command failed")?;
            if !matches!(pending.work, Work::Navigate { .. })
                && let Some(context) = &context
            {
                self.contexts.validate(context)?;
            }
            Ok(value)
        })();
        if let Some(reason) = pending.prevented {
            return self.failed(pending.work, reason, true);
        }
        let value = match guarded {
            Ok(value) => value,
            Err(e) => return self.failed(pending.work, e.to_string(), false),
        };
        match pending.work {
            Work::Page(_) => Err("browser page reply took the action path".into()),
            Work::Init(init) => {
                if init.page && init.step == 1 {
                    self.contexts
                        .tree(init.session.as_deref(), &value["frameTree"])?;
                }
                self.initialized(init)
            }
            Work::CreateWorld => Ok(()),
            Work::EnterSecret { action, job } => {
                let Intent::Secret { reference, .. } = &action.proposal.intent else {
                    return Err("invalid secret entry intent".into());
                };
                let outcome = Outcome::SecretEntered {
                    reference: reference.clone(),
                };
                self.value(job, outcome)
            }
            Work::Navigate { action, old, job } => self.navigated(action, old, job, value),
            Work::FrameTree(nav) => {
                self.navigation_tree(nav, context.ok_or("destination context absent")?, value)
            }
        }
    }
    fn failed(&mut self, work: Work, reason: String, prevented: bool) -> Result<()> {
        // A dispatched effect whose reply is lost is uncertain; one stopped before dispatch is
        // prevented.
        match work {
            Work::Page(job) => {
                let _ = job.reply.send(Err(reason));
                Ok(())
            }
            Work::CreateWorld if prevented => Ok(()),
            Work::Init(_) | Work::CreateWorld => {
                Err(format!("browser session initialization failed: {reason}").into())
            }
            Work::EnterSecret { job, .. } if prevented => {
                self.value(job, Outcome::Prevented { reason })
            }
            Work::EnterSecret { job, .. } => self.value(
                job,
                Outcome::Uncertain {
                    reason: "secret entry result was lost".into(),
                },
            ),
            Work::Navigate { job, .. } if prevented => {
                self.value(job, Outcome::Prevented { reason })
            }
            Work::FrameTree(nav) => {
                self.finish(nav.job, Err(reason.into()));
                Ok(())
            }
            Work::Navigate { job, .. } => {
                self.finish(job, Err(reason.into()));
                Ok(())
            }
        }
    }
}
