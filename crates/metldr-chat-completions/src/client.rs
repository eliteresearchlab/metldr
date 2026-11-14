use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Prompt, Result, TextEvent, sse, stream};
use metldr_browser_tools::{BROWSER_POLICY, definitions};
use stream::Turn;

/// Cheap to clone: clones share one connection pool.
#[derive(Clone)]
pub struct Client {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    authorization: Option<reqwest::header::HeaderValue>,
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
                let mut header = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| "invalid Chat credential")?;
                header.set_sensitive(true);
                Ok(header)
            })
            .transpose()?;
        let mut client = reqwest::Client::builder()
            .tls_backend_rustls()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(std::time::Duration::from_secs(timeout_secs.into()));
        // A local name must not resolve off the machine.
        if loopback && url.host_str() == Some("localhost") {
            let port = url.port_or_known_default().ok_or("invalid Chat endpoint")?;
            client = client.resolve(
                "localhost",
                std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            );
        }
        if let Some(pem) = trust_ca_pem {
            if pem.len() > 16_384 {
                return Err("Chat trust certificate exceeds bound".into());
            }
            let certificates = reqwest::Certificate::from_pem_bundle(pem)
                .map_err(|_| "invalid Chat trust certificate")?;
            if certificates.is_empty() {
                return Err("empty Chat trust certificate".into());
            }
            client = client.tls_certs_only(certificates);
        }
        Ok(Self {
            client: client.build().map_err(|_| "Chat client unavailable")?,
            endpoint: url.to_string(),
            model: model.into(),
            authorization,
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
        let turn = self
            .send(&self.text_request(system, input), stop, emit)
            .await?;
        if !turn.calls.is_empty() {
            return Err("Chat text request returned a tool call".into());
        }
        Ok(turn.usage.unwrap_or_default())
    }

    fn text_request(&self, system: &str, input: &str) -> Value {
        json!({"model":self.model,
            "messages":[{"role":"system","content":system},{"role":"user","content":input}],
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
        let mut sending = self.client.post(&self.endpoint);
        if let Some(authorization) = &self.authorization {
            sending = sending.header("authorization", authorization.clone());
        }
        let sending = sending
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body(body)
            .send();
        let mut response = tokio::select! {
            biased;
            _ = wait_for_stop(stop) => return Err("Chat request cancelled".into()),
            response = sending => response.map_err(|_| "Chat transport failed")?,
        };
        if response.status() != reqwest::StatusCode::OK {
            return Err("Chat endpoint returned an error".into());
        }
        if response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .is_none_or(|v| !v.trim().eq_ignore_ascii_case("text/event-stream"))
        {
            return Err("Chat endpoint did not return an event stream".into());
        }
        let mut sse = sse::Sse::new("Chat");
        let mut state = stream::State::default();
        let mut bytes = 0usize;
        let mut rendered = String::new();
        loop {
            let chunk = tokio::select! {
                biased;
                _ = wait_for_stop(stop) => return Err("Chat request cancelled".into()),
                chunk = response.chunk() => chunk.map_err(|_| "Chat stream transport failed")?,
            };
            let Some(chunk) = chunk else { break };
            bytes = bytes.saturating_add(chunk.len());
            if bytes > 524_288 {
                return Err("Chat stream exceeds bound".into());
            }
            let mut rest = &chunk[..];
            while let Some((event, data)) = sse.next(&mut rest)? {
                cancelled(stop)?;
                if let Some(event) = state.frame(&event, &data)? {
                    if let TextEvent::Delta(text) = &event {
                        rendered.push_str(text);
                    }
                    emit(event)?;
                }
            }
        }
        cancelled(stop)?;
        sse.finish()?;
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

async fn wait_for_stop(stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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
