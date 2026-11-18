use crate::{
    Result,
    browser_action::{Action, Element, Intent, Outcome, Proposal, SecretKind, Target, named_refs},
    browser_io::{Client, Notice, Operation},
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::Instant,
};

mod actor;
mod admission;
mod events;
mod network;
mod page;
mod runtime;

const TIMEOUT: Duration = Duration::from_secs(8);
type Reply = oneshot::Sender<runtime::Result<Value>>;
struct Job {
    reply: Reply,
    deadline: Instant,
    admission: Option<Arc<admission::Admission>>,
}
struct Request {
    kind: Kind,
    job: Job,
}
enum Kind {
    /// A command from Browse's page layer; see `page::route`.
    Page {
        method: String,
        session: Option<String>,
        params: Value,
    },
    PrepareAct {
        tool: String,
        act: String,
        args: Value,
        elements: Vec<Element>,
    },
    /// Opens the act window of an approved step, so the extension passes its trusted input.
    ActBegin(Action),
    /// Closes the act window of the action with this id.
    ActEnd(String),
    PrepareSecret {
        tool: String,
        element: Element,
        kind: SecretKind,
    },
    /// Types the consented secret into the focused field; the owner adds the secret.
    EnterSecret(Action),
    PrepareNavigation {
        tool: String,
        destination: String,
    },
    Navigate(Action),
    PageLogs,
    NetworkRequests,
}

pub(super) struct AttachFailure {
    pub(super) message: String,
    pub(super) released: bool,
}
pub(super) struct Engine {
    requests: mpsc::Sender<Request>,
    stop: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    task: JoinHandle<()>,
    client: Client,
    tab: i32,
    page: page::Page,
}

impl Engine {
    pub(super) fn is_live(&self) -> bool {
        !self.task.is_finished() && !self.requests.is_closed()
    }

    pub(super) async fn attach(
        client: Client,
        source: &Target,
        events: mpsc::Receiver<Notice>,
        stop: Arc<AtomicBool>,
    ) -> std::result::Result<Self, AttachFailure> {
        if crate::model::protected_page(&source.url) {
            return Err(AttachFailure {
                message: "protected browser page is not accessible".into(),
                released: true,
            });
        }
        let deadline = Instant::now() + TIMEOUT;
        let setup = async {
            if stop.load(Ordering::Acquire) {
                return Err("browser task stopped".into());
            }
            let attached = client
                .call(Operation::Attach {
                    tab: source.tab,
                    document: source.document.clone(),
                    url: source.url.clone(),
                })
                .await?;
            if attached["grant_revision"] != 1 {
                return Err("invalid browser grant revision".into());
            }
            let target = &attached["targetInfo"];
            let target_id = target["targetId"]
                .as_str()
                .ok_or("invalid browser target")?;
            if target["url"].as_str() != Some(source.url.as_str()) || target["type"] != "page" {
                return Err("browser target changed during attachment".into());
            }
            let transport = runtime::Transport::new();
            let contexts = runtime::ContextMap::new(target_id)?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((transport, contexts))
        };
        let (transport, contexts) = match tokio::time::timeout_at(deadline, setup).await {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                return Err(release_error(
                    &client,
                    source.tab,
                    format!("browser initialization failed: {error}"),
                )
                .await);
            }
            Err(_) => {
                return Err(release_error(
                    &client,
                    source.tab,
                    "browser initialization deadline expired".into(),
                )
                .await);
            }
        };
        let (requests, rx) = mpsc::channel(64);
        let (page_events, _) = tokio::sync::broadcast::channel(256);
        let page = page::Page::new(source.tab, requests.clone(), page_events.clone());
        let (ready, readiness) = oneshot::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut task = tokio::spawn(
            actor::Actor::new(
                client.clone(),
                source.clone(),
                transport,
                contexts,
                events,
                rx,
                stop.clone(),
                shutdown.clone(),
                ready,
                deadline,
                page_events,
            )
            .run(),
        );
        match tokio::time::timeout_at(deadline, readiness).await {
            Ok(Ok(Ok(()))) => Ok(Self {
                requests,
                stop,
                shutdown,
                task,
                client,
                tab: source.tab,
                page,
            }),
            _ => {
                shutdown.store(true, Ordering::Release);
                task.abort();
                let _ = (&mut task).await;
                Err(release_error(&client, source.tab, "browser readiness failed".into()).await)
            }
        }
    }

    async fn request<T: DeserializeOwned>(&self, kind: Kind) -> Result<T> {
        self.request_with(kind, None).await
    }
    async fn request_with<T: DeserializeOwned>(
        &self,
        kind: Kind,
        admission: Option<Arc<admission::Admission>>,
    ) -> Result<T> {
        if self.stop.load(Ordering::Acquire) || self.shutdown.load(Ordering::Acquire) {
            return Err("browser task stopped".into());
        }
        let deadline = Instant::now() + TIMEOUT;
        let (reply, rx) = oneshot::channel();
        self.requests
            .try_send(Request {
                kind,
                job: Job {
                    reply,
                    deadline,
                    admission,
                },
            })
            .map_err(|_| "browser actor is unavailable or full")?;
        let value = tokio::time::timeout_at(deadline, rx)
            .await
            .map_err(|_| "browser operation deadline expired")?
            .map_err(|_| "browser actor ended")?
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        Ok(serde_json::from_value(value)?)
    }
    pub(super) async fn snapshot(&self, args: Value) -> Result<Value> {
        self.page.snapshot(args).await
    }
    pub(super) async fn page_logs(&self) -> Result<Value> {
        self.request(Kind::PageLogs).await
    }
    pub(super) async fn network_requests(&self) -> Result<Value> {
        self.request(Kind::NetworkRequests).await
    }
    /// The elements `args` names, as the latest snapshot showed them.
    async fn elements(&self, refs: &[&str]) -> Result<Vec<Element>> {
        let known = self.page.refs().await?;
        refs.iter()
            .map(|reference| {
                let (role, name) = known.get(*reference).ok_or_else(|| {
                    super::Refusal(format!(
                        "ref {reference} is not in the latest snapshot; take a new snapshot"
                    ))
                })?;
                Ok(Element {
                    reference: (*reference).into(),
                    role: bound(role, 64),
                    name: bound(name, 256),
                })
            })
            .collect()
    }
    pub(super) async fn prepare_act(
        &self,
        tool: &str,
        act: &str,
        args: &Value,
    ) -> Result<Proposal> {
        let elements = self.elements(&named_refs(args)).await?;
        self.request(Kind::PrepareAct {
            tool: tool.into(),
            act: act.into(),
            args: args.clone(),
            elements,
        })
        .await
    }
    async fn effect(&self, kind: Kind, lost_reason: &str) -> Outcome {
        let guard = admission::CancelOnDrop::default();
        match self.request_with(kind, Some(guard.0.clone())).await {
            Ok(outcome) => outcome,
            Err(error) if guard.0.cancel() => Outcome::Prevented {
                reason: bound(&format!("browser action was not dispatched: {error}"), 200),
            },
            Err(_) => Outcome::Uncertain {
                reason: lost_reason.into(),
            },
        }
    }
    /// Opens the step's act window, or the outcome that ends the step before any input.
    async fn open_window(&self, action: &Action) -> Option<Outcome> {
        match self
            .effect(
                Kind::ActBegin(action.clone()),
                "the act window was not confirmed",
            )
            .await
        {
            Outcome::Acted { .. } => None,
            outcome => Some(outcome),
        }
    }
    async fn close_window(&self, action: &Action) {
        let _ = self.request::<Value>(Kind::ActEnd(action.id.clone())).await;
    }
    /// Runs an approved page action with the browser's trusted input.
    pub(super) async fn act(&self, action: &Action) -> Outcome {
        let Intent::Act { tool, args, .. } = &action.proposal.intent else {
            return Outcome::Prevented {
                reason: "not a page action".into(),
            };
        };
        if let Some(outcome) = self.open_window(action).await {
            return outcome;
        }
        let result = self.page.tool(tool, args.clone()).await;
        self.close_window(action).await;
        match result {
            Ok(result) => Outcome::Acted {
                result: bound_lines(&result, 4096),
            },
            Err(error) => Outcome::Prevented {
                reason: bound(&error.to_string(), 200),
            },
        }
    }
    pub(super) async fn prepare_secret(
        &self,
        tool: &str,
        reference: &str,
        kind: SecretKind,
    ) -> Result<Proposal> {
        let element = self
            .elements(&[reference])
            .await?
            .pop()
            .ok_or("secret field ref is missing")?;
        self.request(Kind::PrepareSecret {
            tool: tool.into(),
            element,
            kind,
        })
        .await
    }
    pub(super) async fn enter_secret(&self, action: &Action) -> Outcome {
        let Intent::Secret { element, .. } = &action.proposal.intent else {
            return Outcome::Prevented {
                reason: "not a secret entry".into(),
            };
        };
        if let Some(outcome) = self.open_window(action).await {
            return outcome;
        }
        let outcome = match self.page.focus_secret_field(&element.reference).await {
            Ok(()) => {
                self.effect(
                    Kind::EnterSecret(action.clone()),
                    "secret entry result was lost",
                )
                .await
            }
            Err(error) => Outcome::Prevented {
                reason: bound(&error.to_string(), 200),
            },
        };
        self.close_window(action).await;
        outcome
    }
    pub(super) async fn prepare_navigation(
        &self,
        tool: &str,
        destination: &str,
    ) -> Result<Proposal> {
        self.request(Kind::PrepareNavigation {
            tool: tool.into(),
            destination: destination.into(),
        })
        .await
    }
    pub(super) async fn navigate(&self, action: &Action) -> Outcome {
        self.effect(
            Kind::Navigate(action.clone()),
            "navigation destination or effect could not be verified",
        )
        .await
    }
    pub(super) async fn detach(mut self) -> Result<()> {
        self.shutdown.store(true, Ordering::Release);
        let task: Result<()> = match tokio::time::timeout(TIMEOUT, &mut self.task).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) if e.is_cancelled() => Ok(()),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => {
                self.task.abort();
                let _ = (&mut self.task).await;
                Err("browser actor cleanup deadline expired".into())
            }
        };
        let task = task.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("browser actor cleanup failed: {e}").into()
        });
        let detached = self
            .client
            .call(Operation::Detach { tab: self.tab })
            .await
            .map(|_| ())
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("browser detach was not confirmed: {e}").into()
            });
        match (task, detached) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(e), Ok(())) | (Ok(()), Err(e)) => Err(e),
            (Err(a), Err(b)) => Err(format!("{a}; browser detach failed: {b}").into()),
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.task.abort();
    }
}
async fn release_error(client: &Client, tab: i32, message: String) -> AttachFailure {
    match client.call(Operation::Detach { tab }).await {
        Ok(_) => AttachFailure {
            message,
            released: true,
        },
        Err(e) => AttachFailure {
            message: format!("{message}; browser detach failed: {e}"),
            released: false,
        },
    }
}

/// `text` cut to at most `max` bytes at a character boundary, one line, for a reason.
fn bound(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    bound_lines(&flat, max)
}

/// `text` cut to at most `max` bytes at a character boundary; newlines stay.
fn bound_lines(text: &str, max: usize) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    let mut end = clean.len().min(max);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean[..end].to_owned()
}
