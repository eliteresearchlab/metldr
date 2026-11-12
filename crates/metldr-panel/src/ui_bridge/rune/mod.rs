use super::*;
use ::rune::{
    App, AppHandle, AppSpec, Cx, El, ScrollSpec, TextEdit, TextStyle, Tx, col, oklch, text,
};
use wasm_bindgen_futures::spawn_local;

mod composer;
mod fonts;
mod glimm;
mod messages;

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
    connected: Option<Result<Rc<Core>, FlowError>>,
    accepted_draft: Option<String>,
    error: Option<String>,
}

struct Updates {
    presentation: RefCell<Presentation>,
    dirty: Cell<bool>,
    closed: Cell<bool>,
    wake: ::rune::Waker,
}

impl Updates {
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
        presentation.messages.apply(event)?;
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
    menu: Option<Menu>,
    source_engaged: bool,
    selection: String,
    mode: Mode,
    celebration: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            session: Session::Connecting,
            updates: None,
            observation: None,
            draft: TextEdit::default(),
            menu: None,
            source_engaged: false,
            selection: String::new(),
            mode: Mode::Configured,
            celebration: 0,
        }
    }
}

impl State {
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
        let Some(core) = self.core().cloned() else { return };
        let Some(updates) = self.updates.clone() else { return };
        match core.decide_from_rune(decision, tx) {
            Ok(work) => spawn_local(async move { updates.finish(work.await); }),
            Err(error) => self.fail(error),
        }
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
                Ok(stopping) => spawn_local(async move {
                    updates.finish(stopping.await.map(|_| ()));
                }),
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
            Ok(stopping) => spawn_local(async move {
                updates.finish(stopping.await.map(|_| ()));
            }),
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

fn setup(state: &mut State, wake: ::rune::Waker) {
    let updates = Rc::new(Updates {
        presentation: RefCell::new(Presentation {
            messages: Messages::new(),
            connected: None,
            accepted_draft: None,
            error: None,
        }),
        dirty: Cell::new(false),
        closed: Cell::new(false),
        wake,
    });
    state.updates = Some(Rc::clone(&updates));
    spawn_local(async move {
        let connected = async {
            let flow = FlowClient::connect().await?;
            let view = flow.current_page().await?;
            let core = UiBridge::from_view(flow, view)?.core;
            let history = core.history(None).await?;
            if updates.closed.get() {
                return Err(FlowError::Disconnected);
            }
            updates
                .presentation
                .borrow_mut()
                .messages
                .apply(Event::MessagesSnapshot {
                    messages: history.messages,
                })?;
            let weak = Rc::downgrade(&updates);
            *core.page.changed.borrow_mut() = Some(Rc::new(move || {
                if let Some(updates) = weak.upgrade() {
                    updates.changed();
                }
            }));
            Ok(core)
        }
        .await;
        if !updates.closed.get() {
            updates.presentation.borrow_mut().connected = Some(connected);
            updates.changed();
        }
    });
}

fn poll(state: &mut State, _: f64, _: &mut Tx) -> bool {
    let Some(updates) = state.updates.clone() else {
        return false;
    };
    if !updates.dirty.replace(false) {
        return false;
    }
    let connected = updates.presentation.borrow_mut().connected.take();
    if let Some(connected) = connected {
        match connected {
            Ok(core) => {
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
                        state.draft.replace_text(draft);
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
    let accepted = updates.presentation.borrow_mut().accepted_draft.take();
    if let Some(accepted) = accepted {
        if state.draft.text == accepted {
            state.draft.clear();
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
    true
}

fn key(state: &mut State, key: &::rune::KeyEvent, _: &mut Tx) -> bool {
    if key.key == ::rune::Key::Escape && state.menu.is_some() {
        state.menu = None;
        state.source_engaged = false;
        true
    } else {
        false
    }
}

fn view(state: &State, cx: &mut Cx<State>) -> El {
    let (ink, page) = if cx.system_dark() {
        (oklch(0.964, 0.002, 247.839), oklch(0.209, 0.004, 264.477))
    } else {
        (oklch(0.247, 0.006, 258.361), oklch(0.985, 0.001, 286.376))
    };
    let mut thread = col().key("messages").w_full().gap(16.0).padding(16.0);
    if let Some(updates) = &state.updates {
        let presentation = updates.presentation.borrow();
        for message in &presentation.messages.messages {
            if let Some(content) = message.text() {
                thread = thread.child(
                    text(content)
                        .key(message.id())
                        .w_full()
                        .wrap()
                        .font(TextStyle::new(14.0, 400.0).lh(21.0))
                        .color(ink)
                        .selectable(true),
                );
            }
        }
        if let Some(error) = &presentation.error {
            thread = thread.child(text(error).key("error").w_full().wrap().label(error));
        }
    }
    if matches!(state.session, Session::Connecting) {
        thread = thread.child(text("Opening metldr").label("Opening metldr"));
    }
    let Some(core) = state.core() else {
        if matches!(state.session, Session::Failed) {
            thread = thread.child(
                text("Reload")
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
    if let Some((action, can_approve, can_decline)) = core.rune_browser_action() {
        let display = crate::browser_action_display::display(&action);
        let mut approval = col().key("browser-approval").w_full().gap(8.0)
            .child(text(display.title).wrap());
        for detail in display.details {
            approval = approval.child(text(format!("{}: {}", detail.label, detail.value)).wrap());
        }
        for (approve, enabled, label) in [(true, can_approve, "Approve"), (false, can_decline, "Decline")] {
            let decision = browser_actions::DecisionKey { id: action.id.clone(), revision: action.revision, approve };
            approval = approval.child(text(label).key(decision.element_key()).role("button")
                .focusable(true).disabled(!enabled).label(label).native_activation()
                .on_press_tx(move |state: &mut State, tx| state.decide_browser_action(decision.clone(), tx)));
        }
        thread = thread.child(approval);
    }
    if !connection.connected() {
        thread = thread.child(
            text("Reconnect")
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
            text("Review saved work")
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
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(page)
        .color(ink)
        .child(
            thread
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
pub fn start_rune_chat(capture: Function) -> Result<(), JsValue> {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.is_some() {
            return Err(crate::err(
                "Rune already owns this document. Reload to reopen it.",
            ));
        }
        crate::flow::install_capture(capture);
        let spec = AppSpec {
            fonts: fonts::FONTS,
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
