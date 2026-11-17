use super::{CAPACITY, CallId, MAX_MESSAGE, RawEvent, Result, TIMEOUT, WireCommand};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use tokio::time::{Instant, sleep_until};

/// The caller owns context selection, approvals and actual Chrome dispatch.
/// This transport orders each admitted command against the notices and replies around it.
pub struct Transport {
    pending: HashMap<CallId, Pending>,
    commands: VecDeque<WireCommand>,
    incoming: VecDeque<Incoming>,
    undecoded_events: usize,
    admitted: u64,
    ended: bool,
}

pub enum Output {
    Dispatch(Dispatch),
    Event(RawEvent),
    Reply { id: CallId, result: Result<Value> },
}

pub struct Dispatch {
    pub id: CallId,
    pub method: String,
    pub session: Option<String>,
    pub params: Value,
}

enum Incoming {
    Event(RawEvent),
    Reply(CallId, Result<Value>),
}

struct Pending {
    command: WireCommand,
    deadline: Instant,
    state: State,
}

#[derive(PartialEq)]
enum State {
    Queued,
    Offered,
    Dispatched,
    Replied,
}

impl Transport {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            commands: VecDeque::new(),
            incoming: VecDeque::new(),
            undecoded_events: 0,
            admitted: 0,
            ended: false,
        }
    }

    pub fn submit(&mut self, method: String, session: Option<String>, params: Value) -> Result<CallId> {
        self.live()?;
        if self.pending.len() >= CAPACITY || self.admitted == 1_000_000 {
            return Err("browser command capacity reached".into());
        }
        let command = WireCommand {
            id: CallId(self.admitted),
            method,
            session,
            params,
        };
        command.validate()?;
        if command.method.len() + command.session.as_ref().map_or(0, String::len) + json_len(&command.params)? > MAX_MESSAGE {
            return Err("browser command exceeds message limit".into());
        }
        let id = command.id;
        self.pending.insert(
            id,
            Pending {
                command: command.clone(),
                deadline: Instant::now() + TIMEOUT,
                state: State::Queued,
            },
        );
        self.commands.push_back(command);
        self.admitted += 1;
        Ok(id)
    }

    /// Queue notices before polling dispatch, in the order received from Chrome.
    pub fn event(&mut self, event: RawEvent) -> Result<()> {
        self.live()?;
        if event.validate().is_err() {
            return self.fail("invalid browser event");
        }
        if self.undecoded_events == CAPACITY {
            return self.fail("browser event decode capacity reached");
        }
        if event.method.len() + json_len(&event.params)? > MAX_MESSAGE {
            return self.fail("browser message exceeds limit");
        }
        self.incoming.push_back(Incoming::Event(event));
        self.undecoded_events += 1;
        Ok(())
    }

    pub fn dispatch(&mut self, command: &Dispatch) -> Result<()> {
        self.live()?;
        let Some(pending) = self.pending.get(&command.id) else {
            return self.fail("unknown browser dispatch");
        };
        if pending.deadline <= Instant::now() {
            return self.fail("browser dispatch deadline expired");
        }
        if self.undecoded_events != 0 {
            return Err("browser notices must be decoded before dispatch".into());
        }
        if pending.state != State::Offered
            || pending.command.method != command.method
            || pending.command.session != command.session
            || pending.command.params != command.params
        {
            return self.fail("browser dispatch does not match admitted request");
        }
        self.pending.get_mut(&command.id).unwrap().state = State::Dispatched;
        Ok(())
    }

    pub fn reject(&mut self, id: CallId) -> Result<()> {
        self.live()?;
        let Some(pending) = self.pending.get(&id) else {
            return self.fail("unknown rejected browser request");
        };
        if pending.state != State::Offered || pending.deadline <= Instant::now() {
            return self.fail("rejected browser request is stale or already dispatched");
        }
        self.answer(id, Err("browser command failed".into()));
        Ok(())
    }

    pub fn reply(&mut self, id: CallId, result: Result<Value>) -> Result<()> {
        self.live()?;
        let Some(pending) = self.pending.get(&id) else {
            return self.fail("unknown browser reply");
        };
        if pending.deadline <= Instant::now() {
            return self.fail("browser reply deadline expired");
        }
        if pending.state != State::Dispatched {
            return self.fail("browser reply was not dispatched or was already supplied");
        }
        let result = match result {
            Ok(value) if value.is_object() => {
                if json_len(&value)? > MAX_MESSAGE {
                    return self.fail("browser message exceeds limit");
                }
                Ok(value)
            }
            Ok(_) => return self.fail("browser result must be an object"),
            Err(_) => Err("browser command failed".into()),
        };
        self.answer(id, result);
        Ok(())
    }

    pub fn events_pending(&self) -> bool {
        self.undecoded_events != 0
    }

    pub async fn next(&mut self) -> Result<Output> {
        self.live()?;
        let deadline = self.pending.values().map(|p| p.deadline).min();
        if deadline.is_some_and(|d| d <= Instant::now()) {
            return self.fail("browser command deadline expired");
        }
        if self.undecoded_events == 0
            && let Some(command) = self.commands.pop_front()
        {
            let pending = self
                .pending
                .get_mut(&command.id)
                .ok_or("unknown browser command")?;
            if pending.state != State::Queued {
                return self.fail("browser command does not match admitted request");
            }
            pending.state = State::Offered;
            return Ok(Output::Dispatch(Dispatch {
                id: command.id,
                method: command.method,
                session: command.session,
                params: command.params,
            }));
        }
        match self.incoming.pop_front() {
            Some(Incoming::Event(event)) => {
                self.undecoded_events -= 1;
                Ok(Output::Event(event))
            }
            Some(Incoming::Reply(id, result)) => {
                self.pending.remove(&id);
                Ok(Output::Reply { id, result })
            }
            // Nothing is ready: an idle transport waits for the caller; a pending command fails at its deadline.
            None => match deadline {
                Some(deadline) => {
                    sleep_until(deadline).await;
                    self.fail("browser command deadline expired")
                }
                None => std::future::pending().await,
            },
        }
    }

    pub fn close(mut self) {
        self.ended = true;
        self.pending.clear();
        self.commands.clear();
        self.incoming.clear();
    }

    fn answer(&mut self, id: CallId, result: Result<Value>) {
        self.pending.get_mut(&id).unwrap().state = State::Replied;
        self.incoming.push_back(Incoming::Reply(id, result));
    }

    fn live(&self) -> Result<()> {
        if self.ended {
            Err("browser transport ended".into())
        } else {
            Ok(())
        }
    }

    fn fail<T>(&mut self, message: &str) -> Result<T> {
        self.ended = true;
        Err(message.into())
    }
}

fn json_len(value: &Value) -> Result<usize> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_err(|_| "invalid browser message")?;
    Ok(count.0)
}

#[cfg(test)]
mod checks {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn normal_connection_preserves_events_before_dispatch_and_rejects_duplicate_reply() {
        let mut transport = Transport::new();
        let id = transport
            .submit(
                "Runtime.enable".into(),
                Some("child-session".into()),
                json!({}),
            )
            .unwrap();
        let event = RawEvent {
            method: "Runtime.bindingCalled".into(),
            session: Some("child-session".into()),
            params: json!({"futureEnum":"unknown","payload":{"text":"kept"}}),
        };
        transport.event(event.clone()).unwrap();
        match transport.next().await.unwrap() {
            Output::Event(actual) => assert_eq!(actual, event),
            _ => panic!("dispatch overtook a queued lifecycle notice"),
        }
        match transport.next().await.unwrap() {
            Output::Dispatch(command) => {
                assert_eq!(command.id, id);
                assert_eq!(command.session.as_deref(), Some("child-session"));
                assert_eq!(command.method, "Runtime.enable");
                transport.dispatch(&command).unwrap();
            }
            _ => panic!("admitted command did not reach dispatch"),
        }
        transport.reply(id, Ok(json!({}))).unwrap();
        match transport.next().await.unwrap() {
            Output::Reply { id: reply, result } => {
                assert_eq!(reply, id);
                assert_eq!(result.unwrap(), json!({}));
            }
            _ => panic!("the reply did not come back"),
        }
        assert!(transport.reply(id, Ok(json!({}))).is_err());
        transport.close();
    }
}

#[cfg(test)]
mod deadline_check {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn late_reply_ends_the_connection_without_another_poll() {
        let mut transport = Transport::new();
        let id = transport
            .submit("Page.enable".into(), None, json!({}))
            .unwrap();
        let Output::Dispatch(command) = transport.next().await.unwrap() else {
            panic!("command was not offered")
        };
        transport.dispatch(&command).unwrap();
        // Advance the paused test clock past the reply deadline instead of sleeping through it.
        tokio::time::pause();
        tokio::time::advance(TIMEOUT + std::time::Duration::from_millis(10)).await;
        assert_eq!(
            transport.reply(id, Ok(json!({}))).unwrap_err(),
            "browser reply deadline expired"
        );
        assert!(
            transport
                .submit("Page.enable".into(), None, json!({}))
                .is_err()
        );
        transport.close();
    }
}
