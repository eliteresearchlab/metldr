use crate::{
    Result,
    agent_settings::{Kind, Profile as AgentProfile, Selection as AgentSelection},
    flow::Owner,
    https_profile::{Credential, Profile},
    model::Store,
    provider_settings::{Protocol, Selection as ProviderSelection},
};
use serde_json::{Value, json};

pub(crate) fn viewed_run(state: &Store) -> Option<String> {
    // A native projection also includes its workflow head when viewing an older task.
    let latest = state
        .workflows
        .values()
        .map(|workflow| workflow.last_run.as_str())
        .chain(
            state
                .codex_workflows
                .values()
                .map(|workflow| workflow.last_run.as_str()),
        )
        .chain(
            state
                .opencode_workflows
                .values()
                .map(|workflow| workflow.last_run.as_str()),
        )
        .next();
    state
        .view
        .runs
        .iter()
        .find(|run| latest != Some(run.id.as_str()))
        .or_else(|| state.view.runs.first())
        .map(|run| run.id.clone())
}

impl Owner {
    pub(super) fn refresh_saved_settings(&mut self) -> Result<()> {
        let run = viewed_run(&self.state);
        let previous_agents = serde_json::to_vec(&self.state.agent_settings)?;
        let previous_providers =
            serde_json::to_vec(&(&self.state.providers.scope, &self.state.providers.slots))?;
        let ((providers, agents), _) = self.persist_records(|edit, scope| {
            Ok((
                edit.settings(scope)?,
                Some(edit.project(scope, run.as_deref())?),
            ))
        })?;
        if previous_agents != serde_json::to_vec(&agents)? {
            for kind in [
                crate::agent_settings::Kind::Pi,
                crate::agent_settings::Kind::Codex,
                crate::agent_settings::Kind::Opencode,
            ] {
                self.refresh_agent_runtime(kind);
            }
        }
        if previous_providers != serde_json::to_vec(&(&providers.scope, &providers.slots))? {
            for protocol in crate::provider_settings::Protocol::ALL {
                if providers.managed(protocol) {
                    self.set_provider_route(protocol, None);
                    self.settings.states.remove(&protocol);
                }
            }
        }
        Ok(())
    }

    pub(super) fn persist_settings(&mut self, next: Store) -> Result<()> {
        let providers = self.state.providers.clone();
        let agents = self.state.agent_settings.clone();
        let run = viewed_run(&self.state);
        self.persist_records(|edit, scope| {
            edit.replace_settings(
                scope,
                &providers,
                &agents,
                next.providers,
                next.agent_settings,
            )?;
            Ok(((), Some(edit.refresh_view(scope, run.as_deref())?)))
        })?;
        Ok(())
    }

    /// Answers a browser tool's read of the settings for a live browser run.
    pub(super) fn browser_settings_read(
        &self,
        run: &str,
        ack: std::sync::mpsc::SyncSender<std::result::Result<Value, String>>,
    ) -> Result<()> {
        let result = if self.operations.get(run).is_some_and(|operation| {
            operation.browser_io.is_some()
                && !operation.stop.load(std::sync::atomic::Ordering::Acquire)
        }) {
            Ok(self.browser_settings_snapshot())
        } else {
            Err("browser run is not active".to_owned())
        };
        let _ = ack.send(result);
        Ok(())
    }

    // Credentials, endpoints, trust roots and agent file paths are never included.
    fn browser_settings_snapshot(&self) -> Value {
        let providers: Vec<Value> = Protocol::ALL
            .into_iter()
            .map(|protocol| {
                let slot = self.state.providers.slot(protocol);
                let state = match &slot.selection {
                    ProviderSelection::Unmanaged => "unmanaged",
                    ProviderSelection::Removed => "removed",
                    ProviderSelection::Configured { .. } => "configured",
                };
                let route = self.provider_route(protocol);
                json!({
                    "protocol": protocol.name(),
                    "stored": {"state": state, "revision": slot.revision},
                    "effective": {
                        "source": route_source(self.state.providers.managed(protocol), route),
                    },
                })
            })
            .collect();
        let agents: Vec<Value> = [Kind::Pi, Kind::Codex, Kind::Opencode]
            .into_iter()
            .map(|kind| {
                let slot = self.state.agent_settings.slot(kind);
                let (state, detail, source, runtime) = match &slot.selection {
                    AgentSelection::Unmanaged => ("unmanaged", json!({}), "none", "none"),
                    AgentSelection::Removed => ("removed", json!({}), "none", "none"),
                    AgentSelection::Configured { profile } => (
                        "configured",
                        agent_detail(profile),
                        "saved",
                        self.agent_runtime
                            .states
                            .get(&kind)
                            .copied()
                            .unwrap_or("unavailable"),
                    ),
                };
                json!({
                    "agent": kind,
                    "stored": {"state": state, "revision": slot.revision, "detail": detail},
                    "effective": {"source": source, "runtime": runtime},
                })
            })
            .collect();
        // Offline mode is a security switch, so it is reported and never agent-writable.
        json!({"agent_writable": false, "providers": providers, "agents": agents})
    }
}

// A saved setting replaces the launch route. An unsaved route names where its
// credential comes from; the credential itself is never read or returned here.
fn route_source(managed: bool, route: Option<&Profile>) -> &'static str {
    match (managed, route) {
        (_, None) => "none",
        (true, Some(_)) => "saved",
        (false, Some(Profile::Fixture { .. })) => "fixture",
        (false, Some(Profile::Https { credential, .. })) => match credential {
            Credential::Environment(_) => "environment",
            Credential::None | Credential::Pinned(_) | Credential::Keyring { .. } => "launch",
        },
    }
}

fn agent_detail(profile: &AgentProfile) -> Value {
    match profile {
        AgentProfile::Pi { offline, .. } => json!({"offline": offline}),
        AgentProfile::Codex { .. } | AgentProfile::Opencode { .. } => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::https_profile::Binding;

    #[test]
    fn environment_override_is_the_effective_value_until_a_setting_is_saved() {
        let route = Profile::Https {
            binding: Binding {
                endpoint: "https://example.com/v1/messages".into(),
                model: "model-a".into(),
                revision: 1,
                fingerprint: "0".repeat(64),
            },
            credential: Credential::Environment("METLDR_TEST_KEY".into()),
            trust_pem: None,
            timeout_secs: None,
            chat: Default::default(),
        };
        assert_eq!(route_source(false, Some(&route)), "environment");
        assert_eq!(route_source(true, Some(&route)), "saved");
        assert_eq!(route_source(false, None), "none");
        let pi = AgentProfile::Pi {
            executable: "/bin/pi".into(),
            workspace: "/workspace".into(),
            session_dir: "/sessions".into(),
            provider: "/p".into(),
            model: "sk-test-0".into(),
            offline: true,
        };
        assert_eq!(agent_detail(&pi), json!({"offline": true}));
    }
}
