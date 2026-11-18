

use crate::flow::FlowClient;
use crate::model::{
    AgentSessionView, BrowserRouteView, ChatRouteView, FlowError, GoogleRouteView, PageView,
    Protocol, ResponsesRouteView, RunId, SourceId, UserText,
};

pub(crate) mod choices;

pub(crate) enum Destination {
    Model,
    Browser {
        fingerprint: Option<String>,
    },
    Chat {
        fingerprint: String,
    },
    Responses {
        fingerprint: String,
    },
    Google {
        fingerprint: String,
    },
    PageText {
        protocol: Protocol,
        fingerprint: String,
    },
    Pi,
    Codex,
    OpenCode,
    Continue {
        workflow: RunId,
        revision: u64,
    },
    ContinueCodex {
        workflow: RunId,
        revision: u64,
    },
    ContinueOpenCode {
        workflow: RunId,
        revision: u64,
    },
    Claude,
    ContinueClaude {
        workflow: RunId,
        revision: u64,
    },
}

fn ready(view: &PageView, session: &AgentSessionView) -> bool {
    session.ready
        && view
            .pi_profile
            .as_ref()
            .is_some_and(|profile| session.profile == profile.fingerprint)
        && view.source().is_some()
}

fn codex_ready(view: &PageView, session: &AgentSessionView) -> bool {
    session.ready
        && view
            .codex_profile
            .as_ref()
            .is_some_and(|profile| session.profile == profile.fingerprint)
        && view.source().is_some()
}

/// The chat's most recently created session that can continue now.
fn newest<'a>(
    sessions: &'a [AgentSessionView],
    ready: impl Fn(&AgentSessionView) -> bool,
) -> Option<&'a AgentSessionView> {
    sessions.iter().filter(|session| ready(session)).max_by_key(|session| {
        let id = session.workflow.as_str();
        id.rsplit(':').next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0)
    })
}

/// Whether a Claude Code profile is saved and ready.
fn claude_configured(view: &PageView) -> bool {
    view.agent_settings.agents.iter().any(|row| {
        row.agent == crate::model::AgentKind::Claude && row.state == crate::model::AgentSettingState::Ready
    })
}

fn claude_ready(view: &PageView, session: &AgentSessionView) -> bool {
    session.ready && claude_configured(view) && view.source().is_some()
}

fn opencode_ready(view: &PageView, session: &AgentSessionView) -> bool {
    session.ready
        && view
            .opencode_profile
            .as_ref()
            .is_some_and(|profile| session.profile == profile.fingerprint)
        && view.source().is_some()
}

fn page_route(view: &PageView, protocol: Protocol) -> Option<(&str, &str, &str)> {
    match protocol {
        Protocol::Messages => view.browser_route.as_ref().and_then(|route| match route {
            BrowserRouteView::HttpsMessages {
                endpoint,
                model,
                fingerprint,
                ..
            } => Some((endpoint.as_str(), model.as_str(), fingerprint.as_str())),
            BrowserRouteView::Messages { .. } => None,
        }),
        Protocol::ChromeAi | Protocol::SystemOne => None,
        Protocol::Responses => view.responses_route.as_ref().map(
            |ResponsesRouteView::HttpsResponses {
                 endpoint,
                 model,
                 fingerprint,
                 ..
             }| (endpoint.as_str(), model.as_str(), fingerprint.as_str()),
        ),
        Protocol::ChatCompletions => view.chat_route.as_ref().map(
            |ChatRouteView::HttpsChatCompletions {
                 endpoint,
                 model,
                 fingerprint,
                 ..
             }| (endpoint.as_str(), model.as_str(), fingerprint.as_str()),
        ),
        Protocol::GoogleInteractions => view.google_route.as_ref().map(
            |GoogleRouteView::HttpsGoogleInteractions {
                 endpoint,
                 model,
                 fingerprint,
                 ..
             }| (endpoint.as_str(), model.as_str(), fingerprint.as_str()),
        ),
    }
}

pub(crate) fn resolve(view: &PageView, value: &str) -> Result<Destination, String> {
    match value {
        "" => Err("Choose a destination before sending.".into()),
        "model" if view.fixture_available => Ok(Destination::Model),
        "model" => Err("The test model is unavailable. Choose a configured destination.".into()),
        "browser" => match view.browser_route.as_ref() {
            Some(BrowserRouteView::Messages { .. }) => {
                Ok(Destination::Browser { fingerprint: None })
            }
            Some(BrowserRouteView::HttpsMessages { fingerprint, .. }) => Ok(Destination::Browser {
                fingerprint: Some(fingerprint.clone()),
            }),
            None => Err("The browser task is unavailable.".into()),
        },
        "responses" => match view.responses_route.as_ref() {
            Some(ResponsesRouteView::HttpsResponses { fingerprint, .. }) => {
                Ok(Destination::Responses {
                    fingerprint: fingerprint.clone(),
                })
            }
            None => Err("The OpenAI Responses destination is unavailable.".into()),
        },
        "chat" => match view.chat_route.as_ref() {
            Some(ChatRouteView::HttpsChatCompletions { fingerprint, .. }) => {
                Ok(Destination::Chat {
                    fingerprint: fingerprint.clone(),
                })
            }
            None => Err("Chat Completions is unavailable.".into()),
        },
        "google" => match view.google_route.as_ref() {
            Some(GoogleRouteView::HttpsGoogleInteractions { fingerprint, .. }) => {
                Ok(Destination::Google {
                    fingerprint: fingerprint.clone(),
                })
            }
            None => Err("Google Interactions is unavailable.".into()),
        },
        value if value.starts_with("page:") => {
            let protocol = match value {
                "page:messages" => Protocol::Messages,
                "page:responses" => Protocol::Responses,
                "page:chat_completions" => Protocol::ChatCompletions,
                "page:google_interactions" => Protocol::GoogleInteractions,
                _ => {
                    return Err("The selected page questions provider is unavailable.".into());
                }
            };
            let fingerprint = page_route(view, protocol)
                .map(|(_, _, fingerprint)| fingerprint.to_owned())
                .ok_or_else(|| {
                    format!(
                        "{} page questions are unavailable.",
                        Protocol::label(protocol)
                    )
                })?;
            Ok(Destination::PageText {
                protocol,
                fingerprint,
            })
        }
        // One chat keeps one session per agent: its newest ready session continues (B1.12.4).
        "pi" if view.pi_profile.is_some() => Ok(newest(&view.agent_sessions, |s| ready(view, s))
            .map_or(Destination::Pi, |s| Destination::Continue { workflow: s.workflow.clone(), revision: s.revision })),
        "pi" => Err("Pi is unavailable. Choose another destination or configure Pi.".into()),
        "codex" if view.codex_profile.is_some() => Ok(newest(&view.codex_sessions, |s| codex_ready(view, s))
            .map_or(Destination::Codex, |s| Destination::ContinueCodex { workflow: s.workflow.clone(), revision: s.revision })),
        "codex" => Err(
            "Codex configuration is unavailable. Choose another destination or configure Codex."
                .into(),
        ),
        "opencode" if view.opencode_profile.is_some() => Ok(newest(&view.opencode_sessions, |s| opencode_ready(view, s))
            .map_or(Destination::OpenCode, |s| Destination::ContinueOpenCode { workflow: s.workflow.clone(), revision: s.revision })),
        "opencode" => Err("OpenCode is unavailable. Configure an OpenCode profile first.".into()),
        "claude" if claude_configured(view) => Ok(newest(&view.claude_sessions, |s| claude_ready(view, s))
            .map_or(Destination::Claude, |s| Destination::ContinueClaude { workflow: s.workflow.clone(), revision: s.revision })),
        "claude" => Err("Claude Code is unavailable. Configure a Claude Code profile first.".into()),
        _ => {
            if let Some(id) = value.strip_prefix("claude:") {
                let session = view.claude_sessions.iter().find(|session| session.workflow.as_str() == id && claude_ready(view, session))
                    .ok_or_else(|| "The selected Claude Code session is unavailable. Choose a destination before sending.".to_owned())?;
                return Ok(Destination::ContinueClaude {
                    workflow: session.workflow.clone(),
                    revision: session.revision,
                });
            }
            if let Some(id) = value.strip_prefix("opencode:") {
                let session = view.opencode_sessions.iter().find(|session| session.workflow.as_str() == id && opencode_ready(view, session))
                    .ok_or_else(|| "The selected OpenCode session is unavailable. Choose a destination before sending.".to_owned())?;
                return Ok(Destination::ContinueOpenCode {
                    workflow: session.workflow.clone(),
                    revision: session.revision,
                });
            }
            if let Some(id) = value.strip_prefix("codex:") {
                let session = view.codex_sessions.iter().find(|session| session.workflow.as_str() == id && codex_ready(view, session))
                    .ok_or_else(|| "The selected Codex session is unavailable. Choose a destination before sending.".to_owned())?;
                return Ok(Destination::ContinueCodex {
                    workflow: session.workflow.clone(),
                    revision: session.revision,
                });
            }
            let session = view
                .agent_sessions
                .iter()
                .find(|session| session.workflow.as_str() == value && ready(view, session))
                .ok_or_else(|| {
                    "The selected Pi session is unavailable. Choose a destination before sending."
                        .to_owned()
                })?;
            Ok(Destination::Continue {
                workflow: session.workflow.clone(),
                revision: session.revision,
            })
        }
    }
}

pub(crate) async fn dispatch(
    flow: &FlowClient,
    scope: metldr_history_pages::Scope,
    source: SourceId,
    destination: Destination,
    text: UserText,
    codex_mode: &str,
    summary: bool,
) -> Result<RunId, FlowError> {
    match destination {
        Destination::Model => flow.ask(scope, source, text).await,
        Destination::Browser { fingerprint } => {
            flow.browser_task(scope, source, text, fingerprint).await
        }
        Destination::Responses { fingerprint } => {
            flow.responses_task(scope, source, text, fingerprint).await
        }
        Destination::Chat { fingerprint } => flow.chat_task(scope, source, text, fingerprint).await,
        Destination::Google { fingerprint } => {
            flow.google_task(scope, source, text, fingerprint).await
        }
        Destination::PageText {
            protocol,
            fingerprint,
        } => {
            flow.page_text(
                scope,
                source,
                protocol,
                fingerprint,
                if summary { crate::flow::PageTextPrompt::Summary } else { crate::flow::PageTextPrompt::Answer(text) },
            )
            .await
        }
        Destination::Pi => flow.pi_prompt(scope, source, text).await,
        Destination::Codex => flow.codex_prompt(scope, source, text, codex_mode).await,
        Destination::OpenCode => flow.opencode_prompt(scope, source, text).await,
        Destination::Continue { workflow, revision } => {
            flow.pi_follow_up(scope, workflow, revision, &source, text).await
        }
        Destination::ContinueCodex { workflow, revision } => {
            flow.codex_follow_up(scope, workflow, revision, &source, text, codex_mode)
                .await
        }
        Destination::ContinueOpenCode { workflow, revision } => {
            flow.opencode_follow_up(scope, workflow, revision, &source, text)
                .await
        }
        Destination::Claude => flow.harness_prompt(scope, "claude", source, text).await,
        Destination::ContinueClaude { workflow, revision } => {
            flow.harness_follow_up(scope, "claude", workflow, revision, &source, text)
                .await
        }
    }
}
