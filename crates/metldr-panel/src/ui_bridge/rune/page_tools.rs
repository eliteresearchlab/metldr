//! Settings for the page tools: the word popup, the dictionaries it reads offline, the cached
//! page summaries and the guide. The dictionaries belong to the service worker, which downloads
//! them; this shows what it reports and asks it for changes.

use super::preferences::{Palette, Typography, switch};
use super::*;
use ::rune::{El, col, row};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use wasm_bindgen::closure::Closure;

const POPUP: &str = "wordPopupEnabled";
/// The service worker's dictionary keys; a change to any of them is a new state to show.
const DICTIONARY_KEYS: [&str; 3] = ["dictDownloadProgress", "downloadingLanguages", "selectedLanguages"];

#[derive(Default, serde::Deserialize)]
struct Dictionaries {
    languages: Vec<Language>,
    selected: Vec<String>,
    downloaded: Vec<String>,
    /// Each running download's percent.
    progress: BTreeMap<String, u8>,
}

#[derive(serde::Deserialize)]
struct Language {
    code: String,
    name: String,
}

#[derive(Default)]
struct Inbox {
    popup: Option<bool>,
    dictionaries: Option<Dictionaries>,
    cleared: Option<bool>,
}

#[derive(Default)]
pub(super) struct PageTools {
    inbox: Rc<RefCell<Inbox>>,
    wake: Option<::rune::Waker>,
    listener: Option<(JsValue, Closure<dyn FnMut(JsValue, JsValue)>)>,
    popup: bool,
    dictionaries: Option<Dictionaries>,
    /// The last clear of the cached summaries: whether it worked.
    cleared: Option<bool>,
}

impl Drop for PageTools {
    fn drop(&mut self) {
        if let Some((event, callback)) = self.listener.take() {
            let _ = crate::call(&event, "removeListener", &[callback.as_ref().clone()]);
        }
    }
}

impl PageTools {
    /// Reads the word popup setting and the dictionaries, and follows their changes. Reading
    /// the dictionaries resumes their downloads, as opening the panel always has.
    pub(super) fn setup(&mut self, wake: ::rune::Waker) {
        self.wake = Some(wake);
        let Ok(event) = crate::chrome_api("storage").and_then(|s| crate::get(&s, "onChanged")) else {
            return;
        };
        let (inbox, wake) = (Rc::clone(&self.inbox), self.wake.clone());
        let callback = Closure::<dyn FnMut(JsValue, JsValue)>::new(move |changes: JsValue, area: JsValue| {
            if area.as_string().as_deref() != Some("local") {
                return;
            }
            let has = |key: &str| crate::get(&changes, key).is_ok_and(|v| !v.is_undefined());
            if has(POPUP) {
                let on = crate::get(&changes, POPUP).and_then(|c| crate::get(&c, "newValue"));
                inbox.borrow_mut().popup = Some(on.ok().and_then(|v| v.as_bool()) != Some(false));
                if let Some(wake) = &wake {
                    wake();
                }
            }
            if DICTIONARY_KEYS.iter().any(|key| has(key)) {
                ask(&inbox, wake.clone(), json!({"type": "DICTIONARY", "action": "state"}));
            }
        });
        if crate::call(&event, "addListener", &[callback.as_ref().clone()]).is_ok() {
            self.listener = Some((event, callback));
        }
        let (inbox, wake) = (Rc::clone(&self.inbox), self.wake.clone());
        spawn_local(async move {
            let stored = match crate::chrome_api("storage").and_then(|s| crate::get(&s, "local")) {
                Ok(local) => crate::await_call(&local, "get", &[POPUP.into()]).await.ok(),
                Err(_) => None,
            };
            let on = stored.and_then(|items| crate::get(&items, POPUP).ok()).and_then(|v| v.as_bool());
            inbox.borrow_mut().popup = Some(on != Some(false));
            if let Some(wake) = &wake {
                wake();
            }
        });
        ask(&self.inbox, self.wake.clone(), json!({"type": "DICTIONARY", "action": "state"}));
    }

    /// Takes what arrived since the last frame; true when the section changes.
    pub(super) fn poll(&mut self) -> bool {
        let mut inbox = self.inbox.borrow_mut();
        let changed = inbox.popup.is_some() || inbox.dictionaries.is_some() || inbox.cleared.is_some();
        self.popup = inbox.popup.take().unwrap_or(self.popup);
        if let Some(dictionaries) = inbox.dictionaries.take() {
            self.dictionaries = Some(dictionaries);
        }
        if let Some(cleared) = inbox.cleared.take() {
            self.cleared = Some(cleared);
        }
        changed
    }

    fn set_popup(&mut self, on: bool) {
        self.popup = on;
        let items = js_sys::Object::new();
        let set = js_sys::Reflect::set(&items, &POPUP.into(), &on.into());
        if let (Ok(_), Ok(local)) = (set, crate::chrome_api("storage").and_then(|s| crate::get(&s, "local"))) {
            let _ = crate::call(&local, "set", &[items.into()]);
        }
    }

    fn dictionary(&self, action: &str, code: &str, on: bool) {
        ask(&self.inbox, self.wake.clone(), json!({"type": "DICTIONARY", "action": action, "code": code, "on": on}));
    }

    fn clear_summaries(&mut self) {
        self.cleared = None;
        let (inbox, wake) = (Rc::clone(&self.inbox), self.wake.clone());
        spawn_local(async move {
            let reply = gateway::ask_extension(&json!({"type": "CLEAR_SUMMARY_CACHE"})).await;
            inbox.borrow_mut().cleared = Some(reply.is_some_and(|r| r["ok"] == true));
            if let Some(wake) = &wake {
                wake();
            }
        });
    }

    pub(super) fn view(&self, typography: Typography, palette: &Palette) -> El {
        let popup = switch(typography, palette, "page-tools-popup", "Word lookup on selection", self.popup, true, |state: &mut State, on, _| {
            state.sound(crate::interaction_audio::Cue::Press);
            state.page_tools.set_popup(on);
        });
        let clear = super::preferences::outlined(typography, "Clear cached summaries", "page-tools-clear").border(1.0, typography.line);
        let clear = metldr_ui::button::button(clear, Default::default(), |state: &mut State, _| {
            state.sound(crate::interaction_audio::Cue::Release);
            state.page_tools.clear_summaries();
        })
        .press(typography.press);
        let guide = super::preferences::outlined(typography, "Open the guide", "page-tools-guide").border(1.0, typography.line);
        let guide = metldr_ui::button::button(guide, Default::default(), |state: &mut State, _| {
            state.sound(crate::interaction_audio::Cue::Press);
            state.open_setup();
        })
        .press(typography.press);
        let mut out = col().w_full().gap(8.0).child(popup).child(row().w_full().gap(8.0).flex_wrap().child(clear).child(guide));
        if let Some(cleared) = self.cleared {
            let notice = if cleared { "Cached summaries cleared." } else { "The cached summaries could not be cleared." };
            out = out.child(typography.text(notice).wrap().key("page-tools-clear-status").role(if cleared { "status" } else { "alert" }));
        }
        out = out.child(typography.text("Dictionaries").role("heading"));
        match &self.dictionaries {
            None => out = out.child(typography.text("Loading dictionaries.").key("page-tools-dictionaries-status")),
            Some(found) => {
                let rows = found.languages.iter().map(|language| self.language(typography, palette, found, language));
                out = out.child(col().key("page-tools-dictionaries").label("Dictionaries").w_full().gap(2.0).children(rows));
            }
        }
        out
    }

    /// One language: a switch that downloads and uses it, its download percent or Ready, and
    /// Delete once it is downloaded.
    fn language(&self, typography: Typography, palette: &Palette, found: &Dictionaries, language: &Language) -> El {
        let code = language.code.clone();
        let on = found.selected.contains(&code);
        let ready = found.downloaded.contains(&code);
        let status = match found.progress.get(&code) {
            Some(percent) => format!("{percent}%"),
            None if ready => "Ready".into(),
            None => String::new(),
        };
        let key = format!("page-tools-language-{code}");
        let toggle = {
            let code = code.clone();
            move |state: &mut State, on, _: &mut Tx| {
                state.sound(crate::interaction_audio::Cue::Press);
                state.page_tools.dictionary("select", &code, on);
            }
        };
        let label = if status.is_empty() { language.name.clone() } else { format!("{}, {status}", language.name) };
        let mut line = row().w_full().gap(8.0).items_center().child(
            switch(typography, palette, &key, &label, on, true, toggle).grow(1.0),
        );
        if ready {
            let delete = super::preferences::outlined(typography, "Delete", format!("page-tools-delete-{code}"))
                .label(format!("Delete {}", language.name))
                .border(1.0, typography.line);
            line = line.child(
                metldr_ui::button::button(delete, Default::default(), move |state: &mut State, _| {
                    state.sound(crate::interaction_audio::Cue::Release);
                    state.page_tools.dictionary("delete", &code, false);
                })
                .press(typography.press),
            );
        }
        line
    }
}

/// Sends `request` to the service worker and puts its dictionary state in `inbox`.
fn ask(inbox: &Rc<RefCell<Inbox>>, wake: Option<::rune::Waker>, request: Value) {
    let inbox = Rc::clone(inbox);
    spawn_local(async move {
        let reply = gateway::ask_extension(&request).await;
        if let Some(found) = reply.and_then(|r| serde_json::from_value::<Dictionaries>(r).ok()) {
            inbox.borrow_mut().dictionaries = Some(found);
            if let Some(wake) = &wake {
                wake();
            }
        }
    });
}
