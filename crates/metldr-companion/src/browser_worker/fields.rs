use super::{Call, engine::Engine};
use crate::{
    Result,
    browser_action::Target,
    browser_action::{Action, Intent, Outcome, Phase, Proposal},
    browser_io::{Client, Notice},
    browser_owner,
    flow::Event,
};
use metldr_browser_tools::valid_handle;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Debug)]
pub(crate) struct PlanRefusal(&'static str);
impl std::fmt::Display for PlanRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for PlanRefusal {}

fn owner_error(error: String) -> Box<dyn std::error::Error + Send + Sync> {
    match error.as_str() {
        "task plan declined" => Box::new(PlanRefusal("task plan declined")),
        "call propose_task_plan first" => Box::new(PlanRefusal("call propose_task_plan first")),
        _ => error.into(),
    }
}

mod navigation;
pub(crate) struct Tools {
    engine: Option<Engine>,
    events: Option<tokio::sync::mpsc::Receiver<Notice>>,
    client: Client,
    current: Target,
    stop: Arc<AtomicBool>,
    observation_generation: u64,
    completed: BTreeMap<String, (Call, Value)>,
    release_unknown: bool,
}

impl Tools {
    pub(crate) fn new(
        client: Client,
        source: Target,
        events: tokio::sync::mpsc::Receiver<Notice>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        Self {
            engine: None,
            events: Some(events),
            client,
            current: source,
            stop,
            observation_generation: 0,
            completed: BTreeMap::new(),
            release_unknown: false,
        }
    }

    pub(crate) async fn dispatch(
        &mut self,
        call_id: &str,
        call: Call,
        stop: &AtomicBool,
        budget: Option<&browser_owner::Budget>,
    ) -> Result<Value> {
        if stop.load(Ordering::Acquire) {
            return Err("browser task stopped".into());
        }
        validate_call(call_id, &call)?;
        if let Some((previous, result)) = self.completed.get(call_id) {
            if previous == &call {
                if matches!(call, Call::Status) {
                    return Ok(self.status());
                }
                return Ok(result.clone());
            }
            return Err("model reused a tool identity with changed arguments".into());
        }
        if self.completed.len() == super::MAX_TOOL_CALLS {
            return Err("browser task step limit reached".into());
        }
        if matches!(&call, Call::Navigate { destination } if crate::model::protected_page(destination))
        {
            let result = serde_json::to_value(Outcome::Prevented {
                reason: "protected browser page is not accessible".into(),
            })?;
            self.completed
                .insert(call_id.to_owned(), (call, result.clone()));
            return Ok(result);
        }
        if let Call::ProposeTaskPlan {
            goal,
            intended_effects,
            scope,
        } = &call
        {
            let (ack, rx) = mpsc::sync_channel(1);
            self.client
                .tx
                .try_send(Event::Browser(browser_owner::Event::Plan {
                    run: self.client.run.clone(),
                    goal: goal.clone(),
                    intended_effects: intended_effects.clone(),
                    scope: scope.clone(),
                    ack,
                }))
                .map_err(|_| "browser owner channel unavailable")?;
            let revision = loop {
                if stop.load(Ordering::Acquire) {
                    return Err("browser task stopped".into());
                }
                match rx.try_recv() {
                    Ok(result) => {
                        break result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                            e.into()
                        })?;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => return Err("plan owner ended".into()),
                    Err(mpsc::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_millis(20)).await
                    }
                }
            };
            let result = json!({"revision":revision,"status":"awaiting user approval"});
            self.completed
                .insert(call_id.to_owned(), (call, result.clone()));
            return Ok(result);
        }
        // Reads run before attachment, so reporting state never attaches the browser.
        if matches!(call, Call::Status | Call::Settings) {
            let result = self.read(&call, stop).await?;
            self.completed
                .insert(call_id.to_owned(), (call, result.clone()));
            return Ok(result);
        }
        self.attach().await?;
        if stop.load(Ordering::Acquire) {
            return Err("browser task stopped".into());
        }
        let handle = match &call {
            Call::Change { handle, .. }
            | Call::ActivateButton { handle }
            | Call::SelectOption { handle, .. }
            | Call::Scroll { handle, .. }
            | Call::SetNativeControl { handle, .. } => Some(handle),
            _ => None,
        };
        if let Some(handle) = handle
            && self
                .engine
                .as_ref()
                .ok_or("browser is not attached")?
                .child_action(handle)
                .await?
        {
            let result = serde_json::to_value(Outcome::Prevented {
                reason: "actions inside frames are not supported yet".into(),
            })?;
            self.completed
                .insert(call_id.to_owned(), (call, result.clone()));
            return Ok(result);
        }
        let result = match &call {
            Call::SetNativeControl { handle, checked } => {
                let prepared = self
                    .engine
                    .as_ref()
                    .ok_or("browser is not attached")?
                    .prepare_control(call_id, handle, *checked)
                    .await?;
                match prepared {
                    super::engine::ControlPreparation::NoOp { state } => {
                        let (ack, rx) = mpsc::sync_channel(1);
                        self.client
                            .tx
                            .try_send(Event::Browser(browser_owner::Event::NoOp {
                                run: self.client.run.clone(),
                                ack,
                                budget: budget.cloned(),
                            }))
                            .map_err(|_| "browser owner channel unavailable")?;
                        loop {
                            if stop.load(Ordering::Acquire) {
                                return Err("browser task stopped".into());
                            }
                            match rx.try_recv() {
                                Ok(result) => {
                                    result.map_err(owner_error)?;
                                    break;
                                }
                                Err(mpsc::TryRecvError::Disconnected) => {
                                    return Err("plan owner ended".into());
                                }
                                Err(mpsc::TryRecvError::Empty) => {
                                    tokio::time::sleep(Duration::from_millis(20)).await
                                }
                            }
                        }
                        json!({"kind":"control_no_op", "state":state, "scope":"local_observation", "untrusted":true})
                    }
                    super::engine::ControlPreparation::Propose { proposal } => {
                        self.perform(proposal, stop, budget).await?
                    }
                }
            }
            Call::Observe => self.observe().await?,
            Call::ObserveWithin { within } => self.observe_within(Some(within.clone())).await?,
            Call::Navigate { destination } => {
                self.navigate(call_id, destination, stop, budget).await?
            }
            Call::Change { handle, value } => {
                self.change(call_id, handle, value, stop, budget).await?
            }
            Call::ActivateButton { handle } => {
                let proposal = self
                    .engine
                    .as_ref()
                    .ok_or("browser is not attached")?
                    .prepare_button(call_id, handle)
                    .await?;
                self.perform(proposal, stop, budget).await?
            }
            Call::SelectOption { handle, option } => {
                let proposal = self
                    .engine
                    .as_ref()
                    .ok_or("browser is not attached")?
                    .prepare_selection(call_id, handle, option)
                    .await?;
                self.perform(proposal, stop, budget).await?
            }
            Call::Scroll {
                handle,
                direction,
                pixels,
            } => {
                let proposal = self
                    .engine
                    .as_ref()
                    .ok_or("browser is not attached")?
                    .prepare_scroll(call_id, handle, *direction, *pixels)
                    .await?;
                self.perform(proposal, stop, budget).await?
            }
            // Read-only: the captured output is returned as data and triggers no effect.
            Call::PageLogs => {
                self.engine
                    .as_ref()
                    .ok_or("browser is not attached")?
                    .page_logs()
                    .await?
            }
            Call::Status | Call::Settings | Call::ProposeTaskPlan { .. } => {
                return Err("read-only browser call reached the attached path".into());
            }
        };
        self.completed
            .insert(call_id.to_owned(), (call, result.clone()));
        Ok(result)
    }

    async fn read(&self, call: &Call, stop: &AtomicBool) -> Result<Value> {
        match call {
            Call::Status => Ok(self.status()),
            Call::Settings => self.settings(stop).await,
            _ => Err("browser call is not a read".into()),
        }
    }

    // The actor remains live while transport cleanup runs after a detach notice.
    fn status(&self) -> Value {
        json!({"attached": self.engine.as_ref().is_some_and(Engine::is_live), "release_confirmed": !self.release_unknown})
    }

    // The owner holds the saved settings, so the read waits for its answer.
    async fn settings(&self, stop: &AtomicBool) -> Result<Value> {
        let (ack, rx) = mpsc::sync_channel(1);
        self.client
            .tx
            .try_send(Event::BrowserSettings {
                run: self.client.run.clone(),
                ack,
            })
            .map_err(|_| "settings owner channel unavailable")?;
        loop {
            if stop.load(Ordering::Acquire) {
                return Err("browser task stopped".into());
            }
            match rx.try_recv() {
                Ok(value) => {
                    if stop.load(Ordering::Acquire) {
                        return Err("browser task stopped".into());
                    }
                    return value.map_err(Into::into);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("settings owner ended".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn attach(&mut self) -> Result<()> {
        if self.engine.is_some() {
            return Ok(());
        }
        match Engine::attach(
            self.client.clone(),
            &self.current,
            self.events.take().ok_or("browser events unavailable")?,
            self.stop.clone(),
        )
        .await
        {
            Ok(engine) => {
                self.engine = Some(engine);
                Ok(())
            }
            Err(failure) => {
                self.release_unknown = !failure.released;
                Err(failure.message.into())
            }
        }
    }

    async fn observe(&mut self) -> Result<Value> {
        self.observe_within(None).await
    }

    async fn observe_within(&mut self, within: Option<String>) -> Result<Value> {
        self.observation_generation = self
            .observation_generation
            .checked_add(1)
            .ok_or("browser observation capacity reached")?;
        self.engine
            .as_ref()
            .ok_or("browser is not attached")?
            .observe(self.observation_generation, within)
            .await
    }

    async fn change(
        &mut self,
        tool_use_id: &str,
        handle: &str,
        value: &str,
        stop: &AtomicBool,
        budget: Option<&browser_owner::Budget>,
    ) -> Result<Value> {
        let proposal = self
            .engine
            .as_ref()
            .ok_or("browser is not attached")?
            .prepare_change(tool_use_id, handle, value)
            .await?;
        self.perform(proposal, stop, budget).await
    }

    async fn perform(
        &self,
        proposal: Proposal,
        stop: &AtomicBool,
        budget: Option<&browser_owner::Budget>,
    ) -> Result<Value> {
        let approved = action(&self.client, Some(stop), |ack| {
            browser_owner::Event::Proposed {
                proposal,
                ack,
                budget: budget.cloned(),
            }
        })
        .await?;
        let settled = match &approved.state {
            Phase::Approved => {
                let dispatched = action(&self.client, Some(stop), |ack| {
                    browser_owner::Event::Dispatch {
                        action: approved.id.clone(),
                        revision: approved.revision,
                        ack,
                    }
                })
                .await?;
                let engine = self.engine.as_ref().ok_or("browser is not attached")?;
                let outcome = match &dispatched.proposal.intent {
                    Intent::Field { .. } => engine.change(&dispatched).await,
                    Intent::ActivateButton { .. } => engine.activate_button(&dispatched).await,
                    Intent::SelectOption { .. } => engine.select_option(&dispatched).await,
                    Intent::Scroll { .. } => engine.scroll(&dispatched).await,
                    Intent::SetNativeControl { .. } => engine.set_native_control(&dispatched).await,
                    Intent::Navigate { .. } => {
                        return Err("navigation requires its own result owner".into());
                    }
                };
                action_observed(&self.client, &dispatched, outcome).await?
            }
            Phase::Settled { .. } => approved,
            _ => return Err("cached browser action is not settled or approved".into()),
        };
        let Phase::Settled { outcome } = settled.state else {
            return Err("browser result not settled".into());
        };
        if matches!(outcome, Outcome::Uncertain { .. }) {
            return Err("browser effect is uncertain; automatic continuation stopped".into());
        }
        Ok(serde_json::to_value(outcome)?)
    }

    pub(crate) async fn close(mut self) -> Result<()> {
        let engine_result = match self.engine.take() {
            Some(engine) => engine.detach().await,
            None => Ok(()),
        };
        engine_result?;
        if self.release_unknown {
            return Err("browser release was not confirmed after failed attachment".into());
        }
        Ok(())
    }
}

fn validate_call(call_id: &str, call: &Call) -> Result<()> {
    if call_id.is_empty() || call_id.len() > 128 || call_id.chars().any(char::is_control) {
        return Err("invalid browser tool id".into());
    }
    match call {
        Call::SetNativeControl { handle, .. } if valid_handle(handle) => Ok(()),
        Call::SetNativeControl { .. } => Err("invalid native control handle".into()),
        Call::Observe => Ok(()),
        Call::ObserveWithin { within } if valid_handle(within) => Ok(()),
        Call::ObserveWithin { .. } => Err("invalid observation scope ref".into()),
        Call::Navigate { destination }
            if !destination.is_empty()
                && destination.len() <= 2048
                && !destination.chars().any(char::is_control) =>
        {
            Ok(())
        }
        Call::Navigate { .. } => Err("invalid browser navigation URL".into()),
        Call::Change { handle, value } if valid_handle(handle) && value.len() <= 1024 => Ok(()),
        Call::Change { .. } => Err("invalid browser field change".into()),
        Call::ActivateButton { handle } if valid_handle(handle) => Ok(()),
        Call::ActivateButton { .. } => Err("invalid browser button handle".into()),
        Call::SelectOption { handle, option }
            if [handle, option].into_iter().all(|id| valid_handle(id)) =>
        {
            Ok(())
        }
        Call::SelectOption { .. } => Err("invalid browser selection handle".into()),
        Call::Scroll { handle, pixels, .. }
            if valid_handle(handle) && (1..=2000).contains(pixels) =>
        {
            Ok(())
        }
        Call::Scroll { .. } => Err("invalid browser scroll request".into()),
        Call::PageLogs => Ok(()),
        Call::Status | Call::Settings | Call::ProposeTaskPlan { .. } => Ok(()),
    }
}

async fn action(
    client: &Client,
    stop: Option<&AtomicBool>,
    event: impl FnOnce(mpsc::SyncSender<std::result::Result<Action, String>>) -> browser_owner::Event,
) -> Result<Action> {
    let (ack, rx) = mpsc::sync_channel(1);
    client
        .tx
        .try_send(Event::Browser(event(ack)))
        .map_err(|_| "browser owner channel unavailable")?;
    loop {
        match rx.try_recv() {
            Ok(value) => return value.map_err(owner_error),
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err("browser approval owner ended".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if stop.is_some_and(|stop| stop.load(Ordering::Acquire)) {
            return Err("browser action cancelled".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn action_observed(client: &Client, action: &Action, outcome: Outcome) -> Result<Action> {
    // Stop must not abandon an effect outcome that still needs durable settlement.
    self::action(client, None, |ack| browser_owner::Event::Observed {
        action: action.id.clone(),
        revision: action.revision,
        outcome,
        ack,
    })
    .await
}

#[cfg(test)]
mod failed_attach_check {
    use super::*;
    use crate::{browser_io::Operation, flow::Event as FlowEvent, model::Page};
    use std::sync::mpsc;

    #[test]
    fn only_plan_refusals_allow_the_run_to_continue() {
        for message in ["task plan declined", "call propose_task_plan first"] {
            let error = owner_error(message.into());
            assert!(error.is::<PlanRefusal>());
            assert_eq!(error.to_string(), message);
        }
        assert!(!owner_error("browser dispatch failed".into()).is::<PlanRefusal>());
    }

    fn source() -> Page {
        Page {
            tab: 7,
            url: "https://example.test/".into(),
            title: "Example".into(),
            document: "document".into(),
            text: "page text".into(),
        }
    }

    #[tokio::test]
    async fn protected_navigation_is_refused_before_attachment() {
        let call = Call::Navigate {
            destination: "chrome://settings/".into(),
        };
        let (tx, rx) = mpsc::sync_channel::<FlowEvent>(8);
        let (_notices, events) = tokio::sync::mpsc::channel(1);
        let mut tools = Tools::new(
            Client {
                run: "run".into(),
                tx,
            },
            Target::from(&source()),
            events,
            Arc::new(AtomicBool::new(false)),
        );
        let result = tools
            .dispatch("tool", call, &AtomicBool::new(false), None)
            .await
            .unwrap();
        assert_eq!(
            result,
            serde_json::json!({"kind":"prevented","reason":"protected browser page is not accessible"})
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn stopped_calls_never_attach_or_return_cached_results() {
        let (tx, rx) = mpsc::sync_channel::<FlowEvent>(8);
        let client = Client {
            run: "stopped".into(),
            tx,
        };
        let (_notices, events) = tokio::sync::mpsc::channel(1);
        let mut tools = Tools::new(
            client,
            Target::from(&source()),
            events,
            Arc::new(AtomicBool::new(false)),
        );
        tools.completed.insert(
            "cached".into(),
            (Call::Observe, serde_json::json!({"cached":true})),
        );
        let stop = AtomicBool::new(true);
        assert!(
            tools
                .dispatch("cached", Call::Observe, &stop, None)
                .await
                .is_err()
        );
        assert!(
            tools
                .dispatch("fresh", Call::Observe, &stop, None)
                .await
                .is_err()
        );
        assert!(rx.try_recv().is_err(), "Stop allowed browser I/O");
    }

    #[tokio::test]
    async fn failed_attach_reports_whether_browser_release_was_confirmed() {
        for (release_succeeds, close_succeeds) in [(true, true), (false, false)] {
            let run = format!("failed-attach-{release_succeeds}");
            let (tx, rx) = mpsc::sync_channel::<FlowEvent>(8);
            let client = Client { run, tx };
            let owner = std::thread::spawn(move || {
                while let Ok(event) = rx.recv() {
                    let FlowEvent::BrowserIo(call) = event else {
                        panic!("unexpected event")
                    };
                    match call.operation {
                        Operation::Attach { .. } => {
                            let _ = call.ack.send(Err("attach failure".into()));
                        }
                        Operation::Detach { .. } => {
                            let result = if release_succeeds {
                                Ok(serde_json::json!({"released":true}))
                            } else {
                                Err("release failure".into())
                            };
                            let _ = call.ack.send(result);
                            break;
                        }
                        _ => panic!("unexpected command"),
                    }
                }
            });
            let (_notice_tx, notice_rx) = tokio::sync::mpsc::channel(1);
            let mut tools = Tools::new(
                client,
                Target::from(&source()),
                notice_rx,
                Arc::new(AtomicBool::new(false)),
            );
            let result = tools
                .dispatch("call-1", Call::Observe, &AtomicBool::new(false), None)
                .await;
            assert!(result.is_err());
            assert_eq!(
                tools.close().await.is_ok(),
                close_succeeds,
                "failed attachment cleanup result"
            );
            owner.join().unwrap();
        }
    }

    #[tokio::test]
    async fn status_reports_the_detached_state_without_attaching() {
        for release_succeeds in [true, false] {
            let run = format!("status-{release_succeeds}");
            let (tx, rx) = mpsc::sync_channel::<FlowEvent>(8);
            let client = Client { run, tx };
            // After the one release, the owner exits; a status that attached would find no owner.
            let owner = std::thread::spawn(move || {
                while let Ok(event) = rx.recv() {
                    let FlowEvent::BrowserIo(call) = event else {
                        panic!("unexpected event")
                    };
                    match call.operation {
                        Operation::Attach { .. } => {
                            let _ = call.ack.send(Err("attach failure".into()));
                        }
                        Operation::Detach { .. } => {
                            let result = if release_succeeds {
                                Ok(serde_json::json!({"released":true}))
                            } else {
                                Err("release failure".into())
                            };
                            let _ = call.ack.send(result);
                            break;
                        }
                        _ => panic!("unexpected command"),
                    }
                }
            });
            let (_notice_tx, notice_rx) = tokio::sync::mpsc::channel(1);
            let mut tools = Tools::new(
                client,
                Target::from(&source()),
                notice_rx,
                Arc::new(AtomicBool::new(false)),
            );
            assert!(
                tools
                    .dispatch("call-1", Call::Observe, &AtomicBool::new(false), None)
                    .await
                    .is_err()
            );
            tools.completed.insert(
                "status-1".into(),
                (
                    Call::Status,
                    serde_json::json!({"attached": true, "release_confirmed": !release_succeeds}),
                ),
            );
            let status = tools
                .dispatch("status-1", Call::Status, &AtomicBool::new(false), None)
                .await
                .expect("status is a local read");
            assert_eq!(
                status,
                serde_json::json!({"attached": false, "release_confirmed": release_succeeds}),
                "status must report the detached state after a failed attach"
            );
            assert_eq!(
                tools
                    .dispatch("status-1", Call::Observe, &AtomicBool::new(false), None)
                    .await
                    .unwrap_err()
                    .to_string(),
                "model reused a tool identity with changed arguments"
            );
            assert_eq!(
                tools.close().await.is_ok(),
                release_succeeds,
                "failed attachment cleanup result"
            );
            owner.join().unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn observed_wait_does_not_starve_its_runtime() {
        let (tx, rx) = mpsc::sync_channel::<FlowEvent>(1);
        let client = Client {
            run: "settle".into(),
            tx,
        };
        let (progress, progressed) = mpsc::channel();
        let owner = std::thread::spawn(move || {
            let Ok(FlowEvent::Browser(browser_owner::Event::Observed { ack, .. })) = rx.recv()
            else {
                panic!("expected an observed event");
            };
            // The fallback lets the blocking baseline fail instead of hanging the check.
            let result = if progressed.recv_timeout(Duration::from_secs(1)).is_ok() {
                "owner saw runtime progress"
            } else {
                "runtime stalled"
            };
            let _ = ack.send(Err(result.into()));
        });
        tokio::spawn(async move {
            let _ = progress.send(());
        });
        let action = Action {
            id: "action-1".into(),
            revision: 3,
            proposal: Proposal {
                run: "settle".into(),
                tool_use_id: "tool-1".into(),
                url: "https://example.test/article".into(),
                binding: crate::browser_action::Binding {
                    tab: 7,
                    document: "document-1".into(),
                    target: "target-1".into(),
                    frame: "frame-1".into(),
                    session: "session-1".into(),
                    unique_context: "unique-1".into(),
                    grant_revision: 1,
                },
                intent: Intent::ActivateButton {
                    button_id: "metldr-button".into(),
                    button_label: "Submit".into(),
                },
            },
            state: Phase::Dispatching,
        };
        let result = action_observed(&client, &action, Outcome::ButtonClickObserved).await;
        owner.join().unwrap();
        assert_eq!(
            result.unwrap_err().to_string(),
            "owner saw runtime progress"
        );
    }
}
