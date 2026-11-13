use super::*;
use ::rune::{
    App, AppHandle, AppSpec, Cx, El, ScrollSpec, TextEdit, TextStyle, Tx, col, oklch, row, text,
};
use wasm_bindgen_futures::spawn_local;

mod activities;
mod answers;
mod composer;
mod conversations;
mod corrections;
mod fonts;
mod glimm;
mod messages;
mod pi_replies;
mod preferences;
mod replies;
mod settings;
mod sources;

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
    conversations: conversations::Conversations,
    tabs: conversations::Tabs,
    focus_tab_after_titles: Option<String>,
    activities: Activities,
    browser_receipts_open: bool,
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
            conversations: conversations::Conversations::default(),
            tabs: conversations::Tabs::default(),
            focus_tab_after_titles: None,
            activities: Activities::default(),
            browser_receipts_open: false,
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
            self.sound(crate::interaction_audio::Cue::Tick);
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

    fn running(&self) -> bool {
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
        !self.draft.text.trim().is_empty()
            && !self.running()
            && self.core().is_some_and(|core| {
                core.page.controls_ready()
                    && !core.page.settings_blocked()
                    && core.page.flow.borrow().is_connected()
            })
            && self
                .choices()
                .iter()
                .any(|choice| choice.value == self.selection && choice.enabled)
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

    fn decide_browser_action(&mut self, decision: browser_actions::DecisionKey, tx: &mut Tx) {
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
        if !self.can_send() {
            return;
        }
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
                text: UserText::try_from(self.draft.text.as_str())?,
                mode: match self.mode {
                    Mode::Configured => "configured",
                    Mode::Plan => "plan",
                }
                .into(),
            };
            let sent_text = self.draft.text.clone();
            let events = Rc::clone(&updates);
            core.run(request, move |event| {
                if matches!(event, Event::RunStarted { .. }) {
                    events.presentation.borrow_mut().accepted_draft = Some(sent_text.clone());
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
            if let Some(ui) = ui {
                if let Ok(stopping) = core.stop(&ui) {
                    spawn_local(async move {
                        let _ = stopping.await;
                    });
                }
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
                            .map(|choice| choice.value)
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
            }
        }
    }
    if let Some(core) = state.core().cloned() {
        let running = state.running();
        match state.conversations.poll(&core, running) {
            Some(conversations::Completion::Navigation(core)) => {
                state.switch_core(core, &updates);
                return true;
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
    if let Some(accepted) = accepted {
        if state.draft.text == accepted {
            state.clear_draft();
        }
    }
    if !state.running() {
        state.observation = None;
    }
    if state.selection.is_empty() {
        if let Some(choice) = state.choices().into_iter().find(|choice| choice.enabled) {
            state.selection = choice.value;
        }
    }
    replies::sync(state);
    pi_replies::sync(state);
    corrections::sync(state);
    if let Some(core) = state.core().cloned() {
        state.settings.sync(&core);
        state.sources.sync(&core, &updates);
        let presentation = updates.presentation.borrow();
        state
            .activities
            .sync(&core, &presentation.messages.messages, &updates);
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
                .press(typography.ink.alpha(0.12))
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
        }
    }

    #[test]
    fn declined_plan_has_no_buttons_and_untrusted_input_is_ignored() {
        fn view(_: &State, _: &mut Cx<State>) -> El {
            plan_view(
                &sample_plan(crate::model::PlanState::Declined),
                true,
                preferences::Typography {
                    scale: 1.0,
                    ink: oklch(0.5, 0.0, 0.0),
                },
                State::decide_plan,
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
            preferences::Typography {
                scale: 1.0,
                ink: oklch(0.5, 0.0, 0.0),
            },
            |state: &mut PlanTest, plan, approve, _| {
                *state.pending.borrow_mut() = Some((plan.run.clone(), plan.revision));
                state
                    .replies
                    .push(crate::flow::plan_reply(&plan.run, plan.revision, approve));
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
    fn enter_activates_each_followup_once_and_ignores_repeat() {
        fn view(_: &[u32; 2], _: &mut Cx<[u32; 2]>) -> El {
            followup_controls(
                preferences::Typography {
                    scale: 1.0,
                    ink: oklch(0.5, 0.0, 0.0),
                },
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

fn toggle_browser_receipts(state: &mut State, _: &mut Tx) {
    state.browser_receipts_open = !state.browser_receipts_open;
}

fn plan_key(plan: &crate::model::TaskPlanView, approve: bool) -> String {
    format!(
        "plan:{}:{}:{}",
        plan.run.as_str(),
        plan.revision,
        u8::from(approve)
    )
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
        PlanState::Approved => card.child(typography.text("Approved")),
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
                        .press(typography.ink.alpha(0.12))
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
    let mut thread = col().key("messages").w_full().gap(16.0).padding(16.0);
    if let Some(updates) = &state.updates {
        let presentation = updates.presentation.borrow();
        for message in &presentation.messages.messages {
            if let Some(content) = message.text() {
                let id = message.id();
                let user = matches!(message, crate::ag_ui::AgUiMessage::User { .. });
                let body = typography
                    .text(content)
                    .wrap()
                    .font(typography.font(14.0, 400.0).lh(21.0 * typography.scale))
                    .color(ink)
                    .selectable(true);
                let message = if user {
                    row().w_full().justify_end().child(
                        col()
                            .max_w(620.0)
                            .padding(12.0)
                            .radius(12.0)
                            .fill(palette.field)
                            .child(body),
                    )
                } else {
                    col().max_w(620.0).child(body)
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
                        .view(id, updates, typography)
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
        return col().w_full().h_full().fill(page).color(ink).child(thread);
    };
    let connection = connection::UiConnectionSnapshot::read(core);
    if let Some(plan) = core.page.view.borrow().task_plan.as_ref() {
        let enabled = core.ensure_current().is_ok()
            && core.flow().is_connected()
            && core.page.view.borrow().active_run() == Some(&plan.run)
            && core.plan_operation.borrow().as_ref() != Some(&(plan.run.clone(), plan.revision));
        thread = thread.child(plan_view(plan, enabled, typography, State::decide_plan));
    }
    if !core.history_loading.get()
        && state
            .updates
            .as_ref()
            .is_some_and(|updates| updates.presentation.borrow().messages.messages.is_empty())
    {
        thread = thread.child(
            typography
                .text("Welcome to metldr")
                .role("heading")
                .level(1),
        );
        thread = thread.child(typography.text("What do you want to do?").wrap());
    }
    if state.conversations.is_open() {
        return col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .fill(page)
            .color(ink)
            .child(
                state
                    .conversations
                    .view(state)
                    .padding(16.0)
                    .grow(1.0)
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    }),
            );
    }
    if state.settings_open {
        return col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .fill(page)
            .color(ink)
            .child(
                typography
                    .text("Back to chat")
                    .key("settings-back")
                    .role("button")
                    .focusable(true)
                    .padding(12.0)
                    .press(ink.alpha(0.12))
                    .on_press_tx(|state: &mut State, tx| state.close_settings(tx))
                    .on_key(|state: &mut State, key, tx| {
                        composer::enter(state, key, tx, State::close_settings)
                    }),
            )
            .child(
                col()
                    .gap(16.0)
                    .child(state.preferences.view(State::preference))
                    .child(state.settings.view(core, cx, typography, palette))
                    .padding(16.0)
                    .grow(1.0)
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    }),
            );
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
    thread = thread.child(state.sources.view(typography));
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
        thread = thread.child(browser_action_view(action, typography));
    }
    let receipts: Vec<_> = actions
        .iter()
        .filter(|action| action.display.receipt)
        .collect();
    if !receipts.is_empty() {
        thread = thread.child(
            typography
                .text(format!("Browser action results ({})", receipts.len()))
                .key("browser-receipts-toggle")
                .role("button")
                .focusable(true)
                .expanded(state.browser_receipts_open)
                .on_press_tx(toggle_browser_receipts)
                .on_key(|state: &mut State, key, tx| {
                    if key.key == ::rune::Key::Enter || key.key == ::rune::Key::Text(" ".into()) {
                        if !tx.key_repeat() {
                            toggle_browser_receipts(state, tx);
                        }
                        true
                    } else {
                        false
                    }
                }),
        );
        if state.browser_receipts_open {
            for action in receipts {
                thread = thread.child(browser_action_view(action, typography));
            }
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
    if let Some(menu) = composer::menu(state, cx) {
        composer = composer.child(menu);
    }
    let error = state
        .updates
        .as_ref()
        .and_then(|updates| updates.presentation.borrow().error.clone());
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(page)
        .color(ink)
        .child(
            row()
                .w_full()
                .padding(8.0)
                .child(
                    typography
                        .text("Saved chats")
                        .key("saved-chats-open")
                        .role("button")
                        .focusable(true)
                        .padding(6.0)
                        .press(ink.alpha(0.12))
                        .on_press_tx(|state: &mut State, tx| state.open_conversations(tx))
                        .on_key(|state: &mut State, key, tx| {
                            composer::enter(state, key, tx, State::open_conversations)
                        }),
                )
                .child(
                    typography
                        .text("Settings")
                        .key("settings-open")
                        .role("button")
                        .focusable(true)
                        .padding(6.0)
                        .press(ink.alpha(0.12))
                        .on_press_tx(|state: &mut State, tx| state.open_settings(tx))
                        .on_key(|state: &mut State, key, tx| {
                            composer::enter(state, key, tx, State::open_settings)
                        }),
                ),
        )
        .child(
            typography
                .text(if connection.stale() {
                    "The conversation changed. Reload the panel."
                } else if connection.reconnecting() {
                    "Reconnecting"
                } else if !connection.connected() {
                    "Connection lost"
                } else if connection.uncertain() {
                    "Check saved work before starting again"
                } else if let Some(error) = &error {
                    error
                } else if let Some(error) = &state.conversations.error {
                    error
                } else if matches!(*core.owner.borrow(), Owner::Capturing) {
                    "Reading current page"
                } else if !core.rune_native_requests().0.is_empty()
                    || core.rune_browser_action().is_some()
                {
                    "Waiting for approval"
                } else if state.running() {
                    "Running"
                } else if state.conversations.busy || core.history_loading.get() {
                    "Loading history"
                } else {
                    "Ready"
                })
                .key("conversation-status")
                .role("status")
                .padding(8.0),
        )
        .child(state.conversations.tabs(state, core))
        .child(state.conversations.toolbar(state))
        .child(
            thread
                .drawn_rect_into(&state.thread_rect)
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: true }),
        )
        .child(col().w_full().px(16.0).pb(24.0).center_y().child(composer))
}

thread_local! {
    static HOST: RefCell<Option<AppHandle<State>>> = const { RefCell::new(None) };
}

#[wasm_bindgen]
pub fn start_rune_chat(capture: Function, fonts: js_sys::Array) -> Result<(), JsValue> {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.is_some() {
            return Err(crate::err(
                "Rune already owns this document. Reload to reopen it.",
            ));
        }
        let fonts: Vec<Vec<u8>> = fonts
            .iter()
            .map(|font| js_sys::Uint8Array::new(&font).to_vec())
            .collect();
        fonts::validate(&fonts).map_err(|error| crate::err(&error))?;
        crate::flow::install_capture(capture);
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

#[wasm_bindgen]
pub fn stop_rune_chat() {
    HOST.with(|host| {
        if let Some(host) = host.borrow().as_ref() {
            host.quit();
        }
    });
}
