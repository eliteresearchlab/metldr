use super::*;
use std::sync::atomic::AtomicUsize;

/// Page setup commands sent together, in order, before `Runtime.runIfWaitingForDebugger`.
const PAGE_SETUP: usize = 9;

impl Actor {
    pub(super) fn initialize(&mut self, session: Option<String>, page: bool) -> Result<()> {
        let deadline = if self.ready.is_some() {
            self.init_deadline
        } else {
            Instant::now() + TIMEOUT
        };
        if !page {
            return self.initialize_step(Initialization {
                session,
                page,
                step: 0,
                deadline,
                remaining: None,
            });
        }
        // The browser handles one session's commands in arrival order, so pipelining keeps the
        // same execution order as waiting for each reply and saves one round trip per command.
        let remaining = Arc::new(AtomicUsize::new(PAGE_SETUP));
        for step in 0..PAGE_SETUP {
            self.initialize_step(Initialization {
                session: session.clone(),
                page,
                step,
                deadline,
                remaining: Some(remaining.clone()),
            })?;
        }
        Ok(())
    }
    pub(super) fn initialize_step(&mut self, init: Initialization) -> Result<()> {
        let command = if !init.page {
            (init.step == 0).then(|| ("Runtime.runIfWaitingForDebugger", json!({})))
        } else {
            match init.step {
                0 => Some(("Page.enable", json!({}))),
                1 => Some(("Page.getFrameTree", json!({}))),
                2 => Some(("Runtime.enable", json!({}))),
                3 => Some(("Page.setLifecycleEventsEnabled", json!({"enabled":true}))),
                4 => Some(("Network.enable", json!({}))),
                5 => Some(("Network.setCacheDisabled", json!({"cacheDisabled":false}))),
                6 => Some((
                    "Target.setAutoAttach",
                    json!({"autoAttach":true,"flatten":true,"waitForDebuggerOnStart":false}),
                )),
                7 => Some(("Performance.enable", json!({}))),
                8 => Some(("Log.enable", json!({}))),
                PAGE_SETUP => {
                    // Every setup command, including Runtime.enable and Log.enable, has answered,
                    // so the replay of earlier output is over.
                    self.page_logs.arm_session(init.session.as_deref());
                    Some(("Runtime.runIfWaitingForDebugger", json!({})))
                }
                _ => None,
            }
        };
        if let Some((method, params)) = command {
            self.queue(
                method,
                init.session.clone(),
                params,
                Work::Init(init),
                None,
                None,
            )
        } else {
            self.contexts.ready(init.session.as_deref())?;
            Ok(())
        }
    }
    /// Advances setup after one reply: the last pipelined page reply releases the final step.
    pub(super) fn initialized(&mut self, mut init: Initialization) -> Result<()> {
        if let Some(remaining) = init.remaining.take() {
            if remaining.fetch_sub(1, Ordering::AcqRel) != 1 {
                return Ok(());
            }
            init.step = PAGE_SETUP;
            return self.initialize_step(init);
        }
        init.step += 1;
        self.initialize_step(init)
    }
}
