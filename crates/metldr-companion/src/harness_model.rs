//! Saved state for harnesses on the generic path (B1.12). A harness is named by its
//! registry key; its native session id is the continuation key.

use crate::{
    Result,
    agent_model::{Launch, digest},
    model::{Source, Store},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, mpsc},
};

/// One harness on the generic path. Adding a harness is one adapter module and one entry
/// in [`ADAPTERS`]; Pi, Codex and OpenCode keep their own records until they migrate.
pub(crate) struct Adapter {
    pub key: &'static str,
    pub kind: crate::agent_settings::Kind,
    pub valid_session: fn(&str) -> Result<()>,
    pub execute: fn(
        Job,
        Arc<AtomicBool>,
        mpsc::SyncSender<crate::flow::Event>,
        tokio::sync::mpsc::Receiver<crate::browser_io::Notice>,
    ),
}

pub(crate) const ADAPTERS: &[Adapter] = &[Adapter {
    key: "claude",
    kind: crate::agent_settings::Kind::Claude,
    valid_session: crate::claude::valid_session,
    execute: crate::claude::execute,
}];

pub(crate) fn adapter(key: &str) -> Result<&'static Adapter> {
    ADAPTERS
        .iter()
        .find(|adapter| adapter.key == key)
        .ok_or_else(|| "unknown harness".into())
}

/// Whether `key` names a harness of settings kind `kind`.
pub(crate) fn is_kind(key: &str, kind: crate::agent_settings::Kind) -> bool {
    adapter(key).is_ok_and(|adapter| adapter.kind == kind)
}

/// One run handed to a harness adapter.
pub(crate) struct Job {
    pub run: String,
    pub workflow: String,
    pub profile: Profile,
    pub binding: Binding,
    pub source: Source,
    pub target: crate::browser_action::Target,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    pub executable: PathBuf,
    pub workspace: PathBuf,
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        for path in [&self.executable, &self.workspace] {
            if !path.is_absolute()
                || path.to_string_lossy().len() > 2048
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err("invalid agent profile path".into());
            }
        }
        Ok(())
    }
}

/// Checks a native session id for `harness`.
pub(crate) fn valid_session(harness: &str, session: &str) -> Result<()> {
    (adapter(harness)?.valid_session)(session)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Binding {
    Pending,
    Bound { session: String },
    ReleaseUnknown { previous: Option<String> },
}

impl Binding {
    pub fn block_recovery(&mut self) {
        let previous = match self {
            Self::Pending => None,
            Self::Bound { session } => Some(session.clone()),
            Self::ReleaseUnknown { .. } => return,
        };
        *self = Self::ReleaseUnknown { previous };
    }

    /// Whether a saved workflow may move from this binding to `after`.
    pub fn can_become(&self, after: &Self) -> bool {
        match (self, after) {
            (Self::Pending, Self::Pending | Self::Bound { .. })
            | (Self::Pending, Self::ReleaseUnknown { previous: None }) => true,
            (Self::Bound { session: a }, Self::Bound { session: b }) => a == b,
            (
                Self::Bound { session },
                Self::ReleaseUnknown {
                    previous: Some(previous),
                },
            ) => session == previous,
            (Self::ReleaseUnknown { previous: a }, Self::ReleaseUnknown { previous: b }) => a == b,
            _ => false,
        }
    }

    /// The session to resume, `None` for a new one.
    pub fn resume(&self, harness: &str) -> Result<Option<String>> {
        match self {
            Self::Pending => Ok(None),
            Self::Bound { session } => {
                valid_session(harness, session)?;
                Ok(Some(session.clone()))
            }
            Self::ReleaseUnknown { .. } => Err("harness session ownership is unknown".into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Workflow {
    pub harness: String,
    pub profile: Profile,
    pub fingerprint: String,
    pub source: Source,
    pub revision: u64,
    pub last_run: String,
    pub binding: Binding,
}

impl Workflow {
    pub fn validate(&self) -> Result<()> {
        if adapter(&self.harness).is_err()
            || self.revision == 0
            || self.fingerprint != digest(&self.profile)?
        {
            return Err("invalid harness workflow".into());
        }
        self.profile.validate()?;
        crate::model::check_page_fields(&self.source.page)?;
        match &self.binding {
            Binding::Bound { session }
            | Binding::ReleaseUnknown {
                previous: Some(session),
            } => valid_session(&self.harness, session),
            _ => Ok(()),
        }
    }
}

/// Checks saved harness workflows against the runs that launched them.
pub(crate) fn validate(state: &Store) -> Result<()> {
    let native =
        state.workflows.len() + state.codex_workflows.len() + state.opencode_workflows.len();
    if native + state.harness_workflows.len() > 16 {
        return Err("too many native workflows".into());
    }
    let valid_id = |id: &str| {
        id.strip_prefix(&format!("{}:", state.view.store))
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|n| n > 0 && n < state.next)
    };
    let launched = |run: &str| match state.launches.get(run) {
        Some(Launch::Harness {
            harness, workflow, ..
        }) => Some((harness, workflow)),
        _ => None,
    };
    for run in &state.view.runs {
        let Some((harness, workflow)) = launched(&run.id) else {
            continue;
        };
        let w = state
            .harness_workflows
            .get(workflow)
            .ok_or("missing harness workflow")?;
        if run.kind != "agent" || (run.id == w.last_run && run.source != w.source.id) || &w.harness != harness {
            return Err("invalid harness task record".into());
        }
    }
    for (id, w) in &state.harness_workflows {
        w.validate()?;
        let last = state
            .view
            .runs
            .iter()
            .rev()
            .find(|run| launched(&run.id).is_some_and(|(_, w)| w == id));
        if !valid_id(id)
            || !valid_id(&w.source.id)
            || state.workflows.contains_key(id)
            || state.codex_workflows.contains_key(id)
            || state.opencode_workflows.contains_key(id)
            || last.is_none_or(|run| run.id != w.last_run)
        {
            return Err("invalid harness workflow identity".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_resume_only_a_valid_bound_session() {
        let session = "07070707-0707-4707-8707-070707070707";
        assert_eq!(Binding::Pending.resume("claude").unwrap(), None);
        let bound = Binding::Bound {
            session: session.into(),
        };
        assert_eq!(bound.resume("claude").unwrap().as_deref(), Some(session));
        assert!(
            Binding::Bound {
                session: "../x".into()
            }
            .resume("claude")
            .is_err()
        );
        let mut lost = bound.clone();
        lost.block_recovery();
        assert!(lost.resume("claude").is_err());
        assert!(
            adapter("codex").is_err(),
            "codex keeps its own records until it migrates"
        );
    }
}
