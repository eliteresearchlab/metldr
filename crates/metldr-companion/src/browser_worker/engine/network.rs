//! Network requests the owned tab made since its last top-level navigation (B4.13).
//! Only metadata is kept: no headers and no bodies. URLs go through the page log
//! sanitizer, and a URL that still looks like a secret is redacted whole.

use super::events::{
    CREDENTIAL, REDACTED, Subscriptions, URL_LIMIT, bounded, sanitize_url, secret,
};
use crate::Result;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// Most requests one tab keeps.
const ENTRY_LIMIT: usize = 100;
/// Most stored text bytes one tab keeps, so one result stays small.
const BYTE_LIMIT: usize = 24 * 1024;
const FIELD_LIMIT: usize = 64;

#[derive(Serialize)]
struct Entry {
    #[serde(skip)]
    id: String,
    method: String,
    url: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failed: Option<String>,
}

impl Entry {
    fn size(&self) -> usize {
        self.method.len()
            + self.url.len()
            + self.kind.as_ref().map_or(0, String::len)
            + self.mime.as_ref().map_or(0, String::len)
            + self.failed.as_ref().map_or(0, String::len)
    }
}

#[derive(Default)]
struct Log {
    entries: VecDeque<Entry>,
    bytes: usize,
    evicted: usize,
}

#[derive(Clone, Default)]
pub(super) struct NetworkLog(Arc<Mutex<Log>>);

impl NetworkLog {
    pub(super) fn register(&self, subscriptions: &mut Subscriptions) {
        let log = self.clone();
        subscriptions.subscribe("Network.requestWillBeSent", move |event| {
            log.request(&event.params)
        });
        let log = self.clone();
        subscriptions.subscribe("Network.responseReceived", move |event| {
            log.response(&event.params)
        });
        let log = self.clone();
        subscriptions.subscribe("Network.loadingFinished", move |event| {
            let size = event.params["encodedDataLength"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0);
            log.update(&event.params, |entry| entry.bytes = size.map(|n| n as u64));
        });
        let log = self.clone();
        subscriptions.subscribe("Network.loadingFailed", move |event| {
            // Only a bounded Chrome net error code is kept, never free text.
            let code = event.params["errorText"]
                .as_str()
                .filter(|code| {
                    code.strip_prefix("net::ERR_").is_some_and(|name| {
                        (1..=48).contains(&name.len())
                            && name
                                .bytes()
                                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                    })
                })
                .unwrap_or("failed")
                .to_owned();
            log.update(&event.params, |entry| entry.failed = Some(code));
        });
        let log = self.clone();
        subscriptions.subscribe("Page.frameNavigated", move |event| {
            if event.session.is_none() && event.params["frame"].get("parentId").is_none() {
                log.clear();
            }
        });
    }

    pub(super) fn snapshot(&self) -> Result<Value> {
        let log = self.0.lock().map_err(|_| "network log state unavailable")?;
        Ok(json!({
            "kind": "network_requests",
            "scope": "local_observation",
            "untrusted": true,
            "evicted": log.evicted,
            "requests": serde_json::to_value(&log.entries)?,
        }))
    }

    fn request(&self, params: &Value) {
        let (Some(id), Some(raw)) = (
            field(params, "requestId"),
            params["request"]["url"].as_str(),
        ) else {
            return;
        };
        let Ok(url) = reqwest::Url::parse(raw) else {
            return;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        let url = if secret(url.path()) || CREDENTIAL.is_match(raw) {
            REDACTED.to_owned()
        } else {
            bounded(&sanitize_url(raw), URL_LIMIT)
        };
        let entry = Entry {
            id,
            method: field(&params["request"], "method").unwrap_or_else(|| "GET".into()),
            url,
            kind: field(params, "type"),
            status: None,
            mime: None,
            bytes: None,
            failed: None,
        };
        let Ok(mut log) = self.0.lock() else {
            return;
        };
        // A redirect reuses the request id; the newest hop replaces the old entry.
        if let Some(index) = log.entries.iter().position(|old| old.id == entry.id) {
            let old = log.entries.remove(index).expect("index from position");
            log.bytes -= old.size();
        }
        log.bytes += entry.size();
        log.entries.push_back(entry);
        while log.entries.len() > ENTRY_LIMIT || log.bytes > BYTE_LIMIT {
            let Some(oldest) = log.entries.pop_front() else {
                break;
            };
            log.bytes -= oldest.size();
            log.evicted += 1;
        }
    }

    fn response(&self, params: &Value) {
        let status = params["response"]["status"]
            .as_u64()
            .and_then(|status| u16::try_from(status).ok())
            .filter(|status| (100..=599).contains(status));
        let mime = field(&params["response"], "mimeType");
        self.update(params, |entry| {
            entry.status = status;
            entry.mime = mime;
        });
    }

    fn update(&self, params: &Value, change: impl FnOnce(&mut Entry)) {
        let Some(id) = params["requestId"].as_str() else {
            return;
        };
        let Ok(mut log) = self.0.lock() else {
            return;
        };
        let Some(index) = log.entries.iter().position(|entry| entry.id == id) else {
            return;
        };
        let before = log.entries[index].size();
        change(&mut log.entries[index]);
        let after = log.entries[index].size();
        log.bytes = log.bytes - before + after;
    }

    fn clear(&self) {
        if let Ok(mut log) = self.0.lock() {
            *log = Log::default();
        }
    }
}

/// A short printable field, or nothing.
fn field(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| {
            !text.is_empty()
                && text.len() <= FIELD_LIMIT
                && text.bytes().all(|b| b.is_ascii_graphic())
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn requests_stay_bounded_redacted_and_clear_on_navigation() {
        let log = NetworkLog::default();
        log.request(&json!({"requestId":"1","type":"Document","request":{"method":"GET","url":"https://user:pw@shop.example/a?token=abc#x"}}));
        log.response(&json!({"requestId":"1","response":{"status":200,"mimeType":"text/html"}}));
        log.request(&json!({"requestId":"2","type":"XHR","request":{"method":"POST","url":"https://shop.example/reset-password/q1w2e3"}}));
        log.update(&json!({"requestId":"2"}), |entry| {
            entry.failed = Some("net::ERR_FAILED".into())
        });
        log.request(&json!({"requestId":"3","request":{"url":"data:text/plain,hi"}}));
        let snapshot = log.snapshot().unwrap();
        let requests = snapshot["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 2, "non http requests are dropped");
        assert_eq!(
            requests[0]["url"],
            "https://shop.example/a?token=%5Bredacted%5D"
        );
        assert_eq!(requests[0]["status"], 200);
        assert!(requests[0].get("id").is_none());
        assert_eq!(
            requests[1]["url"], REDACTED,
            "a secret looking path is redacted whole"
        );
        for index in 0..=ENTRY_LIMIT {
            log.request(&json!({"requestId":format!("n{index}"),"request":{"url":"https://shop.example/x"}}));
        }
        let snapshot = log.snapshot().unwrap();
        assert_eq!(snapshot["requests"].as_array().unwrap().len(), ENTRY_LIMIT);
        assert_eq!(snapshot["evicted"], 3);
        log.clear();
        assert!(
            log.snapshot().unwrap()["requests"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
