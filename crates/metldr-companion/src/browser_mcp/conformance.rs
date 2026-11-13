//! MCP conformance checks.
//!
//! These pin the two things a later tool or revision must not change by
//! accident: the revision this server negotiates, and the exact bytes of the
//! `tools/list` and `tools/call` replies.

use super::*;

// The fixture keeps a trailing newline so the file stays readable; the wire
// body never ends with one.
const TOOLS_LIST: &str = include_str!("golden/tools_list.json");
const RESPONSES: &str = include_str!("golden/responses.json");

/// One request/reply exchange over the native listener.
async fn exchange(server: &Server, version: Option<&str>, body: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (tx, _rx) = mpsc::channel(8);
    let active = Arc::new(Calls::default());
    let client = async {
        let mut socket = TcpStream::connect(server.address).await.unwrap();
        let mut request = format!(
            "POST /mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\n",
            server.address, server.capability
        );
        if let Some(version) = version {
            request.push_str(&format!("Mcp-Protocol-Version: {version}\r\n"));
        }
        request.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        socket.read_to_end(&mut reply).await.unwrap();
        String::from_utf8(reply).unwrap()
    };
    let service = async {
        let (socket, _) = server.listener.accept().await.unwrap();
        handle_connection(
            socket,
            server.address,
            &server.capability,
            &server.run,
            tx,
            active,
        )
        .await
        .unwrap();
    };
    let (reply, _) = timeout(Duration::from_secs(2), async {
        tokio::join!(client, service)
    })
    .await
    .unwrap();
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_ascii_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, body.to_owned())
}

/// One JSON-RPC message answered by the dispatcher, with an optional tool reply.
async fn respond(message: Value, reply: Option<std::result::Result<Value, String>>) -> String {
    let (tx, mut rx) = mpsc::channel(8);
    let active = Arc::new(Calls::default());
    let responder = async move {
        if let Some(reply) = reply
            && let Some(Inbound::Call(request)) = rx.recv().await
        {
            let _ = request.reply.send(reply.map(Arc::new));
        }
    };
    let message = process_message(message, "owned:2", tx, active, None);
    let (result, _) = tokio::join!(message, responder);
    result.unwrap().unwrap()
}

#[test]
fn every_supported_revision_negotiates_to_the_newest_supported_one() {
    for revision in protocol::SUPPORTED {
        assert_eq!(protocol::negotiate(revision), protocol::latest());
        assert!(protocol::accepts_header(revision));
    }
    for older in ["2025-06-18", "2025-03-26", "2024-11-05", "2024-10-07", ""] {
        assert_eq!(protocol::negotiate(older), protocol::latest());
        assert!(
            !protocol::accepts_header(older),
            "an unnegotiated revision must not be accepted"
        );
    }
}

#[tokio::test]
async fn later_requests_must_carry_the_negotiated_revision() {
    let server = Server::bind("0123456789abcdef0123456789abcdef:2")
        .await
        .unwrap();
    let list = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    for (version, expected) in [
        (Some(protocol::latest()), 200),
        (Some("2025-03-26"), 400),
        (Some("2024-11-05"), 400),
        (None, 400),
    ] {
        let (status, _) = exchange(&server, version, list).await;
        assert_eq!(status, expected, "unexpected status for {version:?}");
    }
}

#[tokio::test]
async fn tools_list_and_tool_responses_keep_their_exact_bytes() {
    assert_eq!(
        respond(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}), None).await,
        TOOLS_LIST.trim_end_matches('\n'),
        "the tools/list wire body changed"
    );
    let expected: std::collections::BTreeMap<String, String> =
        serde_json::from_str(RESPONSES).unwrap();
    for (case, message, reply) in [
        (
            "observe",
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"observe_page","arguments":{}}}),
            Some(Ok(
                json!({"url":"https://example.com/","title":"Example","text":"Body","fields":[],"buttons":[]}),
            )),
        ),
        (
            "navigate",
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"navigate_page","arguments":{"url":"https://example.com/report"}}}),
            Some(Ok(json!({"observed":true}))),
        ),
        (
            "unknown tool",
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"unknown_tool","arguments":{}}}),
            None,
        ),
        (
            "invalid arguments",
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"set_field_value","arguments":{"handle":"field_1"}}}),
            None,
        ),
        (
            "failed dispatch",
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"observe_page","arguments":{}}}),
            Some(Err("browser observation failed".into())),
        ),
        (
            "initialize",
            json!({"jsonrpc":"2.0","id":7,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
            None,
        ),
        (
            "ping",
            json!({"jsonrpc":"2.0","id":8,"method":"ping"}),
            None,
        ),
        (
            "unknown method",
            json!({"jsonrpc":"2.0","id":9,"method":"nope"}),
            None,
        ),
    ] {
        assert_eq!(
            respond(message, reply).await,
            expected[case],
            "the {case} wire body changed"
        );
    }
}

#[tokio::test]
async fn tools_list_advertises_every_registered_tool_in_order() {
    let first = respond(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}), None).await;
    let second = respond(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}), None).await;
    assert_eq!(
        first.replace("\"id\":1", "\"id\":2"),
        second,
        "tools/list is not deterministic"
    );
    let value: Value = serde_json::from_str(&first).unwrap();
    let tools = value["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    let registered: Vec<&str> = metldr_browser_tools::tools::TOOLS
        .iter()
        .map(|tool| tool.name)
        .collect();
    assert_eq!(names, registered);
    for tool in tools {
        let annotations = tool["annotations"].as_object().unwrap();
        assert_eq!(annotations.len(), 4, "a tool lost a safety hint");
    }
}
