use super::*;
use metldr_ui::collapsible;
use ::rune::{
    App, AppHandle, AppSpec, Cx, El, ScrollSpec, TextEdit, Tx, col, row,
};
use wasm_bindgen_futures::spawn_local;

mod activities;
mod answers;
mod composer;
mod conversations;
mod corrections;
mod fonts;
mod gateway;
mod ground;
use theme::Root as _;
mod glimm;
mod messages;
mod pi_replies;
mod preferences;
mod replies;
mod secrets;
mod settings;
mod sources;
mod theme;

use activities::Activities;
use messages::Messages;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Sources,
    Models,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Configured,
    Plan,
}

enum Session {
    Connecting,
    Ready(Rc<Core>),
    /// No companion: page questions go through the extension's model gateway.
    Gateway(Rc<gateway::Gateway>),
    Failed,
}

struct Presentation {
    messages: Messages,
    older_cursor: Option<String>,
    connected: Option<Result<Rc<Core>, FlowError>>,
    accepted_draft: Option<String>,
    error: Option<String>,
    followup: Option<answers::Followup>,
}

struct Updates {
    presentation: RefCell<Presentation>,
    dirty: Cell<bool>,
    closed: Cell<bool>,
    wake: ::rune::Waker,
    archive_epoch: Rc<Cell<u64>>,
    titles_dirty: Cell<bool>,
    /// A gateway opened after the companion could not connect, for the next poll to adopt.
    gateway: RefCell<Option<Rc<gateway::Gateway>>>,
}

impl Updates {
    fn new(wake: ::rune::Waker, archive_epoch: Rc<Cell<u64>>) -> Rc<Self> {
        Rc::new(Self {
            presentation: RefCell::new(Presentation {
                messages: Messages::new(),
                older_cursor: None,
                connected: None,
                accepted_draft: None,
                error: None,
                followup: None,
            }),
            dirty: Cell::new(false),
            closed: Cell::new(false),
            wake,
            archive_epoch,
            titles_dirty: Cell::new(false),
            gateway: RefCell::default(),
        })
    }

    fn changed(&self) {
        self.dirty.set(true);
        (self.wake)();
    }

    fn finish(&self, result: Result<(), FlowError>) {
        if !self.closed.get() {
            if let Err(error) = result {
                self.presentation.borrow_mut().error = Some(error.to_string());
            }
            self.changed();
        }
    }

    fn event(&self, event: Event) -> Result<(), FlowError> {
        if self.closed.get() {
            return Err(FlowError::Disconnected);
        }
        let mut presentation = self.presentation.borrow_mut();
        if let Event::RunError { message, .. } = &event {
            presentation.error = Some(message.clone());
        }
        if matches!(event, Event::RunFinished { .. } | Event::RunError { .. }) {
            self.titles_dirty.set(true);
        }
        presentation.messages.apply(event)?;
        self.archive_epoch
            .set(self.archive_epoch.get().wrapping_add(1));
        drop(presentation);
        self.changed();
        Ok(())
    }
}

struct State {
    session: Session,
    updates: Option<Rc<Updates>>,
    observation: Option<Rc<active::Active>>,
    draft: TextEdit,
    draft_version: u64,
    corrections: corrections::Corrections,
    menu: Option<Menu>,
    /// The open tooltip: its trigger's key and its text.
    tooltip: Option<(&'static str, &'static str)>,
    source_engaged: bool,
    selection: String,
    mode: Mode,
    celebration: u32,
    replies: replies::Replies,
    pi_replies: pi_replies::PiReplies,
    settings: settings::Settings,
    sources: sources::Sources,
    preferences: preferences::Preferences,
    settings_open: bool,
    /// Settings sections the user opened; kept while the panel stays open.
    settings_sections: std::collections::HashSet<&'static str>,
    /// The home reveal has drawn its hidden first frame and now eases in.
    welcome_revealed: bool,
    conversations: conversations::Conversations,
    tabs: conversations::Tabs,
    focus_tab_after_titles: Option<String>,
    activities: Activities,
    browser_receipts_open: bool,
    secret: secrets::SecretEntry,
    answers: answers::Answers,
    thread_rect: ::rune::Live<[f32; 4]>,
    proposed_followup: Option<answers::Followup>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            session: Session::Connecting,
            updates: None,
            observation: None,
            draft: TextEdit::default(),
            draft_version: 0,
            corrections: corrections::Corrections::default(),
            menu: None,
            tooltip: None,
            source_engaged: false,
            selection: String::new(),
            mode: Mode::Configured,
            celebration: 0,
            replies: replies::Replies::default(),
            pi_replies: pi_replies::PiReplies::default(),
            settings: settings::Settings::default(),
            sources: sources::Sources::default(),
            preferences: preferences::Preferences::default(),
            settings_open: false,
            settings_sections: Default::default(),
            welcome_revealed: false,
            conversations: conversations::Conversations::default(),
            tabs: conversations::Tabs::default(),
            focus_tab_after_titles: None,
            activities: Activities::default(),
            browser_receipts_open: false,
            secret: secrets::SecretEntry::default(),
            answers: answers::Answers::default(),
            thread_rect: ::rune::Live::new([0.0; 4]),
            proposed_followup: None,
        }
    }
}

impl State {
    fn sound(&self, cue: crate::interaction_audio::Cue) {
        let _ = crate::interaction_audio::rune(cue, self.preferences.snapshot());
    }

    fn preference(&mut self, action: preferences::Action, tx: &mut Tx) {
        if matches!(action, preferences::Action::RetrySetup) {
            if let Some(updates) = &self.updates {
                self.preferences.retry_setup(updates.wake.clone(), tx);
            }
        } else {
            self.sound(match action {
                preferences::Action::Sound(on) => crate::interaction_audio::Cue::Latch(on),
                _ => crate::interaction_audio::Cue::Tick,
            });
            self.preferences.apply(action, tx);
        }
    }
    fn draft_changed(&mut self) {
        self.draft_version = self
            .draft_version
            .checked_add(1)
            .expect("draft version exhausted");
    }

    fn replace_draft(&mut self, text: impl Into<String>) {
        self.draft_changed();
        self.draft.replace_text(text);
    }

    fn clear_draft(&mut self) {
        self.draft_changed();
        self.draft.clear();
    }

    fn open_settings(&mut self, tx: &mut Tx) {
        self.sound(crate::interaction_audio::Cue::Press);
        if self.conversations.is_open() {
            self.conversations.close(tx);
        }
        self.menu = None;
        self.source_engaged = false;
        self.settings_open = true;
        tx.focus("settings-back");
    }

    fn close_settings(&mut self, tx: &mut Tx) {
        self.sound(crate::interaction_audio::Cue::Press);
        self.settings.clear_keys();
        self.settings_open = false;
        tx.focus("settings-open");
    }

    fn open_conversations(&mut self, tx: &mut Tx) {
        let (Some(core), Some(updates)) = (self.core().cloned(), self.updates.clone()) else {
            return;
        };
        self.settings.clear_keys();
        self.sound(crate::interaction_audio::Cue::Press);
        self.settings_open = false;
        self.menu = None;
        self.source_engaged = false;
        self.conversations.open(&core, &updates, tx);
    }

    fn switch_core(&mut self, core: Rc<Core>, previous: &Rc<Updates>) {
        if let Some(old) = self.core() {
            let id = old.scope.conversation.clone();
            self.tabs.save_draft(&id, &self.draft.text);
        }
        previous.closed.set(true);
        if let Some(active) = self.observation.take() {
            active.detach();
        }
        if let Some(old) = self.core() {
            old.page.changed.borrow_mut().take();
        }
        self.answers = answers::Answers::default();
        self.activities = Activities::default();
        self.browser_receipts_open = false;
        self.replies = replies::Replies::default();
        self.pi_replies = pi_replies::PiReplies::default();
        self.corrections = corrections::Corrections::default();
        self.settings = settings::Settings::default();
        self.sources = sources::Sources::default();
        self.conversations = conversations::Conversations::default();
        self.proposed_followup = None;
        self.menu = None;
        self.source_engaged = false;
        self.settings_open = false;
        self.selection.clear();
        self.mode = Mode::Configured;
        self.clear_draft();
        self.session = Session::Connecting;
        let updates = Updates::new(previous.wake.clone(), self.conversations.archive_epoch());
        self.updates = Some(Rc::clone(&updates));
        spawn_local(async move {
            let connected = prepare_core(core, &updates).await;
            if !updates.closed.get() {
                updates.presentation.borrow_mut().connected = Some(connected);
                updates.changed();
            }
        });
    }

    fn add_followup(&mut self, tx: &mut Tx) {
        self.sound(crate::interaction_audio::Cue::Press);
        self.use_followup(true, tx);
    }

    fn dismiss_followup(&mut self, tx: &mut Tx) {
        self.sound(crate::interaction_audio::Cue::Release);
        self.use_followup(false, tx);
    }

    fn use_followup(&mut self, add: bool, tx: &mut Tx) {
        if add && self.draft.preedit.is_some() {
            self.fail(FlowError::InvalidInput(
                "Finish composing your draft first.".into(),
            ));
            return;
        }
        let Some(followup) = self.proposed_followup.take() else {
            return;
        };
        if !followup.is_current(&self.activities) {
            return;
        }
        let result = self
            .core()
            .ok_or(FlowError::Disconnected)
            .and_then(|core| core.ensure_current())
            .and_then(|()| {
                self.answers.consume_followup(&followup).map_err(|_| {
                    FlowError::Host("The answer followup is no longer available.".into())
                })
            });
        match result {
            Ok(()) => {
                if add {
                    let text = if self.draft.text.is_empty() {
                        followup.text
                    } else {
                        format!("{}\n\n{}", self.draft.text, followup.text)
                    };
                    self.replace_draft(text);
                    self.menu = None;
                    tx.focus("rune-composer-field");
                }
            }
            Err(error) => {
                self.proposed_followup = Some(followup);
                self.fail(error);
            }
        }
    }

    fn core(&self) -> Option<&Rc<Core>> {
        match &self.session {
            Session::Ready(core) => Some(core),
            _ => None,
        }
    }

    fn choices(&self) -> Vec<crate::composer::choices::Choice> {
        match self.core() {
            Some(core) => crate::composer::choices::list(Some(&core.page.view.borrow())),
            None => vec![],
        }
    }

    /// The selected choice alone, for drawing; the menu builds the full list only when open.
    fn selected_choice(&self) -> Option<crate::composer::choices::Choice> {
        let core = self.core()?;
        crate::composer::choices::selected(Some(&core.page.view.borrow()), &self.selection)
    }

    fn gateway(&self) -> Option<&Rc<gateway::Gateway>> {
        match &self.session {
            Session::Gateway(gateway) => Some(gateway),
            _ => None,
        }
    }

    fn running(&self) -> bool {
        if let Some(gateway) = self.gateway() {
            return gateway.streaming();
        }
        self.core().is_some_and(|core| {
            matches!(
                *core.owner.borrow(),
                Owner::Starting { .. } | Owner::Running { .. } | Owner::Observing(_)
            ) || core
                .page
                .view
                .borrow()
                .runs
                .iter()
                .any(|run| !run.status.is_terminal())
        })
    }

    fn can_send(&self) -> bool {
        !self.draft.text.trim().is_empty() && self.can_start()
    }

    /// A run can start now: the chat is connected, ready and idle, and the selected route can
    /// take one.
    fn can_start(&self) -> bool {
        if self.gateway().is_some() {
            return !self.running();
        }
        !self.running()
            && self.core().is_some_and(|core| {
                core.page.controls_ready()
                    && !core.page.settings_blocked()
                    && core.page.flow.borrow().is_connected()
            })
            && self.core().is_some_and(|core| {
                crate::composer::choices::selected_enabled(Some(&core.page.view.borrow()), &self.selection)
            })
    }

    fn capture_disabled(&self) -> bool {
        self.core().is_none_or(|core| {
            !core.page.controls_ready()
                || core.page.settings_blocked()
                || !core.page.flow.borrow().is_connected()
        })
    }

    fn fail(&self, error: FlowError) {
        if let Some(updates) = &self.updates {
            updates.finish(Err(error));
        }
    }

    fn decide_browser_action(&self, decision: browser_actions::DecisionKey, tx: &mut Tx) {
        let Some(core) = self.core().cloned() else {
            return;
        };
        let Some(updates) = self.updates.clone() else {
            return;
        };
        match core.decide_from_rune(decision, tx) {
            Ok(work) => {
                self.sound(crate::interaction_audio::Cue::Press);
                spawn_local(async move {
                    updates.finish(work.await);
                });
            }
            Err(error) => self.fail(error),
        }
    }

    fn decide_plan(&mut self, plan: crate::model::TaskPlanView, approve: bool, tx: &mut Tx) {
        let key = plan_key(&plan, approve);
        if !tx.consume_activation(&key) {
            return;
        }
        let Some(core) = self.core().cloned() else {
            return;
        };
        let Some(updates) = self.updates.clone() else {
            return;
        };
        let valid = core.ensure_current().is_ok()
            && core.flow().is_connected()
            && !core.page.recovery.borrow().active()
            && !core.page.settings.borrow().blocked()
            && !core.page.navigating.get()
            && core.page.view.borrow().task_plan.as_ref() == Some(&plan)
            && core.page.view.borrow().active_run() == Some(&plan.run)
            && plan.state == crate::model::PlanState::Proposed;
        if !valid {
            return;
        }
        let mut pending = core.plan_operation.borrow_mut();
        if pending.as_ref() == Some(&(plan.run.clone(), plan.revision)) {
            return;
        }
        *pending = Some((plan.run.clone(), plan.revision));
        drop(pending);
        self.sound(crate::interaction_audio::Cue::Press);
        spawn_local(async move {
            let result = core
                .flow()
                .decide_plan(core.scope.clone(), plan.run.clone(), plan.revision, approve)
                .await
                .and_then(|view| core.accept(view).map(|_| ()));
            finish_plan(&core.plan_operation, &plan.run, plan.revision);
            updates.finish(result);
        });
    }

    fn session_plan(&mut self, plan: crate::model::TaskPlanView, grant: bool, tx: &mut Tx) {
        if !tx.consume_activation(&session_key(&plan, grant)) {
            return;
        }
        let Some(core) = self.core().cloned() else {
            return;
        };
        let Some(updates) = self.updates.clone() else {
            return;
        };
        let valid = core.ensure_current().is_ok()
            && core.flow().is_connected()
            && !core.page.recovery.borrow().active()
            && !core.page.settings.borrow().blocked()
            && !core.page.navigating.get()
            && core.page.view.borrow().task_plan.as_ref() == Some(&plan)
            && core.page.view.borrow().active_run() == Some(&plan.run)
            && plan.state == crate::model::PlanState::Approved;
        if !valid {
            return;
        }
        let mut pending = core.plan_operation.borrow_mut();
        if pending.as_ref() == Some(&(plan.run.clone(), plan.revision)) {
            return;
        }
        *pending = Some((plan.run.clone(), plan.revision));
        drop(pending);
        self.sound(if grant {
            crate::interaction_audio::Cue::Press
        } else {
            crate::interaction_audio::Cue::Release
        });
        spawn_local(async move {
            let result = core
                .flow()
                .set_session_grant(core.scope.clone(), plan.run.clone(), plan.revision, grant)
                .await
                .and_then(|view| core.accept(view).map(|_| ()));
            finish_plan(&core.plan_operation, &plan.run, plan.revision);
            updates.finish(result);
        });
    }

    fn select(&mut self, value: String, _: &mut Tx) {
        let result = (|| {
            let core = self.core().ok_or(FlowError::Disconnected)?;
            core.ensure_current()?;
            if !core.page.controls_ready() || core.page.settings_blocked() {
                return Err(FlowError::HistoryBusy);
            }
            crate::composer::resolve(&core.page.view.borrow(), &value)
                .map_err(FlowError::InvalidInput)?;
            core.page.execution_preference.borrow_mut().remember(&value);
            Ok(())
        })();
        match result {
            Ok(()) => {
                if self.selection != value {
                    self.celebration = self.celebration.wrapping_add(1);
                }
                self.selection = value;
                self.sound(crate::interaction_audio::Cue::Tick);
                self.menu = None;
                self.draft.focused = true;
            }
            Err(error) => self.fail(error),
        }
    }

    fn send(&mut self, _: &mut Tx) {
        if let (Some(gateway), Some(updates)) = (self.gateway().cloned(), self.updates.clone()) {
            if self.can_send() {
                let text = self.draft.text.trim().to_owned();
                self.clear_draft();
                self.sound(crate::interaction_audio::Cue::Pulse);
                spawn_local(async move {
                    let asked = gateway.ask(text, Rc::clone(&updates)).await;
                    updates.finish(asked);
                });
            }
            return;
        }
        if self.can_send() {
            self.start_run(self.draft.text.clone(), false);
        }
    }

    /// Summarizes the saved page through the selected route, leaving the draft as it is.
    fn summarize(&mut self, _: &mut Tx) {
        if self.can_start() {
            self.start_run(SUMMARY_REQUEST.into(), true);
        }
    }

    fn start_run(&mut self, text: String, summary: bool) {
        let Some(core) = self.core().cloned() else {
            return;
        };
        let Some(updates) = self.updates.clone() else {
            return;
        };
        let admitted = (|| {
            let request = UiRunRequest {
                thread: core.scope.conversation.clone(),
                id: identity()?,
                message: identity()?,
                destination: self.selection.clone(),
                text: UserText::try_from(text.as_str())?,
                mode: match self.mode {
                    Mode::Configured => "configured",
                    Mode::Plan => "plan",
                }
                .into(),
                summary,
            };
            let events = Rc::clone(&updates);
            core.run(request, move |event| {
                // A sent draft leaves the composer once its run starts; a summary never took it.
                if !summary && matches!(event, Event::RunStarted { .. }) {
                    events.presentation.borrow_mut().accepted_draft = Some(text.clone());
                }
                events.event(event)
            })
        })();
        match admitted {
            Ok(running) => {
                self.sound(crate::interaction_audio::Cue::Pulse);
                self.conversations.invalidate_archive();
                updates.presentation.borrow_mut().error = None;
                self.menu = None;
                spawn_local(async move {
                    updates.finish(running.await);
                });
            }
            Err(error) => self.fail(error),
        }
    }

    fn capture(&mut self, _: &mut Tx) {
        let Some(core) = self.core() else { return };
        let Some(updates) = self.updates.clone() else {
            return;
        };
        match core.capture() {
            Ok(capturing) => {
                self.sound(crate::interaction_audio::Cue::Press);
                self.menu = None;
                self.draft.focused = true;
                spawn_local(async move {
                    updates.finish(capturing.await);
                });
            }
            Err(error) => self.fail(error),
        }
    }

    fn stop(&mut self, _: &mut Tx) {
        let Some(updates) = self.updates.clone() else {
            return;
        };
        if let Some(gateway) = self.gateway() {
            self.sound(crate::interaction_audio::Cue::Release);
            updates.finish(gateway.stop());
            return;
        }
        if let Some(active) = &self.observation {
            match active.stop() {
                Ok(stopping) => {
                    self.sound(crate::interaction_audio::Cue::Release);
                    spawn_local(async move {
                        updates.finish(stopping.await.map(|_| ()));
                    });
                }
                Err(error) => self.fail(error),
            }
            return;
        }
        let Some(core) = self.core() else { return };
        let ui = match &*core.owner.borrow() {
            Owner::Starting { ui, .. } | Owner::Running { ui, .. } => ui.clone(),
            _ => return,
        };
        match core.stop(&ui) {
            Ok(stopping) => {
                self.sound(crate::interaction_audio::Cue::Release);
                spawn_local(async move {
                    updates.finish(stopping.await.map(|_| ()));
                });
            }
            Err(error) => self.fail(error),
        }
    }

    fn reconnect(&mut self, _: &mut Tx) {
        let Some(core) = self.core() else { return };
        let Some(updates) = self.updates.clone() else {
            return;
        };
        let recovery = (|| {
            core.ensure_current()?;
            if !connection::UiConnectionSnapshot::read(core).can_reconnect() {
                return Err(FlowError::HistoryBusy);
            }
            connection::Recovery::begin(Rc::clone(&core.page))
        })();
        match recovery {
            Ok(recovery) => spawn_local(async move {
                updates.finish(recovery.exact().await.map(|_| ()));
            }),
            Err(error) => self.fail(error),
        }
    }

    fn review(&mut self, _: &mut Tx) {
        let result = (|| {
            let window = web_sys::window().ok_or(())?;
            if let Some(core) = self.core() {
                window
                    .session_storage()
                    .map_err(|_| ())?
                    .ok_or(())?
                    .set_item(
                        &format!("metldr:review-draft:{}", core.scope.conversation),
                        &self.draft.text,
                    )
                    .map_err(|_| ())?;
            }
            window.location().reload().map_err(|_| ())
        })();
        if result.is_err() {
            self.fail(FlowError::Host(
                "Your draft could not be kept. Copy it before reloading.".into(),
            ));
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.preferences.close();
        if let Some(updates) = &self.updates {
            updates.closed.set(true);
        }
        if let Some(active) = &self.observation {
            active.detach();
        }
        if let Some(core) = self.core() {
            core.page.changed.borrow_mut().take();
            let ui = match &*core.owner.borrow() {
                Owner::Starting { ui, .. } | Owner::Running { ui, .. } => Some(ui.clone()),
                _ => None,
            };
            if let Some(ui) = ui
                && let Ok(stopping) = core.stop(&ui)
            {
                spawn_local(async move {
                    let _ = stopping.await;
                });
            }
        }
    }
}

fn identity() -> Result<String, FlowError> {
    let crypto = crate::get(&js_sys::global(), "crypto")
        .map_err(|_| FlowError::Protocol("Request identity is unavailable.".into()))?;
    crate::call(&crypto, "randomUUID", &[])
        .ok()
        .and_then(|value| value.as_string())
        .filter(|value| crate::ag_ui::valid_id(value))
        .ok_or_else(|| FlowError::Protocol("Request identity is unavailable.".into()))
}

async fn prepare_core(core: Rc<Core>, updates: &Rc<Updates>) -> Result<Rc<Core>, FlowError> {
    let history = core.history(None).await?;
    if updates.closed.get() {
        return Err(FlowError::Disconnected);
    }
    {
        let mut presentation = updates.presentation.borrow_mut();
        presentation.messages.replace_archive(history.messages)?;
        presentation.older_cursor = history.older_cursor;
    }
    let weak = Rc::downgrade(updates);
    *core.page.changed.borrow_mut() = Some(Rc::new(move || {
        if let Some(updates) = weak.upgrade() {
            updates.changed();
        }
    }));
    Ok(core)
}

fn setup(state: &mut State, wake: ::rune::Waker) {
    state.preferences.setup(wake.clone());
    let updates = Updates::new(wake, state.conversations.archive_epoch());
    state.updates = Some(Rc::clone(&updates));
    spawn_local(async move {
        let connected = async {
            let flow = FlowClient::connect().await?;
            let view = flow.current_page().await?;
            let core = UiBridge::from_view(flow, view)?.core;
            prepare_core(core, &updates).await
        }
        .await;
        if !updates.closed.get() {
            updates.presentation.borrow_mut().connected = Some(connected);
            updates.changed();
        }
    });
}

fn poll(state: &mut State, _: f64, tx: &mut Tx) -> bool {
    let appearance_changed = state.preferences.poll();
    let Some(updates) = state.updates.clone() else {
        return appearance_changed;
    };
    if !updates.dirty.replace(false) {
        return appearance_changed;
    }
    let connected = updates.presentation.borrow_mut().connected.take();
    if let Some(connected) = connected {
        match connected {
            Ok(core) => {
                let (draft, focus_tab) = state.tabs.arrive(&core.scope.conversation);
                state
                    .conversations
                    .set_older_cursor(updates.presentation.borrow_mut().older_cursor.take());
                let key = format!("metldr:review-draft:{}", core.scope.conversation);
                let restored = web_sys::window()
                    .ok_or(())
                    .and_then(|window| window.session_storage().map_err(|_| ())?.ok_or(()))
                    .and_then(|storage| {
                        let draft = storage.get_item(&key).map_err(|_| ())?;
                        Ok((storage, draft))
                    });
                match restored {
                    Ok((storage, Some(draft))) => {
                        state.replace_draft(draft);
                        if storage.remove_item(&key).is_err() {
                            state.fail(FlowError::Host(
                                "Your draft is shown, but its saved copy could not be cleared."
                                    .into(),
                            ));
                        }
                    }
                    Ok((_, None)) => {}
                    Err(()) => state.fail(FlowError::Host(
                        "Your saved draft could not be opened.".into(),
                    )),
                }
                if let Some(draft) = draft.filter(|draft| !draft.is_empty()) {
                    state.replace_draft(draft);
                }
                state.selection = core
                    .page
                    .execution_preference
                    .borrow()
                    .value()
                    .or_else(|| {
                        crate::composer::choices::list(Some(&core.page.view.borrow()))
                            .into_iter()
                            .find(|choice| choice.enabled)
                            .map(|choice| choice.value.into_owned())
                    })
                    .unwrap_or_default();
                let active = core
                    .page
                    .view
                    .borrow()
                    .runs
                    .iter()
                    .find(|run| !run.status.is_terminal())
                    .map(|run| Rc::new(active::Active::new(Rc::clone(&core), run)));
                state.session = Session::Ready(core);
                if let Some(core) = state.core().cloned() {
                    state.conversations.refresh_titles(&core, &updates);
                }
                if focus_tab {
                    let id = &state.core().unwrap().scope.conversation;
                    state.focus_tab_after_titles = Some(format!("chat-tab-{id}"));
                } else {
                    tx.focus("rune-composer-field");
                }
                if let Some(active) = active {
                    let observing = identity().and_then(|id| {
                        let events = Rc::clone(&updates);
                        active.watch(id, move |event| events.event(event))
                    });
                    match observing {
                        Ok(observing) => {
                            state.observation = Some(active);
                            let completion = Rc::clone(&updates);
                            spawn_local(async move {
                                completion.finish(observing.await);
                            });
                        }
                        Err(error) => state.fail(error),
                    }
                }
            }
            Err(error) => {
                state.session = Session::Failed;
                state.fail(error);
                // Without the companion, the extension's own model gateway can still answer.
                let opening = Rc::clone(&updates);
                spawn_local(async move {
                    if let Some(gateway) = gateway::Gateway::open(&opening).await {
                        *opening.gateway.borrow_mut() = Some(gateway);
                        opening.changed();
                    }
                });
            }
        }
    }
    if let Some(gateway) = updates.gateway.borrow_mut().take() {
        state.session = Session::Gateway(gateway);
        updates.presentation.borrow_mut().error = None;
    }
    if let Some(core) = state.core().cloned() {
        let running = state.running();
        match state.conversations.poll(&core, running) {
            Some(conversations::Completion::Navigation(core)) => {
                state.switch_core(core, &updates);
                return true;
            }
            Some(conversations::Completion::Search) => {
                state.conversations.refresh_titles(&core, &updates);
            }
            Some(conversations::Completion::Archive(page)) => {
                let replaced = updates
                    .presentation
                    .borrow_mut()
                    .messages
                    .replace_archive(page.messages);
                match replaced {
                    Ok(()) => {
                        state.conversations.invalidate_archive();
                        state.conversations.set_older_cursor(page.older_cursor);
                    }
                    Err(error) => state.fail(error),
                }
            }
            None => {}
        }
        state.conversations.update_titles(&mut state.tabs);
        if updates.titles_dirty.get() && !state.conversations.busy && !state.running() {
            updates.titles_dirty.set(false);
            state.conversations.refresh_titles(&core, &updates);
        }
        if !state.conversations.busy
            && let Some(key) = state.focus_tab_after_titles.take()
        {
            tx.focus(key);
        }
    }
    let accepted = updates.presentation.borrow_mut().accepted_draft.take();
    if let Some(accepted) = accepted
        && state.draft.text == accepted
    {
        state.clear_draft();
    }
    if !state.running() {
        state.observation = None;
    }
    if state.selection.is_empty()
        && let Some(choice) = state.choices().into_iter().find(|choice| choice.enabled)
    {
        state.selection = choice.value.into_owned();
    }
    replies::sync(state);
    pi_replies::sync(state);
    corrections::sync(state);
    secrets::sync(state);
    if let Some(core) = state.core().cloned() {
        state.settings.sync(&core);
        state.sources.sync(&core, &updates);
        let presentation = updates.presentation.borrow();
        state
            .activities
            .sync(&core, &presentation.messages.messages);
        if !presentation.messages.messages.is_empty() {
            // A later empty chat replays the home reveal.
            state.welcome_revealed = false;
        }
        if state
            .answers
            .sync(&state.activities, &presentation.messages.messages, &updates)
            .is_err()
        {
            drop(presentation);
            state.fail(FlowError::Host(
                "The interactive answer could not be opened.".into(),
            ));
        }
    } else {
        state.settings.clear_keys();
    }
    let followup = updates.presentation.borrow_mut().followup.take();
    if let Some(followup) = followup {
        state.proposed_followup = Some(followup);
    }
    if state
        .proposed_followup
        .as_ref()
        .is_some_and(|followup| !followup.is_current(&state.activities))
    {
        state.proposed_followup = None;
    }
    true
}

fn followup_controls<S: 'static>(
    typography: preferences::Typography,
    add: fn(&mut S, &mut Tx),
    dismiss: fn(&mut S, &mut Tx),
) -> El {
    let mut actions = row().gap(8.0);
    for (label, action) in [("Add to draft", add), ("Dismiss", dismiss)] {
        actions = actions.child(
            typography
                .text(label)
                .key(label)
                .role("button")
                .focusable(true)
                .native_activation()
                .press(typography.press)
                .press_scale(theme::PRESS_SCALE)
                .on_press_tx(action)
                .on_key(move |state: &mut S, key, tx| composer::enter(state, key, tx, action)),
        );
    }
    actions
}

#[cfg(test)]
mod followup_tests {
    use super::*;
    use ::rune::{Env, Key, KeyEvent, Mods, Runtime};

    fn sample_plan(state: crate::model::PlanState) -> crate::model::TaskPlanView {
        crate::model::TaskPlanView {
            proposer: "Agent".into(),
            run: serde_json::from_value(serde_json::json!("0123456789abcdef0123456789abcdef:1"))
                .unwrap(),
            goal: "Find information".into(),
            intended_effects: vec!["Open a page".into()],
            scope: "One site".into(),
            revision: 7,
            state,
            session_grant: None,
        }
    }

    #[test]
    fn declined_plan_has_no_buttons_and_untrusted_input_is_ignored() {
        fn view(_: &State, _: &mut Cx<State>) -> El {
            plan_view(
                &sample_plan(crate::model::PlanState::Declined),
                true,
                preferences::Typography::new(1.0, &preferences::Palette::for_theme("light")),
                State::decide_plan,
                State::session_plan,
            )
        }
        let mut rt = Runtime::new(State::default(), view, (640.0, 300.0), Env::default());
        rt.frame(0.0);
        let elements = rt.elements();
        assert!(
            elements
                .iter()
                .any(|el| el.role == "group" && el.label.as_deref() == Some("Task plan"))
        );
        assert!(
            elements
                .iter()
                .any(|el| el.text.as_deref() == Some("Declined"))
        );
        assert!(!elements.iter().any(|el| el.role == "button"));
        let plan = sample_plan(crate::model::PlanState::Proposed);
        assert!(!Tx::default().consume_activation(&plan_key(&plan, true)));
        assert_ne!(plan_key(&plan, true), plan_key(&plan, false));
    }

    fn granted_plan() -> crate::model::TaskPlanView {
        let mut plan = sample_plan(crate::model::PlanState::Approved);
        plan.session_grant = Some(crate::model::SessionGrantView {
            remaining_s: 1201,
            bound: true,
        });
        plan
    }

    fn plan_card(plan: &crate::model::TaskPlanView) -> El {
        plan_view(
            plan,
            true,
            preferences::Typography::new(1.0, &preferences::Palette::for_theme("light")),
            State::decide_plan,
            State::session_plan,
        )
    }

    fn card_buttons_and_status(
        view: fn(&State, &mut Cx<State>) -> El,
    ) -> (Vec<String>, Vec<String>) {
        let mut rt = Runtime::new(State::default(), view, (640.0, 400.0), Env::default());
        rt.frame(0.0);
        let elements = rt.elements();
        let buttons = elements
            .iter()
            .filter(|el| el.role == "button")
            .filter_map(|el| el.label.clone())
            .collect();
        let status = elements
            .iter()
            .filter(|el| el.role == "status")
            .filter_map(|el| el.text.clone())
            .collect();
        (buttons, status)
    }

    #[test]
    fn approved_plan_offers_session_approval_and_shows_an_active_grant() {
        fn approved(_: &State, _: &mut Cx<State>) -> El {
            plan_card(&sample_plan(crate::model::PlanState::Approved))
        }
        fn granted(_: &State, _: &mut Cx<State>) -> El {
            plan_card(&granted_plan())
        }
        let (buttons, _) = card_buttons_and_status(approved);
        assert_eq!(buttons, ["Auto-approve safe steps for 30 minutes"]);
        let (buttons, status) = card_buttons_and_status(granted);
        assert_eq!(buttons, ["Extend 30 minutes", "Stop auto-approval"]);
        assert_eq!(status, ["Safe steps are auto-approved for 21 more min"]);
        let plan = granted_plan();
        assert!(!Tx::default().consume_activation(&session_key(&plan, false)));
        assert_eq!(
            crate::flow::session_grant(&plan.run, plan.revision, false),
            serde_json::json!({"op":"revoke_session", "run":plan.run})
        );
    }

    #[derive(Default)]
    struct PlanTest {
        pending: RefCell<Option<(RunId, u32)>>,
        replies: Vec<serde_json::Value>,
    }

    fn test_plan_view(state: &PlanTest, _: &mut Cx<PlanTest>) -> El {
        let plan = sample_plan(crate::model::PlanState::Proposed);
        let enabled = state.pending.borrow().as_ref() != Some(&(plan.run.clone(), plan.revision));
        plan_view(
            &plan,
            enabled,
            preferences::Typography::new(1.0, &preferences::Palette::for_theme("light")),
            |state: &mut PlanTest, plan, approve, _| {
                *state.pending.borrow_mut() = Some((plan.run.clone(), plan.revision));
                state
                    .replies
                    .push(crate::flow::plan_reply(&plan.run, plan.revision, approve));
            },
            |state: &mut PlanTest, plan, grant, _| {
                state
                    .replies
                    .push(crate::flow::session_grant(&plan.run, plan.revision, grant));
            },
        )
    }

    #[test]
    fn enter_approves_the_displayed_revision_once() {
        let mut rt = Runtime::new(
            PlanTest::default(),
            test_plan_view,
            (640.0, 300.0),
            Env::default(),
        );
        rt.set_tab_visits_controls(true);
        rt.frame(0.0);
        rt.key(KeyEvent {
            key: Key::Tab,
            mods: Mods::default(),
        });
        rt.frame(0.0);
        let plan = sample_plan(crate::model::PlanState::Proposed);
        assert_eq!(
            rt.elements()
                .iter()
                .find(|el| el.focused)
                .and_then(|el| el.key.as_deref()),
            Some(plan_key(&plan, true).as_str())
        );
        let enter = KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        };
        rt.key(enter.clone());
        rt.repeat_key(enter);
        rt.frame(0.0);
        assert_eq!(
            rt.state.replies,
            vec![
                serde_json::json!({"op":"plan_reply", "run":plan.run, "revision":7, "approve":true})
            ]
        );
    }

    #[test]
    fn failed_reply_reenables_buttons_without_clearing_newer_work() {
        let plan = sample_plan(crate::model::PlanState::Proposed);
        let state = PlanTest::default();
        *state.pending.borrow_mut() = Some((plan.run.clone(), plan.revision));
        let mut rt = Runtime::new(state, test_plan_view, (640.0, 300.0), Env::default());
        rt.frame(0.0);
        assert!(
            rt.elements()
                .iter()
                .filter(|el| el.role == "button")
                .all(|el| !el.focusable)
        );
        finish_plan(&rt.state.pending, &plan.run, plan.revision + 1);
        assert!(rt.state.pending.borrow().is_some());
        finish_plan(&rt.state.pending, &plan.run, plan.revision);
        let mut rt = Runtime::new(rt.state, test_plan_view, (640.0, 300.0), Env::default());
        rt.frame(0.0);
        let buttons: Vec<_> = rt
            .elements()
            .into_iter()
            .filter(|el| el.role == "button")
            .collect();
        assert_eq!(buttons.len(), 2);
        assert!(buttons.iter().all(|el| el.focusable));
    }

    #[test]
    fn message_roles_and_startup_error_match_the_reference() {
        let updates = Updates::new(std::sync::Arc::new(|| {}), Rc::new(Cell::new(0)));
        {
            let mut presentation = updates.presentation.borrow_mut();
            presentation.error = Some("Connection failed".into());
            presentation
                .messages
                .replace_archive(vec![
                    crate::ag_ui::AgUiMessage::User {
                        id: "user".into(),
                        content: "Question".into(),
                    },
                    crate::ag_ui::AgUiMessage::Assistant {
                        id: "assistant".into(),
                        content: "Answer".into(),
                    },
                ])
                .unwrap();
        }
        let mut state = State::default();
        state.session = Session::Failed;
        state.updates = Some(updates);
        let mut rt = Runtime::new(state, view, (640.0, 600.0), Env::default());
        rt.frame(0.0);
        let elements = rt.elements();
        let user = elements
            .iter()
            .find(|el| el.key.as_deref() == Some("user"))
            .unwrap();
        let assistant = elements
            .iter()
            .find(|el| el.key.as_deref() == Some("assistant"))
            .unwrap();
        assert_eq!((user.role, user.label.as_deref()), ("none", None));
        assert_eq!(
            (assistant.role, assistant.label.as_deref()),
            ("article", None)
        );
        let errors: Vec<_> = elements
            .iter()
            .filter(|el| el.text.as_deref() == Some("Connection failed"))
            .collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].role, "status");
    }

    #[test]
    fn an_empty_chat_centres_and_an_overflowing_one_starts_at_the_top() {
        fn view(tall: &bool, _: &mut Cx<bool>) -> El {
            col().h_full().child(
                col()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .child(
                        col()
                            .key("body")
                            .w_full()
                            .h(if *tall { 300.0 } else { 40.0 })
                            .my_auto(),
                    )
                    .scroll(ScrollSpec { stick_bottom: false }),
            )
        }
        for (tall, top) in [(false, 30.0), (true, 0.0)] {
            let mut rt = Runtime::new(tall, view, (200.0, 100.0), Env::default());
            rt.frame(0.0);
            let body = rt
                .elements()
                .into_iter()
                .find(|el| el.key.as_deref() == Some("body"))
                .unwrap();
            assert_eq!(body.rect[1], top);
        }
    }

    #[test]
    fn enter_activates_each_followup_once_and_ignores_repeat() {
        fn view(_: &[u32; 2], _: &mut Cx<[u32; 2]>) -> El {
            followup_controls(
                preferences::Typography::new(1.0, &preferences::Palette::for_theme("light")),
                |counts: &mut [u32; 2], _| counts[0] += 1,
                |counts: &mut [u32; 2], _| counts[1] += 1,
            )
        }
        let mut rt = Runtime::new([0, 0], view, (640.0, 100.0), Env::default());
        rt.set_tab_visits_controls(true);
        rt.frame(0.0);
        for (label, expected) in [("Add to draft", [1, 0]), ("Dismiss", [1, 1])] {
            rt.key(KeyEvent {
                key: Key::Tab,
                mods: Mods::default(),
            });
            rt.frame(0.0);
            assert_eq!(
                rt.elements()
                    .into_iter()
                    .find(|el| el.focused)
                    .and_then(|el| el.key)
                    .as_deref(),
                Some(label)
            );
            let enter = KeyEvent {
                key: Key::Enter,
                mods: Mods::default(),
            };
            rt.key(enter.clone());
            rt.repeat_key(enter);
            rt.frame(0.0);
            assert_eq!(rt.state, expected);
        }
    }
}

fn key(state: &mut State, key: &::rune::KeyEvent, tx: &mut Tx) -> bool {
    if key.key == ::rune::Key::Escape && state.conversations.is_open() {
        state.conversations.close(tx);
        true
    } else if key.key == ::rune::Key::Escape && state.settings_open {
        state.close_settings(tx);
        true
    } else if key.key == ::rune::Key::Escape && state.menu.is_some() {
        state.menu = None;
        state.source_engaged = false;
        true
    } else {
        false
    }
}

fn plan_key(plan: &crate::model::TaskPlanView, approve: bool) -> String {
    format!(
        "plan:{}:{}:{}",
        plan.run.as_str(),
        plan.revision,
        u8::from(approve)
    )
}

fn session_key(plan: &crate::model::TaskPlanView, grant: bool) -> String {
    format!(
        "plan-session:{}:{}:{}",
        plan.run.as_str(),
        plan.revision,
        u8::from(grant)
    )
}

fn plan_button<S: 'static>(
    typography: preferences::Typography,
    key: String,
    label: &'static str,
    enabled: bool,
    plan: &crate::model::TaskPlanView,
    flag: bool,
    act: fn(&mut S, crate::model::TaskPlanView, bool, &mut Tx),
) -> El {
    let press_plan = plan.clone();
    let key_plan = plan.clone();
    typography
        .text(label)
        .key(key)
        .role("button")
        .label(label)
        .focusable(enabled)
        .disabled(!enabled)
        .native_activation()
        .press(typography.press)
        .press_scale(theme::PRESS_SCALE)
        .on_press_tx(move |state: &mut S, tx| act(state, press_plan.clone(), flag, tx))
        .on_key(move |state: &mut S, key, tx| {
            if key.key != ::rune::Key::Enter {
                return false;
            }
            if !tx.key_repeat() {
                act(state, key_plan.clone(), flag, tx);
            }
            true
        })
}

fn finish_plan(pending: &RefCell<Option<(RunId, u32)>>, run: &RunId, revision: u32) {
    let mut pending = pending.borrow_mut();
    if pending
        .as_ref()
        .is_some_and(|key| key.0 == *run && key.1 == revision)
    {
        *pending = None;
    }
}

fn plan_view<S: 'static>(
    plan: &crate::model::TaskPlanView,
    enabled: bool,
    typography: preferences::Typography,
    decide: fn(&mut S, crate::model::TaskPlanView, bool, &mut Tx),
    session: fn(&mut S, crate::model::TaskPlanView, bool, &mut Tx),
) -> El {
    use crate::model::PlanState;
    let mut card = col()
        .key(format!("task-plan:{}", plan.run.as_str()))
        .role("group")
        .label("Task plan")
        .w_full()
        .gap(8.0)
        .child(typography.text("Task plan").wrap())
        .child(
            typography
                .text(format!("Proposed by {}", plan.proposer))
                .wrap(),
        )
        .child(typography.text(format!("Goal: {}", plan.goal)).wrap());
    for effect in &plan.intended_effects {
        card = card.child(typography.text(format!("Effect: {effect}")).wrap());
    }
    card = card.child(typography.text(format!("Scope: {}", plan.scope)).wrap());
    match plan.state {
        PlanState::Approved => {
            card = card.child(typography.text("Approved"));
            match &plan.session_grant {
                None => card.child(plan_button(
                    typography,
                    session_key(plan, true),
                    "Auto-approve safe steps for 30 minutes",
                    enabled,
                    plan,
                    true,
                    session,
                )),
                Some(grant) => {
                    let minutes = grant.remaining_s.div_ceil(60);
                    let status = if grant.bound {
                        format!("Safe steps are auto-approved for {minutes} more min")
                    } else {
                        format!(
                            "Safe steps are auto-approved for {minutes} more min after you approve the next one"
                        )
                    };
                    card.child(typography.text(status).wrap().role("status"))
                        .child(plan_button(
                            typography,
                            session_key(plan, true),
                            "Extend 30 minutes",
                            enabled,
                            plan,
                            true,
                            session,
                        ))
                        .child(plan_button(
                            typography,
                            session_key(plan, false),
                            "Stop auto-approval",
                            enabled,
                            plan,
                            false,
                            session,
                        ))
                }
            }
        }
        PlanState::Declined => card.child(typography.text("Declined")),
        PlanState::Proposed => {
            for (approve, label) in [(true, "Approve"), (false, "Decline")] {
                let press_plan = plan.clone();
                let key_plan = plan.clone();
                card = card.child(
                    typography
                        .text(label)
                        .key(plan_key(plan, approve))
                        .role("button")
                        .label(label)
                        .focusable(enabled)
                        .disabled(!enabled)
                        .native_activation()
                        .press(typography.press)
                        .press_scale(theme::PRESS_SCALE)
                        .on_press_tx(move |state: &mut S, tx| {
                            decide(state, press_plan.clone(), approve, tx)
                        })
                        .on_key(move |state: &mut S, key, tx| {
                            if key.key != ::rune::Key::Enter {
                                return false;
                            }
                            if !tx.key_repeat() {
                                decide(state, key_plan.clone(), approve, tx);
                            }
                            true
                        }),
                );
            }
            card
        }
    }
}

fn browser_action_view(
    row: &browser_actions::UiBrowserAction,
    typography: preferences::Typography,
) -> El {
    let action = &row.action;
    let display = &row.display;
    let mut card = col()
        .key(format!("browser-action:{}", action.id.as_str()))
        .w_full()
        .gap(8.0)
        .child(typography.text(display.title).wrap())
        .child(typography.text(&display.status).wrap());
    if !display.details.iter().any(|detail| {
        matches!(detail.part, "source" | "destination")
            || matches!(detail.label, "Source" | "Destination")
    }) {
        card = card.child(typography.text(&action.url).wrap());
    }
    for detail in &display.details {
        card = card.child(
            typography
                .text(format!("{}: {}", detail.label, detail.value))
                .wrap(),
        );
    }
    if action.phase == crate::model::BrowserActionPhase::Pending {
        if row.decline && !row.approve {
            card = card.child(typography.text("Approval needs access to the original page. You can still decline this action.").wrap());
        }
        for (approve, enabled, label) in [
            (false, row.decline, "Decline"),
            (true, row.approve, "Approve"),
        ] {
            let decision = browser_actions::DecisionKey {
                id: action.id.clone(),
                revision: action.revision,
                approve,
            };
            card = card.child(
                typography
                    .text(label)
                    .key(decision.element_key())
                    .role("button")
                    .focusable(true)
                    .disabled(!enabled)
                    .label(format!("{label} browser action at {}", action.url))
                    .native_activation()
                    .on_press_tx(move |state: &mut State, tx| {
                        state.decide_browser_action(decision.clone(), tx)
                    }),
            );
        }
    }
    card
}

fn view(state: &State, cx: &mut Cx<State>) -> El {
    let palette = state.preferences.palette();
    let (ink, page) = (palette.ink, palette.page);
    let typography = state.preferences.typography();
    let mut thread = col().key("messages").w_full().gap(16.0);
    if let Some(updates) = &state.updates {
        let presentation = updates.presentation.borrow();
        for message in &presentation.messages.messages {
            if let Some(content) = message.text() {
                let id = message.id();
                let user = matches!(message, crate::ag_ui::AgUiMessage::User { .. });
                let body = typography
                    .text(content)
                    .wrap()
                    .font(typography.font(13.0, 400.0).lh(13.0 * 1.625 * typography.scale))
                    .color(ink)
                    .selectable(true);
                let message = if user {
                    // Inset from the left, wider on a wide panel; a new message rises 8 px into place.
                    let wide = state.thread_rect.get()[2] >= 640.0;
                    row()
                        .w_full()
                        .justify_end()
                        .pl(if wide { 96.0 } else { 40.0 })
                        .child(
                            col()
                                .px(14.0)
                                .py(8.0)
                                .radius(12.0)
                                .fill(palette.field)
                                .border(1.0, palette.line)
                                .child(body),
                        )
                        .transition(theme::rise())
                } else {
                    col().w_full().child(body)
                };
                let message = message.key(id);
                thread = thread.child(if user {
                    message.role("none")
                } else {
                    message.role("article")
                });
            } else if let crate::ag_ui::AgUiMessage::Activity { id, .. } = message {
                thread = thread.child(
                    state
                        .activities
                        .view(id, updates, typography, palette)
                        .key(id)
                        .w_full(),
                );
            }
        }
        if state.running() && presentation.messages.waiting() {
            thread = thread.child(typography.text("Working").key("working").role("status"));
        }
    }
    if matches!(state.session, Session::Connecting) {
        thread = thread.child(typography.text("Connecting").role("status"));
    }
    let Some(core) = state.core() else {
        if let Some(gateway) = state.gateway() {
            // No companion: the chat, a line naming the route that answers, and the composer.
            let presentation = state.updates.as_ref().map(|updates| updates.presentation.borrow());
            let error = presentation.as_ref().and_then(|presentation| presentation.error.clone());
            drop(presentation);
            let line = error.unwrap_or_else(|| {
                format!("Answering with {}, without the companion", gateway.route)
            });
            let mut composer = col().w_full().max_w(720.0).child(composer::view(state, cx));
            if let Some(menu) = composer::menu(state) {
                composer = composer.child(menu);
            }
            return col()
                .w_full()
                .h_full()
                .fill(page)
                .color(ink)
                .child(thread.padding(16.0).grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: true }))
                .child(
                    row()
                        .w_full()
                        .px(16.0)
                        .gap(12.0)
                        .items_center()
                        .child(typography.text(line).role("status").wrap().grow(1.0).color(palette.ink_2))
                        .child(
                            typography
                                .text("New chat")
                                .key("gateway-new-chat")
                                .role("button")
                                .focusable(true)
                                .disabled(gateway.streaming())
                                .native_activation()
                                .on_press_tx(|state: &mut State, _| {
                                    let updates = state.updates.clone();
                                    if let (Some(gateway), Some(updates)) = (state.gateway(), updates) {
                                        gateway.clear(&updates);
                                    }
                                }),
                        ),
                )
                .child(col().w_full().items_center().padding(12.0).child(composer));
        }
        if matches!(state.session, Session::Failed) {
            if let Some(error) = state
                .updates
                .as_ref()
                .and_then(|updates| updates.presentation.borrow().error.clone())
            {
                thread = thread.child(typography.text(error).role("status").wrap());
            }
            thread = thread.child(
                typography
                    .text("Reload")
                    .role("button")
                    .focusable(true)
                    .on_press_tx(|state: &mut State, tx| state.review(tx))
                    .on_key(|state: &mut State, key, tx| {
                        composer::enter(state, key, tx, State::review)
                    }),
            );
        }
        return col()
            .w_full()
            .h_full()
            .fill(page)
            .color(ink)
            .child(thread.padding(16.0));
    };
    let connection = connection::UiConnectionSnapshot::read(core);
    if let Some(plan) = core.page.view.borrow().task_plan.as_ref() {
        let enabled = core.ensure_current().is_ok()
            && core.flow().is_connected()
            && core.page.view.borrow().active_run() == Some(&plan.run)
            && core.plan_operation.borrow().as_ref() != Some(&(plan.run.clone(), plan.revision));
        thread = thread.child(plan_view(
            plan,
            enabled,
            typography,
            State::decide_plan,
            State::session_plan,
        ));
    }
    // Layout follows whether the chat has messages, so it does not jump when history finishes
    // loading; the welcome holds its space and only its reveal waits for the load.
    let empty = state
        .updates
        .as_ref()
        .is_none_or(|updates| updates.presentation.borrow().messages.messages.is_empty());
    if state.settings_open {
        // Only an open section builds its body: a collapsed one would be dropped unseen.
        let open = |section: &Section| state.settings_sections.contains(section.key);
        let (providers, agents) = state.settings.view(core, cx, typography, palette, (open(&PROVIDERS), open(&AGENTS)));
        let appearance = open(&APPEARANCE).then(|| state.preferences.view(State::preference));
        let presets = open(&PROVIDERS).then(|| state.settings.preset_list(typography, palette)).flatten();
        let title = row()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                typography
                    .text("Settings")
                    .font(typography.font(16.0, 600.0))
                    .role("heading"),
            )
            .child(conversations::button(
                palette,
                typography,
                "Close",
                "settings-back".into(),
                true,
                |state: &mut State, tx| state.close_settings(tx),
            )
            .label("Back to chat"));
        return col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .fill(palette.canvas)
            .color(ink)
            .padding(10.0)
            .material(palette)
            .child(
                col()
                    .key("settings-card")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .gap(12.0)
                    .padding(16.0)
                    .fill(page)
                    .border(1.0, palette.line)
                    .radius(14.0)
                    .clip_rounded(14.0)
                    .child(title)
                    .child(
                        col()
                            .w_full()
                            .gap(10.0)
                            .child(settings_section(state, &APPEARANCE, appearance))
                            .child(settings_section(state, &PROVIDERS, providers))
                            .child(settings_section(state, &AGENTS, agents))
                            .grow(1.0)
                            .min_h(0.0)
                            .scroll(ScrollSpec {
                                stick_bottom: false,
                            }),
                    ),
            )
            .children(presets);
    }
    if let Some(followup) = &state.proposed_followup {
        thread = thread.child(
            col()
                .gap(8.0)
                .child(typography.text(&followup.text).wrap())
                .child(followup_controls(
                    typography,
                    State::add_followup,
                    State::dismiss_followup,
                )),
        );
    }
    for reply in replies::view(state, core) {
        thread = thread.child(reply);
    }
    for reply in pi_replies::view(state, core) {
        thread = thread.child(reply);
    }
    for correction in corrections::view(state, core) {
        thread = thread.child(correction);
    }
    let browser = core.rune_browser_actions();
    if let Some(notice) = browser.notice() {
        let role = if !browser.connected() || browser.stale() || browser.uncertain() {
            "alert"
        } else {
            "status"
        };
        thread = thread.child(typography.text(notice).role(role).wrap());
    }
    let actions = browser.actions();
    for action in actions.iter().filter(|action| !action.display.receipt) {
        thread = thread.child(if secrets::pending(action) {
            secrets::card(action, state, typography)
        } else {
            browser_action_view(action, typography)
        });
    }
    let receipts: Vec<_> = actions
        .iter()
        .filter(|action| action.display.receipt)
        .collect();
    if !receipts.is_empty() {
        let toggle_label = format!("Browser action results ({})", receipts.len());
        let toggle = typography
            .legend(toggle_label.as_str())
            .label(toggle_label)
            .key("browser-receipts-toggle");
        thread = thread.child(collapsible::trigger(
            toggle,
            Some("browser-receipts"),
            Default::default(),
            state.browser_receipts_open,
            |state: &mut State, open, _| state.browser_receipts_open = open,
        ));
        let list = col()
            .w_full()
            .gap(16.0)
            .children(receipts.into_iter().map(|action| browser_action_view(action, typography)));
        if let Some(panel) = collapsible::panel(
            list,
            "browser-receipts",
            "browser-receipts-toggle",
            Default::default(),
            state.browser_receipts_open,
        ) {
            thread = thread.child(panel);
        }
    }
    if !connection.connected() {
        thread = thread.child(
            typography
                .text("Reconnect")
                .role("button")
                .focusable(true)
                .disabled(!connection.can_reconnect())
                .on_press_tx(|state: &mut State, tx| state.reconnect(tx))
                .on_key(|state: &mut State, key, tx| {
                    composer::enter(state, key, tx, State::reconnect)
                }),
        );
    }
    if connection.uncertain() || connection.stale() {
        thread = thread.child(
            typography
                .text("Review saved work")
                .role("button")
                .focusable(true)
                .disabled(connection.reconnecting())
                .on_press_tx(|state: &mut State, tx| state.review(tx))
                .on_key(|state: &mut State, key, tx| {
                    composer::enter(state, key, tx, State::review)
                }),
        );
    }
    let mut composer = col().w_full().max_w(720.0).child(composer::view(state, cx));
    if let Some(menu) = composer::menu(state) {
        composer = composer.child(menu);
    }
    // The status reads the error in place: the presentation stays borrowed only while this
    // block builds the line, and no message is copied for it.
    let p = palette;
    let (line, lamp) = {
        let presentation = state.updates.as_ref().map(|updates| updates.presentation.borrow());
        let error = presentation.as_ref().and_then(|presentation| presentation.error.as_deref());
        let status = if connection.stale() {
                        "The conversation changed. Reload the panel."
                    } else if connection.reconnecting() {
                        "Reconnecting"
                    } else if !connection.connected() {
                        "Connection lost"
                    } else if connection.uncertain() {
                        "Check saved work before starting again"
                    } else if let Some(error) = error {
                        error
                    } else if let Some(error) = &state.conversations.error {
                        error
                    } else if matches!(*core.owner.borrow(), Owner::Capturing) {
                        "Reading current page"
                    } else if core.rune_has_native_requests()
                        || core.rune_browser_action().is_some()
                    {
                        "Waiting for approval"
                    } else if state.running() {
                        "Running"
                    } else if state.conversations.busy || core.history_loading.get() {
                        "Loading history"
                    } else {
                        "Ready"
                    };
        // One fixed line under the controls: a longer status never rewraps the row or moves the
        // thread below it.
        // The lamp beside the status: lost or failing is red, working amber, ready green. A molded
        // theme prints the line on its gray LCD.
        let fault = connection.stale() || !connection.connected() || connection.uncertain() || error.is_some()
            || state.conversations.error.is_some();
        let lamp = if fault { p.red } else if status == "Ready" { p.green } else { p.orange };
        let line = typography
            .text(status)
            .font(typography.font(12.0, 400.0).features("'tnum' 1").lh(16.0 * typography.scale))
            .color(if p.molded() { p.print } else if p.neon() || p.retro() { p.accent } else { p.ink_2 })
            .single_line()
            .truncate()
            .h(16.0 * typography.scale)
            .grow(1.0)
            .key("conversation-status")
            .role("status");
        (line, lamp)
    };
    // An empty chat under a flat theme lists its entries as the reference lists suggestions;
    // otherwise they are chips in one wrapping row.
    let rows = empty && !p.crafted();
    let toolbar = if rows { col().w_full() } else { row().w_full().flex_wrap().gap(8.0).items_center() };
    let toolbar = toolbar
        .child(conversations::entry(
            p,
            typography,
            "Saved chats",
            "saved-chats-open".into(),
            &conversations::HISTORY,
            rows,
            true,
            |state: &mut State, tx| state.open_conversations(tx),
        )
        .has_popup("dialog"))
        .child(conversations::entry(
            p,
            typography,
            "Settings",
            "settings-open".into(),
            &conversations::TUNE,
            rows,
            true,
            |state: &mut State, tx| state.open_settings(tx),
        ))
        .child(conversations::entry(
            p,
            typography,
            "Summarize page",
            "summarize-page".into(),
            &conversations::SUMMARY,
            rows,
            state.can_start(),
            |state: &mut State, tx| state.summarize(tx),
        ));
    let toolbar = metldr_ui::toolbar::toolbar(
        state.conversations.toolbar(state, toolbar, rows),
        "Chat actions",
        metldr_ui::toolbar::Props {
            axis: if rows { ::rune::FocusAxis::Vertical } else { ::rune::FocusAxis::Horizontal },
            wrap: true,
        },
    );
    let readout = row().w_full().gap(6.0).items_center().child(theme::led(lamp, p)).child(line);
    let readout = if p.molded() {
        theme::lcd(readout.px(10.0).py(5.0), p)
    } else if p.neon() {
        // A neon status reads as a HUD strip: the line in the accent over a dark glass edged in it.
        readout.px(10.0).py(5.0).radius(2.0).fill(p.page).border(1.0, p.accent.alpha(0.35))
    } else if p.retro() {
        // A retro status is a terminal line: the accent on the darkest ground, stamped in.
        readout.px(10.0).py(5.0).radius(4.0).fill(p.lip).border(1.5, p.line_strong)
    } else {
        readout
    };
    let toolbar = col().w_full().gap(6.0).child(toolbar).child(readout);
    let shown = cx.is_reduced_motion() || state.welcome_revealed;
    let mut head = col().w_full().gap(if empty { 24.0 } else { 16.0 });
    if empty {
        // Two lines with tight leading and negative tracking, as the reference's 26 px heading.
        let hero = |line: &str| {
            typography
                .text(line)
                .font(
                    typography
                        .font(26.0, 400.0)
                        .lh(26.0 * 1.35 * typography.scale)
                        .tracking(-0.02 * 26.0 * typography.scale),
                )
                .wrap()
        };
        let greeting = hero("Welcome to metldr").color(p.ink_3).role("heading").level(1);
        let question = hero("What do you want to do?").color(p.ink);
        // While history loads the lines hold their space unseen, so the toolbar and composer under
        // them never move when the reveal starts.
        let lines = if core.history_loading.get() {
            col().child(greeting).child(question).opacity(0.0).a11y_hidden()
        } else {
            let lines = col()
                .child(theme::reveal(greeting, shown, theme::REVEAL_GREETING))
                .child(theme::reveal(question, shown, theme::REVEAL_QUESTION));
            if shown {
                lines
            } else {
                lines.on_drawn_layout_tx(|state: &mut State, _, _| {
                    state.welcome_revealed = true;
                })
            }
        };
        head = head.child(lines);
    }
    // An empty chat follows the reference: the composer 28 points under the welcome, then the
    // entries 24 points under it, the status and the saved page. A chat with messages pins the
    // toolbar under the tabs, so Settings and paging stay reachable while the thread scrolls,
    // and docks the composer under the thread.
    let (pinned, composer) = if empty {
        head = head.child(composer.pt(4.0)).child(toolbar);
        (None, None)
    } else {
        (
            Some(col().w_full().px(16.0).py(8.0).items_center().child(toolbar.max_w(720.0))),
            Some(col().w_full().px(16.0).pb(24.0).center_y().child(composer)),
        )
    };
    head = head.child(state.sources.view(typography, p));
    let body = col()
        .w_full()
        .max_w(720.0)
        .gap(32.0)
        .child(head)
        .child(thread);
    // Auto margins centre an empty chat and, unlike centring the viewport, keep an overflowing
    // welcome reachable at the top of a short panel or at 200% zoom.
    let body = if empty { body.my_auto() } else { body };
    let scroll = col()
        .w_full()
        .px(16.0)
        .pt(32.0)
        .pb(if empty { 40.0 } else { 24.0 })
        .items_center()
        .child(body)
        .drawn_rect_into(&state.thread_rect)
        .grow(1.0)
        .min_h(0.0);
    let scroll = metldr_ui::scroll_area::viewport(scroll, "Messages", true);
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(p.canvas)
        .color(ink)
        .padding(10.0)
        .material(p)
        .child(
            theme::module(col(), p)
                .key("chat-card")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .radius(14.0)
                .clip_rounded(14.0)
                .child(state.conversations.tabs(state, core))
                .child(col().w_full().h(1.0).fill(p.line))
                .child(metldr_ui::tabs::panel(
                    col().key("chat-panel").w_full().grow(1.0).min_h(0.0).children(pinned).child(scroll).children(composer),
                    state.tabs.title(&core.scope.conversation).unwrap_or("New chat"),
                )),
        )
        .children(state.conversations.is_open().then(|| {
            let saved = metldr_ui::dialog::dialog(
                state.conversations.view(state),
                "Saved chats",
                None,
                "saved-chats-open",
                Default::default(),
                |state: &mut State| state.conversations.dismiss(),
            );
            metldr_ui::dialog::backdrop(saved, palette.scrim)
        }))
        .children(theme::tooltip(state))
}

/// A collapsible settings card: a 44 px header with the title and a chevron, and the body below
/// it while open.
/// One collapsible card of the settings page, with its keys spelled out so a frame builds none.
struct Section {
    key: &'static str,
    trigger: &'static str,
    panel: &'static str,
    title: &'static str,
}

const APPEARANCE: Section = Section {
    key: "appearance",
    trigger: "settings-section-appearance",
    panel: "settings-panel-appearance",
    title: "Appearance and sound",
};
const PROVIDERS: Section = Section {
    key: "providers",
    trigger: "settings-section-providers",
    panel: "settings-panel-providers",
    title: "AI providers",
};
const AGENTS: Section = Section {
    key: "agents",
    trigger: "settings-section-agents",
    panel: "settings-panel-agents",
    title: "Installed agents",
};

fn settings_section(state: &State, section: &'static Section, body: Option<El>) -> El {
    let Section { key, trigger, panel, title } = *section;
    let palette = state.preferences.palette();
    let typography = state.preferences.typography();
    let open = state.settings_sections.contains(key);
    let header = row()
        .key(trigger)
        .label(title)
        .w_full()
        .min_h(44.0)
        .px(14.0)
        .gap(6.0)
        .items_center()
        .child(
            typography
                .legend(title)
                .font(typography.font(14.0, 600.0))
                .grow(1.0),
        )
        .child(
            ::rune::el::icon(&composer::CHEVRON, 11.0)
                .color(palette.ink_3)
                .icon_rotation(if open { std::f32::consts::PI } else { 0.0 }),
        )
        .hover(palette.hover)
        .press(palette.press);
    let header = collapsible::trigger(header, Some(panel), Default::default(), open, move |state: &mut State, open, _| {
        if open {
            state.settings_sections.insert(key);
        } else {
            state.settings_sections.remove(key);
        }
    });
    let mut card = col()
        .w_full()
        .radius(10.0)
        .fill(palette.surface)
        .border(1.0, palette.line)
        .child(header);
    let body = col().w_full().px(14.0).pb(14.0).children(body);
    if let Some(panel) = collapsible::panel(
        body,
        panel,
        trigger,
        Default::default(),
        open,
    ) {
        card = card.child(panel);
    }
    card
}

/// What a summary shows as the user's message in the thread.
const SUMMARY_REQUEST: &str = "Summarize this page.";

thread_local! {
    static HOST: RefCell<Option<AppHandle<State>>> = const { RefCell::new(None) };
    /// The page's font loader, called once with `"cjk"` when the chat first lays out CJK text.
    static WANT_FONTS: RefCell<Option<Function>> = const { RefCell::new(None) };
}

/// Asks the page for the CJK font the first frame CJK text was laid out.
fn ask_for_cjk() {
    if ::rune::text::cjk_seen() {
        WANT_FONTS.with(|want| {
            if let Some(want) = want.borrow_mut().take() {
                let _ = want.call1(&JsValue::NULL, &JsValue::from_str("cjk"));
            }
        });
    }
}

#[wasm_bindgen]
pub fn start_rune_chat(capture: Function, fonts: js_sys::Array, want_fonts: Function) -> Result<(), JsValue> {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.is_some() {
            return Err(crate::err(
                "Rune already owns this document. Reload to reopen it.",
            ));
        }
        let fonts = fonts::read(&fonts, &fonts::NAMES)?;
        crate::flow::install_capture(capture);
        WANT_FONTS.with(|want| *want.borrow_mut() = Some(want_fonts));
        let spec = AppSpec {
            fonts: fonts::retain(fonts),
            families: fonts::FAMILIES,
            setup: Some(setup),
            poll: Some(poll),
            on_key: Some(key),
            ..AppSpec::new("metldr", (640.0, 720.0), State::default, view)
        };
        let app = App::new(spec)
            .canvas_id("metldr-rune-chat")
            .full_viewport()
            .tab_visits_controls(true)
            .on_frame_drawn(|runtime| {
                ask_for_cjk();
                if let Some(updates) = &runtime.state.updates {
                    runtime.state.answers.drawn(
                        if runtime.state.settings_open || runtime.state.conversations.is_open() {
                            [0.0; 4]
                        } else {
                            runtime.state.thread_rect.get()
                        },
                        updates,
                        &runtime.state.activities,
                        runtime.state.preferences.palette(),
                        runtime.state.preferences.typography(),
                    );
                }
            })
            .compositor(glimm::Glimm::default());
        let handle = app.handle();
        app.try_run().map_err(|error| crate::err(&error))?;
        *host = Some(handle);
        Ok(())
    })
}

/// Adds a font group the page loads after start: `fallbacks` once the chat has drawn, `cjk`
/// once the chat asks for it. Text in those scripts lays out again on the next frame. A group
/// added before does nothing.
#[wasm_bindgen]
pub fn add_rune_chat_fonts(group: String, fonts: js_sys::Array) -> Result<(), JsValue> {
    let names = fonts::group(&group).ok_or_else(|| crate::err("Unknown font group."))?;
    if let Some(fonts) = fonts::retain_group(&group, fonts::read(&fonts, names)?) {
        ::rune::text::add_fonts(fonts);
        HOST.with(|host| {
            if let Some(host) = host.borrow().as_ref() {
                host.wake();
            }
        });
    }
    Ok(())
}

#[wasm_bindgen]
pub fn stop_rune_chat() {
    HOST.with(|host| {
        if let Some(host) = host.borrow().as_ref() {
            host.quit();
        }
    });
}
