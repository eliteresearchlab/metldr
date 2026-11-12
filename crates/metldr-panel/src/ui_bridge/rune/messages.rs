use std::collections::{HashMap, HashSet};

use crate::{
    ag_ui::{AgUiMessage, Event, MAX_HISTORY_MESSAGES, MessageRole},
    model::{FlowError, RunId},
};

pub(super) struct Messages {
    pub(super) messages: Vec<AgUiMessage>,
    scope: DisplayScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DisplayScope {
    History,
    AwaitingRun { boundary: usize },
    Run { boundary: usize },
}

impl Messages {
    pub(super) fn new() -> Self {
        Self {
            messages: vec![],
            scope: DisplayScope::History,
        }
    }

    pub(super) fn replace_archive(&mut self, messages: Vec<AgUiMessage>) -> Result<(), FlowError> {
        validate(&messages)?;
        self.messages = messages;
        self.scope = DisplayScope::History;
        Ok(())
    }

    pub(super) fn apply(&mut self, event: Event) -> Result<(), FlowError> {
        match event {
            Event::RunStarted { .. } => {
                validate(&self.messages)?;
                self.scope = DisplayScope::AwaitingRun {
                    boundary: self.messages.len(),
                };
            }
            Event::MessagesSnapshot { messages } => {
                return self.replace_snapshot(messages);
            }
            Event::TextMessageStart { message_id, role } => {
                if !crate::ag_ui::valid_id(&message_id) {
                    return Err(protocol("invalid or duplicate text message identity"));
                }
                let awaiting = matches!(self.scope, DisplayScope::AwaitingRun { .. });
                if let Some((index, existing)) = self
                    .messages
                    .iter()
                    .enumerate()
                    .find(|(_, message)| message.id() == message_id)
                {
                    if (!awaiting && index < self.boundary())
                        || !matches!(
                            (role, existing),
                            (MessageRole::Assistant, AgUiMessage::Assistant { .. })
                                | (MessageRole::User, AgUiMessage::User { .. })
                        )
                    {
                        return Err(protocol("text message start conflicts with its snapshot"));
                    }
                    if !awaiting {
                        return Ok(());
                    }
                }
                let message = match role {
                    MessageRole::User => AgUiMessage::User {
                        id: message_id,
                        content: String::new(),
                    },
                    MessageRole::Assistant => AgUiMessage::Assistant {
                        id: message_id,
                        content: String::new(),
                    },
                };
                if awaiting {
                    return self.replace_snapshot(vec![message]);
                }
                if self.messages.len() == MAX_HISTORY_MESSAGES {
                    return Err(protocol(
                        "message snapshot exceeds the conversation message bound",
                    ));
                }
                self.messages.push(message);
            }
            Event::TextMessageContent { message_id, delta } => {
                let boundary = self.boundary();
                let Some(AgUiMessage::Assistant { content, .. }) = self.messages[boundary..]
                    .iter_mut()
                    .find(|message| message.id() == message_id)
                else {
                    return Err(protocol("text delta has no matching assistant message"));
                };
                if delta.is_empty() {
                    return Ok(());
                }
                content.push_str(&delta);
            }
            Event::ActivitySnapshot {
                message_id,
                activity,
                replace,
            } => {
                if !crate::ag_ui::valid_id(&message_id) {
                    return Err(protocol("invalid activity message identity"));
                }
                let existing = self
                    .messages
                    .iter()
                    .position(|message| message.id() == message_id);
                if !replace && matches!(self.scope, DisplayScope::AwaitingRun { .. }) {
                    return self.replace_snapshot(vec![AgUiMessage::Activity {
                        id: message_id,
                        activity,
                    }]);
                }
                let existing = existing.filter(|index| *index >= self.boundary());
                if existing.is_none()
                    && self
                        .messages
                        .iter()
                        .any(|message| message.id() == message_id)
                {
                    return Err(protocol("activity snapshot conflicts with earlier history"));
                }
                match (replace, existing) {
                    (false, None) if self.messages.len() < MAX_HISTORY_MESSAGES => {
                        self.messages.push(AgUiMessage::Activity {
                            id: message_id,
                            activity,
                        });
                    }
                    (true, Some(index))
                        if matches!(self.messages[index], AgUiMessage::Activity { .. }) =>
                    {
                        self.messages[index] = AgUiMessage::Activity {
                            id: message_id,
                            activity,
                        };
                    }
                    _ => {
                        return Err(protocol(
                            "activity snapshot does not match replace semantics",
                        ));
                    }
                }
            }
            Event::TextMessageEnd { .. } | Event::RunFinished { .. } | Event::RunError { .. } => {}
        }
        Ok(())
    }

    fn boundary(&self) -> usize {
        match self.scope {
            DisplayScope::History => 0,
            DisplayScope::AwaitingRun { boundary } | DisplayScope::Run { boundary } => boundary,
        }
    }

    fn replace_snapshot(&mut self, messages: Vec<AgUiMessage>) -> Result<(), FlowError> {
        if messages.is_empty() && matches!(self.scope, DisplayScope::AwaitingRun { .. }) {
            return Ok(());
        }
        let roles: HashMap<_, _> = self
            .messages
            .iter()
            .map(|message| (message.id(), std::mem::discriminant(message)))
            .collect();
        if messages.iter().any(|message| {
            roles
                .get(message.id())
                .is_some_and(|role| *role != std::mem::discriminant(message))
        }) {
            return Err(protocol(
                "message snapshot changes an existing message role",
            ));
        }
        let (boundary, scope) = match self.scope {
            DisplayScope::AwaitingRun { boundary } => {
                let first = &messages[0];
                let run_prefix = entry_run(first.id()).map(|run| format!("{run}:"));
                let boundary = self
                    .messages
                    .iter()
                    .position(|old| {
                        run_prefix.as_ref().map_or_else(
                            || old.id() == first.id(),
                            |prefix| old.id().starts_with(prefix),
                        )
                    })
                    .unwrap_or(boundary);
                (boundary, DisplayScope::Run { boundary })
            }
            scope => (self.boundary(), scope),
        };
        if messages.len() > MAX_HISTORY_MESSAGES - boundary {
            return Err(protocol(
                "message snapshot exceeds the conversation message bound",
            ));
        }
        let mut proposed = self.messages[..boundary].to_vec();
        proposed.extend(messages);
        validate(&proposed)?;
        self.messages = proposed;
        self.scope = scope;
        Ok(())
    }
}

fn entry_run(id: &str) -> Option<RunId> {
    let mut parts = id.splitn(3, ':');
    let store = parts.next()?;
    let local = parts.next()?;
    parts.next().filter(|suffix| !suffix.is_empty())?;
    serde_json::from_value(serde_json::Value::String(format!("{store}:{local}"))).ok()
}

fn validate(messages: &[AgUiMessage]) -> Result<(), FlowError> {
    if messages.len() > MAX_HISTORY_MESSAGES {
        return Err(protocol(
            "message snapshot exceeds the conversation message bound",
        ));
    }
    let mut ids = HashSet::with_capacity(messages.len());
    if messages
        .iter()
        .any(|message| !crate::ag_ui::valid_id(message.id()) || !ids.insert(message.id()))
    {
        return Err(protocol(
            "message snapshot contains an invalid or duplicate identity",
        ));
    }
    Ok(())
}

fn protocol(message: &str) -> FlowError {
    FlowError::Protocol(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ag_ui::{Activity, TableReference};
    use metldr_conversation::ArtifactId;

    fn activity(name: &str) -> Activity {
        Activity::Table(TableReference {
            run_id: serde_json::from_value(serde_json::json!("0123456789abcdef0123456789abcdef:7"))
                .unwrap(),
            artifact_id: ArtifactId::new(name.into()).unwrap(),
        })
    }

    fn user(id: &str) -> AgUiMessage {
        AgUiMessage::User {
            id: id.into(),
            content: "question".into(),
        }
    }

    fn started() -> Event {
        Event::RunStarted {
            thread_id: "thread".into(),
            run_id: "run".into(),
        }
    }

    #[test]
    fn archive_replacement_resets_run_scope_and_rejects_invalid_pages_atomically() {
        let mut state = Messages::new();
        state.replace_archive(vec![user("first")]).unwrap();
        state.apply(started()).unwrap();
        state.replace_archive(vec![user("archive")]).unwrap();
        assert_eq!(state.scope, DisplayScope::History);
        state.apply(started()).unwrap();
        let scope = state.scope;
        assert!(
            state
                .replace_archive(vec![user("same"), user("same")])
                .is_err()
        );
        assert_eq!(state.scope, scope);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id(), "archive");
        state
            .apply(Event::MessagesSnapshot {
                messages: vec![user("next")],
            })
            .unwrap();
        assert_eq!(
            state
                .messages
                .iter()
                .map(AgUiMessage::id)
                .collect::<Vec<_>>(),
            vec!["archive", "next"]
        );
    }

    #[test]
    fn snapshots_replace_messages_and_activity_replacement_keeps_position() {
        let mut state = Messages::new();
        state
            .apply(Event::MessagesSnapshot {
                messages: vec![AgUiMessage::User {
                    id: "user".into(),
                    content: "question".into(),
                }],
            })
            .unwrap();
        state
            .apply(Event::ActivitySnapshot {
                message_id: "activity".into(),
                activity: activity("old"),
                replace: false,
            })
            .unwrap();
        state
            .apply(Event::ActivitySnapshot {
                message_id: "activity".into(),
                activity: activity("new"),
                replace: true,
            })
            .unwrap();
        assert!(
            matches!(state.messages.as_slice(), [AgUiMessage::User { .. }, AgUiMessage::Activity { activity: Activity::Table(TableReference { artifact_id, .. }), .. }] if artifact_id.as_str() == "new")
        );
        state
            .apply(Event::MessagesSnapshot {
                messages: vec![AgUiMessage::User {
                    id: "user".into(),
                    content: "question".into(),
                }],
            })
            .unwrap();
        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn text_delta_requires_exact_assistant_identity_and_appends_unicode() {
        let mut state = Messages::new();
        state
            .apply(Event::MessagesSnapshot {
                messages: vec![AgUiMessage::Assistant {
                    id: "answer".into(),
                    content: "hé".into(),
                }],
            })
            .unwrap();
        state
            .apply(Event::TextMessageStart {
                message_id: "answer".into(),
                role: MessageRole::Assistant,
            })
            .unwrap();
        assert!(
            state
                .apply(Event::TextMessageContent {
                    message_id: "other".into(),
                    delta: "!".into(),
                })
                .is_err()
        );
        state
            .apply(Event::TextMessageContent {
                message_id: "answer".into(),
                delta: "llo".into(),
            })
            .unwrap();
        assert!(
            matches!(&state.messages[0], AgUiMessage::Assistant { content, .. } if content == "héllo")
        );
    }

    #[test]
    fn run_snapshots_keep_earlier_history_and_replace_current_activity() {
        let mut state = Messages::new();
        let earlier = vec![
            user("earlier"),
            AgUiMessage::Assistant {
                id: "earlier-answer".into(),
                content: "saved answer".into(),
            },
        ];
        state
            .apply(Event::MessagesSnapshot {
                messages: earlier.clone(),
            })
            .unwrap();
        state.apply(started()).unwrap();
        state
            .apply(Event::MessagesSnapshot {
                messages: vec![
                    AgUiMessage::User {
                        id: "user".into(),
                        content: "question".into(),
                    },
                    AgUiMessage::Activity {
                        id: "stale".into(),
                        activity: activity("old"),
                    },
                ],
            })
            .unwrap();
        state
            .apply(Event::MessagesSnapshot {
                messages: vec![
                    AgUiMessage::User {
                        id: "user".into(),
                        content: "question".into(),
                    },
                    AgUiMessage::Assistant {
                        id: "answer".into(),
                        content: "done".into(),
                    },
                ],
            })
            .unwrap();
        assert_eq!(&state.messages[..2], earlier.as_slice());
        assert_eq!(state.messages.len(), 4);
        assert!(matches!(state.messages[2], AgUiMessage::User { .. }));
        assert!(matches!(state.messages[3], AgUiMessage::Assistant { .. }));
    }

    #[test]
    fn reconnect_replaces_loaded_run_entries_including_questionless_activity() {
        for (question, tail_only) in [(false, false), (true, false), (false, true), (true, true)] {
            let question_id = "0123456789abcdef0123456789abcdef:7:question";
            let details_id = "0123456789abcdef0123456789abcdef:7:details";
            let answer_id = "0123456789abcdef0123456789abcdef:7:answer";
            let mut state = Messages::new();
            let mut history = vec![user("earlier")];
            if question && !tail_only {
                history.push(user(question_id));
            }
            if !tail_only {
                history.push(AgUiMessage::Activity {
                    id: details_id.into(),
                    activity: activity("old"),
                });
            }
            history.push(AgUiMessage::Assistant {
                id: answer_id.into(),
                content: "partial".into(),
            });
            state
                .apply(Event::MessagesSnapshot { messages: history })
                .unwrap();
            state.apply(started()).unwrap();
            let before = (state.messages.clone(), state.scope);
            state
                .apply(Event::MessagesSnapshot { messages: vec![] })
                .unwrap();
            assert_eq!((state.messages.clone(), state.scope), before);
            if question {
                state
                    .apply(Event::MessagesSnapshot {
                        messages: vec![user(question_id)],
                    })
                    .unwrap();
            }
            state
                .apply(Event::ActivitySnapshot {
                    message_id: details_id.into(),
                    activity: activity("new"),
                    replace: false,
                })
                .unwrap();
            state
                .apply(Event::TextMessageStart {
                    message_id: answer_id.into(),
                    role: MessageRole::Assistant,
                })
                .unwrap();
            state
                .apply(Event::TextMessageContent {
                    message_id: answer_id.into(),
                    delta: "fresh".into(),
                })
                .unwrap();
            assert_eq!(state.messages[0], user("earlier"));
            assert_eq!(state.messages.len(), if question { 4 } else { 3 });
            assert!(
                matches!(state.messages.last(), Some(AgUiMessage::Assistant { content, .. }) if content == "fresh")
            );
        }
    }

    #[test]
    fn rejected_run_events_leave_messages_and_scope_unchanged() {
        let mut state = Messages::new();
        state
            .apply(Event::MessagesSnapshot {
                messages: vec![AgUiMessage::Assistant {
                    id: "earlier".into(),
                    content: "saved".into(),
                }],
            })
            .unwrap();
        state.apply(started()).unwrap();
        let before = (state.messages.clone(), state.scope);
        assert!(
            state
                .apply(Event::MessagesSnapshot {
                    messages: vec![user("earlier")]
                })
                .is_err()
        );
        assert_eq!((state.messages.clone(), state.scope), before);
        assert!(
            state
                .apply(Event::MessagesSnapshot {
                    messages: vec![user("")]
                })
                .is_err()
        );
        assert_eq!((state.messages.clone(), state.scope), before);
        state
            .apply(Event::MessagesSnapshot {
                messages: vec![user("current")],
            })
            .unwrap();
        for event in [
            Event::MessagesSnapshot {
                messages: vec![user("earlier")],
            },
            Event::MessagesSnapshot {
                messages: vec![user("current"), user("current")],
            },
            Event::TextMessageStart {
                message_id: "earlier".into(),
                role: MessageRole::Assistant,
            },
            Event::TextMessageContent {
                message_id: "earlier".into(),
                delta: "changed".into(),
            },
            Event::ActivitySnapshot {
                message_id: "earlier".into(),
                activity: activity("new"),
                replace: false,
            },
        ] {
            let before = (state.messages.clone(), state.scope);
            assert!(state.apply(event).is_err());
            assert_eq!((state.messages.clone(), state.scope), before);
        }
        let mut full = Messages::new();
        full.apply(Event::MessagesSnapshot {
            messages: (0..MAX_HISTORY_MESSAGES)
                .map(|i| user(&format!("old-{i}")))
                .collect(),
        })
        .unwrap();
        full.apply(started()).unwrap();
        let before = (full.messages.clone(), full.scope);
        assert!(
            full.apply(Event::MessagesSnapshot {
                messages: vec![user("new")]
            })
            .is_err()
        );
        assert_eq!((full.messages, full.scope), before);
    }
}
