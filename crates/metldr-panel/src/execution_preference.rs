const STORAGE_KEY: &str = "metldr.composer.destination";
pub(crate) const MAX_EXECUTION_CHOICE_BYTES: usize = 1024;

#[derive(Default)]
pub(crate) struct ExecutionPreference {
    current: Option<String>,
    persisted: Option<String>,
}

impl ExecutionPreference {
    pub(crate) fn load() -> Self {
        let value = web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item(STORAGE_KEY).ok().flatten())
            .filter(|value| value.len() <= MAX_EXECUTION_CHOICE_BYTES)
            .map(|value| agent_of_session(&value).map_or(value, str::to_owned));
        Self {
            current: value.clone(),
            persisted: value,
        }
    }

    pub(crate) fn value(&self) -> Option<String> {
        self.current.clone()
    }

    pub(crate) fn remember(&mut self, value: &str) {
        self.remember_with(value, |value| {
            let storage = web_sys::window()
                .and_then(|window| window.local_storage().ok().flatten())
                .ok_or(())?;
            storage.set_item(STORAGE_KEY, value).map_err(|_| ())
        });
    }

    fn remember_with(&mut self, value: &str, write: impl FnOnce(&str) -> Result<(), ()>) {
        if self.current.as_deref() == Some(value) && self.persisted.as_deref() == Some(value) {
            return;
        }
        self.current = Some(value.to_owned());
        if self.persisted.as_deref() != Some(value) && write(value).is_ok() {
            self.persisted = Some(value.to_owned());
        }
    }
}

/// A chat keeps one session per agent, so a saved pick of one session from the former menu
/// becomes a pick of its agent: `claude:<id>` is Claude Code and a bare workflow id is Pi.
fn agent_of_session(value: &str) -> Option<&'static str> {
    let (head, tail) = value.split_once(':')?;
    match head {
        "codex" => Some("codex"),
        "opencode" => Some("opencode"),
        "claude" => Some("claude"),
        _ if head.len() == 32
            && head.bytes().all(|b| b.is_ascii_hexdigit())
            && !tail.is_empty()
            && tail.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some("pi")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::ExecutionPreference;

    #[test]
    fn memory_survives_storage_failure_and_failed_write_can_retry() {
        let mut preference = ExecutionPreference::default();
        let mut writes = 0;
        preference.remember_with("codex:workflow", |_| {
            writes += 1;
            Err(())
        });
        assert_eq!(preference.value().as_deref(), Some("codex:workflow"));
        preference.remember_with("codex:workflow", |_| {
            writes += 1;
            Ok(())
        });
        preference.remember_with("codex:workflow", |_| {
            writes += 1;
            Ok(())
        });
        assert_eq!(preference.value().as_deref(), Some("codex:workflow"));
        assert_eq!(writes, 2);
    }

    #[test]
    fn a_saved_session_pick_becomes_its_agent() {
        use super::agent_of_session;
        assert_eq!(agent_of_session("claude:0123456789abcdef0123456789abcdef:3"), Some("claude"));
        assert_eq!(agent_of_session("0123456789abcdef0123456789abcdef:3"), Some("pi"));
        assert_eq!(agent_of_session("page:responses"), None);
        assert_eq!(agent_of_session("codex"), None);
    }
}
