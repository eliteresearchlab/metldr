use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Prompt, Result, TextEvent, sse, stream};
use metldr_browser_tools::{BROWSER_POLICY, definitions};
use stream::Turn;

/// Cheap to clone: clones share one connection pool.
#[derive(Clone)]
pub struct Client {
    http: sse::http::Http,
    model: String,
    text_max: Option<usize>,
}

/// Response timeout when the connection does not set one.
pub const DEFAULT_TIMEOUT_SECS: u16 = 45;
/// Shortest and longest response timeout a connection may set.
pub const TIMEOUT_SECS: std::ops::RangeInclusive<u16> = 5..=600;

impl Client {
    pub fn new(
        endpoint: &str,
        model: &str,
        bearer_token: Option<&str>,
        trust_ca_pem: Option<&[u8]>,
        timeout_secs: u16,
    ) -> Result<Self> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| "invalid Chat endpoint")?;
        let loopback = url.scheme() == "http" && loopback_host(&url);
        if endpoint.len() > 1024
            || (url.scheme() != "https" && !loopback)
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
        {
            return Err("invalid Chat endpoint".into());
        }
        if model.is_empty() || model.len() > 128 || !model.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("invalid Chat model".into());
        }
        if !TIMEOUT_SECS.contains(&timeout_secs) {
            return Err("invalid Chat response timeout".into());
        }
        // A compatible endpoint may run without a key; then no authorization header is sent.
        let authorization = bearer_token
            .map(|token| {
                if token.is_empty()
                    || token.len() > 4096
                    || !token.bytes().all(|b| b.is_ascii_graphic())
                {
                    return Err("invalid Chat credential");
                }
                Ok(format!("Bearer {token}"))
            })
            .transpose()?;
        let localhost = loopback && url.host_str() == Some("localhost");
        let http = sse::http::Http::new(sse::http::Options {
            provider: "Chat",
            url: url.as_str(),
            timeout_secs,
            localhost_port: if localhost {
                url.port_or_known_default()
            } else {
                None
            },
            trust_ca_pem,
            headers: &[],
            secret: authorization
                .as_deref()
                .map(|value| ("authorization", value)),
        })?;
        Ok(Self {
            http,
            model: model.into(),
            text_max: None,
        })
    }

    /// Streams one text response without offering tools. Partial text can precede an error.
    pub async fn text(
        self,
        system: &str,
        input: &str,
        stop: &AtomicBool,
        emit: impl FnMut(TextEvent) -> Result<()>,
    ) -> Result<crate::Usage> {
        self.converse(system, &[sse::Turn::user(input)], stop, emit)
            .await
    }

    /// Streams the reply to a conversation that ends with the user's turn, without tools; the
    /// reply may be as long as a page tool answer (64 KB).
    pub async fn turns(
        mut self,
        system: &str,
        turns: &[sse::Turn<'_>],
        stop: &AtomicBool,
        emit: impl FnMut(TextEvent) -> Result<()>,
    ) -> Result<crate::Usage> {
        self.text_max = Some(65_536);
        self.converse(system, turns, stop, emit).await
    }

    async fn converse(
        self,
        system: &str,
        turns: &[sse::Turn<'_>],
        stop: &AtomicBool,
        emit: impl FnMut(TextEvent) -> Result<()>,
    ) -> Result<crate::Usage> {
        if turns.is_empty() || turns.len() > metldr_conversation::MAX_MESSAGES {
            return Err("text conversation has no turns or too many".into());
        }
        let turn = self
            .send(&self.text_request(system, turns), stop, emit)
            .await?;
        if !turn.calls.is_empty() {
            return Err("Chat text request returned a tool call".into());
        }
        Ok(turn.usage.unwrap_or_default())
    }

    fn text_request(&self, system: &str, turns: &[sse::Turn<'_>]) -> Value {
        let mut messages = vec![json!({"role":"system","content":system})];
        for turn in turns {
            let role = if turn.assistant { "assistant" } else { "user" };
            messages.push(json!({"role": role, "content": turn.text}));
        }
        json!({"model":self.model,"messages":messages,
            "stream":true,"stream_options":{"include_usage":true},"max_tokens":4096})
    }

    pub(super) fn request(&self, prompt: Prompt<'_>) -> Value {
        let context = json!({"instruction":prompt.instruction,"source":{
            "title":prompt.title,"url":prompt.url,"text":prompt.text}});
        let tools: Vec<_> = definitions()
            .into_iter()
            .map(|definition| {
                json!({"type":"function","function":{
                "name":definition.name,"description":definition.description,
                "parameters":definition.input_schema}})
            })
            .collect();
        json!({"model":self.model,
            "messages":[{"role":"system","content":BROWSER_POLICY},
                {"role":"user","content":context.to_string()}],
            "stream":true,"stream_options":{"include_usage":true},
            "max_tokens":4096,"tools":tools})
    }

    pub(super) async fn send(
        &self,
        request: &Value,
        stop: &AtomicBool,
        mut emit: impl FnMut(TextEvent) -> Result<()>,
    ) -> Result<Turn> {
        cancelled(stop)?;
        let body = serde_json::to_vec(request)?;
        if body.len() > 131_072 {
            return Err("Chat request exceeds bound".into());
        }
        let events = self.http.events(body, stop).await?;
        let sse = sse::Sse::new("Chat");
        let mut state = stream::State::default();
        state.text_max = self.text_max;
        let mut rendered = String::new();
        events
            .frames(stop, sse, 524_288, |event, data| -> Result<()> {
                if let Some(event) = state.frame(event, data)? {
                    if let TextEvent::Delta(text) = &event {
                        rendered.push_str(text);
                    }
                    emit(event)?;
                }
                Ok(())
            })
            .await?;
        let turn = state.finish()?;
        if rendered != turn.text {
            return Err("Chat streamed text differs from completed output".into());
        }
        if !turn.text.is_empty() {
            emit(TextEvent::Final(turn.text.clone()))?;
        }
        Ok(turn)
    }
}

pub(super) fn cancelled(stop: &AtomicBool) -> Result<()> {
    if stop.load(Ordering::Acquire) {
        Err("Chat request cancelled".into())
    } else {
        Ok(())
    }
}

/// True for `localhost` and loopback IP literals: the only hosts plain HTTP may reach.
pub fn loopback_host(url: &reqwest::Url) -> bool {
    match url.host_str() {
        Some(host) if host.eq_ignore_ascii_case("localhost") => true,
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    }
}
