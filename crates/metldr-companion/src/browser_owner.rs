use crate::{
    Result,
    browser_action::{Action, Outcome, Proposal},
    flow::{Owner, OwnerReply, acknowledge},
};
use std::sync::atomic::Ordering;

type Reply = OwnerReply<Action>;

mod grants;
mod review;
mod secret;
pub(crate) use grants::{GRANT_TTL, SessionGrant};
pub(crate) use review::Budget;
#[cfg(test)]
pub(crate) use secret::now_ms;
pub(crate) use secret::{Held as HeldSecret, SecretText};

pub(super) enum Event {
    Plan {
        run: String,
        goal: String,
        intended_effects: Vec<String>,
        scope: String,
        ack: OwnerReply<u32>,
    },
    Proposed {
        proposal: Proposal,
        ack: Reply,
        budget: Option<Budget>,
    },
    Dispatch {
        action: String,
        revision: u64,
        ack: Reply,
    },
    Observed {
        action: String,
        revision: u64,
        outcome: Outcome,
        ack: Reply,
    },
}

/// The first action after a proposed plan, held until the user decides on the plan.
pub(super) struct PlanHold {
    proposal: Proposal,
    ack: Reply,
    budget: Option<Budget>,
    review: Option<review::Review>,
}

pub(super) struct PendingApproval {
    pub run: String,
    pub action: String,
    pub ack: Reply,
    review: Option<review::Review>,
    dispatch_revision: Option<u64>,
    /// Set for a secret entry: when its consent window closes, in Unix milliseconds.
    secret_expires_ms: Option<u64>,
}

impl PendingApproval {
    fn reply(self, result: std::result::Result<Action, String>) {
        drop(self.review);
        let _ = self.ack.send(result);
    }
}

impl Owner {
    pub(super) fn saved_browser_action(&mut self, id: &str) -> Result<Action> {
        self.record_history.edit(|edit| {
            let execution = edit.action_execution(id)?;
            edit.browser_action(&execution, id)
        })
    }

    fn saved_browser_tool(&mut self, run: &str, tool: &str) -> Result<Option<Action>> {
        self.record_history.edit(|edit| {
            let execution = edit.execution(run)?;
            edit.browser_by_tool(&execution, run, tool)
        })
    }

    pub(super) fn browser_operation(&self, run: &str) -> Result<()> {
        if self
            .operations
            .get(run)
            .is_none_or(|op| op.stop.load(Ordering::Acquire))
        {
            return Err("browser operation is stale or stopped".into());
        }
        Ok(())
    }

    pub(super) fn browser_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Plan {
                run,
                goal,
                intended_effects,
                scope,
                ack,
            } => {
                let result = self.browser_operation(&run).and_then(|_| {
                    self.record_execution(&run, |edit, execution| {
                        edit.propose_plan(
                            execution,
                            crate::history::TaskPlan {
                                proposer: String::new(),
                                goal,
                                intended_effects,
                                scope,
                                revision: 0,
                                state: crate::history::PlanState::Proposed,
                                approver_request: None,
                            },
                        )
                        .map(|plan| plan.revision)
                    })
                });
                return acknowledge(ack, result);
            }
            Event::Proposed {
                proposal,
                ack,
                budget,
            } => {
                let plan = self.record_history.edit(|edit| {
                    let execution = edit.execution(&proposal.run)?;
                    edit.task_plan(&execution)
                });
                match plan {
                    Ok(None) => {
                        return acknowledge(ack, Err("call propose_task_plan first".into()));
                    }
                    Ok(Some(plan)) if plan.state == crate::history::PlanState::Declined => {
                        return acknowledge(ack, Err("task plan declined".into()));
                    }
                    Ok(Some(plan)) if plan.state == crate::history::PlanState::Proposed => {
                        if self.plan_hold.is_some() || self.browser_approval.is_some() {
                            return acknowledge(
                                ack,
                                Err("browser approval already pending".into()),
                            );
                        }
                        if let Err(error) = self.browser_operation(&proposal.run) {
                            return acknowledge(ack, Err(error));
                        }
                        let review = match budget.as_ref().map(Budget::pause).transpose() {
                            Ok(review) => review,
                            Err(error) => return acknowledge(ack, Err(error)),
                        };
                        self.plan_hold = Some(PlanHold {
                            proposal,
                            ack,
                            budget,
                            review,
                        });
                        return Ok(());
                    }
                    Err(error) => return acknowledge(ack, Err(error)),
                    _ => {}
                }
                let now = std::time::Instant::now();
                let auto = match &plan {
                    Ok(Some(plan)) if plan.state == crate::history::PlanState::Approved => self
                        .session_grants
                        .get(&proposal.run)
                        .is_some_and(|grant| grant.covers(&proposal, plan.revision, now)),
                    _ => false,
                };
                let result = (|| -> Result<(Action, bool, Option<review::Review>)> {
                    if let Some(existing) =
                        self.saved_browser_tool(&proposal.run, &proposal.tool_use_id)?
                    {
                        if existing.proposal != proposal {
                            return Err(
                                "browser tool identity was reused with a changed proposal".into()
                            );
                        }
                        return Ok((existing, false, None));
                    }
                    self.browser_operation(&proposal.run)?;
                    if self.browser_approval.is_some() {
                        return Err("browser approval already pending".into());
                    }
                    if budget.as_ref().is_some_and(Budget::expired) {
                        return Err("browser tool deadline expired before proposal".into());
                    }
                    let run = proposal.run.clone();
                    self.record_execution(&run, |edit, execution| {
                        let (action, created) = edit.propose_browser(execution, proposal)?;
                        if !created {
                            return Ok((action, false, None));
                        }
                        if auto {
                            // A session grant approves this Tier 1 step without asking.
                            let approved =
                                edit.decide_browser(execution, &action.id, action.revision, true)?;
                            return Ok((approved, false, None));
                        }
                        // A failed pause must reject the edit, not orphan the action.
                        let review = budget.as_ref().map(Budget::pause).transpose()?;
                        Ok((action, true, review))
                    })
                })();
                match result {
                    Ok((action, false, _)) => {
                        let _ = ack.send(Ok(action));
                        Ok(())
                    }
                    Ok((action, true, review)) => {
                        let secret_expires_ms = match &action.proposal.intent {
                            crate::browser_action::Intent::Secret { expires_at_ms, .. } => {
                                Some(*expires_at_ms)
                            }
                            _ => None,
                        };
                        self.browser_approval = Some(PendingApproval {
                            run: action.proposal.run,
                            action: action.id,
                            ack,
                            review,
                            dispatch_revision: None,
                            secret_expires_ms,
                        });
                        Ok(())
                    }
                    Err(error) => acknowledge(ack, Err(error)),
                }
            }
            Event::Dispatch {
                action,
                revision,
                ack,
            } => {
                let result = (|| -> Result<Action> {
                    let record = self.saved_browser_action(&action)?;
                    self.browser_operation(&record.proposal.run)?;
                    let run = record.proposal.run.clone();
                    self.record_execution(&run, |edit, execution| {
                        edit.dispatch_browser(execution, &action, revision)
                    })
                })();
                if result
                    .as_ref()
                    .err()
                    .and_then(|e| e.downcast_ref::<crate::history::PlanError>())
                    == Some(&crate::history::PlanError::Held)
                {
                    if self.browser_approval.is_some() {
                        return acknowledge(ack, Err("browser approval already pending".into()));
                    }
                    let run = self.saved_browser_action(&action)?.proposal.run;
                    self.browser_approval = Some(PendingApproval {
                        run,
                        action,
                        ack,
                        review: None,
                        dispatch_revision: Some(revision),
                        secret_expires_ms: None,
                    });
                    return Ok(());
                }
                acknowledge(ack, result)
            }
            Event::Observed {
                action,
                revision,
                outcome,
                ack,
            } => {
                let result = (|| -> Result<Action> {
                    let record = self.saved_browser_action(&action)?;
                    if !self.operations.contains_key(&record.proposal.run) {
                        return Err("browser result came from a stale operation".into());
                    }
                    let run = record.proposal.run.clone();
                    self.record_execution(&run, |edit, execution| {
                        edit.settle_browser(execution, &action, revision, outcome)
                    })
                })();
                if let Ok(record) = &result {
                    self.drop_secret(&record.proposal.run);
                }
                acknowledge(ack, result)
            }
        }
    }

    pub(super) fn browser_decide(
        &mut self,
        action: &str,
        revision: u64,
        approve: bool,
    ) -> Result<()> {
        let pending = self
            .browser_approval
            .as_ref()
            .filter(|p| p.action == action && p.dispatch_revision.is_none())
            .ok_or("browser action is not awaiting this approval")?;
        if approve && pending.secret_expires_ms.is_some() {
            return Err("type the secret to enter it".into());
        }
        self.browser_operation(&pending.run)?;
        let run = pending.run.clone();
        let record = self.record_execution(&run, |edit, execution| {
            edit.decide_browser(execution, action, revision, approve)
        })?;
        if approve && let Some(grant) = self.session_grants.get_mut(&run) {
            grant.bind(&record.proposal, std::time::Instant::now());
        }
        if let Some(pending) = self.browser_approval.take() {
            pending.reply(Ok(record));
        }
        Ok(())
    }

    pub(super) fn cancel_browser_approval(&mut self, run: &str) {
        self.session_grants.remove(run);
        self.drop_secret(run);
        if let Some(held) = self.plan_hold.take_if(|p| p.proposal.run == run) {
            let _ = held
                .ack
                .send(Err("browser action stopped before dispatch".into()));
        }
        if let Some(pending) = self.browser_approval.take_if(|p| p.run == run) {
            pending.reply(Err("browser action stopped before dispatch".into()));
        }
    }

    pub(super) fn release_plan_dispatch(&mut self, run: &str) -> Result<()> {
        if let Some(held) = self.plan_hold.take_if(|p| p.proposal.run == run) {
            drop(held.review);
            self.browser_event(Event::Proposed {
                proposal: held.proposal,
                ack: held.ack,
                budget: held.budget,
            })?;
        }
        if let Some(pending) = self
            .browser_approval
            .take_if(|p| p.run == run && p.dispatch_revision.is_some())
        {
            self.browser_event(Event::Dispatch {
                action: pending.action,
                revision: pending.dispatch_revision.unwrap(),
                ack: pending.ack,
            })?;
        }
        Ok(())
    }

    pub(super) fn fail_plan_dispatch(&mut self, run: &str) -> Result<()> {
        if let Some(held) = self.plan_hold.take_if(|p| p.proposal.run == run) {
            let _ = held.ack.send(Err("task plan declined".into()));
        }
        if self
            .browser_approval
            .as_ref()
            .is_some_and(|p| p.run == run && p.dispatch_revision.is_some())
        {
            self.record_execution(run, |edit, execution| {
                edit.prevent_browser(execution, run, "task plan declined")
            })?;
            if let Some(pending) = self.browser_approval.take() {
                pending.reply(Err("task plan declined".into()));
            }
        }
        Ok(())
    }
}
