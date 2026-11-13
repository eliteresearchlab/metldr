use super::{Edit, Execution, record, storage};
use crate::{
    Result,
    history::schema::{PLAN_GRANTS, Record, TASK_PLANS, encode},
    model::valid_request_id,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskPlan {
    pub goal: String,
    pub intended_effects: Vec<String>,
    pub scope: String,
    pub revision: u32,
    pub state: PlanState,
    pub approver_request: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlanState {
    Proposed,
    Approved,
    Declined,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) enum FutureProfile {}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanGrant {
    pub run: String,
    pub browser_session: String,
    pub target: String,
    pub plan_revision: u32,
    pub expiry: u64,
    // B4.1 sets the profile and grants then require it.
    pub profile: Option<FutureProfile>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PlanError {
    Stale,
    AlreadyReviewed,
    Untrusted,
    Held,
    Invalid,
}
impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "task plan: {self:?}")
    }
}
impl std::error::Error for PlanError {}

impl Edit<'_> {
    pub(crate) fn task_plan(&self, execution: &Execution) -> Result<Option<TaskPlan>> {
        self.require_execution(execution, execution.run())?;
        let saved: Option<Record<TaskPlan>> =
            storage(|| record(self.write, TASK_PLANS, execution.run()))?;
        if let Some(saved) = &saved
            && (saved.conversation != execution.conversation()
                || saved.revision != u64::from(saved.value.revision)
                || !valid_plan(&saved.value))
        {
            return Err(PlanError::Invalid.into());
        }
        Ok(saved.map(|saved| saved.value))
    }

    #[allow(dead_code)]
    pub(crate) fn propose_plan(
        &mut self,
        execution: &Execution,
        mut plan: TaskPlan,
    ) -> Result<TaskPlan> {
        if self.run(execution, execution.run())?.status != crate::model::Status::Running {
            return Err(PlanError::Invalid.into());
        }
        if self
            .browser_open(execution, execution.run())?
            .is_some_and(|action| matches!(action.state, crate::browser_action::Phase::Dispatching))
        {
            return Err(PlanError::Held.into());
        }
        plan.revision = self
            .task_plan(execution)?
            .map_or(Some(1), |p| p.revision.checked_add(1))
            .ok_or(PlanError::Invalid)?;
        plan.state = PlanState::Proposed;
        plan.approver_request = None;
        if !valid_plan(&plan) {
            return Err(PlanError::Invalid.into());
        }
        self.save_plan(execution, &plan)?;
        Ok(plan)
    }

    pub(crate) fn review_reply(
        &mut self,
        execution: &Execution,
        revision: u32,
        approve: bool,
        request: &str,
    ) -> Result<()> {
        if !valid_request_id(request) {
            return Err(PlanError::Untrusted.into());
        }
        let mut plan = self.task_plan(execution)?.ok_or(PlanError::Stale)?;
        if plan.revision != revision {
            return Err(PlanError::Stale.into());
        }
        if plan.state != PlanState::Proposed {
            return Err(PlanError::AlreadyReviewed.into());
        }
        plan.state = if approve {
            PlanState::Approved
        } else {
            PlanState::Declined
        };
        plan.approver_request = Some(request.to_owned());
        self.save_plan(execution, &plan)
    }

    pub(super) fn require_approved_plan(&self, execution: &Execution) -> Result<()> {
        if self
            .task_plan(execution)?
            .is_some_and(|plan| plan.state != PlanState::Approved)
        {
            return Err(PlanError::Held.into());
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn put_plan_grant(
        &mut self,
        execution: &Execution,
        grant: &PlanGrant,
    ) -> Result<()> {
        let plan = self.task_plan(execution)?.ok_or(PlanError::Invalid)?;
        if grant.run != execution.run()
            || grant.plan_revision != plan.revision
            || plan.state != PlanState::Approved
            || grant.expiry == 0
            || grant.browser_session.is_empty()
            || grant.browser_session.len() > 128
            || grant.target.is_empty()
            || grant.target.len() > 128
        {
            return Err(PlanError::Invalid.into());
        }
        storage(|| {
            self.write.open_table(PLAN_GRANTS)?.insert(
                execution.run(),
                encode(&Record {
                    conversation: execution.conversation().to_owned(),
                    revision: u64::from(grant.plan_revision),
                    value: grant,
                })?
                .as_slice(),
            )?;
            Ok(())
        })?;
        self.changed = true;
        Ok(())
    }

    fn save_plan(&mut self, execution: &Execution, plan: &TaskPlan) -> Result<()> {
        storage(|| {
            self.write.open_table(TASK_PLANS)?.insert(
                execution.run(),
                encode(&Record {
                    conversation: execution.conversation().to_owned(),
                    revision: u64::from(plan.revision),
                    value: plan,
                })?
                .as_slice(),
            )?;
            Ok(())
        })?;
        self.changed = true;
        Ok(())
    }
}

fn valid_plan(plan: &TaskPlan) -> bool {
    let valid = |s: &str| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control);
    valid(&plan.goal)
        && valid(&plan.scope)
        && plan.intended_effects.len() <= 32
        && plan.intended_effects.iter().all(|s| valid(s))
        && match (&plan.state, &plan.approver_request) {
            (PlanState::Proposed, None) => true,
            (PlanState::Approved | PlanState::Declined, Some(id)) => valid_request_id(id),
            _ => false,
        }
}
