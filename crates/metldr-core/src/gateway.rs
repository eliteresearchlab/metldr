//! The extension's model gateway: every route that needs no companion. Chrome built-in AI,
//! the OpenAI compatible servers people run on this machine, and the user's own key on the four
//! provider protocols. This module holds the route table and the port messages; `gateway/web.rs`
//! does the browser work.

use serde::{Deserialize, Serialize};

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub mod web;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Builtin,
    Local,
    Key,
}

/// The wire protocol a route speaks. The one list for the gateway, the panel's settings and the
/// companion's saved provider slots, whose names are stored, so they never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Messages,
    ChatCompletions,
    Responses,
    GoogleInteractions,
    ChromeAi,
}

impl Protocol {
    /// The protocols a user's own endpoint and key can use.
    pub const KEYED: [Self; 4] = [
        Self::Messages,
        Self::ChatCompletions,
        Self::Responses,
        Self::GoogleInteractions,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
            Self::GoogleInteractions => "google_interactions",
            Self::ChromeAi => "chrome_ai",
        }
    }

    /// How the protocol is shown.
    pub fn label(self) -> &'static str {
        match self {
            Self::Messages => "Messages",
            Self::ChatCompletions => "Chat Completions",
            Self::Responses => "OpenAI Responses",
            Self::GoogleInteractions => "Google Interactions",
            Self::ChromeAi => "Chrome built-in AI",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Route {
    pub id: &'static str,
    /// How the route is shown.
    pub name: &'static str,
    pub kind: Kind,
    pub protocol: Protocol,
    /// The fixed base address; the custom Chat route takes the user's.
    pub base: &'static str,
    /// Where model names are listed, relative to `base`.
    pub models: &'static str,
    /// Where replies stream from, relative to `base`.
    pub stream: &'static str,
}

pub const ROUTES: &[Route] = &[
    Route {
        id: "chrome-ai",
        name: "Chrome built-in AI",
        kind: Kind::Builtin,
        protocol: Protocol::ChromeAi,
        base: "",
        models: "",
        stream: "",
    },
    Route {
        id: "ollama",
        name: "Ollama",
        kind: Kind::Local,
        protocol: Protocol::ChatCompletions,
        base: "http://127.0.0.1:11434/v1",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "lmstudio",
        name: "LM Studio",
        kind: Kind::Local,
        protocol: Protocol::ChatCompletions,
        base: "http://127.0.0.1:1234/v1",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "llamacpp",
        name: "llama.cpp",
        kind: Kind::Local,
        protocol: Protocol::ChatCompletions,
        base: "http://127.0.0.1:8080/v1",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "openai",
        name: "OpenAI",
        kind: Kind::Key,
        protocol: Protocol::Responses,
        base: "https://api.openai.com/v1",
        models: "/models",
        stream: "/responses",
    },
    Route {
        id: "anthropic",
        name: "Anthropic",
        kind: Kind::Key,
        protocol: Protocol::Messages,
        base: "https://api.anthropic.com/v1",
        models: "/models",
        stream: "/messages",
    },
    Route {
        id: "google",
        name: "Google",
        kind: Kind::Key,
        protocol: Protocol::GoogleInteractions,
        base: "https://generativelanguage.googleapis.com",
        models: "/v1beta/models",
        stream: "/v1/interactions",
    },
    Route {
        id: "openrouter",
        name: "OpenRouter",
        kind: Kind::Key,
        protocol: Protocol::ChatCompletions,
        base: "https://openrouter.ai/api/v1",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "groq",
        name: "Groq",
        kind: Kind::Key,
        protocol: Protocol::ChatCompletions,
        base: "https://api.groq.com/openai/v1",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "deepseek",
        name: "DeepSeek",
        kind: Kind::Key,
        protocol: Protocol::ChatCompletions,
        base: "https://api.deepseek.com",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "mistral",
        name: "Mistral",
        kind: Kind::Key,
        protocol: Protocol::ChatCompletions,
        base: "https://api.mistral.ai/v1",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "together",
        name: "Together",
        kind: Kind::Key,
        protocol: Protocol::ChatCompletions,
        base: "https://api.together.ai/v1",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "xai",
        name: "xAI",
        kind: Kind::Key,
        protocol: Protocol::ChatCompletions,
        base: "https://api.x.ai/v1",
        models: "/models",
        stream: "/chat/completions",
    },
    Route {
        id: "chat",
        name: "Custom endpoint",
        kind: Kind::Key,
        protocol: Protocol::ChatCompletions,
        base: "",
        models: "/models",
        stream: "/chat/completions",
    },
];

pub fn route(id: &str) -> Option<&'static Route> {
    ROUTES.iter().find(|route| route.id == id)
}

/// The model servers on this computer.
pub fn local_routes() -> impl Iterator<Item = &'static Route> {
    ROUTES.iter().filter(|route| route.kind == Kind::Local)
}

/// The services a user brings a key for, each at a fixed address.
pub fn key_routes() -> impl Iterator<Item = &'static Route> {
    ROUTES.iter().filter(|route| route.kind == Kind::Key && !route.base.is_empty())
}

/// The routes with a fixed address that a Chat Completions setting can point at.
pub fn chat_routes() -> impl Iterator<Item = &'static Route> {
    ROUTES.iter().filter(|route| route.protocol == Protocol::ChatCompletions && !route.base.is_empty())
}

impl Route {
    /// Where replies stream from.
    pub fn endpoint(&self) -> String {
        format!("{}{}", self.base, self.stream)
    }

    /// The host and port the route reaches, such as `127.0.0.1:11434`.
    pub fn address(&self) -> &'static str {
        let rest = self.base.split_once("://").map_or(self.base, |(_, rest)| rest);
        rest.split('/').next().unwrap_or(rest)
    }

    /// Whether the route runs without a key.
    pub fn keyless(&self) -> bool {
        self.kind != Kind::Key
    }
}

/// What `detect` and `connect` report for one route.
#[derive(Debug, Serialize)]
pub struct RouteState {
    pub route: &'static str,
    pub kind: Kind,
    /// ready, downloadable, downloading, absent, needs-key or error.
    pub state: &'static str,
    /// Where text sent on this route goes: "on this device" or the service's host name.
    pub destination: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// One message on the `metldr-gateway` port.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub enum Request {
    Detect {
        id: u64,
    },
    Connect {
        id: u64,
        route: String,
        key: Option<String>,
        endpoint: Option<String>,
        model: Option<String>,
    },
    Forget {
        id: u64,
        route: String,
    },
    Test {
        id: u64,
        route: String,
        model: Option<String>,
    },
    Prefer {
        id: u64,
        route: String,
        model: String,
    },
    Stream {
        id: u64,
        route: Option<String>,
        model: Option<String>,
        system: String,
        messages: Vec<Message>,
    },
    Stop {
        id: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// Longest system text plus conversation the gateway sends on.
pub const MAX_INPUT: usize = 120_000;
pub const MAX_MODELS: usize = 200;

impl Request {
    pub fn id(&self) -> u64 {
        match self {
            Self::Detect { id }
            | Self::Connect { id, .. }
            | Self::Forget { id, .. }
            | Self::Test { id, .. }
            | Self::Prefer { id, .. }
            | Self::Stream { id, .. }
            | Self::Stop { id } => *id,
        }
    }
}

/// Accepts at most 64 turns that end with the user's, within `MAX_INPUT` bytes in all.
pub fn check_conversation(system: &str, messages: &[Message]) -> Result<(), &'static str> {
    let size = messages
        .iter()
        .fold(system.len(), |sum, m| sum.saturating_add(m.content.len()));
    if messages.is_empty()
        || messages.len() > 64
        || messages.last().is_some_and(|m| m.role != Role::User)
    {
        return Err("A conversation needs one to 64 turns and must end with yours.");
    }
    if size > MAX_INPUT {
        return Err("This conversation is too long to send.");
    }
    Ok(())
}

/// The model names in a provider's model list: `data[].id` (OpenAI compatible and Anthropic)
/// or `models[].name` without its `models/` prefix (Google). Bounded and sorted.
pub fn model_names(body: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let listed = value["data"]
        .as_array()
        .map(|items| (items, "id"))
        .or_else(|| value["models"].as_array().map(|items| (items, "name")));
    let Some((items, field)) = listed else {
        return Vec::new();
    };
    let mut names: Vec<String> = items
        .iter()
        .filter_map(|item| item[field].as_str())
        .map(|name| name.strip_prefix("models/").unwrap_or(name))
        .filter(|name| {
            !name.is_empty() && name.len() <= 128 && name.bytes().all(|b| b.is_ascii_graphic())
        })
        .map(str::to_owned)
        .collect();
    names.sort_unstable();
    names.dedup();
    names.truncate(MAX_MODELS);
    names
}

/// The base address of a custom OpenAI compatible endpoint: https anywhere, or http on this
/// machine, with any trailing `/chat/completions` or slash removed.
pub fn custom_base(endpoint: &str) -> Result<String, &'static str> {
    const INVALID: &str = "Use an https address, or http on this computer.";
    let endpoint = endpoint.trim();
    let base = endpoint
        .strip_suffix("/chat/completions")
        .unwrap_or(endpoint)
        .trim_end_matches('/');
    let (scheme, rest) = base.split_once("://").ok_or(INVALID)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
    let loopback = matches!(name, "localhost" | "127.0.0.1" | "[::1]");
    let allowed = scheme == "https" || (scheme == "http" && loopback);
    if !allowed
        || host.is_empty()
        || host.contains('@')
        || base.len() > 1000
        || base.contains(['?', '#', ' '])
    {
        return Err(INVALID);
    }
    Ok(base.to_owned())
}

/// The host a route sends text to, as shown beside its key.
pub fn destination(route: &Route, base: &str) -> String {
    let rest = base.split_once("://").map_or(base, |(_, rest)| rest);
    let host = rest.split(['/', ':']).next().unwrap_or(rest);
    match host {
        _ if route.kind != Kind::Key => "on this device".into(),
        "localhost" | "127.0.0.1" | "[" => "on this device".into(),
        "" => "the endpoint you set".into(),
        host => host.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_parse_and_unknown_fields_are_refused() {
        let stream = r#"{"op":"stream","id":7,"system":"Be brief.","messages":[{"role":"user","content":"Hi"}]}"#;
        let request: Request = serde_json::from_str(stream).unwrap();
        assert_eq!(request.id(), 7);
        assert!(serde_json::from_str::<Request>(r#"{"op":"detect","id":1,"key":"x"}"#).is_err());
        let Request::Stream {
            messages, system, ..
        } = request
        else {
            panic!("stream")
        };
        assert!(check_conversation(&system, &messages).is_ok());
        let ends_with_reply = [Message {
            role: Role::Assistant,
            content: "Hi".into(),
        }];
        assert!(check_conversation("", &ends_with_reply).is_err());
    }

    #[test]
    fn model_lists_from_every_protocol_shape() {
        let openai =
            br#"{"data":[{"id":"gpt-b"},{"id":"gpt-a"},{"id":"gpt-a"},{"id":"bad name"}]}"#;
        assert_eq!(model_names(openai), ["gpt-a", "gpt-b"]);
        let google = br#"{"models":[{"name":"models/gemini-x"}]}"#;
        assert_eq!(model_names(google), ["gemini-x"]);
        assert!(model_names(b"not json").is_empty());
    }

    #[test]
    fn custom_endpoints_need_https_or_this_computer() {
        assert_eq!(
            custom_base("https://api.example.com/v1/chat/completions").unwrap(),
            "https://api.example.com/v1"
        );
        assert_eq!(
            custom_base("http://localhost:9000/v1/").unwrap(),
            "http://localhost:9000/v1"
        );
        assert!(custom_base("http://192.168.1.4:8080/v1").is_err());
        assert!(custom_base("https://user@host/v1").is_err());
        let chat = route("chat").unwrap();
        assert_eq!(
            destination(chat, "https://api.example.com:8443/v1"),
            "api.example.com"
        );
        assert_eq!(
            destination(route("ollama").unwrap(), ROUTES[1].base),
            "on this device"
        );
        assert_eq!(
            destination(chat, "http://127.0.0.1:9000/v1"),
            "on this device"
        );
        assert_eq!(destination(chat, ""), "the endpoint you set");
    }
}
