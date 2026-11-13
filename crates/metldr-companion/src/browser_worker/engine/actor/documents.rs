//! HTTP status and redirect chain of recent document loads, read from Network events
//! (B3.4). Only `type: "Document"` requests count, keyed by loader, so a navigation can
//! report the evidence for exactly the document it confirmed.
use super::*;
use crate::browser_action::{NavigationEvidence, REDIRECT_LIMIT};
use std::collections::VecDeque;

/// Most document loads remembered at once.
const LOADER_LIMIT: usize = 8;

#[derive(Default)]
pub(super) struct Documents(VecDeque<(String, NavigationEvidence)>);

impl Documents {
    pub(super) fn event(&mut self, event: &runtime::RawEvent) {
        // The method is a cheap string check; most events stop here before any JSON lookup.
        let request = match event.method.as_ref() {
            "Network.requestWillBeSent" => true,
            "Network.responseReceived" => false,
            _ => return,
        };
        let params = &event.params;
        if params["type"] != "Document" {
            return;
        }
        let Some(loader) = params["loaderId"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128)
        else {
            return;
        };
        match request {
            true => {
                // The chain reaches the model, so query values and fragments are removed:
                // an OAuth callback code must not leave the page through a redirect.
                let redirect = params["redirectResponse"]["url"]
                    .as_str()
                    .map(super::super::events::sanitize_url);
                let entry = self.entry(loader);
                if let Some(url) = redirect
                    && entry.redirects.len() < REDIRECT_LIMIT
                    && url.len() <= 2048
                    && crate::model::validate_page_url(&url).is_ok()
                {
                    entry.redirects.push(url);
                }
            }
            false => {
                let status = params["response"]["status"]
                    .as_u64()
                    .and_then(|status| u16::try_from(status).ok())
                    .filter(|status| (100..=599).contains(status));
                self.entry(loader).status = status;
            }
        }
    }

    /// Takes the evidence for `loader`; absent evidence is empty, not an error.
    pub(super) fn take(&mut self, loader: &str) -> NavigationEvidence {
        self.0
            .iter()
            .position(|(key, _)| key == loader)
            .and_then(|index| self.0.remove(index))
            .map(|(_, evidence)| evidence)
            .unwrap_or_default()
    }

    fn entry(&mut self, loader: &str) -> &mut NavigationEvidence {
        let index = match self.0.iter().position(|(key, _)| key == loader) {
            Some(index) => index,
            None => {
                if self.0.len() == LOADER_LIMIT {
                    self.0.pop_front();
                }
                self.0
                    .push_back((loader.to_owned(), NavigationEvidence::default()));
                self.0.len() - 1
            }
        };
        &mut self.0[index].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(method: &str, params: Value) -> runtime::RawEvent {
        serde_json::from_value(json!({"method": method, "params": params})).unwrap()
    }

    #[test]
    fn records_redirects_and_final_status_for_document_loads_only() {
        let mut documents = Documents::default();
        documents.event(&event(
            "Network.requestWillBeSent",
            json!({"type":"Document","loaderId":"L1","request":{"url":"http://127.0.0.1/a"}}),
        ));
        documents.event(&event(
            "Network.requestWillBeSent",
            json!({"type":"Document","loaderId":"L1","redirectResponse":{"url":"http://127.0.0.1/a?code=secret#f","status":302}}),
        ));
        documents.event(&event(
            "Network.responseReceived",
            json!({"type":"Image","loaderId":"L1","response":{"status":404}}),
        ));
        documents.event(&event(
            "Network.responseReceived",
            json!({"type":"Document","loaderId":"L1","response":{"url":"http://127.0.0.1/b","status":200}}),
        ));
        let evidence = documents.take("L1");
        assert_eq!(evidence.status, Some(200));
        assert_eq!(
            evidence.redirects,
            ["http://127.0.0.1/a?code=%5Bredacted%5D"]
        );
        assert!(documents.take("L1").is_empty());
        for index in 0..=LOADER_LIMIT {
            documents.event(&event(
                "Network.responseReceived",
                json!({"type":"Document","loaderId":format!("L{index}"),"response":{"status":200}}),
            ));
        }
        assert!(
            documents.take("L0").is_empty(),
            "the oldest loader is dropped"
        );
    }
}
