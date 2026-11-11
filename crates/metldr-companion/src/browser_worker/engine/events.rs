//! CDP event subscriptions for the tab this actor owns.
//!
//! A feature registers handlers for the CDP methods it needs. The actor offers
//! each subscribed event to those handlers only when the event belongs to a
//! session of the owned tab, so a handler never sees another tab's events.

use super::{runtime::RawEvent, scripts::SECRET_METADATA_PATTERN};
use crate::Result;
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

type Handler = Box<dyn FnMut(&RawEvent) + Send>;

/// Handlers by CDP method. Features register once, when the actor is built.
#[derive(Default)]
pub(super) struct Subscriptions {
    handlers: BTreeMap<&'static str, Vec<Handler>>,
}

impl Subscriptions {
    pub(super) fn subscribe(
        &mut self,
        method: &'static str,
        handler: impl FnMut(&RawEvent) + Send + 'static,
    ) {
        self.handlers
            .entry(method)
            .or_default()
            .push(Box::new(handler));
    }

    /// Offers an event to the handlers for its method. Returns `false` when the
    /// event came from a session this tab does not own, and the caller must drop
    /// it. A method without handlers returns `true`, so the caller handles it as usual.
    pub(super) fn offer(&mut self, event: &RawEvent, owned: impl Fn(Option<&str>) -> bool) -> bool {
        let method: &str = &event.method;
        let Some(handlers) = self.handlers.get_mut(method) else {
            return true;
        };
        if !owned(event.session.as_deref()) {
            return false;
        }
        for handler in handlers {
            handler(event);
        }
        true
    }
}

/// Most entries one tab keeps. Matches the upstream page log cap.
const ENTRY_LIMIT: usize = 50;
/// Most stored level, text and URL bytes one tab keeps, so one `page_logs` result stays small.
const BYTE_LIMIT: usize = 16 * 1024;
/// Longest text kept for one entry. Longer text is cut at a character boundary.
const TEXT_LIMIT: usize = 512;
const URL_LIMIT: usize = 256;
/// Console arguments read for one message.
const ARGUMENT_LIMIT: usize = 32;
const REDACTED: &str = "[redacted]";

static SECRET: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(SECRET_METADATA_PATTERN).ok());

/// Whether text carries a secret label. An unreadable pattern counts as a match, so nothing leaks.
fn secret(text: &str) -> bool {
    SECRET
        .as_ref()
        .is_none_or(|pattern| pattern.is_match(&text.to_lowercase()))
}

#[derive(Serialize)]
struct Entry {
    source: &'static str,
    level: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

impl Entry {
    fn bytes(&self) -> usize {
        self.level.len() + self.text.len() + self.url.as_ref().map_or(0, String::len)
    }
}

#[derive(Default)]
struct Log {
    /// Recording starts only after the browser has replayed earlier output.
    armed: bool,
    entries: VecDeque<Entry>,
    bytes: usize,
    evicted: usize,
}

impl Log {
    fn push(&mut self, entry: Entry) {
        self.bytes += entry.bytes();
        self.entries.push_back(entry);
        while self.entries.len() > ENTRY_LIMIT || self.bytes > BYTE_LIMIT {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= oldest.bytes();
            self.evicted += 1;
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.evicted = 0;
    }
}

/// Console messages and log entries captured for the owned tab.
#[derive(Clone, Default)]
pub(super) struct PageLogs(Arc<Mutex<Log>>);

impl PageLogs {
    /// Registers the page log handlers. Each closure keeps a handle to the same buffer.
    pub(super) fn register(&self, subscriptions: &mut Subscriptions) {
        let logs = self.clone();
        subscriptions.subscribe("Runtime.consoleAPICalled", move |event| {
            logs.console(&event.params);
        });
        let logs = self.clone();
        subscriptions.subscribe("Log.entryAdded", move |event| {
            logs.log_entry(&event.params);
        });
        let logs = self.clone();
        subscriptions.subscribe("Page.frameNavigated", move |event| {
            // Only the top document replaces the page. A subframe load has a parent frame.
            if event.session.is_none() && event.params["frame"].get("parentId").is_none() {
                logs.clear();
            }
        });
    }

    /// Starts recording. The actor calls this after `Log.enable` answers, because
    /// the browser replays earlier console and log output in reply to it.
    pub(super) fn arm(&self) {
        if let Ok(mut log) = self.0.lock() {
            log.armed = true;
        }
    }

    pub(super) fn snapshot(&self) -> Result<Value> {
        let log = self.0.lock().map_err(|_| "page log state unavailable")?;
        let entries = serde_json::to_value(&log.entries)?;
        Ok(json!({
            "kind": "page_logs",
            "scope": "local_observation",
            "untrusted": true,
            "evicted": log.evicted,
            "entries": entries,
        }))
    }

    fn console(&self, params: &Value) {
        let level = params["type"].as_str().unwrap_or("log");
        let text = params["args"]
            .as_array()
            .map(|args| {
                args.iter()
                    .take(ARGUMENT_LIMIT)
                    .map(argument)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let url = params["stackTrace"]["callFrames"][0]["url"].as_str();
        self.push("console", level, &text, url);
    }

    fn log_entry(&self, params: &Value) {
        let entry = &params["entry"];
        self.push(
            "log",
            entry["level"].as_str().unwrap_or("info"),
            entry["text"].as_str().unwrap_or_default(),
            entry["url"].as_str(),
        );
    }

    fn push(&self, source: &'static str, level: &str, text: &str, url: Option<&str>) {
        let Ok(mut log) = self.0.lock() else {
            return;
        };
        if !log.armed {
            return;
        }
        let entry = Entry {
            source,
            level: bounded(level, TEXT_LIMIT),
            // Redaction reads the whole text, before it is cut to the entry limit.
            text: if secret(text) {
                REDACTED.to_owned()
            } else {
                bounded(text, TEXT_LIMIT)
            },
            url: url
                .filter(|url| !secret(url))
                .map(|url| bounded(url, URL_LIMIT)),
        };
        log.push(entry);
    }

    fn clear(&self) {
        if let Ok(mut log) = self.0.lock() {
            log.clear();
        }
    }
}

/// Text for one console argument, as the page would print it.
fn argument(arg: &Value) -> String {
    if let Some(value) = arg.get("value") {
        return match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
    }
    ["unserializableValue", "description", "type"]
        .into_iter()
        .find_map(|key| arg[key].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// The longest prefix of `text` within `limit` bytes that ends on a character boundary.
fn bounded(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod checks {
    use super::*;

    fn event(session: Option<&str>, method: &str, params: Value) -> RawEvent {
        RawEvent {
            method: method.to_owned().into(),
            session: session.map(str::to_owned),
            params,
        }
    }

    fn console(session: Option<&str>, text: &str) -> RawEvent {
        event(
            session,
            "Runtime.consoleAPICalled",
            json!({"type":"log","args":[{"type":"string","value":text}]}),
        )
    }

    fn texts(logs: &PageLogs) -> Vec<String> {
        logs.snapshot().unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["text"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn page_logs_stay_bounded_scoped_redacted_and_clear_on_navigation() {
        let logs = PageLogs::default();
        let mut subscriptions = Subscriptions::default();
        logs.register(&mut subscriptions);
        logs.arm();
        let owned = |session: Option<&str>| session.is_none();

        for n in 0..=ENTRY_LIMIT {
            subscriptions.offer(&console(None, &format!("line {n}")), owned);
        }
        assert_eq!(
            texts(&logs),
            (1..=ENTRY_LIMIT)
                .map(|n| format!("line {n}"))
                .collect::<Vec<_>>()
        );

        let before = texts(&logs).len();
        subscriptions.offer(
            &event(
                None,
                "Page.frameNavigated",
                json!({"frame":{"id":"main","url":"https://example.test/"}}),
            ),
            owned,
        );
        assert_eq!((before, texts(&logs).len()), (ENTRY_LIMIT, 0));

        subscriptions.offer(&console(None, "password: hunter2"), owned);
        assert_eq!(texts(&logs), vec![REDACTED]);

        let kept = texts(&logs);
        let delivered = subscriptions.offer(&console(Some("other-tab"), "leak"), owned);
        assert_eq!((delivered, texts(&logs)), (false, kept));
    }
    #[test]
    fn callback_url_values_are_redacted() {
        let logs = PageLogs::default();
        logs.arm();
        logs.log_entry(&json!({"entry":{"text":"callback https://example.test/cb?code=private#fragment", "url":"https://example.test/cb?code=private#fragment"}}));
        let entry = &logs.snapshot().unwrap()["entries"][0];
        assert!(!entry.to_string().contains("private"));
        assert!(!entry.to_string().contains("fragment"));
    }

    #[test]
    fn child_replay_is_dropped_and_root_survives_navigation() {
        let logs = PageLogs::default();
        let mut subscriptions = Subscriptions::default();
        logs.register(&mut subscriptions);
        logs.arm();
        let owned = |_: Option<&str>| true;
        subscriptions.offer(&console(Some("child"), "replay"), owned);
        assert!(texts(&logs).is_empty());
        subscriptions.offer(&event(None, "Page.frameNavigated", json!({"frame":{"id":"main"}})), owned);
        subscriptions.offer(&console(None, "new page"), owned);
        assert_eq!(texts(&logs), vec!["new page"]);
    }

    #[test]
    fn last_argument_label_redacts_complete_message() {
        let logs = PageLogs::default();
        logs.arm();
        let mut args = vec![json!({"value":"x".repeat(40)}); 31];
        args.push(json!({"value":"password: private"}));
        logs.console(&json!({"args":args}));
        assert_eq!(texts(&logs), vec![REDACTED]);
    }

}
