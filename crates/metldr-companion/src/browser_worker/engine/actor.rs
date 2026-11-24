use super::{
    Job, Kind, Operation, Request, Result, TIMEOUT, Target,
    events::{PageLogs, Subscriptions},
    runtime::{self, ContextMap, FrameContext, Transport},
};
use crate::{
    browser_action::{Action, Binding, Intent, Outcome, Phase, Proposal},
    browser_io::{Client, Notice},
};
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use runtime::CallId;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::Instant,
};

mod documents;
mod initialization;
mod navigation;
mod requests;
mod results;

struct Initialization {
    session: Option<String>,
    page: bool,
    step: usize,
    deadline: Instant,
    /// Page setup commands still unanswered. They are sent together because the
    /// browser runs one session's commands in arrival order.
    remaining: Option<Arc<std::sync::atomic::AtomicUsize>>,
}
struct Pending {
    work: Work,
    context: Option<FrameContext>,
    deadline: Instant,
    prevented: Option<String>,
}
enum Work {
    Init(Initialization),
    CreateWorld,
    /// The consented secret typed into the focused field; the owner adds the secret.
    EnterSecret {
        action: Action,
        job: Job,
    },
    Navigate {
        action: Action,
        old: FrameContext,
        job: Job,
    },
    FrameTree(Navigation),
    /// A command from Browse's page layer; it answers its caller and leaves the actor free.
    Page(Job),
}
struct Navigation {
    action: Action,
    old: FrameContext,
    frame: String,
    loader: String,
    job: Job,
}
enum Completion {
    Command(CallId),
    Confirmation(Navigation, FrameContext, String),
    /// The extension opened or closed an act window.
    Window(Job),
}

pub(super) struct Actor {
    client: Client,
    source: Target,
    grant_revision: u64,
    granted_root: Option<FrameContext>,
    navigation_granted: bool,
    transport: Transport,
    contexts: ContextMap,
    events: mpsc::Receiver<Notice>,
    requests: mpsc::Receiver<Request>,
    stop: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    ready: Option<oneshot::Sender<runtime::Result<()>>>,
    init_deadline: Instant,
    pending: HashMap<CallId, Pending>,
    offers: VecDeque<runtime::Dispatch>,
    inflight: FuturesUnordered<BoxFuture<'static, (Completion, runtime::Result<Value>)>>,
    /// The approved step whose act window is open, and when it closes at the latest.
    acting: Option<(String, Instant)>,
    busy: bool,
    seen: HashSet<(String, u64)>,
    navigation: Option<Navigation>,
    documents: documents::Documents,
    subscriptions: Subscriptions,
    page_logs: PageLogs,
    network: super::network::NetworkLog,
    page_events: broadcast::Sender<browse_core::CdpEvent>,
}
impl Actor {
    pub(super) fn new(
        client: Client,
        source: Target,
        transport: Transport,
        contexts: ContextMap,
        events: mpsc::Receiver<Notice>,
        requests: mpsc::Receiver<Request>,
        stop: Arc<AtomicBool>,
        shutdown: Arc<AtomicBool>,
        ready: oneshot::Sender<runtime::Result<()>>,
        deadline: Instant,
        page_events: broadcast::Sender<browse_core::CdpEvent>,
    ) -> Self {
        let page_logs = PageLogs::default();
        let mut subscriptions = Subscriptions::default();
        page_logs.register(&mut subscriptions);
        let network = super::network::NetworkLog::default();
        network.register(&mut subscriptions);
        Self {
            client,
            source,
            grant_revision: 1,
            granted_root: None,
            navigation_granted: false,
            transport,
            contexts,
            events,
            requests,
            stop,
            shutdown,
            ready: Some(ready),
            init_deadline: deadline,
            pending: HashMap::new(),
            offers: VecDeque::new(),
            inflight: FuturesUnordered::new(),
            acting: None,
            busy: false,
            seen: HashSet::new(),
            navigation: None,
            documents: documents::Documents::default(),
            subscriptions,
            page_logs,
            network,
            page_events,
        }
    }
    /// Requests own execution outcomes, so the drive result ends here with the transport.
    pub(super) async fn run(mut self) {
        let _ = self.drive().await;
        self.transport.close();
    }
    async fn drive(&mut self) -> Result<()> {
        self.initialize(None, true)?;
        loop {
            if self.stop.load(Ordering::Acquire) || self.shutdown.load(Ordering::Acquire) {
                return Ok(());
            }
            loop {
                match self.events.try_recv() {
                    Ok(notice) => self.notice(notice)?,
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        return Err("browser event channel ended".into());
                    }
                }
            }
            let now = Instant::now();
            if self.ready.is_some() && now >= self.init_deadline
                || self.pending.values().any(|p| now >= p.deadline)
                || self
                    .navigation
                    .as_ref()
                    .is_some_and(|n| now >= n.job.deadline)
            {
                return Err("browser actor deadline expired".into());
            }
            if !self.transport.events_pending() {
                if !self
                    .pending
                    .values()
                    .any(|p| matches!(p.work, Work::Init(_) | Work::CreateWorld))
                {
                    if self.ready.is_some() && self.contexts.is_ready() {
                        self.granted_root = self.contexts.main_context();
                        let _ = self.ready.take().unwrap().send(Ok(()));
                    }
                    self.advance_navigation()?;
                }
                if let Some(command) = self.offers.pop_front() {
                    self.dispatch(command)?;
                }
            }
            tokio::select! {
                biased;
                notice=self.events.recv()=>self.notice(notice.ok_or("browser event channel ended")?)?,
                output=self.transport.next()=>match output? {
                    runtime::Output::Event(event)=>self.transport_event(event)?,
                    runtime::Output::Dispatch(command)=>{
                        if self.offers.len()==64 {return Err("browser offered command limit".into())}
                        self.offers.push_back(command);
                    }
                    runtime::Output::Reply{id,result}=>self.result(id,result)?,
                },
                completed=self.inflight.next(),if !self.inflight.is_empty()=>{
                    let (completion,result)=completed.ok_or("browser completion stream ended")?;
                    match completion {
                        Completion::Command(id)=>self.transport.reply(id,result)?,
                        Completion::Confirmation(nav,context,url)=>self.confirmed(nav,context,url,result)?,
                        Completion::Window(job)=>{
                            if result.is_err() {self.acting=None}
                            self.finish(job,result.map_err(Into::into));
                        }
                    }
                }
                request=self.requests.recv(),if self.ready.is_none()=>{
                    let Some(request)=request else{return Ok(())};self.request(request)?;
                }
                _=tokio::time::sleep(std::time::Duration::from_millis(10))=>(),
            }
        }
    }
    fn transport_event(&mut self, event: runtime::RawEvent) -> Result<()> {
        // A subscribed event from an unowned session is dropped before any handler or context state sees it.
        if !self
            .subscriptions
            .offer(&event, |session| self.contexts.owns(session))
        {
            return Ok(());
        }
        self.documents.event(&event);
        if self.page_events.receiver_count() != 0 && self.contexts.owns(event.session.as_deref()) {
            let _ = self.page_events.send(browse_core::CdpEvent {
                method: event.method.clone(),
                params: event.params.clone(),
                session_id: event.session.clone(),
            });
        }
        let change = self.contexts.event(&event)?;
        if event.method == "Target.detachedFromTarget" {
            self.page_logs
                .retain_sessions(|session| self.contexts.owns(session));
        }
        match change {
            runtime::Change::Attached { session, page } => {
                self.initialize(Some(session), page)?
            }
            runtime::Change::CreateWorld { session, frame } if self.world_allowed() => self.queue(
                "Page.createIsolatedWorld",
                session,
                json!({"frameId":frame,"worldName":runtime::AUTOMATION_WORLD,"grantUniveralAccess":false}),
                Work::CreateWorld,
                if self.ready.is_none() && !self.navigation_granted {
                    self.granted_root.clone()
                } else {
                    None
                },
            )?,
            runtime::Change::None | runtime::Change::CreateWorld { .. } => (),
        }
        Ok(())
    }
    fn notice(&mut self, notice: Notice) -> Result<()> {
        if notice.method == "metldr.detached" {
            return Err("browser attachment ended".into());
        }
        self.transport.event(runtime::RawEvent {
            method: notice.method,
            session: notice.session,
            params: notice.params,
        })?;
        Ok(())
    }
    fn world_allowed(&self) -> bool {
        self.ready.is_some()
            || self.navigation_granted
            || self
                .granted_root
                .as_ref()
                .is_some_and(|root| self.contexts.validate(root).is_ok())
    }
    fn queue(
        &mut self,
        method: impl Into<String>,
        session: Option<String>,
        params: Value,
        work: Work,
        context: Option<FrameContext>,
    ) -> Result<()> {
        let deadline = work
            .job()
            .map(|j| j.deadline)
            .unwrap_or_else(|| match &work {
                Work::Init(init) => init.deadline,
                _ => Instant::now() + TIMEOUT,
            });
        let id = self.transport.submit(method.into(), session, params)?;
        self.pending.insert(
            id,
            Pending {
                work,
                context,
                deadline,
                prevented: None,
            },
        );
        Ok(())
    }
    fn dispatch(&mut self, command: runtime::Dispatch) -> Result<()> {
        let pending = self
            .pending
            .get(&command.id)
            .ok_or("unknown actor dispatch")?;
        let guard = (|| -> Result<()> {
            if matches!(pending.work, Work::CreateWorld) && !self.world_allowed() {
                return Err("browser setup grant is stale".into());
            }
            if Instant::now() >= pending.deadline
                || pending.work.job().is_some_and(|j| j.reply.is_closed())
            {
                return Err("browser caller expired before dispatch".into());
            }
            if let Some(context) = &pending.context {
                self.contexts.validate(context)?;
            }
            match &pending.work {
                Work::EnterSecret { action, .. } | Work::Navigate { action, .. } => self
                    .validate_action(
                        action,
                        pending.context.as_ref().ok_or("missing action context")?,
                    )?,
                _ => (),
            }
            Ok(())
        })();
        if let Err(error) = guard {
            self.pending.get_mut(&command.id).unwrap().prevented = Some(error.to_string());
            self.transport.reject(command.id)?;
            return Ok(());
        }
        let deadline = pending.deadline;
        if self.stop.load(Ordering::Acquire) || self.shutdown.load(Ordering::Acquire) {
            return Err("browser task stopped before dispatch".into());
        }
        self.transport.dispatch(&command)?;
        let navigating = matches!(pending.work, Work::Navigate { .. });
        let operation = match &pending.work {
            Work::EnterSecret { action, .. } => Operation::EnterSecret {
                action: action.id.clone(),
                revision: action.revision,
                binding: action.proposal.binding.clone(),
                url: action.proposal.url.clone(),
                session: command.session,
                params: command.params,
            },
            Work::Navigate { action, .. } => {
                let Intent::Navigate { destination } = &action.proposal.intent else {
                    return Err("invalid navigation intent".into());
                };
                Operation::Navigate {
                    action: action.id.clone(),
                    revision: action.revision,
                    binding: action.proposal.binding.clone(),
                    url: action.proposal.url.clone(),
                    destination: destination.clone(),
                    params: command.params,
                }
            }
            _ => Operation::Command {
                tab: self.source.tab,
                session: command.session,
                method: command.method.to_string(),
                params: command.params,
            },
        };
        let reply = match &pending.work {
            Work::EnterSecret { job, .. } | Work::Navigate { job, .. } => job
                .admission
                .as_ref()
                .ok_or("browser action admission absent")?
                .begin(&self.client, operation)?,
            _ => self.client.begin(operation)?,
        };
        if navigating {
            self.navigation_granted = true;
        }
        let id = command.id;
        self.inflight.push(
            async move { (Completion::Command(id), wait_reply(deadline, reply).await) }.boxed(),
        );
        Ok(())
    }
    fn context(&self) -> Result<FrameContext> {
        self.contexts
            .main_context()
            .ok_or_else(|| "main frame context unavailable".into())
    }
    fn binding(&self, context: &FrameContext) -> Binding {
        Binding {
            tab: self.source.tab,
            document: self.source.document.clone(),
            target: context.target_id().into(),
            frame: context.frame_id().into(),
            session: context.session_id().into(),
            unique_context: context.unique_context_id().into(),
            grant_revision: self.grant_revision,
        }
    }
    fn validate_action(&self, action: &Action, context: &FrameContext) -> Result<()> {
        if action.state != Phase::Dispatching
            || action.proposal.run != self.client.run
            || action.proposal.url != self.source.url
            || action.proposal.binding != self.binding(context)
        {
            return Err("browser action does not match the current approved binding".into());
        }
        self.contexts.validate(context)?;
        Ok(())
    }
    fn finish(&mut self, job: Job, result: Result<Value>) {
        self.busy = false;
        self.navigation_granted = false;
        let _ = job.reply.send(result.map_err(|e| e.to_string()));
    }
    fn value(&mut self, job: Job, value: impl Serialize) -> Result<()> {
        let result = serde_json::to_value(value)?;
        self.finish(job, Ok(result));
        Ok(())
    }
}
impl Work {
    fn job(&self) -> Option<&Job> {
        match self {
            Self::EnterSecret { job, .. } | Self::Navigate { job, .. } | Self::Page(job) => {
                Some(job)
            }
            Self::FrameTree(nav) => Some(&nav.job),
            Self::Init(_) | Self::CreateWorld => None,
        }
    }
}
async fn wait_reply(
    deadline: Instant,
    reply: oneshot::Receiver<runtime::Result<Value>>,
) -> runtime::Result<Value> {
    tokio::time::timeout_at(deadline, reply)
        .await
        .map_err(|_| "browser RPC deadline expired".to_owned())?
        .map_err(|_| "browser RPC owner closed".to_owned())?
}
