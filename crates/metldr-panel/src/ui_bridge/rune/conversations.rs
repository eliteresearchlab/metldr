use super::*;
use crate::history::Change;
use ::rune::Icon;

#[derive(Default)]
pub(super) struct Tabs {
    rows: Vec<Tab>,
    closing: Option<String>,
    selected_title: Option<(String, String)>,
}

struct Tab {
    id: String,
    title: String,
    draft: String,
}

impl Tabs {
    fn contains(&self, id: &str) -> bool {
        self.rows.iter().any(|tab| tab.id == id)
    }

    fn next_after_close(&self, id: &str) -> Option<String> {
        self.rows
            .iter()
            .rev()
            .find(|tab| tab.id != id)
            .map(|tab| tab.id.clone())
    }

    pub(super) fn save_draft(&mut self, id: &str, draft: &str) {
        if let Some(tab) = self.rows.iter_mut().find(|tab| tab.id == id) {
            tab.draft = draft.to_owned();
        }
    }

    pub(super) fn arrive(&mut self, id: &str) -> (Option<String>, bool) {
        let closing = self.closing.take();
        if let Some(closing) = &closing {
            self.rows.retain(|tab| &tab.id != closing);
        }
        if !self.contains(id) {
            self.rows.push(Tab {
                id: id.into(),
                title: self
                    .selected_title
                    .take()
                    .filter(|(selected, _)| selected == id)
                    .map(|(_, title)| title)
                    .unwrap_or_else(|| "New chat".into()),
                draft: String::new(),
            });
        }
        let draft = self
            .rows
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.draft.clone());
        (draft, closing.is_some())
    }

    fn titles(&mut self, page: &super::super::conversations::Page) {
        for tab in &mut self.rows {
            if let Some(item) = page
                .conversations
                .iter()
                .find(|item| item.id == tab.id && !item.title.text.is_empty())
            {
                tab.title.clone_from(&item.title.text);
            }
        }
    }
}

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

    pub(super) fn update_titles(&self, tabs: &mut Tabs) {
        if let Some(page) = &self.page {
            tabs.titles(page);
        }
    }

    pub(super) fn refresh_titles(&mut self, core: &Rc<Core>, updates: &Rc<Updates>) {
        self.load_chats(core, updates, None);
    }

    pub(super) fn tabs(&self, state: &State, core: &Core) -> El {
        let typography = state.preferences.typography();
        let p = state.preferences.palette();
        let enabled =
            !self.busy && !state.running() && !matches!(*core.owner.borrow(), Owner::Navigating(_));
        let mut out = row()
            .role("group")
            .label("Chats")
            .gap(4.0)
            .h(44.0)
            .px(8.0)
            .items_center();
        for tab in &state.tabs.rows {
            let id = tab.id.clone();
            let close_id = id.clone();
            let key_id = id.clone();
            let close_key_id = id.clone();
            let active = id == core.scope.conversation;
            let select = typography
                .text(&tab.title)
                .font(typography.font(12.5, 500.0))
                .color(if active { p.ink } else { p.ink_2 })
                .truncate()
                .grow(1.0)
                .min_w(0.0)
                .key(format!("chat-tab-{id}"))
                .role("toggle")
                .focusable(true)
                .disabled(!enabled)
                .checked(active)
                .native_activation()
                .on_press_tx(move |state: &mut State, _| {
                    if state
                        .core()
                        .is_some_and(|core| core.scope.conversation != id)
                    {
                        action(state, Action::Select(id.clone()));
                    }
                })
                .on_key(move |state: &mut State, key, tx| {
                    activate(key, tx, |_| {
                        if state
                            .core()
                            .is_some_and(|core| core.scope.conversation != key_id)
                        {
                            action(state, Action::Select(key_id.clone()));
                        }
                    })
                });
            let close = icon_button(&CLOSE, 11.0, 24.0, 5.0, p)
                .key(format!("chat-close-{}", tab.id))
                .label(format!("Close {}", tab.title))
                .disabled(!enabled)
                .native_activation()
                .on_press_tx(move |state: &mut State, tx| close_tab(state, &close_id, tx))
                .on_key(move |state: &mut State, key, tx| {
                    activate(key, tx, |tx| close_tab(state, &close_key_id, tx))
                });
            let tab = row()
                .w(144.0)
                .h(28.0)
                .pl(10.0)
                .pr(2.0)
                .gap(2.0)
                .radius(7.0)
                .items_center()
                .opacity(if enabled { 1.0 } else { 0.48 })
                .child(select)
                .child(close);
            out = out.child(if active {
                tab.fill(p.hover_2)
            } else {
                tab.hover(p.hover)
            });
        }
        out.child(
            icon_button(&super::composer::PLUS, 15.0, 28.0, 7.0, p)
                .key("chat-new")
                .label("New chat")
                .ml(2.0)
                .disabled(!enabled)
                .native_activation()
                .on_press_tx(|state: &mut State, _| action(state, Action::New))
                .on_key(|state: &mut State, key, tx| {
                    activate(key, tx, |_| action(state, Action::New))
                }),
        )
        .w_full()
        .scroll_x()
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
        let p = state.preferences.palette();
        let Some(core) = state.core() else {
            return col();
        };
        let retry = matches!(*core.owner.borrow(), Owner::Navigating(_));
        let available = !self.busy && !state.running() && !retry;
        let mut out = chat_list()
            .child(typography.text("Saved chats").role("heading"))
            .child(button(
                p,
                typography,
                "Close",
                "saved-chats-close".into(),
                true,
                |state, tx| state.conversations.close(tx),
            ))
            .child(button(
                p,
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
                p,
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
                    p,
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
            p,
            typography,
            "Recent chats",
            "saved-chats-recent".into(),
            !self.busy && !retry,
            |state, _| action(state, Action::Recent),
        ))
        .child(button(
            p,
            typography,
            "Older chats",
            "saved-chats-older".into(),
            !self.busy && !retry && self.page.as_ref().is_some_and(|p| p.older_cursor.is_some()),
            |state, _| action(state, Action::OlderChats),
        ))
    }

    /// Older and Latest messages, and a paging error, for the chat toolbar row.
    pub(super) fn toolbar(&self, state: &State) -> Vec<El> {
        let typography = state.preferences.typography();
        let p = state.preferences.palette();
        let available = !self.busy && !state.running();
        let mut out = vec![button(
            p,
            typography,
            "Older messages",
            "archive-older".into(),
            available && self.older_cursor.is_some(),
            |state, _| action(state, Action::OlderArchive),
        )];
        if let Some(error) = &self.error {
            out.push(typography.text(error).wrap().role("alert"));
        }
        out.push(button(
            p,
            typography,
            "Latest messages",
            "archive-latest".into(),
            available,
            |state, _| action(state, Action::LatestArchive),
        ));
        out
    }
}

const CLOSE: Icon = Icon {
    name: "metldr-close",
    view: 24.0,
    body: "<path d=\"M18 6 6 18M6 6l12 12\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.4\" stroke-linecap=\"round\"/>",
};

/// A square icon control: `size` points wide, icon `glyph` points, muted ink.
fn icon_button(icon: &'static Icon, glyph: f32, size: f32, radius: f32, p: &super::preferences::Palette) -> El {
    row()
        .size(size, size)
        .radius(radius)
        .center()
        .child(::rune::el::icon(icon, glyph).color(p.ink_3))
        .role("button")
        .focusable(true)
        .hover(p.hover)
        .press(p.hover_2)
        .press_scale(super::theme::PRESS_SCALE)
}

fn chat_list() -> El {
    col().w_full().gap(8.0).instant_changes()
}

fn activate(key: &::rune::KeyEvent, tx: &mut Tx, action: impl FnOnce(&mut Tx)) -> bool {
    if key.key != ::rune::Key::Enter && key.key != ::rune::Key::Text(" ".into()) {
        return false;
    }
    if !tx.key_repeat() {
        action(tx);
    }
    true
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
    if matches!(action, Action::New | Action::Select(_)) {
        if state.draft.preedit.is_some() {
            state.conversations.error = Some("Finish composing your draft first.".into());
            return;
        }
        state.tabs.closing = None;
    }
    match action {
        Action::New => state
            .conversations
            .navigate(&core, &updates, Some(Change::Create)),
        Action::Select(id) => {
            if !state.tabs.contains(&id)
                && !state
                    .conversations
                    .page
                    .as_ref()
                    .is_some_and(|page| page.conversations.iter().any(|item| item.id == id))
            {
                state.conversations.error = Some(FlowError::HistoryStale.to_string());
                return;
            }
            state.tabs.selected_title = state
                .conversations
                .page
                .as_ref()
                .and_then(|page| {
                    page.conversations
                        .iter()
                        .find(|item| item.id == id && !item.title.text.is_empty())
                })
                .map(|item| (item.id.clone(), item.title.text.clone()));
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

fn close_tab(state: &mut State, id: &str, tx: &mut Tx) {
    let Some(core) = state.core() else { return };
    let active = core.scope.conversation.clone();
    if id != active {
        state.tabs.rows.retain(|tab| tab.id != id);
        tx.focus(format!("chat-tab-{active}"));
        return;
    }
    let next = state.tabs.next_after_close(id);
    action(state, next.map_or(Action::New, Action::Select));
    if state.conversations.busy {
        state.tabs.closing = Some(id.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_chat_rows_do_not_move_after_the_list_updates() {
        fn view(loading: &bool, _: &mut Cx<bool>) -> El {
            let mut list = chat_list();
            if *loading {
                list = list.child(::rune::text("Loading chats").h(30.0));
            }
            list.child(::rune::text("North report").key("saved-chat"))
        }
        let mut rt = ::rune::Runtime::new(true, view, (360.0, 300.0), ::rune::Env::default());
        rt.frame(0.0);
        rt.update(|loading, _| *loading = false);
        rt.frame(0.0);
        let row = rt
            .elements()
            .into_iter()
            .find(|el| el.key.as_deref() == Some("saved-chat"))
            .unwrap();
        assert_eq!(row.rect, row.target);
    }

    #[test]
    fn opening_switching_closing_and_starting_a_chat_preserves_other_tabs() {
        let mut tabs = Tabs::default();
        tabs.arrive("first");
        tabs.save_draft("first", "unfinished");
        tabs.arrive("second");
        assert_eq!(tabs.arrive("first").0.as_deref(), Some("unfinished"));
        assert_eq!(tabs.next_after_close("first").as_deref(), Some("second"));
        tabs.closing = Some("first".into());
        assert_eq!(tabs.arrive("second"), (Some(String::new()), true));
        assert!(!tabs.contains("first"));
        tabs.closing = Some("second".into());
        assert_eq!(tabs.next_after_close("second"), None);
        tabs.arrive("new");
        assert!(tabs.contains("new"));
        assert!(!tabs.contains("second"));
    }
}

pub(super) fn button(
    p: &super::preferences::Palette,
    typography: super::preferences::Typography,
    label: &str,
    key: String,
    enabled: bool,
    action: impl Fn(&mut State, &mut Tx) + 'static,
) -> El {
    let el = row()
        .key(key)
        .min_h(32.0)
        .px(8.0)
        .items_center()
        .radius(8.0)
        .border(1.0, p.line)
        .fill(p.field)
        .child(
            typography
                .text(label)
                .font(typography.font(12.0, 400.0))
                .color(p.ink_2),
        )
        .role("button")
        .label(label)
        .focusable(true)
        .disabled(!enabled);
    if !enabled {
        return el.opacity(0.5);
    }
    let action = Rc::new(action);
    let press = Rc::clone(&action);
    el.hover(p.hover)
        .press(p.hover_2)
        .press_scale(super::theme::PRESS_SCALE)
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
