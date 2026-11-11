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
    borrow::Cow,
    collections::{BTreeMap, HashSet, VecDeque},
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

static SECRET: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(&format!("(?i:{SECRET_METADATA_PATTERN})")).ok());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s<>"']+"#).unwrap());
static PEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)-----BEGIN [A-Z ]*PRIVATE KEY-----").unwrap());
static CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)\b(?:bearer|basic)\s+[a-z0-9+/_.=-]+|\beyJ[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+|-----BEGIN [A-Z ]*PRIVATE KEY-----|\b(?:sk-[a-zA-Z0-9_-]{16,}|gh[pousr]_[a-zA-Z0-9_]{16,}|github_pat_[a-zA-Z0-9_]{16,}|AKIA[A-Z0-9]{16})\b"
).unwrap()
});

/// Whether text carries a secret label. An unreadable pattern counts as a match, so nothing leaks.
fn secret(text: &str) -> bool {
    SECRET.as_ref().is_none_or(|pattern| pattern.is_match(text))
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
    armed: HashSet<Option<String>>,
    epoch: u64,
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
        self.epoch = self.epoch.wrapping_add(1);
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
            logs.console_for(event.session.as_deref(), &event.params);
        });
        let logs = self.clone();
        subscriptions.subscribe("Log.entryAdded", move |event| {
            logs.log_entry_for(event.session.as_deref(), &event.params);
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
    pub(super) fn arm_session(&self, session: Option<&str>) {
        if let Ok(mut log) = self.0.lock() {
            log.armed.insert(session.map(str::to_owned));
        }
    }

    pub(super) fn retain_sessions(&self, owned: impl Fn(Option<&str>) -> bool) {
        if let Ok(mut log) = self.0.lock() {
            log.armed.retain(|session| owned(session.as_deref()));
        }
    }

    #[cfg(test)]
    fn arm(&self) {
        self.arm_session(None);
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

    #[cfg(test)]
    fn console(&self, params: &Value) {
        self.console_for(None, params);
    }

    fn console_for(&self, session: Option<&str>, params: &Value) {
        let args = params["args"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        // Keep the scan independent of the output limit, including labels spanning arguments.
        let mut tail = String::new();
        let mut sensitive = false;
        for arg in args.iter().take(ARGUMENT_LIMIT) {
            let part = argument(arg);
            let mut end = part.len().min(128);
            while !part.is_char_boundary(end) {
                end -= 1;
            }
            tail.push(' ');
            tail.push_str(&part[..end]);
            sensitive |= secret(&part)
                || secret(&tail)
                || CREDENTIAL.is_match(&part)
                || CREDENTIAL.is_match(&tail);
            let mut start = part.len().saturating_sub(128);
            while !part.is_char_boundary(start) {
                start += 1;
            }
            tail.clear();
            tail.push_str(&part[start..]);
        }
        let mut text = String::new();
        if sensitive {
            text.push_str(REDACTED);
        } else {
            for (index, arg) in args.iter().take(ARGUMENT_LIMIT).enumerate() {
                if text.len() >= TEXT_LIMIT {
                    break;
                }
                if index > 0 {
                    text.push(' ');
                }
                text.push_str(&sanitize(&argument(arg)));
                text = bounded(&text, TEXT_LIMIT);
            }
        }
        let url = params["stackTrace"]["callFrames"][0]["url"].as_str();
        self.push(
            session,
            "console",
            params["type"].as_str().unwrap_or("log"),
            &text,
            url,
        );
    }

    #[cfg(test)]
    fn log_entry(&self, params: &Value) {
        self.log_entry_for(None, params);
    }

    fn log_entry_for(&self, session: Option<&str>, params: &Value) {
        let entry = &params["entry"];
        self.push(
            session,
            "log",
            entry["level"].as_str().unwrap_or("info"),
            entry["text"].as_str().unwrap_or_default(),
            entry["url"].as_str(),
        );
    }

    fn push(
        &self,
        session: Option<&str>,
        source: &'static str,
        level: &str,
        text: &str,
        url: Option<&str>,
    ) {
        let Ok(mut log) = self.0.lock() else {
            return;
        };
        if !log.armed.contains(&session.map(str::to_owned)) {
            return;
        }
        let entry = Entry {
            source,
            level: bounded(level, TEXT_LIMIT),
            text: if secret(text) || PEM.is_match(text) {
                REDACTED.to_owned()
            } else {
                bounded(&sanitize(text), TEXT_LIMIT)
            },
            url: url
                .filter(|url| !secret(url))
                .map(|url| bounded(&sanitize_url(url), URL_LIMIT)),
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
fn argument(arg: &Value) -> Cow<'_, str> {
    if let Some(value) = arg.get("value") {
        return match value {
            Value::String(text) => Cow::Borrowed(text),
            other => Cow::Owned(other.to_string()),
        };
    }
    Cow::Borrowed(
        ["unserializableValue", "description", "type"]
            .into_iter()
            .find_map(|key| arg[key].as_str())
            .unwrap_or_default(),
    )
}

fn sanitize_url(value: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(value) else {
        return REDACTED.to_owned();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return REDACTED.to_owned();
    }
    if url.set_username("").is_err() || url.set_password(None).is_err() {
        return REDACTED.to_owned();
    }
    url.set_fragment(None);
    if url.query().is_some() {
        let keys: Vec<_> = url.query_pairs().map(|(key, _)| key.into_owned()).collect();
        url.set_query(None);
        let mut query = url.query_pairs_mut();
        for key in keys {
            query.append_pair(&key, REDACTED);
        }
    }
    url.into()
}

fn sanitize(text: &str) -> String {
    let urls = URL.replace_all(text, |captures: &regex::Captures<'_>| {
        sanitize_url(&captures[0])
    });
    CREDENTIAL.replace_all(&urls, REDACTED).into_owned()
}

/// The longest prefix of `text` within `limit` bytes that ends on a character boundary.
fn bounded(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(start) = text[..end].rfind('[') {
        if text[start..].starts_with(REDACTED) && start + REDACTED.len() > end {
            end = start;
        }
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
        assert!(entry.to_string().contains("code="));
        let sanitized = sanitize_url("https://user:pass@example.test/cb?code=private#fragment");
        assert!(
            !sanitized.contains("user")
                && !sanitized.contains("pass")
                && !sanitized.contains("private")
        );
        assert_eq!(sanitize_url("https://%"), REDACTED);
        assert_eq!(sanitize("Bearer abcdef"), REDACTED);
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
        subscriptions.offer(
            &event(None, "Page.frameNavigated", json!({"frame":{"id":"main"}})),
            owned,
        );
        subscriptions.offer(&console(None, "new page"), owned);
        assert_eq!(texts(&logs), vec!["new page"]);
        logs.arm_session(Some("child"));
        subscriptions.offer(&console(Some("child"), "live"), owned);
        logs.retain_sessions(|session| session.is_none());
        subscriptions.offer(&console(Some("child"), "after detach"), owned);
        assert_eq!(texts(&logs), vec!["new page", "live"]);
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
