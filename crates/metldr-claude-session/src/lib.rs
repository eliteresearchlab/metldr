#![doc = include_str!("../README.md")]

use serde::Deserialize;
use serde_json::{Value, json};
use std::{borrow::Cow, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn invalid(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}

/// The longest stream-json line accepted from Claude Code.
const LINE_LIMIT: usize = 4 * 1024 * 1024;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// A Claude Code session id: a lowercase UUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionId(String);

impl SessionId {
    pub fn saved(value: &str) -> Result<Self> {
        let groups: Vec<_> = value.split('-').map(str::len).collect();
        if groups != [8, 4, 4, 4, 12]
            || !value
                .bytes()
                .all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("invalid Claude Code session id"));
        }
        Ok(Self(value.into()))
    }

    /// A version 4 UUID from 16 random bytes the caller supplies.
    pub fn from_random(mut bytes: [u8; 16]) -> Self {
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Self(format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub enum Session {
    New(SessionId),
    Resume(SessionId),
}

/// An HTTP MCP server Claude Code may use, and nothing else.
pub struct McpServer {
    pub name: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

pub struct Launch<'a> {
    pub executable: &'a Path,
    pub workspace: &'a Path,
    pub session: Session,
    pub model: Option<&'a str>,
    pub servers: &'a [McpServer],
    /// Extra environment, for example an owned test endpoint.
    pub env: &'a [(&'a str, String)],
}

pub enum Event {
    /// The session started: model, tool names and MCP server states.
    Ready {
        model: String,
        tools: Vec<String>,
        mcp_servers: Value,
    },
    /// One streamed piece of assistant text, read without building a JSON tree.
    Text(String),
    Partial(Value),
    Assistant(Value),
    ToolResult(Value),
    Permission(Permission),
    Retry {
        attempt: u64,
    },
    Finished {
        is_error: bool,
        stop_reason: Option<String>,
        text: Option<String>,
    },
}

/// One tool use Claude Code asks the user to allow.
#[derive(Debug)]
pub struct Permission {
    pub request_id: String,
    pub tool_name: String,
    pub tool_use_id: Option<String>,
    pub description: Option<String>,
    pub blocked_path: Option<String>,
    pub input: Value,
}

pub struct ClaudeSession {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    session: SessionId,
    line: Vec<u8>,
    next_request: u64,
}

impl ClaudeSession {
    /// Starts Claude Code in stream-json mode. Its permission prompts come back as
    /// [`Event::Permission`] and only the given MCP servers are loaded.
    pub fn open(launch: Launch<'_>) -> Result<Self> {
        let mut servers = serde_json::Map::new();
        for server in launch.servers {
            if !valid_name(&server.name)
                || !(server.url.starts_with("http://") || server.url.starts_with("https://"))
                || server.url.len() > 2048
                || server.url.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
                || server.headers.len() > 32
                || server
                    .headers
                    .iter()
                    .any(|(k, v)| !valid_name(k) || v.len() > 8192 || v.bytes().any(|b| b.is_ascii_control()))
                || servers.contains_key(&server.name)
            {
                return Err(invalid("invalid Claude Code MCP server"));
            }
            let headers: serde_json::Map<_, _> = server
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            servers.insert(
                server.name.clone(),
                json!({"type":"http","url":server.url,"headers":headers}),
            );
        }
        if launch
            .model
            .is_some_and(|model| model.is_empty() || model.len() > 128 || !model.bytes().all(|b| b.is_ascii_graphic()))
        {
            return Err(invalid("invalid Claude Code model"));
        }
        let mut command = Command::new(launch.executable);
        command
            .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json"])
            .args(["--verbose", "--include-partial-messages", "--permission-prompt-tool", "stdio"])
            .arg("--strict-mcp-config")
            .arg("--mcp-config")
            .arg(json!({ "mcpServers": servers }).to_string());
        let session = match launch.session {
            Session::New(id) => {
                command.args(["--session-id", id.as_str()]);
                id
            }
            Session::Resume(id) => {
                command.args(["--resume", id.as_str()]);
                id
            }
        };
        if let Some(model) = launch.model {
            command.args(["--model", model]);
        }
        for (key, value) in launch.env {
            command.env(key, value);
        }
        let mut child = command
            .current_dir(launch.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| invalid("no stdout"))?);
        Ok(Self {
            child,
            stdin,
            stdout,
            session,
            line: Vec::new(),
            next_request: 0,
        })
    }

    pub fn id(&self) -> &SessionId {
        &self.session
    }

    /// Sends one user turn.
    pub async fn prompt(&mut self, text: &str) -> Result<()> {
        self.write(json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":text}]}}))
            .await
    }

    /// Answers a permission request. A denial carries its reason to the model.
    pub async fn answer(&mut self, permission: &Permission, allow: bool, reason: &str) -> Result<()> {
        let response = if allow {
            json!({"behavior":"allow","updatedInput":permission.input})
        } else {
            json!({"behavior":"deny","message":reason})
        };
        self.write(json!({"type":"control_response","response":{"subtype":"success","request_id":permission.request_id,"response":response}}))
            .await
    }

    /// Asks Claude Code to stop the current turn. It still ends with [`Event::Finished`].
    pub async fn interrupt(&mut self) -> Result<()> {
        self.next_request += 1;
        let id = format!("metldr-{}", self.next_request);
        self.write(json!({"type":"control_request","request_id":id,"request":{"subtype":"interrupt"}}))
            .await
    }

    /// The next event; `None` when Claude Code closed its output.
    pub async fn next(&mut self) -> Result<Option<Event>> {
        loop {
            if !self.read().await? {
                return Ok(None);
            }
            let event = parse_line(&self.line, self.session.as_str());
            self.line.clear();
            if let Some(event) = event? {
                return Ok(Some(event));
            }
        }
    }

    /// Ends input and waits for Claude Code to exit, killing it after a bound.
    pub async fn close(mut self) -> Result<std::process::ExitStatus> {
        drop(self.stdin.take());
        match timeout(CLOSE_TIMEOUT, self.child.wait()).await {
            Ok(status) => Ok(status?),
            Err(_) => {
                self.child.kill().await?;
                Ok(self.child.wait().await?)
            }
        }
    }

    async fn write(&mut self, value: Value) -> Result<()> {
        let mut line = serde_json::to_vec(&value)?;
        line.push(b'\n');
        let stdin = self.stdin.as_mut().ok_or_else(|| invalid("Claude Code input closed"))?;
        stdin.write_all(&line).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Reads one non-empty line into `self.line`; `false` at the end of output. Cancelling it
    /// keeps the bytes already taken, so a caller may time it out and call it again.
    async fn read(&mut self) -> Result<bool> {
        loop {
            let buffer = self.stdout.fill_buf().await?;
            if buffer.is_empty() {
                return if self.line.is_empty() {
                    Ok(false)
                } else {
                    Err(invalid("Claude Code output ended inside a line"))
                };
            }
            let (chunk, done) = match memchr::memchr(b'\n', buffer) {
                Some(end) => (&buffer[..end], Some(end + 1)),
                None => (buffer, None),
            };
            if self.line.len() + chunk.len() > LINE_LIMIT {
                return Err(invalid("Claude Code line exceeds its bound"));
            }
            self.line.extend_from_slice(chunk);
            let used = done.unwrap_or(buffer.len());
            self.stdout.consume(used);
            if done.is_some() && !self.line.is_empty() {
                return Ok(true);
            }
        }
    }
}

/// The borrowed shape of a streamed text delta, the most frequent line by far.
#[derive(Deserialize)]
struct DeltaLine<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
    #[serde(borrow)]
    session_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    event: Option<DeltaEvent<'a>>,
}

#[derive(Deserialize)]
struct DeltaEvent<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
    #[serde(borrow)]
    delta: Option<Delta<'a>>,
}

#[derive(Deserialize)]
struct Delta<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
    #[serde(borrow)]
    text: Option<Cow<'a, str>>,
}

/// Turns one stream-json line into an event; `None` for lines MetLDR does not use.
pub fn parse_line(line: &[u8], session: &str) -> Result<Option<Event>> {
    // Text deltas skip the JSON tree: unknown fields are skipped and strings borrowed.
    if memchr::memmem::find(line, b"\"text_delta\"").is_some()
        && let Ok(fast) = serde_json::from_slice::<DeltaLine<'_>>(line)
        && fast.kind == "stream_event"
        && let Some(DeltaEvent { kind, delta: Some(Delta { kind: delta, text: Some(text) }) }) = fast.event
        && kind == "content_block_delta"
        && delta == "text_delta"
    {
        if fast.session_id.is_some_and(|id| id != session) {
            return Err(invalid("Claude Code answered for another session"));
        }
        return Ok(Some(Event::Text(text.into_owned())));
    }
    let value: Value = serde_json::from_slice(line)?;
    if value["session_id"].as_str().is_some_and(|id| id != session) {
        return Err(invalid("Claude Code answered for another session"));
    }
    let event = match (value["type"].as_str(), value["subtype"].as_str()) {
        (Some("system"), Some("init")) => Event::Ready {
            model: value["model"].as_str().unwrap_or_default().into(),
            tools: value["tools"]
                .as_array()
                .map(|tools| tools.iter().filter_map(|t| t.as_str().map(Into::into)).collect())
                .unwrap_or_default(),
            mcp_servers: value["mcp_servers"].clone(),
        },
        (Some("system"), Some("api_retry")) => Event::Retry {
            attempt: value["attempt"].as_u64().unwrap_or_default(),
        },
        (Some("stream_event"), _) => Event::Partial(value["event"].clone()),
        (Some("assistant"), _) => Event::Assistant(value["message"].clone()),
        (Some("user"), _) => Event::ToolResult(value["message"].clone()),
        (Some("result"), _) => Event::Finished {
            is_error: value["is_error"].as_bool().unwrap_or(true),
            stop_reason: value["stop_reason"].as_str().map(Into::into),
            text: value["result"].as_str().map(Into::into),
        },
        (Some("control_request"), _) if value["request"]["subtype"] == "can_use_tool" => {
            let request = &value["request"];
            let text = |key: &str| request[key].as_str().map(String::from);
            Event::Permission(Permission {
                request_id: value["request_id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 128)
                    .ok_or_else(|| invalid("Claude Code permission without an id"))?
                    .into(),
                tool_name: text("tool_name").ok_or_else(|| invalid("permission without a tool"))?,
                tool_use_id: text("tool_use_id"),
                description: text("description"),
                blocked_path: text("blocked_path"),
                input: request["input"].clone(),
            })
        }
        (Some("control_response"), _) if value["response"]["subtype"] != "success" => {
            return Err(invalid("Claude Code refused a control request"));
        }
        _ => return Ok(None),
    };
    Ok(Some(event))
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

