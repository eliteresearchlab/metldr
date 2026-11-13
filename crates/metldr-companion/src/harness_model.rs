//! Saved state for harnesses on the generic path (B1.12). A harness is named by its
//! registry key; its native session id is the continuation key.

use crate::{Result, model::Source};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Harness keys this path accepts today. Pi, Codex and OpenCode keep their own records
/// until they migrate.
pub(crate) const KEYS: [&str; 1] = ["claude"];

pub(crate) fn valid_key(key: &str) -> bool {
    KEYS.contains(&key)
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
                return Err("invalid harness profile path".into());
            }
        }
        Ok(())
    }
}

/// Checks a native session id for `harness`.
pub(crate) fn valid_session(harness: &str, session: &str) -> Result<()> {
    match harness {
        "claude" => metldr_claude_session::SessionId::saved(session).map(|_| ()),
        _ => Err("unknown harness".into()),
    }
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
        if !valid_key(&self.harness) || self.revision == 0 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_resume_only_a_valid_bound_session() {
        let session = "07070707-0707-4707-8707-070707070707";
        assert_eq!(Binding::Pending.resume("claude").unwrap(), None);
        let bound = Binding::Bound { session: session.into() };
        assert_eq!(bound.resume("claude").unwrap().as_deref(), Some(session));
        assert!(Binding::Bound { session: "../x".into() }.resume("claude").is_err());
        let mut lost = bound.clone();
        lost.block_recovery();
        assert!(lost.resume("claude").is_err());
        assert!(!valid_key("codex"), "codex keeps its own records until it migrates");
    }
}
