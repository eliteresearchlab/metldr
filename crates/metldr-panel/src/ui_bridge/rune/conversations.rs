use super::*;
use crate::history::Change;

// Results are handed to the shared State only while their original Core is current.
enum ResultPage {
    Chats(Result<super::super::conversations::Page, FlowError>),
    Archive(u64, Result<super::super::history::Page, FlowError>),
    Navigation(Result<UiBridge, FlowError>),
}

pub(super) enum Completion {
    Archive(super::super::history::Page),
    Navigation(Rc<Core>),
}

#[derive(Default)]
pub(super) struct Conversations {
    open: bool,
    pub(super) busy: bool,
    page: Option<super::super::conversations::Page>,
    older_cursor: Option<String>,
    pub(super) error: Option<String>,
    archive_generation: Rc<Cell<u64>>,
    pending: Rc<RefCell<Option<(Rc<Core>, ResultPage)>>>,
}

impl Conversations {
    pub(super) fn open(&mut self, core: &Rc<Core>, updates: &Rc<Updates>, tx: &mut Tx) {
        self.open = true;
        tx.focus("saved-chats-close");
        self.load_chats(core, updates, None);
    }

    pub(super) fn close(&mut self, tx: &mut Tx) {
        self.open = false;
        tx.focus("saved-chats-open");
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn set_older_cursor(&mut self, cursor: Option<String>) {
        self.older_cursor = cursor;
    }

    pub(super) fn archive_epoch(&self) -> Rc<Cell<u64>> {
        Rc::clone(&self.archive_generation)
    }

    pub(super) fn invalidate_archive(&self) {
        self.archive_generation
            .set(self.archive_generation.get().wrapping_add(1));
    }

    fn begin(&mut self) -> bool {
        if self.busy {
            return false;
        }
        self.busy = true;
        self.error = None;
        true
    }

    fn complete(
        &self,
        core: Rc<Core>,
        updates: &Rc<Updates>,
        work: impl Future<Output = ResultPage> + 'static,
    ) {
        let pending = Rc::downgrade(&self.pending);
        let updates = Rc::downgrade(updates);
        spawn_local(async move {
            let result = work.await;
            if let Some(pending) = pending.upgrade() {
                *pending.borrow_mut() = Some((core, result));
                if let Some(updates) = updates.upgrade() {
                    updates.changed();
                }
            }
        });
    }

    fn load_chats(&mut self, core: &Rc<Core>, updates: &Rc<Updates>, cursor: Option<String>) {
        if !self.begin() {
            return;
        }
        let owner = Rc::clone(core);
        let work = Rc::clone(core);
        self.complete(owner, updates, async move {
            ResultPage::Chats(work.conversation_page(cursor.as_deref()).await)
        });
    }

    fn load_archive(&mut self, core: &Rc<Core>, updates: &Rc<Updates>, cursor: Option<String>) {
        if !self.begin() {
            return;
        }
        let owner = Rc::clone(core);
        let work = Rc::clone(core);
        let generation = self.archive_generation.get();
        self.complete(owner, updates, async move {
            ResultPage::Archive(generation, work.history(cursor.as_deref()).await)
        });
    }

    fn navigate(&mut self, core: &Rc<Core>, updates: &Rc<Updates>, change: Option<Change>) {
        if !self.begin() {
            return;
        }
        if let Some(change) = change
            && let Err(error) = core.start_conversation_navigation(change)
        {
            self.busy = false;
            self.error = Some(error.to_string());
            return;
        }
        let owner = Rc::clone(core);
        let work = Rc::clone(core);
        self.complete(owner, updates, async move {
            ResultPage::Navigation(work.continue_conversation_navigation().await)
        });
    }

    // Call from the shared poll. The caller applies archive replacement or Core handover.
    pub(super) fn poll(&mut self, current: &Rc<Core>, running: bool) -> Option<Completion> {
        let (owner, result) = self.pending.borrow_mut().take()?;
        self.busy = false;
        if !Rc::ptr_eq(&owner, current) {
            return None;
        }
        if !matches!(&result, ResultPage::Navigation(_))
            && let Err(error) = current.ensure_current()
        {
            self.error = Some(error.to_string());
            return None;
        }
        match result {
            ResultPage::Chats(Ok(page)) => self.page = Some(page),
            ResultPage::Archive(generation, Ok(page))
                if !running && generation == self.archive_generation.get() =>
            {
                return Some(Completion::Archive(page));
            }
            ResultPage::Archive(_, Ok(_)) => self.error = Some(FlowError::HistoryStale.to_string()),
            ResultPage::Navigation(Ok(bridge)) => return Some(Completion::Navigation(bridge.core)),
            ResultPage::Chats(Err(error))
            | ResultPage::Archive(_, Err(error))
            | ResultPage::Navigation(Err(error)) => self.error = Some(error.to_string()),
        }
        None
    }

    pub(super) fn view(&self, state: &State) -> El {
        let typography = state.preferences.typography();
        let Some(core) = state.core() else {
            return col();
        };
        let retry = matches!(*core.owner.borrow(), Owner::Navigating(_));
        let available = !self.busy && !state.running() && !retry;
        let mut out = col()
            .w_full()
            .gap(8.0)
            .child(typography.text("Saved chats").role("heading"))
            .child(button(
                typography,
                "Close",
                "saved-chats-close".into(),
                true,
                |state, tx| state.conversations.close(tx),
            ))
            .child(button(
                typography,
                "New chat",
                "saved-chats-new".into(),
                available,
                |state, _| action(state, Action::New),
            ));
        if self.busy {
            out = out.child(typography.text("Loading chats").role("status"));
        }
        if let Some(error) = &self.error {
            out = out.child(typography.text(error).wrap().role("alert"));
        }
        if retry {
            out = out.child(button(
                typography,
                "Try again",
                "saved-chats-retry".into(),
                !self.busy,
                |state, _| action(state, Action::Retry),
            ));
        }
        if let Some(page) = &self.page {
            if page.conversations.is_empty() {
                out = out.child(typography.text("No saved chats"));
            }
            for item in &page.conversations {
                let id = item.id.clone();
                let title = if item.title.text.is_empty() {
                    "New chat"
                } else {
                    &item.title.text
                };
                out = out.child(button(
                    typography,
                    title,
                    format!("saved-chat-{}", item.id),
                    available,
                    move |state, tx| {
                        if state
                            .core()
                            .is_some_and(|core| core.scope.conversation == id)
                        {
                            state.conversations.close(tx);
                        } else {
                            action(state, Action::Select(id.clone()));
                        }
                    },
                ));
            }
        }
        out.child(button(
            typography,
            "Recent chats",
            "saved-chats-recent".into(),
            !self.busy && !retry,
            |state, _| action(state, Action::Recent),
        ))
        .child(button(
            typography,
            "Older chats",
            "saved-chats-older".into(),
            !self.busy && !retry && self.page.as_ref().is_some_and(|p| p.older_cursor.is_some()),
            |state, _| action(state, Action::OlderChats),
        ))
    }

    pub(super) fn toolbar(&self, state: &State) -> El {
        let typography = state.preferences.typography();
        let available = !self.busy && !state.running();
        let mut out = row().gap(8.0).child(button(
            typography,
            "Older messages",
            "archive-older".into(),
            available && self.older_cursor.is_some(),
            |state, _| action(state, Action::OlderArchive),
        ));
        if let Some(error) = &self.error {
            out = out.child(typography.text(error).wrap().role("alert"));
        }
        out.child(button(
            typography,
            "Latest messages",
            "archive-latest".into(),
            available,
            |state, _| action(state, Action::LatestArchive),
        ))
    }
}

enum Action {
    New,
    Select(String),
    Retry,
    Recent,
    OlderChats,
    OlderArchive,
    LatestArchive,
}
fn action(state: &mut State, action: Action) {
    let (Some(core), Some(updates)) = (state.core().cloned(), state.updates.clone()) else {
        return;
    };
    if state.running()
        && matches!(
            action,
            Action::New | Action::Select(_) | Action::OlderArchive | Action::LatestArchive
        )
    {
        state.conversations.error = Some(FlowError::HistoryBusy.to_string());
        return;
    }
    match action {
        Action::New => state
            .conversations
            .navigate(&core, &updates, Some(Change::Create)),
        Action::Select(id) => {
            if !state
                .conversations
                .page
                .as_ref()
                .is_some_and(|page| page.conversations.iter().any(|item| item.id == id))
            {
                state.conversations.error = Some(FlowError::HistoryStale.to_string());
                return;
            }
            state
                .conversations
                .navigate(&core, &updates, Some(Change::Select { conversation: id }))
        }
        Action::Retry => state.conversations.navigate(&core, &updates, None),
        Action::Recent => state.conversations.load_chats(&core, &updates, None),
        Action::OlderChats => {
            let cursor = state
                .conversations
                .page
                .as_ref()
                .and_then(|p| p.older_cursor.clone());
            if cursor.is_some() {
                state.conversations.load_chats(&core, &updates, cursor);
            }
        }
        Action::OlderArchive => {
            let cursor = state.conversations.older_cursor.clone();
            if cursor.is_some() {
                state.conversations.load_archive(&core, &updates, cursor);
            }
        }
        Action::LatestArchive => state.conversations.load_archive(&core, &updates, None),
    }
}

fn button(
    typography: super::preferences::Typography,
    label: &str,
    key: String,
    enabled: bool,
    action: impl Fn(&mut State, &mut Tx) + 'static,
) -> El {
    let el = typography
        .text(label)
        .key(key)
        .role("button")
        .focusable(true)
        .disabled(!enabled)
        .padding(8.0);
    if !enabled {
        return el;
    }
    let action = Rc::new(action);
    let press = Rc::clone(&action);
    el.press(oklch(0.7, 0.08, 250.0).alpha(0.2))
        .on_press_tx(move |state: &mut State, tx| {
            state.sound(crate::interaction_audio::Cue::Page);
            press(state, tx)
        })
        .on_key(move |state: &mut State, key, tx| {
            if key.key != ::rune::Key::Enter {
                return false;
            }
            if !tx.key_repeat() {
                state.sound(crate::interaction_audio::Cue::Page);
                action(state, tx);
            }
            true
        })
}
