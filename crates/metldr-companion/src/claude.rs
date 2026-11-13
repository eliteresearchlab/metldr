//! The Claude Code adapter on the generic harness path (B1.9.2). It runs the installed CLI
//! with the per-run MetLDR browser MCP server as its only MCP server, turns its permission
//! prompts into native permission cards, and uses the session UUID as continuation key.

use crate::{
    Result,
    agent::{self, Outcome},
    agent_model::Activity,
    browser_io::{Client, Notice},
    browser_mcp::Server,
    browser_worker::{BROWSER_POLICY, Tools},
    flow::Event,
    harness_model::{Binding, Profile},
    model::Source,
    tool_report::Call,
};
use metldr_claude_session::{ClaudeSession, Event as NativeEvent, Launch, McpServer, Session, SessionId};
use metldr_conversation::{Change, MessageKey};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(super) struct Job {
    pub run: String,
    pub workflow: String,
    pub profile: Profile,
    pub binding: Binding,
    pub source: Source,
    pub target: crate::browser_action::Target,
    pub text: String,
}

pub(super) fn execute(
    job: Job,
    stop: Arc<AtomicBool>,
    tx: mpsc::SyncSender<Event>,
    events: tokio::sync::mpsc::Receiver<Notice>,
) {
    let result = (|| -> Result<(Outcome, bool)> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(runtime.block_on(execute_inner(&job, stop, &tx, events)))
    })();
    let (result, released) =
        result.unwrap_or_else(|error| (Outcome::Failed(error.to_string()), true));
    let _ = tx.send(Event::HarnessSettled {
        run: job.run,
        result,
        released,
    });
}

async fn execute_inner(
    job: &Job,
    stop: Arc<AtomicBool>,
    tx: &mpsc::SyncSender<Event>,
    events: tokio::sync::mpsc::Receiver<Notice>,
) -> (Outcome, bool) {
    if stop.load(Ordering::Acquire) {
        return (Outcome::Cancelled, true);
    }
    let server = match Server::bind(&job.run).await {
        Ok(server) => server,
        Err(error) => return (Outcome::Failed(error.to_string()), true),
    };
    let (address, token) = server.endpoint();
    let servers = [McpServer {
        name: "metldr".into(),
        url: format!("http://{address}/mcp"),
        headers: vec![("Authorization".into(), format!("Bearer {token}"))],
    }];
    let done = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let tools = Tools::new(
        Client {
            run: job.run.clone(),
            tx: tx.clone(),
        },
        job.target.clone(),
        events,
        stop.clone(),
    );
    let native = async {
        let result = native(job, &servers, &stop, &failed, tx).await;
        done.store(true, Ordering::Release);
        result
    };
    let browser = async {
        let (result, released) = server
            .serve(tools, stop.clone(), done.clone(), failed.clone())
            .await;
        if result.is_err() || (!done.load(Ordering::Acquire) && !stop.load(Ordering::Acquire)) {
            failed.store(true, Ordering::Release);
        }
        (result, released)
    };
    let (native, (browser, released)) = tokio::join!(native, browser);
    let outcome = match (native, browser) {
        (Ok(outcome), Ok(())) => outcome,
        (Err(error), Ok(())) => Outcome::Failed(error.to_string()),
        (_, Err(error)) => Outcome::Failed(format!("Claude Code browser task failed: {error}")),
    };
    (outcome, released)
}

async fn native(
    job: &Job,
    servers: &[McpServer],
    stop: &AtomicBool,
    failed: &AtomicBool,
    tx: &mpsc::SyncSender<Event>,
) -> Result<Outcome> {
    let session = match job.binding.resume("claude")? {
        Some(id) => Session::Resume(SessionId::saved(&id)?),
        None => {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random)?;
            Session::New(SessionId::from_random(random))
        }
    };
    let mut session = ClaudeSession::open(Launch {
        executable: &job.profile.executable,
        workspace: &job.profile.workspace,
        session,
        model: None,
        servers,
        env: &[],
    })?;
    // The owner saves the session id before the first prompt, so a crash can never orphan it.
    let (ack, receipt) = mpsc::sync_channel(1);
    tx.try_send(Event::HarnessBound {
        run: job.run.clone(),
        workflow: job.workflow.clone(),
        session: session.id().as_str().into(),
        ack,
    })
    .map_err(|_| "Claude Code binding owner is unavailable")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let ready = loop {
        match receipt.try_recv() {
            Ok(ready) => break ready?,
            Err(mpsc::TryRecvError::Disconnected) => return Err("Claude Code binding owner ended".into()),
            Err(mpsc::TryRecvError::Empty) => (),
        }
        if stop.load(Ordering::Acquire) {
            session.close().await?;
            return Ok(Outcome::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err("Claude Code binding was not confirmed".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    if !ready || stop.load(Ordering::Acquire) {
        session.close().await?;
        return Ok(Outcome::Cancelled);
    }
    let message = format!(
        "{}\n\n{BROWSER_POLICY}\nMetLDR browser tools are available as the metldr MCP server. Your own tool permissions and MetLDR browser approvals are separate decisions.",
        agent::message(&job.text, &job.source)?
    );
    session.prompt(&message).await?;
    let result = stream(job, &mut session, stop, failed, tx).await;
    if result.is_err() {
        let _ = session.interrupt().await;
    }
    session.close().await?;
    result
}

struct Pending {
    key: u64,
    permission: metldr_claude_session::Permission,
    answer: mpsc::Receiver<Option<String>>,
}

async fn stream(
    job: &Job,
    session: &mut ClaudeSession,
    stop: &AtomicBool,
    failed: &AtomicBool,
    tx: &mpsc::SyncSender<Event>,
) -> Result<Outcome> {
    let mut stop_wait = agent::StopWait::default();
    let mut pending: Vec<Pending> = Vec::new();
    let mut tools: BTreeMap<String, (String, Call)> = BTreeMap::new();
    let (mut interrupted, mut bytes, mut activities, mut next_key) = (false, 0usize, 0usize, 1u64);
    let mut message: Option<String> = None;
    loop {
        let stopping = stop.load(Ordering::Acquire) || failed.load(Ordering::Acquire);
        stop_wait.check(stopping, Instant::now())?;
        if stopping && !interrupted {
            session.interrupt().await?;
            interrupted = true;
        }
        let mut index = 0;
        while index < pending.len() {
            let answer = if stopping {
                Some(None)
            } else {
                match pending[index].answer.try_recv() {
                    Ok(option) => Some(option),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("Claude Code permission owner ended without a decision".into());
                    }
                }
            };
            match answer {
                Some(answer) => {
                    let request = pending.swap_remove(index);
                    let allow = answer.as_deref() == Some("allow");
                    session
                        .answer(&request.permission, allow, "The user declined this step in MetLDR.")
                        .await?;
                    let _ = tx.send(Event::NativePermissionResolved {
                        run: job.run.clone(),
                        key: request.key,
                    });
                }
                None => index += 1,
            }
        }
        let event = match tokio::time::timeout(Duration::from_millis(30), session.next()).await {
            Ok(event) => event?.ok_or("Claude Code ended without a result")?,
            Err(_) => continue,
        };
        match event {
            NativeEvent::Permission(permission) => {
                if stopping || pending.len() >= 8 || serde_json::to_vec(&permission.input)?.len() > 8192 {
                    session.answer(&permission, false, "MetLDR stopped or bounded this request.").await?;
                    continue;
                }
                let key = next_key;
                next_key = next_key.checked_add(1).ok_or("Claude Code permission key exhausted")?;
                let details = json!({
                    "agent": "claude",
                    "tool": permission.tool_name,
                    "title": permission.description.clone().unwrap_or_else(|| format!("Claude Code wants to use {}", permission.tool_name)),
                    "input": serde_json::to_string(&permission.input)?,
                    "options": [{"id":"allow","name":"Allow once"},{"id":"deny","name":"Deny"}],
                });
                let (ack, answer) = mpsc::sync_channel(1);
                tx.send(Event::NativePermission { run: job.run.clone(), key, details, ack })
                    .map_err(|_| "Claude Code permission owner is unavailable")?;
                pending.push(Pending { key, permission, answer });
            }
            NativeEvent::Partial(event) => match event["type"].as_str() {
                Some("message_start") => {
                    message = event["message"]["id"].as_str().map(String::from);
                }
                Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                    let text = event["delta"]["text"].as_str().unwrap_or_default();
                    let key = message.as_deref().ok_or("Claude Code text has no message")?;
                    bytes = bytes
                        .checked_add(text.len())
                        .filter(|n| *n <= crate::TEXT_MAX)
                        .ok_or("Claude Code output exceeds application bound")?;
                    tx.send(Event::Text {
                        run: job.run.clone(),
                        change: Change::Append {
                            key: MessageKey::native(key.into())?,
                            text: text.into(),
                        },
                    })
                    .map_err(|_| "Claude Code output owner is unavailable")?;
                }
                _ => (),
            },
            NativeEvent::Assistant(message) => {
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        let id = block["id"].as_str().ok_or("Claude Code tool has no id")?;
                        let name = block["name"].as_str().unwrap_or("tool").to_owned();
                        if tools.len() >= 16 || tools.contains_key(id) {
                            return Err("Claude Code tool identity repeated or exceeds bound".into());
                        }
                        let mut call = Call::new(id)?;
                        call.merge_claude(Some(&name), Some(&block["input"]), None);
                        report(job, tx, &mut activities, &name, "started", call.clone())?;
                        tools.insert(id.to_owned(), (name, call));
                    }
                }
            }
            NativeEvent::ToolResult(message) => {
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let Some((name, mut call)) = block["tool_use_id"].as_str().and_then(|id| tools.remove(id)) else {
                        continue;
                    };
                    call.merge_claude(None, None, Some(&block["content"]));
                    let phase = if block["is_error"] == true { "failed" } else { "completed" };
                    report(job, tx, &mut activities, &name, phase, call)?;
                }
            }
            NativeEvent::Retry { .. } | NativeEvent::Ready { .. } => (),
            NativeEvent::Finished { is_error, .. } => {
                if !pending.is_empty() {
                    return Err("Claude Code settled with unanswered permissions".into());
                }
                if failed.load(Ordering::Acquire) {
                    return Err("Claude Code browser service failed during the task".into());
                }
                return match (is_error, interrupted) {
                    (_, true) => Ok(Outcome::Cancelled),
                    (false, false) => Ok(Outcome::Complete),
                    (true, false) => Err("Claude Code reported an error result".into()),
                };
            }
        }
    }
}

fn report(
    job: &Job,
    tx: &mpsc::SyncSender<Event>,
    count: &mut usize,
    tool: &str,
    phase: &str,
    call: Call,
) -> Result<()> {
    *count += 1;
    if *count > 32 {
        return Err("Claude Code activity exceeds application bound".into());
    }
    tx.send(Event::HarnessActivity {
        run: job.run.clone(),
        activity: Activity {
            outcome: None,
            tool: tool.into(),
            phase: phase.into(),
            call: Some(call),
        },
    })
    .map_err(|_| "Claude Code activity owner is unavailable".into())
}

