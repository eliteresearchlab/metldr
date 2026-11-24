//! The panel without the companion: page questions answered through the extension's model
//! gateway (the `metldr-gateway` port in the service worker), with Chrome's built-in AI, a local
//! server or the reader's own key. The chat lives in memory.

use super::*;
use crate::ag_ui::AgUiMessage;
use js_sys::Promise;
use serde_json::{Value, json};
use wasm_bindgen::closure::Closure;

const DETECT: u64 = 1;
/// The search messages the extension reads: the question and the six before it.
const SEARCH_TURNS: usize = 7;
/// The page text a conversation's first turn sends, in bytes.
const PAGE_BUDGET: usize = crate::MAX_PAGE;
/// The unsent page text a later turn may add for its request, in bytes.
const MORE_BUDGET: usize = 4_096;
/// How an attached document's source is named, in place of a page URL.
pub(super) const DOCUMENT: &str = "file:";
/// Each page's chat, kept in `chrome.storage.local` under this prefix and the page's key, so it
/// survives the panel closing and comes back with the page.
const SAVED: &str = "metldr.gateway.chat:";
/// The saved pages' keys, least recent first; past [`KEPT_CHATS`] the oldest chat goes.
const INDEX: &str = "metldr.gateway.chats";
const KEPT_CHATS: usize = 50;
/// Turns sent to the model: the first exchange (with the page) and the latest ones.
const KEPT_TURNS: usize = 24;
const KEPT_SHOWN: usize = 50;

/// The chat as saved: the page, what the model saw and what the reader saw.
#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    page: Option<String>,
    turns: Vec<(String, String)>,
    shown: Vec<(String, String)>,
    /// The page's sections, numbered from one, for its citations.
    sections: metldr_core::page::Sections,
    /// Each shown answer's citations: section number, its opening words and the page.
    cited: Vec<Vec<Citation>>,
}

/// One citation under an answer: the section number, its opening words and the page it is on.
pub(super) type Citation = (usize, String, String);

/// One ready gateway route the reader can pick.
pub(super) struct RouteChoice {
    route: String,
    model: Option<String>,
    /// As people read it.
    pub(super) label: String,
}

pub(super) struct Gateway {
    port: JsValue,
    /// The ready routes, and the one that answers.
    pub(super) routes: Vec<RouteChoice>,
    chosen: Cell<usize>,
    next: Cell<u64>,
    streaming: Cell<Option<u64>>,
    /// What the model has seen so far: role and content, the page source in the first turn.
    turns: RefCell<Vec<(&'static str, String)>>,
    /// The page the conversation is about, by URL.
    page: RefCell<Option<String>>,
    /// Each question with its answer, as the reader saw them, and its answer's message id.
    shown: RefCell<Vec<(String, String)>>,
    shown_ids: RefCell<Vec<String>>,
    /// The current page's sections, numbered from one in what the model saw.
    sections: RefCell<metldr_core::page::Sections>,
    /// Citations by answer message id.
    citations: RefCell<std::collections::HashMap<String, Vec<Citation>>>,
    /// A page summary or a search answer is on its way.
    waiting: Cell<bool>,
    /// Questions go to everything the reader has read and saved, not the page.
    everywhere: Cell<bool>,
    /// The search chat as the extension's search takes it: each question and answer.
    searches: RefCell<Vec<(&'static str, String)>>,
    /// The page last summarized, by URL, so opening the panel again does not repeat it.
    summarized: RefCell<Option<String>>,
    /// An attached document, by name with its markdown: the source in place of the page until
    /// New chat.
    /// The markdown is empty once the sections hold it.
    document: RefCell<Option<(String, String)>>,
    /// The page the shown chat belongs to, by [`page_key`].
    key: RefCell<String>,
    /// A page the reader moved to while an answer was on its way; its chat follows once idle.
    pending: RefCell<Option<String>>,
}

/// Posts `message` on `port`.
fn post(port: &JsValue, message: &Value) -> Result<(), FlowError> {
    let message = crate::flow::json_value(message)?;
    crate::call(port, "postMessage", &[message])
        .map(|_| ())
        .map_err(|_| FlowError::Host("The model gateway closed. Reload the panel.".into()))
}

/// A JavaScript message as JSON.
pub(super) fn json_of(message: &JsValue) -> Option<Value> {
    let raw = js_sys::JSON::stringify(message).ok()?.as_string()?;
    serde_json::from_str(&raw).ok()
}

impl Gateway {
    /// The gateway with a ready route, or None when no route can answer.
    pub(super) async fn open(updates: &Rc<Updates>) -> Option<Rc<Self>> {
        let runtime = crate::chrome_api("runtime").ok()?;
        let name = crate::flow::json_value(&json!({"name":"metldr-gateway"})).ok()?;
        let port = crate::call(&runtime, "connect", &[name]).ok()?;
        // The detect reply resolves this promise; stream events go to the opened gateway.
        let resolve: Rc<RefCell<Option<js_sys::Function>>> = Rc::default();
        let promise = Promise::new(&mut |done, _| *resolve.borrow_mut() = Some(done));
        let opened: Rc<RefCell<Option<Rc<Self>>>> = Rc::default();
        let (weak, slot) = (Rc::downgrade(updates), Rc::clone(&opened));
        let listener = Closure::<dyn FnMut(JsValue)>::new(move |message: JsValue| {
            let Some(value) = json_of(&message) else { return };
            if value["id"] == DETECT {
                if let Some(done) = resolve.borrow_mut().take() {
                    let _ = done.call1(&JsValue::NULL, &message);
                }
                return;
            }
            let gateway = slot.borrow().clone();
            if let (Some(gateway), Some(updates)) = (gateway, weak.upgrade()) {
                gateway.event(&value, &updates);
            }
        });
        let on_message = crate::get(&port, "onMessage").ok()?;
        crate::call(&on_message, "addListener", &[listener.as_ref().clone()]).ok()?;
        listener.forget();
        post(&port, &json!({"id":DETECT,"op":"detect"})).ok()?;
        let reply = json_of(&wasm_bindgen_futures::JsFuture::from(promise).await.ok()?)?;
        let (routes, chosen) = ready_routes(&reply)?;
        let gateway = Rc::new(Self {
            port,
            routes,
            chosen: Cell::new(chosen),
            next: Cell::new(DETECT + 1),
            streaming: Cell::new(None),
            turns: RefCell::default(),
            page: RefCell::default(),
            shown: RefCell::default(),
            shown_ids: RefCell::default(),
            waiting: Cell::new(false),
            everywhere: Cell::new(false),
            searches: RefCell::default(),
            summarized: RefCell::default(),
            document: RefCell::default(),
            key: RefCell::default(),
            pending: RefCell::default(),
            sections: RefCell::default(),
            citations: RefCell::default(),
        });
        load_index().await;
        if let Ok((_, url)) = active_tab().await {
            *gateway.key.borrow_mut() = page_key(&url);
            gateway.restore(updates).await;
        }
        *opened.borrow_mut() = Some(Rc::clone(&gateway));
        gateway.watch_tabs(updates);
        Some(gateway)
    }

    /// The route that answers, as people read it.
    pub(super) fn route(&self) -> &str {
        &self.routes[self.chosen.get()].label
    }

    pub(super) fn chosen(&self) -> usize {
        self.chosen.get()
    }

    /// The sections `answer` cites, with their opening words and the current page.
    fn cite(&self, answer: &str) -> Vec<Citation> {
        let sections = self.sections.borrow();
        let page = self.page.borrow().clone().unwrap_or_default();
        let cited = metldr_core::page::cited(answer, sections.len());
        cited.into_iter().filter_map(|n| Some((n, sections.opening(n, 8)?, page.clone()))).collect()
    }

    /// The citations under answer `id`.
    pub(super) fn citations(&self, id: &str) -> Vec<Citation> {
        self.citations.borrow().get(id).cloned().unwrap_or_default()
    }

    /// Answers with route `index` from now on, and keeps it as the gateway's preferred route.
    pub(super) fn choose(&self, index: usize) -> Result<(), FlowError> {
        let Some(choice) = self.routes.get(index) else { return Ok(()) };
        self.chosen.set(index);
        let id = self.next.get();
        self.next.set(id + 1);
        let model = choice.model.clone().unwrap_or_default();
        post(&self.port, &json!({"id":id,"op":"prefer","route":choice.route,"model":model}))
    }

    /// Whether questions go to everything the reader has read, not the page.
    pub(super) fn everywhere(&self) -> bool {
        self.everywhere.get()
    }

    /// Sends questions to everything read, or back to the page; an earlier error no longer applies.
    pub(super) fn set_everywhere(&self, on: bool, updates: &Updates) {
        self.everywhere.set(on);
        updates.presentation.borrow_mut().error = None;
        updates.changed();
    }

    /// Asks `text` of everything the reader has read and saved, through the extension's search;
    /// the sources the answer cites open their pages.
    async fn search(self: Rc<Self>, text: String, updates: Rc<Updates>) -> Result<(), FlowError> {
        let id = format!("gateway-{}", self.next.get());
        self.next.set(self.next.get() + 1);
        push_bounded(&mut self.searches.borrow_mut(), ("user", text.clone()));
        show(&updates, &id, text.clone(), String::new());
        updates.presentation.borrow_mut().error = None;
        self.waiting.set(true);
        updates.changed();

        let reply = self.search_reply().await;
        self.waiting.set(false);
        let Some(answer) = reply["content"].as_str().filter(|_| reply["ok"] == true) else {
            self.searches.borrow_mut().pop();
            updates.changed();
            let error = reply["error"].as_str().unwrap_or("Search could not answer.");
            return Err(FlowError::Host(error.to_owned()));
        };

        self.citations.borrow_mut().insert(id.clone(), search_citations(answer, &reply["sources"]));
        push_bounded(&mut self.searches.borrow_mut(), ("assistant", answer.to_owned()));
        let mut presentation = updates.presentation.borrow_mut();
        if let Some(AgUiMessage::Assistant { content, .. }) = presentation.messages.messages.last_mut() {
            answer.clone_into(content);
        }
        drop(presentation);
        self.shown.borrow_mut().push((text, answer.to_owned()));
        self.shown_ids.borrow_mut().push(id);
        self.persist();
        updates.changed();
        Ok(())
    }

    /// The extension's search reply to the search chat so far.
    async fn search_reply(&self) -> Value {
        let message = |(role, content): &(&str, String)| json!({"role":role,"content":content});
        let messages: Vec<Value> = self.searches.borrow().iter().map(message).collect();
        ask_extension(&json!({"type":"GLOBAL_CHAT","messages":messages})).await.unwrap_or_default()
    }

    pub(super) fn streaming(&self) -> bool {
        self.streaming.get().is_some() || self.waiting.get()
    }

    /// The active tab's summary from the extension's page tools: cached per page and, when not
    /// `manual`, only where the summary settings call for one. It joins the chat as an exchange.
    pub(super) async fn summarize(
        self: Rc<Self>,
        updates: Rc<Updates>,
        manual: bool,
    ) -> Result<(), FlowError> {
        let (_, url) = active_tab().await?;
        if !manual && self.summarized.borrow().as_deref() == Some(url.as_str()) {
            return Ok(());
        }
        self.waiting.set(true);
        updates.changed();
        // The page is read as the chat reads it (a PDF tab's file included), then summarized.
        let reply = match capture(true).await {
            Ok(page) => {
                let trigger = if manual { "manual" } else { "auto" };
                let mut about = page.about;
                about["title"] = Value::String(page.title);
                about["content"] = Value::String(page.markdown);
                if page.pdf {
                    about["contentType"] = "application/pdf".into();
                }
                let request = json!({"type":"SUMMARIZE_PAGE","tabId":page.tab,"force":manual,"trigger":trigger,"page":about});
                ask_extension(&request).await.unwrap_or_default()
            }
            Err(error) => json!({"error": error.to_string()}),
        };
        self.waiting.set(false);
        updates.changed();
        let Some(text) = summary_text(&reply["summary"]).filter(|_| reply["success"] == true) else {
            return match reply["error"].as_str() {
                Some(error) if manual => Err(FlowError::Host(error.to_owned())),
                _ if manual && reply["skip"] == true => {
                    Err(FlowError::Host("This page has too little text to summarize.".into()))
                }
                _ => Ok(()),
            };
        };
        *self.summarized.borrow_mut() = Some(url);
        let id = self.next.get();
        self.next.set(id + 1);
        show(&updates, &format!("gateway-{id}"), "Summarize page".into(), text.clone());
        self.shown.borrow_mut().push(("Summarize page".into(), text));
        self.shown_ids.borrow_mut().push(format!("gateway-{id}"));
        self.persist();
        updates.changed();
        Ok(())
    }

    /// Asks `text` about the page in the active tab, capturing it when the conversation starts
    /// or the tab shows another page.
    pub(super) async fn ask(self: Rc<Self>, text: String, updates: Rc<Updates>) -> Result<(), FlowError> {
        if self.everywhere.get() {
            return self.search(text, updates).await;
        }
        // An attached document's text moves into the sections on its first turn; later turns
        // find it there, so it is never copied or read again.
        let attached = self.document.borrow_mut().as_mut().map(|(name, markdown)| {
            (format!("{DOCUMENT}{name}"), name.clone(), std::mem::take(markdown))
        });
        let (url, title, markdown) = match attached {
            Some(attached) => attached,
            None => {
                let page = capture(false).await?;
                self.switch(&page_key(&page.url), &updates).await;
                (page.url, page.title, page.markdown)
            }
        };
        let same_page = self.page.borrow().as_deref() == Some(url.as_str());
        // The page goes into one buffer as numbered sections: the whole page when it fits, else
        // its outline and the sections ranked for the request; later turns add unsent sections.
        let mut sections = self.sections.borrow_mut();
        let mut content = String::new();
        if same_page {
            if !markdown.is_empty() {
                sections.refresh(markdown);
            }
        } else {
            *sections = metldr_core::page::Sections::new(markdown);
            content = format!("Source: {title} ({url})\n\n");
            *self.page.borrow_mut() = Some(url);
            if !sections.fits(PAGE_BUDGET) {
                content.push_str("The page is longer than this message. ");
                content.push_str("Its outline, by section and size:\n");
                sections.write_outline(PAGE_BUDGET / 4, &mut content);
                content.push_str("\nThe sections for this request:\n\n");
            }
        }
        let budget = if same_page { MORE_BUDGET } else { PAGE_BUDGET.saturating_sub(content.len()) };
        let picked = sections.pick(&text, budget);
        if same_page && !picked.is_empty() {
            content.push_str("More from the page:\n\n");
        }
        sections.write_numbered(&picked, &mut content);
        drop(sections);
        if !content.is_empty() {
            content.push_str("\n\nRequest: ");
        }
        content.push_str(&text);
        let id = self.next.get();
        self.next.set(id + 1);
        self.turns.borrow_mut().push(("user", content));
        self.shown.borrow_mut().push((text.clone(), String::new()));
        self.shown_ids.borrow_mut().push(format!("gateway-{id}"));
        show(&updates, &format!("gateway-{id}"), text, String::new());
        updates.presentation.borrow_mut().error = None;
        self.streaming.set(Some(id));
        let turns = self.turns.borrow();
        let messages: Vec<Value> = kept(&turns)
            .map(|(role, content)| json!({"role":role,"content":content}))
            .collect();
        drop(turns);
        let choice = &self.routes[self.chosen.get()];
        let request = json!({"id":id,"op":"stream","route":choice.route,"model":choice.model,
            "system":metldr_core::page::ANSWER,"messages":messages});
        post(&self.port, &request)?;
        updates.changed();
        Ok(())
    }

    /// Makes the document `name`, read as `markdown`, the chat's source and asks for its summary.
    pub(super) async fn attach(
        self: Rc<Self>,
        name: String,
        markdown: String,
        updates: Rc<Updates>,
    ) -> Result<(), FlowError> {
        self.everywhere.set(false);
        *self.document.borrow_mut() = Some((name, markdown));
        self.ask("Summarize this document.".into(), updates).await
    }

    pub(super) fn stop(&self) -> Result<(), FlowError> {
        match self.streaming.get() {
            Some(id) => post(&self.port, &json!({"id":id,"op":"stop"})),
            None => Ok(()),
        }
    }

    /// One stream event: a delta grows the answer, done settles it, an error says why.
    fn event(&self, value: &Value, updates: &Updates) {
        if self.streaming.get().is_none_or(|id| value["id"] != id) {
            return;
        }
        let mut presentation = updates.presentation.borrow_mut();
        let last = presentation.messages.messages.last_mut();
        let Some(AgUiMessage::Assistant { id, content }) = last else { return };
        match value["type"].as_str() {
            Some("delta") => content.push_str(value["text"].as_str().unwrap_or_default()),
            Some("done") => {
                if let Some(text) = value["text"].as_str() {
                    text.clone_into(content);
                }
                self.turns.borrow_mut().push(("assistant", content.clone()));
                let cites = self.cite(content);
                self.citations.borrow_mut().insert(id.clone(), cites);
                if let Some(last) = self.shown.borrow_mut().last_mut() {
                    last.1.clone_from(content);
                }
                self.streaming.set(None);
                self.persist();
            }
            _ => {
                // The question goes unanswered, so the model will not see it next turn.
                self.turns.borrow_mut().pop();
                self.shown.borrow_mut().pop();
                self.shown_ids.borrow_mut().pop();
                self.streaming.set(None);
                let message = value["message"].as_str().unwrap_or("The model could not answer.");
                presentation.error = Some(message.to_owned());
            }
        }
        drop(presentation);
        updates.changed();
    }
}

impl Gateway {
    /// The chat to save, bounded: the first exchange and the latest turns, the latest answers.
    /// Follows the reader across tabs and pages: a tab switch or a navigation in the active tab
    /// brings that page's chat.
    fn watch_tabs(self: &Rc<Self>, updates: &Rc<Updates>) {
        let Ok(tabs) = crate::chrome_api("tabs") else { return };
        let (gateway, updates) = (Rc::downgrade(self), Rc::downgrade(updates));
        let follow = Rc::new(move || {
            let (Some(gateway), Some(updates)) = (gateway.upgrade(), updates.upgrade()) else { return };
            spawn_local(async move {
                if let Ok((_, url)) = active_tab().await {
                    gateway.follow(&url, &updates).await;
                }
            });
        });
        let activated = Rc::clone(&follow);
        let on_activated = Closure::<dyn FnMut(JsValue)>::new(move |_: JsValue| activated());
        // Only a changed address counts; titles, icons and loading states do not.
        let on_updated = Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_: JsValue, change: JsValue| {
            if crate::get(&change, "url").is_ok_and(|url| !url.is_undefined()) {
                follow();
            }
        });
        let listeners = [("onActivated", on_activated.as_ref()), ("onUpdated", on_updated.as_ref())];
        for (event, listener) in listeners {
            if let Ok(event) = crate::get(&tabs, event) {
                let _ = crate::call(&event, "addListener", &[listener.clone()]);
            }
        }
        // They live as long as the panel.
        on_activated.forget();
        on_updated.forget();
    }

    /// Saves the shown chat under its page.
    fn persist(&self) {
        let key = self.key.borrow().clone();
        if !key.is_empty() {
            spawn_local(save(key, self.saved()));
        }
    }

    /// Shows the chat for the page at `url`, when the reader moved to another page and no
    /// answer is on its way.
    pub(super) async fn follow(&self, url: &str, updates: &Updates) {
        if self.busy() {
            *self.pending.borrow_mut() = Some(url.to_owned());
            return;
        }
        self.switch(&page_key(url), updates).await;
    }

    fn busy(&self) -> bool {
        self.streaming() || self.waiting.get()
    }

    /// The page the reader moved to while busy, once the answer is in.
    pub(super) fn idle_pending(&self) -> Option<String> {
        if self.busy() { None } else { self.pending.take() }
    }

    /// Puts the chat saved for page `key` in place of the shown one, which is already saved.
    async fn switch(&self, key: &str, updates: &Updates) {
        if *self.key.borrow() == key {
            return;
        }
        key.clone_into(&mut self.key.borrow_mut());
        self.turns.borrow_mut().clear();
        self.shown.borrow_mut().clear();
        self.shown_ids.borrow_mut().clear();
        self.citations.borrow_mut().clear();
        self.sections.take();
        self.page.take();
        self.document.take();
        updates.presentation.borrow_mut().messages.messages.clear();
        self.restore(updates).await;
        updates.changed();
    }

    fn saved(&self) -> Saved {
        let turns = self.turns.borrow();
        let turns = kept(&turns).map(|(role, text)| ((*role).to_owned(), text.clone())).collect();
        let (shown, ids) = (self.shown.borrow(), self.shown_ids.borrow());
        let start = shown.len().saturating_sub(KEPT_SHOWN);
        let citations = self.citations.borrow();
        Saved {
            page: self.page.borrow().clone(),
            turns,
            shown: shown[start..].to_vec(),
            sections: self.sections.borrow().clone(),
            cited: ids[start..].iter().map(|id| citations.get(id).cloned()).map(Option::unwrap_or_default).collect(),
        }
    }

    /// Brings back the saved chat: the model's turns and the reader's messages.
    async fn restore(&self, updates: &Updates) {
        let key = self.key.borrow().clone();
        let Some(saved) = load(&key).await else { return };
        let turns = saved.turns.into_iter().map(|(name, text)| Some((role(&name)?, text)));
        let Some(turns) = turns.collect::<Option<Vec<_>>>() else { return };
        *self.turns.borrow_mut() = turns;
        *self.page.borrow_mut() = saved.page;
        let mut citations = self.citations.borrow_mut();
        for (n, (question, answer)) in saved.shown.iter().enumerate() {
            let id = format!("gateway-saved-{n}");
            show(updates, &id, question.clone(), answer.clone());
            citations.insert(id.clone(), saved.cited.get(n).cloned().unwrap_or_default());
            self.shown_ids.borrow_mut().push(id);
        }
        *self.shown.borrow_mut() = saved.shown;
        *self.sections.borrow_mut() = saved.sections;
    }

    /// Starts over: the model forgets the page and the chat, and the saved copy goes.
    pub(super) fn clear(&self, updates: &Updates) {
        if self.streaming() {
            return;
        }
        self.turns.borrow_mut().clear();
        self.shown.borrow_mut().clear();
        self.shown_ids.borrow_mut().clear();
        self.searches.borrow_mut().clear();
        self.citations.borrow_mut().clear();
        self.sections.take();
        *self.page.borrow_mut() = None;
        *self.summarized.borrow_mut() = None;
        self.document.take();
        updates.presentation.borrow_mut().messages.messages.clear();
        spawn_local(forget(self.key.borrow().clone()));
        updates.changed();
    }
}

/// The turns the model sees: all of a short chat, else the first exchange (it holds the page)
/// and the latest turns.
fn kept<T>(turns: &[T]) -> impl Iterator<Item = &T> {
    let (head, tail) = if turns.len() > KEPT_TURNS {
        (&turns[..2], &turns[turns.len() - (KEPT_TURNS - 2)..])
    } else {
        (turns, &turns[..0])
    };
    head.iter().chain(tail)
}

/// A saved turn's role, from the two the model takes.
fn role(name: &str) -> Option<&'static str> {
    ["user", "assistant"].into_iter().find(|role| *role == name)
}

/// Shows one question and its answer (empty while it streams) under ids from `key`.
fn show(updates: &Updates, key: &str, question: String, answer: String) {
    let shown = &mut updates.presentation.borrow_mut().messages.messages;
    shown.push(AgUiMessage::User { id: format!("{key}-user"), content: question });
    shown.push(AgUiMessage::Assistant { id: key.to_owned(), content: answer });
}

fn storage() -> Option<JsValue> {
    crate::get(&crate::chrome_api("storage").ok()?, "local").ok()
}

/// A page as its chat is keyed: its URL without the fragment, except on Gmail, where the
/// fragment names the open thread.
fn page_key(url: &str) -> String {
    match url.split_once('#') {
        Some((page, _)) if !page.contains("://mail.google.com/") => page.to_owned(),
        _ => url.to_owned(),
    }
}

async fn read(key: &str) -> Option<String> {
    let items = crate::await_call(&storage()?, "get", &[JsValue::from_str(key)]).await.ok()?;
    crate::get(&items, key).ok()?.as_string()
}

async fn write(pairs: &[(&str, &str)], gone: &[String]) {
    let Some(storage) = storage() else { return };
    let items = js_sys::Object::new();
    for (key, value) in pairs {
        let _ = js_sys::Reflect::set(&items, &(*key).into(), &(*value).into());
    }
    let _ = crate::await_call(&storage, "set", &[items.into()]).await;
    if !gone.is_empty() {
        let gone: js_sys::Array = gone.iter().map(|key| JsValue::from_str(key)).collect();
        let _ = crate::await_call(&storage, "remove", &[gone.into()]).await;
    }
}

thread_local! {
    /// The saved pages' keys, read once when the gateway opens; every save updates it before it
    /// writes, so two saves never race on the stored copy.
    static KEYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

async fn load_index() {
    let keys = read(INDEX).await.and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default();
    KEYS.with_borrow_mut(|k| *k = keys);
}

/// `key` made the most recent (or removed), the index as stored and the chats that fell off.
fn reindex(key: &str, keep: bool) -> (String, Vec<String>) {
    KEYS.with_borrow_mut(|keys| {
        keys.retain(|k| k != key);
        if keep {
            keys.push(key.to_owned());
        }
        let excess = keys.len().saturating_sub(KEPT_CHATS);
        let gone = keys.drain(..excess).map(|k| format!("{SAVED}{k}")).collect();
        (serde_json::to_string(&*keys).unwrap_or_default(), gone)
    })
}

async fn load(key: &str) -> Option<Saved> {
    serde_json::from_str(&read(&format!("{SAVED}{key}")).await?).ok()
}

/// Saves page `key`'s chat in one write with the index, the page now most recent; past
/// [`KEPT_CHATS`] the oldest chats are removed.
async fn save(key: String, saved: Saved) {
    let Ok(raw) = serde_json::to_string(&saved) else { return };
    let (keys, gone) = reindex(&key, true);
    write(&[(&format!("{SAVED}{key}"), &raw), (INDEX, &keys)], &gone).await;
}

/// Deletes page `key`'s chat.
async fn forget(key: String) {
    let (keys, mut gone) = reindex(&key, false);
    gone.push(format!("{SAVED}{key}"));
    write(&[(INDEX, &keys)], &gone).await;
}

/// The ready routes, named for people, and the preferred one (else the first).
fn ready_routes(reply: &Value) -> Option<(Vec<RouteChoice>, usize)> {
    let preferred = reply["preferred"]["route"].as_str();
    let routes: Vec<RouteChoice> = reply["routes"]
        .as_array()?
        .iter()
        .filter(|route| route["state"] == "ready")
        .filter_map(|route| {
            let name = route["route"].as_str()?;
            let model = route["model"].as_str().filter(|model| !model.is_empty());
            let model = model.map(str::to_owned);
            let label = match (name, &model) {
                ("chrome-ai", _) => "Chrome built-in AI".to_owned(),
                (name, Some(model)) => format!("{model}, {name}"),
                (name, None) => name.to_owned(),
            };
            Some(RouteChoice { route: name.to_owned(), model, label })
        })
        .collect();
    let chosen = routes.iter().position(|choice| Some(choice.route.as_str()) == preferred).unwrap_or(0);
    (!routes.is_empty()).then_some((routes, chosen))
}

/// What a PDF without a text layer says.
pub(super) const SCANNED: &str =
    "This PDF has no text to read. Scanned pages need text recognition, which is not available yet.";

/// The extension's reply to `request` sent to its service worker, or None.
pub(super) async fn ask_extension(request: &Value) -> Option<Value> {
    let runtime = crate::chrome_api("runtime").ok()?;
    let message = crate::flow::json_value(request).ok()?;
    json_of(&crate::await_call(&runtime, "sendMessage", &[message]).await.ok()?)
}

/// The sources `answer` cites, each by number with its title and the page it opens.
fn search_citations(answer: &str, sources: &Value) -> Vec<Citation> {
    let sources = sources.as_array().map(Vec::as_slice).unwrap_or_default();
    let numbered = |n: usize| sources.iter().find(|s| s["index"].as_u64() == Some(n as u64));
    let cite = |n: usize| {
        let source = numbered(n)?;
        let title = source["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("Source");
        Some((n, title.to_owned(), source_page(source["url"].as_str()?)?))
    };
    metldr_core::page::cited(answer, sources.len()).into_iter().filter_map(cite).collect()
}

/// Adds `turn` to the search chat, which keeps only what the extension reads.
fn push_bounded(searches: &mut Vec<(&'static str, String)>, turn: (&'static str, String)) {
    if searches.len() == SEARCH_TURNS {
        searches.remove(0);
    }
    searches.push(turn);
}

/// The page a search source opens: its address, or the Gmail thread of a saved email.
fn source_page(url: &str) -> Option<String> {
    match url.strip_prefix("email://") {
        Some(email) => {
            let thread = email.rsplit('/').next()?;
            Some(format!("https://mail.google.com/mail/u/0/#inbox/{thread}"))
        }
        None => url.starts_with("http").then(|| url.to_owned()),
    }
}

/// A page summary as markdown: its title in bold, then its points.
fn summary_text(summary: &Value) -> Option<String> {
    let bullets: Vec<&str> = summary["bullets"].as_array()?.iter().filter_map(Value::as_str).collect();
    if bullets.is_empty() {
        return None;
    }
    let mut text = summary["title"].as_str().map_or_else(String::new, |title| format!("**{title}**\n\n"));
    text.push_str(&bullets.iter().map(|bullet| format!("- {bullet}")).collect::<Vec<_>>().join("\n"));
    Some(text)
}

/// The active tab: its id and address.
async fn active_tab() -> Result<(f64, String), FlowError> {
    let failure = || FlowError::Host("Select a browser tab.".into());
    let tabs = crate::chrome_api("tabs").map_err(|_| failure())?;
    let query = crate::flow::json_value(&json!({"active":true,"lastFocusedWindow":true}))?;
    let found = crate::await_call(&tabs, "query", &[query]).await;
    let tab = js_sys::Array::from(&found.map_err(|_| failure())?).get(0);
    let id = crate::get(&tab, "id").ok().and_then(|id| id.as_f64()).ok_or_else(failure)?;
    let url = crate::string_property(&tab, "url").map_err(|_| failure())?;
    Ok((id, url))
}

/// Selects `words` in the active tab when it shows `page` and scrolls them into view; opens
/// `page` in a new tab when the reader has moved on.
pub(super) async fn show_words(page: &str, words: &str) -> Result<(), FlowError> {
    let failure = || FlowError::Host("The page could not be shown.".into());
    let (id, current) = active_tab().await?;
    if current.split('#').next() != Some(page) {
        let details = js_sys::Object::new();
        js_sys::Reflect::set(&details, &"url".into(), &page.into()).map_err(|_| failure())?;
        let tabs = crate::chrome_api("tabs").map_err(|_| failure())?;
        return crate::call(&tabs, "create", &[details.into()]).map(|_| ()).map_err(|_| failure());
    }
    let script = crate::flow::capture_function().ok_or_else(failure)?;
    let details = crate::flow::json_value(&json!({"target":{"tabId":id},"args":[words]}))?;
    js_sys::Reflect::set(&details, &"func".into(), script.as_ref()).map_err(|_| failure())?;
    let scripting = crate::chrome_api("scripting").map_err(|_| failure())?;
    crate::await_call(&scripting, "executeScript", &[details]).await.map(|_| ()).map_err(|_| failure())
}

/// The active tab's page, captured without the companion.
struct Captured {
    tab: f64,
    url: String,
    title: String,
    /// The main content as markdown, or a PDF's text.
    markdown: String,
    pdf: bool,
    /// When asked for: what the summary settings judge the page by (`pageSignals`) and its byline.
    about: Value,
}

/// The active tab's main content as markdown; with `about`, also its signals and byline.
async fn capture(about: bool) -> Result<Captured, FlowError> {
    let failure = |message: &str| FlowError::Host(message.into());
    let api = |name: &str| crate::chrome_api(name).map_err(|_| failure("The browser API is unavailable."));
    let (id, url) = active_tab().await?;
    let grant = crate::flow::json_value(&json!({"origins":[crate::site::permission(&url)?]}))?;
    let granted = crate::await_call(&api("permissions")?, "contains", &[grant]).await;
    if granted.ok().and_then(|granted| granted.as_bool()) != Some(true) {
        return Err(failure("Allow metldr on this site in the extension's site access first."));
    }
    let script = crate::flow::capture_function().ok_or_else(|| failure("Page capture is unavailable."))?;
    let details = crate::flow::json_value(&json!({"target":{"tabId":id},"args":[null, about]}))?;
    js_sys::Reflect::set(&details, &"func".into(), script.as_ref())
        .map_err(|_| failure("Page capture is unavailable."))?;
    let values = crate::await_call(&api("scripting")?, "executeScript", &[details])
        .await
        .map_err(|error| FlowError::Host(crate::js_message(&error)))?;
    let observed = crate::get(&js_sys::Array::from(&values).get(0), "result")
        .map_err(|_| failure("The page could not be read."))?;
    let title = crate::string_property(&observed, "title").unwrap_or_default();
    // A PDF tab shows the browser's viewer, so the file is read instead of the page.
    let pdf = crate::get(&observed, "pdf").is_ok_and(|pdf| pdf.is_truthy());
    let mut described = json!({});
    if about && !pdf {
        let meta = crate::get(&observed, "meta").ok().and_then(|meta| json_of(&meta)).unwrap_or_default();
        described = json!({
            "pageSignals": crate::get(&observed, "signals").ok().and_then(|signals| json_of(&signals)),
            "author": meta["author"], "publication": meta["publication"], "publishDate": meta["publishDate"],
        });
    }
    let markdown = if pdf {
        let text = crate::flow::read_pdf(&url).await?;
        if text.is_empty() {
            return Err(failure(SCANNED));
        }
        text
    } else {
        crate::flow::page_text(&observed, &url, crate::reading::FULL_PAGE).await?
    };
    Ok(Captured { tab: id, url, title, markdown, pdf, about: described })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_long_chat_keeps_its_first_exchange_and_latest_turns() {
        let turns: Vec<u32> = (0..30).collect();
        let kept: Vec<u32> = super::kept(&turns).copied().collect();
        assert_eq!(kept.len(), super::KEPT_TURNS);
        assert_eq!(&kept[..3], &[0, 1, 8]);
        assert_eq!(kept.last(), Some(&29));
        assert_eq!(super::kept(&turns[..5]).count(), 5);
    }
}
