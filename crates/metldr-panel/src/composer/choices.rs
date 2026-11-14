use super::*;
use crate::model::AgentKind;

/// A choice in the model menu. Fixed choices borrow their text, so building the menu on
/// each view pass allocates only for saved sessions and configured routes.
pub(crate) struct Choice {
    pub(crate) value: std::borrow::Cow<'static, str>,
    pub(crate) label: std::borrow::Cow<'static, str>,
    pub(crate) enabled: bool,
    /// The legacy panel's option element for this choice.
    pub(super) element: &'static str,
}

/// The fixed rows of the model menu: value, legacy option element and label.
const ROWS: [(&str, &str, &str); 13] = [
    ("model", "execution-model", "Test model"),
    ("browser", "execution-browser", "Browser task, Messages"),
    ("chat", "execution-chat", "Browser task, Chat Completions"),
    (
        "responses",
        "execution-responses",
        "Browser task, OpenAI Responses",
    ),
    (
        "google",
        "execution-google",
        "Browser task, Google Interactions",
    ),
    (
        "page:messages",
        "execution-page-messages",
        "Page questions, Messages",
    ),
    (
        "page:responses",
        "execution-page-responses",
        "Page questions, OpenAI Responses",
    ),
    (
        "page:google_interactions",
        "execution-page-google",
        "Page questions, Google Interactions",
    ),
    (
        "page:chat_completions",
        "execution-page-chat",
        "Page questions, Chat Completions",
    ),
    ("pi", "execution-pi", "Pi"),
    ("codex", "execution-codex", "Codex"),
    ("opencode", "execution-opencode", "OpenCode"),
    ("claude", "execution-claude", "Claude Code"),
];

pub(crate) fn list(view: Option<&PageView>) -> Vec<Choice> {
    ROWS.iter().map(|row| build(view, *row)).collect()
}

/// The choice for `value` alone, without building the rest of the menu.
pub(crate) fn selected(view: Option<&PageView>, value: &str) -> Option<Choice> {
    ROWS.iter()
        .find(|row| row.0 == value)
        .map(|row| build(view, *row))
}

fn build(
    view: Option<&PageView>,
    (value, element, label): (&'static str, &'static str, &'static str),
) -> Choice {
    let mut choice = Choice {
        value: value.into(),
        label: label.into(),
        enabled: view.is_some_and(|view| resolve(view, value).is_ok()),
        element,
    };
    let Some(view) = view else { return choice };
    let route = match &*choice.value {
        "browser" => match &view.browser_route {
            Some(BrowserRouteView::Messages { model }) => {
                choice.label = format!("Browser task, Messages, {model}").into();
                None
            }
            Some(BrowserRouteView::HttpsMessages {
                endpoint, model, ..
            }) => Some((
                "Browser task",
                "Messages",
                endpoint.as_str(),
                model.as_str(),
            )),
            None => None,
        },
        "chat" => view.chat_route.as_ref().map(
            |ChatRouteView::HttpsChatCompletions {
                 endpoint, model, ..
             }| {
                (
                    "Browser task",
                    "Chat Completions",
                    endpoint.as_str(),
                    model.as_str(),
                )
            },
        ),
        "responses" => view.responses_route.as_ref().map(
            |ResponsesRouteView::HttpsResponses {
                 endpoint, model, ..
             }| {
                (
                    "Browser task",
                    "OpenAI Responses",
                    endpoint.as_str(),
                    model.as_str(),
                )
            },
        ),
        "google" => view.google_route.as_ref().map(
            |GoogleRouteView::HttpsGoogleInteractions {
                 endpoint, model, ..
             }| {
                (
                    "Browser task",
                    "Google Interactions",
                    endpoint.as_str(),
                    model.as_str(),
                )
            },
        ),
        value if value.starts_with("page:") => ProviderProtocol::ALL
            .into_iter()
            .find(|protocol| value.strip_prefix("page:") == Some(protocol.name()))
            .and_then(|protocol| {
                page_route(view, protocol).map(|(endpoint, model, _)| {
                    ("Page questions", provider_name(protocol), endpoint, model)
                })
            }),
        _ => None,
    };
    if let Some((task, protocol, endpoint, model)) = route {
        choice.label = format!("{task}, {protocol}, {model}, {endpoint}").into();
    }
    choice
}

#[cfg(test)]
mod checks {
    use super::*;
    use crate::model::{CodexProfileView, PiProfileView};

    #[test]
    fn choices_preserve_all_routes_and_only_ready_matching_sessions() {
        let expected = [
            "model",
            "browser",
            "chat",
            "responses",
            "google",
            "page:messages",
            "page:responses",
            "page:google_interactions",
            "page:chat_completions",
            "pi",
            "codex",
            "opencode",
            "claude",
        ];
        let unavailable = list(None);
        assert_eq!(
            unavailable
                .iter()
                .map(|choice| &*choice.value)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(unavailable.iter().all(|choice| !choice.enabled));
        let mut view = PageView::decode(br#"{"sequence":1,"store":"0123456789abcdef0123456789abcdef","selection":{"store":"0123456789abcdef0123456789abcdef","conversation":"0123456789abcdef0123456789abcdef:100","epoch":"1"},"source":{"id":"0123456789abcdef0123456789abcdef:1","tab":4,"document":"doc-1","url":"http://127.0.0.1:43123/article","title":"North report","text":"North has 17 units."},"runs":[],"native_correction":null,"pi_profile":null,"agent_sessions":[],"browser_route":null}"#).unwrap();
        view.fixture_available = true;
        let endpoint = "https://api.example.test/v1".to_owned();
        let model = "online-model".to_owned();
        let fingerprint = "a".repeat(64);
        view.browser_route = Some(BrowserRouteView::HttpsMessages {
            endpoint: endpoint.clone(),
            model: model.clone(),
            revision: 1,
            fingerprint: fingerprint.clone(),
        });
        view.responses_route = Some(ResponsesRouteView::HttpsResponses {
            endpoint: endpoint.clone(),
            model: model.clone(),
            revision: 1,
            fingerprint: fingerprint.clone(),
        });
        view.chat_route = Some(ChatRouteView::HttpsChatCompletions {
            endpoint: endpoint.clone(),
            model: model.clone(),
            revision: 1,
            fingerprint: fingerprint.clone(),
        });
        view.google_route = Some(GoogleRouteView::HttpsGoogleInteractions {
            endpoint: endpoint.clone(),
            model: model.clone(),
            revision: 1,
            fingerprint: fingerprint.clone(),
        });
        view.pi_profile = Some(PiProfileView {
            fingerprint: fingerprint.clone(),
            provider: "native-provider".into(),
            model,
        });
        view.codex_profile = Some(CodexProfileView {
            fingerprint: fingerprint.clone(),
        });
        view.opencode_profile = view.codex_profile.clone();
        let session = AgentSessionView {
            workflow: serde_json::from_str("\"0123456789abcdef0123456789abcdef:20\"").unwrap(),
            source: view.source().unwrap().clone(),
            profile: fingerprint.clone(),
            revision: 7,
            ready: true,
        };
        view.agent_sessions.push(session.clone());
        view.codex_sessions.push(session.clone());
        view.opencode_sessions.push(session.clone());
        view.claude_sessions.push(session);
        for row in &mut view.agent_settings.agents {
            if row.agent == AgentKind::Claude {
                row.state = crate::model::AgentSettingState::Ready;
            }
        }
        let choices = list(Some(&view));
        // One chat keeps one session per agent: picking the agent continues it.
        use crate::composer::{Destination, resolve};
        for (agent, old) in [
            ("claude", "claude:0123456789abcdef0123456789abcdef:20"),
            ("codex", "codex:0123456789abcdef0123456789abcdef:20"),
            ("opencode", "opencode:0123456789abcdef0123456789abcdef:20"),
            ("pi", "0123456789abcdef0123456789abcdef:20"),
        ] {
            for value in [agent, old] {
                assert!(
                    matches!(
                        resolve(&view, value),
                        Ok(Destination::ContinueClaude { revision: 7, .. }
                            | Destination::ContinueCodex { revision: 7, .. }
                            | Destination::ContinueOpenCode { revision: 7, .. }
                            | Destination::Continue { revision: 7, .. })
                    ),
                    "{value} continues the chat's session"
                );
            }
        }
        assert_eq!(choices.len(), 13);
        assert!(choices.iter().all(|choice| choice.enabled));
        assert_eq!(
            choices[6].label,
            "Page questions, OpenAI Responses, online-model, https://api.example.test/v1"
        );
        assert!(
            choices
                .iter()
                .all(|choice| !choice.label.contains(&fingerprint))
        );
        for sessions in [
            &mut view.agent_sessions,
            &mut view.codex_sessions,
            &mut view.opencode_sessions,
            &mut view.claude_sessions,
        ] {
            sessions[0].ready = false;
        }
        assert!(matches!(resolve(&view, "claude"), Ok(Destination::Claude)));
        assert!(matches!(resolve(&view, "codex"), Ok(Destination::Codex)));
        assert!(matches!(
            resolve(&view, "opencode"),
            Ok(Destination::OpenCode)
        ));
        assert!(matches!(resolve(&view, "pi"), Ok(Destination::Pi)));
        view.responses_route = None;
        let choices = list(Some(&view));
        assert!(!choices[3].enabled && !choices[6].enabled);
        assert_eq!(choices[3].value, "responses");
        assert_eq!(choices[6].value, "page:responses");
    }
}
