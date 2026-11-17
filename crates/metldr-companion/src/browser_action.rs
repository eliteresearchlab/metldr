use crate::{Result, model::Store};
use serde::{Deserialize, Serialize};

mod intent;
mod target;
pub(super) use intent::Intent;
#[cfg(test)]
pub(crate) use intent::element;
pub(crate) use intent::{Element, SECRET_TTL_MS, SecretKind, named_refs};
pub(super) use target::Target;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub tab: i32,
    pub document: String,
    pub target: String,
    pub frame: String,
    pub session: String,
    pub unique_context: String,
    pub grant_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Proposal {
    pub run: String,
    pub tool_use_id: String,
    pub binding: Binding,
    pub url: String,
    pub intent: Intent,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Outcome {
    Navigated {
        binding: Binding,
        url: String,
        loader: String,
        /// Status and redirect chain of the new main document, when the browser reported them.
        #[serde(default, skip_serializing_if = "no_evidence")]
        evidence: Box<NavigationEvidence>,
    },
    /// A page action ran; `result` is the browser's short account of what changed.
    Acted {
        result: String,
    },
    /// The page field holds the consented secret. Only the opaque reference is reported.
    SecretEntered {
        reference: String,
    },
    Prevented {
        reason: String,
    },
    Uncertain {
        reason: String,
    },
}

fn no_evidence(evidence: &NavigationEvidence) -> bool {
    evidence.is_empty()
}

/// Most redirect hops kept for one navigation.
pub(crate) const REDIRECT_LIMIT: usize = 20;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NavigationEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// URLs that answered with a redirect, in order, before the final document.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<String>,
}

impl NavigationEvidence {
    pub(crate) fn is_empty(&self) -> bool {
        self.status.is_none() && self.redirects.is_empty()
    }

    fn valid(&self) -> bool {
        self.status
            .is_none_or(|status| (100..=599).contains(&status))
            && self.redirects.len() <= REDIRECT_LIMIT
            && self
                .redirects
                .iter()
                .all(|url| url.len() <= 2048 && crate::model::validate_page_url(url).is_ok())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Phase {
    Pending,
    Approved,
    Dispatching,
    Settled { outcome: Outcome },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Action {
    pub id: String,
    pub revision: u64,
    pub proposal: Proposal,
    pub state: Phase,
}

impl Action {
    pub(super) fn pending(id: String, proposal: Proposal) -> Result<Self> {
        proposal.validate()?;
        let action = Self {
            id,
            revision: 1,
            proposal,
            state: Phase::Pending,
        };
        action.validate()?;
        Ok(action)
    }

    pub(super) fn decide(&self, revision: u64, approve: bool) -> Result<Self> {
        if self.revision != revision || self.state != Phase::Pending {
            return Err("browser approval is stale".into());
        }
        self.transition(if approve {
            Phase::Approved
        } else {
            Phase::Settled {
                outcome: Outcome::Prevented {
                    reason: "user declined".into(),
                },
            }
        })
    }

    pub(super) fn dispatch(&self, revision: u64) -> Result<Self> {
        if self.revision != revision || self.state != Phase::Approved {
            return Err("browser dispatch requires the current approval".into());
        }
        self.transition(Phase::Dispatching)
    }

    pub(super) fn settle(&self, revision: u64, outcome: Outcome) -> Result<Self> {
        if self.revision != revision || self.state != Phase::Dispatching {
            return Err("browser result is stale or was not dispatched".into());
        }
        validate_outcome(&outcome, &self.proposal)?;
        self.transition(Phase::Settled { outcome })
    }

    pub(super) fn prevent(&self, reason: &str) -> Result<Option<Self>> {
        if !matches!(self.state, Phase::Pending | Phase::Approved) {
            return Ok(None);
        }
        self.transition(Phase::Settled {
            outcome: Outcome::Prevented {
                reason: reason.into(),
            },
        })
        .map(Some)
    }

    pub(super) fn finish(&self, before: &str, after: &str) -> Result<Option<Self>> {
        let outcome = match &self.state {
            Phase::Pending | Phase::Approved => Outcome::Prevented {
                reason: before.into(),
            },
            Phase::Dispatching => Outcome::Uncertain {
                reason: after.into(),
            },
            Phase::Settled { .. } => return Ok(None),
        };
        self.transition(Phase::Settled { outcome }).map(Some)
    }

    fn transition(&self, state: Phase) -> Result<Self> {
        let action = Self {
            revision: self
                .revision
                .checked_add(1)
                .ok_or("browser revision exhausted")?,
            state,
            ..self.clone()
        };
        action.validate()?;
        Ok(action)
    }

    pub(super) fn validate(&self) -> Result<()> {
        self.proposal.validate()?;
        if self.revision == 0 {
            return Err("invalid saved browser action".into());
        }
        match &self.state {
            Phase::Pending if self.revision != 1 => Err("invalid pending revision".into()),
            Phase::Approved if self.revision != 2 => Err("invalid approval revision".into()),
            Phase::Dispatching if self.revision != 3 => Err("invalid dispatch revision".into()),
            Phase::Settled { outcome } => {
                validate_outcome(outcome, &self.proposal)?;
                let valid_revision = match outcome {
                    Outcome::Navigated { .. }
                    | Outcome::Acted { .. }
                    | Outcome::Uncertain { .. } => self.revision == 4,
                    Outcome::SecretEntered { .. } => {
                        self.revision == 4 && matches!(&self.proposal.intent, Intent::Secret { .. })
                    }
                    Outcome::Prevented { .. } => (2..=4).contains(&self.revision),
                };
                if !valid_revision {
                    return Err("invalid settled revision".into());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

impl Proposal {
    pub(crate) fn validate(&self) -> Result<()> {
        let b = &self.binding;
        if b.tab < 0
            || b.grant_revision == 0
            || !bounded(&self.url, 2048)
            || !bounded(&self.tool_use_id, 128)
            || ![
                &b.document,
                &b.target,
                &b.frame,
                &b.session,
                &b.unique_context,
            ]
            .into_iter()
            .all(|s| bounded(s, 128))
        {
            return Err("invalid browser proposal".into());
        }
        self.intent.validate()
    }
}

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

#[cfg(test)]
impl Store {
    pub(super) fn propose_browser(&mut self, proposal: Proposal) -> Result<Action> {
        proposal.validate()?;
        if let Some(action) = self.browser_actions.values().find(|a| {
            a.proposal.run == proposal.run && a.proposal.tool_use_id == proposal.tool_use_id
        }) {
            if action.proposal != proposal {
                return Err("browser tool identity was reused with a changed proposal".into());
            }
            return Ok(action.clone());
        }
        self.browser_run(&proposal.run)?;
        let source = match self.launches.get(&proposal.run) {
            Some(crate::agent_model::Launch::Pi { workflow, .. }) => {
                &self
                    .workflows
                    .get(workflow)
                    .ok_or("missing Pi browser source")?
                    .source
            }
            Some(crate::agent_model::Launch::Codex { workflow, .. }) => {
                &self
                    .codex_workflows
                    .get(workflow)
                    .ok_or("missing Codex browser source")?
                    .source
            }
            Some(crate::agent_model::Launch::OpenCode { workflow, .. }) => {
                &self
                    .opencode_workflows
                    .get(workflow)
                    .ok_or("missing OpenCode browser source")?
                    .source
            }
            _ => self.view.source.as_ref().ok_or("missing browser source")?,
        };
        let run = self
            .view
            .runs
            .iter()
            .find(|r| r.id == proposal.run)
            .ok_or("unknown browser run")?;
        let initial = if self
            .launches
            .get(&proposal.run)
            .and_then(|l| l.native_workflow())
            .is_some()
        {
            self.native_browser_target(&proposal.run)?
        } else {
            Target::from(&source.page)
        };
        if run.source != source.id || !self.browser_target_matches(&proposal, Some(&initial))? {
            return Err("browser proposal does not match the saved source".into());
        }
        if self.browser_actions.len() >= 16
            || self
                .browser_actions
                .values()
                .any(|a| !matches!(a.state, Phase::Settled { .. }))
        {
            return Err("another browser action is pending or capacity is reached".into());
        }
        let action = Action::pending(self.id()?, proposal)?;
        self.browser_actions
            .insert(action.id.clone(), action.clone());
        Ok(action)
    }

    pub(super) fn decide_browser(
        &mut self,
        id: &str,
        revision: u64,
        approve: bool,
    ) -> Result<Action> {
        let action = self
            .browser_actions
            .get(id)
            .ok_or("unknown browser action")?;
        self.browser_run(&action.proposal.run)?;
        let action = action.decide(revision, approve)?;
        self.browser_actions.insert(id.into(), action.clone());
        Ok(action)
    }

    pub(super) fn dispatch_browser(&mut self, id: &str, revision: u64) -> Result<Action> {
        let action = self
            .browser_actions
            .get(id)
            .ok_or("unknown browser action")?;
        self.browser_run(&action.proposal.run)?;
        let action = action.dispatch(revision)?;
        self.browser_actions.insert(id.into(), action.clone());
        Ok(action)
    }

    pub(super) fn settle_browser(
        &mut self,
        id: &str,
        revision: u64,
        outcome: Outcome,
    ) -> Result<Action> {
        let action = self
            .browser_actions
            .get(id)
            .ok_or("unknown browser action")?;
        let action = action.settle(revision, outcome)?;
        self.browser_actions.insert(id.into(), action.clone());
        Ok(action)
    }

    pub(super) fn interrupt_browser(&mut self) -> Result<bool> {
        let ids: Vec<_> = self
            .browser_actions
            .values()
            .filter(|a| !matches!(a.state, Phase::Settled { .. }))
            .map(|a| a.id.clone())
            .collect();
        for id in &ids {
            if let Some(action) = self.browser_actions[id].finish(
                "native host restarted before dispatch",
                "native host restarted after dispatch",
            )? {
                self.browser_actions.insert(id.clone(), action);
            }
        }
        Ok(!ids.is_empty())
    }

    fn browser_run(&self, run: &str) -> Result<()> {
        if !self
            .view
            .runs
            .iter()
            .any(|r| r.id == run && r.status == crate::model::Status::Running)
        {
            return Err("browser run is not active".into());
        }
        Ok(())
    }
}

pub(super) fn validate(state: &Store, valid_id: impl Fn(&str) -> bool) -> Result<()> {
    if state.browser_actions.len() > 16 {
        return Err("browser action capacity reached".into());
    }
    let mut tools = std::collections::BTreeSet::new();
    let mut pending = 0;
    for (id, action) in &state.browser_actions {
        if id != &action.id
            || !valid_id(id)
            || !state.view.runs.iter().any(|r| r.id == action.proposal.run)
            || state.view.runs.iter().any(|r| &r.id == id)
            || state.view.source.as_ref().is_some_and(|s| &s.id == id)
            || !tools.insert((&action.proposal.run, &action.proposal.tool_use_id))
        {
            return Err("invalid saved browser action".into());
        }
        action.validate()?;
        pending += usize::from(!matches!(action.state, Phase::Settled { .. }));
        if !matches!(action.state, Phase::Settled { .. })
            && !state
                .view
                .runs
                .iter()
                .any(|r| r.id == action.proposal.run && !r.status.terminal())
        {
            return Err("nonterminal browser action belongs to a terminal run".into());
        }
    }
    if pending > 1 {
        return Err("multiple pending browser actions".into());
    }
    target::validate(state)?;
    Ok(())
}

fn validate_outcome(outcome: &Outcome, proposal: &Proposal) -> Result<()> {
    match (outcome, &proposal.intent) {
        (Outcome::Acted { result }, Intent::Act { .. })
            if result.len() <= 4096 && !result.chars().any(|c| c.is_control() && c != '\n') =>
        {
            Ok(())
        }
        (
            Outcome::SecretEntered { reference },
            Intent::Secret {
                reference: approved,
                ..
            },
        ) if reference == approved => Ok(()),
        (
            Outcome::Navigated {
                binding,
                url,
                loader,
                evidence,
            },
            Intent::Navigate { .. },
        ) if crate::model::validate_page_url(url).is_ok()
            && evidence.valid()
            && bounded(loader, 128)
            && binding.tab == proposal.binding.tab
            && binding.target == proposal.binding.target
            && binding.frame == proposal.binding.frame
            && binding.session == proposal.binding.session
            && bounded(&binding.document, 128)
            && bounded(&binding.unique_context, 128)
            && binding.document != proposal.binding.document
            && binding.unique_context != proposal.binding.unique_context
            && proposal.binding.grant_revision.checked_add(1) == Some(binding.grant_revision) =>
        {
            Ok(())
        }
        (Outcome::Prevented { reason } | Outcome::Uncertain { reason }, _)
            if bounded(reason, 200) =>
        {
            Ok(())
        }
        _ => Err("invalid browser outcome".into()),
    }
}
