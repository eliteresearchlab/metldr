use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Prompt, Result, sse, stream};
use metldr_browser_tools::{BROWSER_POLICY, definitions};
use metldr_conversation::TextEvent;
use stream::Turn;

pub struct Client {
    http: sse::http::Http,
    model: String,
    text_max: Option<usize>,
}

impl Client {
    pub fn new(
        endpoint: &str,
        model: &str,
        api_key: &str,
        trust_ca_pem: Option<&[u8]>,
    ) -> Result<Self> {
        let url =
            reqwest::Url::parse(endpoint).map_err(|_| "invalid Google Interactions endpoint")?;
        if endpoint.len() > 1024
            || url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
        {
            return Err("invalid Google Interactions endpoint".into());
        }
        if model.is_empty() || model.len() > 128 || !model.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("invalid Google Interactions model".into());
        }
        if api_key.is_empty()
            || api_key.len() > 4096
            || !api_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err("invalid Google Interactions credential".into());
        }
        let http = sse::http::Http::new(sse::http::Options {
            provider: "Google Interactions",
            url: url.as_str(),
            timeout_secs: 45,
            localhost_port: None,
            trust_ca_pem,
            headers: &[],
            secret: Some(("x-goog-api-key", api_key)),
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
        if turn.call.is_some() {
            return Err("Google Interactions text request returned a tool call".into());
        }
        Ok(turn.usage)
    }

    fn text_request(&self, system: &str, turns: &[sse::Turn<'_>]) -> Value {
        let mut input = Vec::with_capacity(turns.len());
        for turn in turns {
            let kind = if turn.assistant {
                "model_output"
            } else {
                "user_input"
            };
            input.push(json!({"type": kind, "content": [{"type": "text", "text": turn.text}]}));
        }
        json!({"model":self.model,"system_instruction":system,"input":input,
            "stream":true,"store":false,"generation_config":{"max_output_tokens":2048}})
    }

    pub(super) fn request(&self, prompt: Prompt<'_>) -> Value {
        let context = json!({"instruction":prompt.instruction,"source":{
            "title":prompt.title,"url":prompt.url,"text":prompt.text}});
        let tools: Vec<_> = definitions().into_iter().map(|definition| {
            let parameters = definition.input_schema;
            json!({"type":"function","name":definition.name,"description":definition.description,
                "parameters":parameters})
        }).collect();
        json!({"model":self.model,"system_instruction":BROWSER_POLICY,
            "input":[{"type":"user_input","content":[{"type":"text","text":context.to_string()}]}],
            "stream":true,"store":false,"generation_config":{"max_output_tokens":2048,"tool_choice":"validated"},
            "tools":tools})
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
            return Err("Google Interactions request exceeds bound".into());
        }
        let events = self.http.events(body, stop).await?;
        let sse = sse::Sse::new("Google");
        let mut state = stream::State::default();
        state.text_max = self.text_max;
        let mut rendered = String::new();
        events
            .frames(stop, sse, 524_288, |event, data| -> Result<()> {
                for text_event in state.frame(event, data)? {
                    if let TextEvent::Delta(text) = &text_event {
                        rendered.push_str(text);
                    }
                    emit(text_event)?;
                }
                Ok(())
            })
            .await?;
        let turn = state.finish()?;
        if rendered != turn.text {
            return Err("Google Interactions streamed text differs from completed output".into());
        }
        Ok(turn)
    }
}

fn cancelled(stop: &AtomicBool) -> Result<()> {
    if stop.load(Ordering::Acquire) {
        Err("Google Interactions request cancelled".into())
    } else {
        Ok(())
    }
}
