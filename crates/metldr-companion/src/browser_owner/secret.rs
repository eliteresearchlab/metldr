//! Secret entry consent (decision 09, B3.5).
//!
//! The user types the secret in the panel for one pending entry. It lives only in memory in the
//! run's browser transport slot until the owner splices it into that entry's debugger command,
//! then it is wiped. It never reaches the model, the browser worker, history or logs.

use crate::{Result, flow::Owner};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

/// A secret typed by the user. Its `Debug` and `Serialize` forms are redacted.
pub(crate) struct SecretText(Zeroizing<String>);

impl<'de> Deserialize<'de> for SecretText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer).map(|text| Self(Zeroizing::new(text)))
    }
}

impl Serialize for SecretText {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str("[redacted]")
    }
}

impl std::fmt::Debug for SecretText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// One approved secret waiting for its dispatch.
pub(crate) struct Held {
    pub action: String,
    pub value: Zeroizing<String>,
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

impl Owner {
    /// Approves the pending secret entry with the secret the user typed.
    pub(crate) fn secret_enter(
        &mut self,
        action: &str,
        revision: u64,
        secret: SecretText,
    ) -> Result<()> {
        let pending = self
            .browser_approval
            .as_ref()
            .filter(|p| p.action == action && p.dispatch_revision.is_none())
            .ok_or("no secret entry is waiting for this action")?;
        let expires = pending
            .secret_expires_ms
            .ok_or("this browser action does not take a secret")?;
        if now_ms() >= expires {
            return Err("secret entry expired".into());
        }
        let text = secret.0;
        if text.is_empty() || text.len() > 1024 || text.chars().any(char::is_control) {
            return Err("enter a secret of at most 1024 bytes without control characters".into());
        }
        let run = pending.run.clone();
        self.browser_operation(&run)?;
        if self
            .operations
            .get(&run)
            .and_then(|operation| operation.browser_io.as_ref())
            .is_none()
        {
            return Err("browser transport is unavailable".into());
        }
        let record = self.record_execution(&run, |edit, execution| {
            edit.decide_browser(execution, action, revision, true)
        })?;
        if let Some(slot) = self
            .operations
            .get_mut(&run)
            .and_then(|operation| operation.browser_io.as_mut())
        {
            slot.hold_secret(Held {
                action: action.to_owned(),
                value: text,
            });
        }
        if let Some(pending) = self.browser_approval.take() {
            pending.reply(Ok(record));
        }
        Ok(())
    }

    /// Prevents a secret entry the user did not answer in time.
    pub(crate) fn expire_secret_entry(&mut self) -> Result<()> {
        let Some(pending) = self.browser_approval.as_ref().filter(|p| {
            p.dispatch_revision.is_none() && p.secret_expires_ms.is_some_and(|at| now_ms() >= at)
        }) else {
            return Ok(());
        };
        let (run, action) = (pending.run.clone(), pending.action.clone());
        let result = self
            .record_execution(&run, |edit, execution| {
                edit.prevent_browser(execution, &run, "secret entry expired")
            })
            .and_then(|()| self.saved_browser_action(&action));
        if let Some(pending) = self.browser_approval.take() {
            pending.reply(result.map_err(|error| error.to_string()));
        }
        Ok(())
    }

    /// Drops any approved secret of `run` that was not dispatched.
    pub(crate) fn drop_secret(&mut self, run: &str) {
        if let Some(slot) = self
            .operations
            .get_mut(run)
            .and_then(|operation| operation.browser_io.as_mut())
        {
            slot.take_secret(None);
        }
    }
}
