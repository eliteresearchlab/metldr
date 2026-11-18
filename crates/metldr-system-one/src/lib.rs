//! TypeSafe's System One API: typed questions about a state, answered with calibrated
//! probabilities instead of generated text. metldr asks one kind, a Choice among at most 255
//! options, and reads the probability of each.

use serde_json::{Map, Value, json};
use std::sync::atomic::AtomicBool;

pub type Result<T> = std::result::Result<T, String>;

/// The most options one Choice may offer.
pub const MAX_OPTIONS: usize = 255;
const REQUEST_BOUND: usize = 131_072;
const ANSWER_BOUND: usize = 65_536;
/// Decisions take 70 to 500 ms; a slower answer is not worth waiting for.
const TIMEOUT_SECS: u16 = 5;

pub struct Client {
    http: metldr_sse::http::Http,
    model: String,
}

/// A Choice answer: each option's probability, in the order the options were given, and the
/// model's confidence in its pick.
#[derive(Debug, PartialEq)]
pub struct Choice {
    pub probabilities: Vec<f32>,
    pub confidence: Option<f32>,
    pub input_tokens: u64,
}

impl Client {
    pub fn new(
        endpoint: &str,
        model: &str,
        api_key: &str,
        trust_ca_pem: Option<&[u8]>,
    ) -> Result<Self> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| "invalid System One endpoint")?;
        if endpoint.len() > 1024
            || url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
        {
            return Err("invalid System One endpoint".into());
        }
        if model.is_empty() || model.len() > 128 || !model.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("invalid System One model".into());
        }
        if api_key.is_empty() || !api_key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("invalid System One credential".into());
        }
        let bearer = format!("Bearer {api_key}");
        let http = metldr_sse::http::Http::new(metldr_sse::http::Options {
            provider: "System One",
            url: url.as_str(),
            timeout_secs: TIMEOUT_SECS,
            localhost_port: None,
            trust_ca_pem,
            headers: &[],
            secret: Some(("authorization", &bearer)),
        })?;
        Ok(Self {
            http,
            model: model.into(),
        })
    }

    /// Which of `options` answers `instructions` about `state`.
    pub async fn choose(
        &self,
        state: &Value,
        instructions: &str,
        options: &[&str],
        stop: &AtomicBool,
    ) -> Result<Choice> {
        let body = request(&self.model, state, instructions, options)?;
        let answer = self.http.json(body, stop, ANSWER_BOUND).await?;
        parse(&answer, options.len())
    }
}

/// One Choice question named `next`, its options named by their place in `options`.
pub fn request(model: &str, state: &Value, instructions: &str, options: &[&str]) -> Result<Vec<u8>> {
    if options.is_empty() || options.len() > MAX_OPTIONS {
        return Err("a System One choice takes 1 to 255 options".into());
    }
    let criteria: Map<String, Value> = options
        .iter()
        .enumerate()
        .map(|(n, option)| (n.to_string(), json!(option)))
        .collect();
    let question = json!({"type": "choice", "instructions": instructions, "criteria": criteria});
    let body = json!({"model": model, "state": state, "questions": {"next": question}});
    let body = serde_json::to_vec(&body).map_err(|_| "System One request unavailable")?;
    if body.len() > REQUEST_BOUND {
        return Err("System One request exceeds bound".into());
    }
    Ok(body)
}

/// The `next` answer of a System One response over `count` options. An option the answer does
/// not name has probability 0.
pub fn parse(bytes: &[u8], count: usize) -> Result<Choice> {
    let invalid = || "System One returned an invalid answer".to_owned();
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let answer = &value["answers"]["next"];
    if answer["type"] != "choice" {
        return Err(invalid());
    }
    let mut probabilities = vec![0.0f32; count];
    for (name, probability) in answer["probabilities"].as_object().ok_or_else(invalid)? {
        let at: usize = name.parse().map_err(|_| invalid())?;
        let p = probability.as_f64().filter(|p| (0.0..=1.0).contains(p)).ok_or_else(invalid)?;
        *probabilities.get_mut(at).ok_or_else(invalid)? = p as f32;
    }
    let confidence = answer["confidence"].as_f64().filter(|c| (0.0..=1.0).contains(c));
    Ok(Choice {
        probabilities,
        confidence: confidence.map(|c| c as f32),
        input_tokens: value["usage"]["input_tokens"].as_u64().unwrap_or(0),
    })
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn a_choice_names_its_options_by_place_and_reads_their_probabilities_back() {
        let state = json!({"goal": "open the faculty list"});
        let body = request("jev-latest", &state, "Which control?", &["link \"Faculty\"", "button \"Search\""]).unwrap();
        let sent: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(sent["questions"]["next"]["criteria"]["1"], "button \"Search\"");
        assert_eq!(sent["model"], "jev-latest");
        let answer = br#"{"model":"jev-latest","answers":{"next":{"type":"choice","choice":"0","confidence":0.7,
            "probabilities":{"0":0.9,"1":0.1}}},"usage":{"input_tokens":120,"output_tokens":12}}"#;
        let choice = parse(answer, 2).unwrap();
        assert_eq!(choice.probabilities, [0.9, 0.1]);
        assert_eq!((choice.confidence, choice.input_tokens), (Some(0.7), 120));
        assert!(parse(br#"{"answers":{"next":{"type":"choice","probabilities":{"5":1.0}}}}"#, 2).is_err());
        assert!(request("jev-latest", &state, "Which?", &[]).is_err());
    }
}
