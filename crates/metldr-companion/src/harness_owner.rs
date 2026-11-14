//! Owner side of harnesses on the generic path (B1.12): start a run, confirm its native session
//! and record its activity. One adapter per harness does the native work.

use crate::{
    Result,
    agent_model::{Activity, Launch},
    flow::{Operation, Owner},
    model::ConversationAction,
};
use std::{
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
};

impl Owner {
    pub(super) fn harness_start(
        &mut self,
        request: &str,
        scope: &crate::history::Selection,
        command: ConversationAction,
    ) -> Result<()> {
        let harness = match &command {
            ConversationAction::HarnessPrompt { harness, .. }
            | ConversationAction::HarnessFollowUp { harness, .. } => harness.clone(),
            _ => return Err("invalid harness command".into()),
        };
        let adapter = crate::harness_model::adapter(&harness)?;
        self.prepare_agent(adapter.kind)?;
        let (key, profile) = self
            .agent_runtime
            .profiles
            .get_key_value(harness.as_str())
            .map(|(key, profile)| (*key, profile.clone()))
            .ok_or("no trusted profile is configured for this agent")?;
        let Some(admission) = self.admit_native_start(
            request,
            scope,
            &command,
            crate::agent_settings::NativeProfile::Harness(key, profile),
        )?
        else {
            return Ok(());
        };
        let crate::native_start::Admission {
            run,
            workflow,
            value,
            text,
            target,
        } = admission;
        let crate::history::NativeWorkflow::Harness(w) = value else {
            return Err("harness admission workflow mismatch".into());
        };
        let job = crate::harness_model::Job {
            run: run.clone(),
            workflow,
            profile: w.profile.clone(),
            binding: w.binding.clone(),
            source: w.source.clone(),
            target,
            text,
        };
        let (slot, events) = crate::browser_io::Slot::new(job.target.tab);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let tx = self.tx.clone();
        let execute = adapter.execute;
        let worker = thread::spawn(move || execute(job, worker_stop, tx, events));
        self.operations.insert(
            run.clone(),
            Operation {
                stop,
                worker,
                correction: None,
                browser_io: Some(Box::new(slot)),
            },
        );
        self.emit(self.started_reply(request, scope, &run)?)
    }

    pub(super) fn harness_bound(
        &mut self,
        run: &str,
        workflow: &str,
        session: String,
        ack: mpsc::SyncSender<std::result::Result<bool, String>>,
    ) -> Result<()> {
        let result = self.native_change(run, move |_, launch, native| {
            let crate::history::NativeWorkflow::Harness(w) = native else {
                return Err("harness binding workflow mismatch".into());
            };
            crate::harness_model::valid_session(&w.harness, &session)?;
            if w.last_run != run {
                return Err("harness binding workflow mismatch".into());
            }
            match &w.binding {
                crate::harness_model::Binding::Bound { session: old } if old != &session => {
                    return Err("harness session identity changed".into());
                }
                crate::harness_model::Binding::ReleaseUnknown { .. } => {
                    return Err("harness session ownership is unknown".into());
                }
                _ => (),
            }
            let Launch::Harness {
                workflow: bound, ..
            } = launch
            else {
                return Err("harness binding has no launch".into());
            };
            if bound.as_str() != workflow {
                return Err("harness launch binding changed".into());
            }
            let changed = !matches!(&w.binding, crate::harness_model::Binding::Bound { .. });
            w.binding = crate::harness_model::Binding::Bound { session };
            if changed {
                w.revision = w
                    .revision
                    .checked_add(1)
                    .ok_or("harness workflow revision exhausted")?;
            }
            Ok(true)
        });
        crate::flow::acknowledge(ack, result)
    }

    pub(super) fn harness_activity(&mut self, run: &str, activity: Activity) -> Result<()> {
        self.native_change(run, move |task, launch, _| {
            let Launch::Harness {
                activity: events, ..
            } = launch
            else {
                return Err("harness activity has no launch".into());
            };
            if events.len() >= 64 || !activity.valid() {
                return Err("invalid harness activity".into());
            }
            if let Some(call) = &activity.call {
                task.output
                    .observe_tool(metldr_conversation::ToolId::new(call.id.clone())?)?;
            }
            events.push(activity);
            Ok(true)
        })?;
        Ok(())
    }
}
