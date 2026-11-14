use metldr_claude_session::{ClaudeSession, Event, Launch, Session, SessionId};
use std::{io::{BufRead, BufReader, Read, Write}, net::TcpListener, path::PathBuf, sync::{Arc, Mutex}};

fn executable() -> Option<PathBuf> {
    std::env::var_os("METLDR_CLAUDE").map(PathBuf::from).or_else(|| {
        let home = std::env::var_os("HOME")?;
        let path = PathBuf::from(home).join(".local/share/mise/installs/claude/latest/claude");
        path.exists().then_some(path)
    })
}

/// A loopback Messages endpoint that asks to run `command` on each odd request and ends the turn
/// on each even one. It records request bodies.
fn fixture(command: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = bodies.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut len = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" { break; }
                if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") { len = v.trim().parse().unwrap(); }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            if !line.starts_with("POST /v1/messages?") && !line.starts_with("POST /v1/messages ") {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n");
                continue;
            }
            let count = { let mut b = seen.lock().unwrap(); b.push(String::from_utf8_lossy(&body).into_owned()); b.len() };
            let block = if count % 2 == 1 {
                format!(r#"{{"type":"tool_use","id":"toolu_{count}","name":"Bash","input":{{}}}}"#)
            } else { r#"{"type":"text","text":""}"#.into() };
            let delta = if count % 2 == 1 {
                serde_json::json!({"type":"input_json_delta","partial_json":serde_json::json!({"command":command,"description":"write the probe"}).to_string()}).to_string()
            } else { r#"{"type":"text_delta","text":"Finished."}"#.into() };
            let stop = if count % 2 == 1 { "tool_use" } else { "end_turn" };
            let events = [
                ("message_start", r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-fixture","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}"#.to_string()),
                ("content_block_start", format!(r#"{{"type":"content_block_start","index":0,"content_block":{block}}}"#)),
                ("content_block_delta", format!(r#"{{"type":"content_block_delta","index":0,"delta":{delta}}}"#)),
                ("content_block_stop", r#"{"type":"content_block_stop","index":0}"#.into()),
                ("message_delta", format!(r#"{{"type":"message_delta","delta":{{"stop_reason":"{stop}"}},"usage":{{"output_tokens":2}}}}"#)),
                ("message_stop", r#"{"type":"message_stop"}"#.into()),
            ];
            let sse: String = events.iter().map(|(e, d)| format!("event: {e}\ndata: {d}\n\n")).collect();
            let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{sse}", sse.len()).as_bytes());
        }
    });
    (port, bodies)
}

async fn turn(session: &mut ClaudeSession, prompt: &str, allow: bool) -> (bool, Vec<String>) {
    session.prompt(prompt).await.unwrap();
    let mut asked = false;
    let mut results = Vec::new();
    loop {
        match session.next().await.unwrap().expect("Claude Code ended early") {
            Event::Permission(permission) => {
                assert_eq!(permission.tool_name, "Bash");
                asked = true;
                session.answer(&permission, allow, "The user declined this step.").await.unwrap();
            }
            Event::ToolResult(message) => results.push(message.to_string()),
            Event::Finished { is_error, .. } => { assert!(!is_error); return (asked, results); }
            _ => {}
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn permission_prompts_return_to_metldr_and_sessions_resume() {
    let Some(executable) = executable() else { eprintln!("Claude Code is not installed; skipped"); return; };
    let (port, bodies) = fixture("touch metldr-probe.txt");
    let dir = std::env::temp_dir().join(format!("metldr-claude-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let env = [
        ("HOME", dir.join("home").display().to_string()),
        ("CLAUDE_CONFIG_DIR", dir.join("home/.claude").display().to_string()),
        ("ANTHROPIC_API_KEY", "sk-ant-fixture".into()),
        ("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}")),
        ("CLAUDE_CODE_MAX_RETRIES", "0".into()),
        ("DISABLE_TELEMETRY", "1".into()),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
        ("DISABLE_AUTOUPDATER", "1".into()),
    ];
    let id = SessionId::from_random([7; 16]);
    let launch = |session| Launch { executable: &executable, workspace: &dir, session, model: None, servers: &[], env: &env };
    let mut session = ClaudeSession::open(launch(Session::New(id.clone()))).unwrap();
    let (asked, results) = turn(&mut session, "write the probe", false).await;
    assert!(asked && results.iter().any(|r| r.contains("declined")), "{results:?}");
    assert!(!dir.join("metldr-probe.txt").exists(), "a denied step must not run");
    assert!(session.close().await.unwrap().success());
    let mut session = ClaudeSession::open(launch(Session::Resume(id))).unwrap();
    let (asked, _) = turn(&mut session, "try again", true).await;
    assert!(asked && dir.join("metldr-probe.txt").exists(), "an allowed step runs");
    assert!(bodies.lock().unwrap().last().unwrap().contains("write the probe"), "the resumed session keeps its history");
    session.close().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
