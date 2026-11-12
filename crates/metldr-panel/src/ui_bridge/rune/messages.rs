use std::collections::HashSet;

use crate::{
    ag_ui::{AgUiMessage, Event, MAX_HISTORY_MESSAGES, MessageRole},
    model::FlowError,
};

pub(super) struct Messages {
    pub(super) messages: Vec<AgUiMessage>,
}

impl Messages {
    pub(super) fn new() -> Self {
        Self { messages: vec![] }
    }

    pub(super) fn apply(&mut self, event: Event) -> Result<(), FlowError> {
        match event {
            Event::MessagesSnapshot { messages } => {
                validate(&messages)?;
                self.messages = messages;
            }
            Event::TextMessageStart { message_id, role } => {
                if !crate::ag_ui::valid_id(&message_id) {
                    return Err(protocol("invalid or duplicate text message identity"));
                }
                if let Some(existing) = self
                    .messages
                    .iter()
                    .find(|message| message.id() == message_id)
                {
                    if matches!(
                        (role, existing),
                        (MessageRole::Assistant, AgUiMessage::Assistant { .. })
                            | (MessageRole::User, AgUiMessage::User { .. })
                    ) {
                        return Ok(());
                    }
                    return Err(protocol("text message start conflicts with its snapshot"));
                }
                if self.messages.len() == MAX_HISTORY_MESSAGES {
                    return Err(protocol(
                        "message snapshot exceeds the conversation message bound",
                    ));
                }
                self.messages.push(match role {
                    MessageRole::User => AgUiMessage::User {
                        id: message_id,
                        content: String::new(),
                    },
                    MessageRole::Assistant => AgUiMessage::Assistant {
                        id: message_id,
                        content: String::new(),
                    },
                });
            }
            Event::TextMessageContent { message_id, delta } => {
                let Some(AgUiMessage::Assistant { content, .. }) = self
                    .messages
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
            Event::RunStarted { .. }
            | Event::TextMessageEnd { .. }
            | Event::RunFinished { .. }
            | Event::RunError { .. } => {}
        }
        Ok(())
    }
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
    fn replacement_snapshot_does_not_leave_stale_activity_or_lose_user_message() {
        let mut state = Messages::new();
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
        assert_eq!(state.messages.len(), 2);
        assert!(matches!(state.messages[0], AgUiMessage::User { .. }));
        assert!(matches!(state.messages[1], AgUiMessage::Assistant { .. }));
    }
}
