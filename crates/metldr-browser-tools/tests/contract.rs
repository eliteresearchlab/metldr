use metldr_browser_tools::{ActTool, Call, SecretKind, decode_call, definitions};
use serde_json::json;

#[test]
fn decodes_the_shared_browser_calls_and_bounds_untrusted_arguments() {
    assert_eq!(decode_call("propose_task_plan", json!({"goal":"Update field", "intended_effects":["Change value"], "scope":"Current page"})).unwrap(), Call::ProposeTaskPlan { goal: "Update field".into(), intended_effects: vec!["Change value".into()], scope: "Current page".into() });
    assert_eq!(
        decode_call("navigate_page", json!({"url":"https://example.com/report"})).unwrap(),
        Call::Navigate {
            destination: "https://example.com/report".into()
        }
    );
    assert_eq!(
        decode_call("snapshot_page", json!({"delta":true,"find":"Total"})).unwrap(),
        Call::Snapshot {
            args: json!({"delta":true,"find":"Total"})
        }
    );
    assert_eq!(
        decode_call("enter_secret", json!({"ref":"e3","kind":"one_time_code"})).unwrap(),
        Call::EnterSecret {
            reference: "e3".into(),
            kind: SecretKind::OneTimeCode
        }
    );
    // Every page action keeps its arguments as given once they pass the tool's checks.
    for (name, arguments) in [
        ("click", json!({"ref":"e12"})),
        (
            "click",
            json!({"ref":"f1e3","button":"right","clickCount":2,"modifiers":["Shift"]}),
        ),
        ("fill", json!({"ref":"e4","value":"ready\nline two"})),
        ("fill", json!({"ref":"e4","value":""})),
        ("fill", json!({"ref":"e5","checked":true})),
        (
            "fill",
            json!({"fields":[{"ref":"e4","value":"a"},{"ref":"e5","checked":false}],"submit":true}),
        ),
        ("type", json!({"text":"rust é","submit":true})),
        ("press", json!({"key":"Backspace","count":3})),
        ("press", json!({"ref":"e4","key":["Control+A","Delete"]})),
        ("press", json!({"key":"é"})),
        ("select", json!({"ref":"e6","values":["Red","Blue"]})),
        ("hover", json!({"ref":"a5"})),
        ("drag", json!({"from":"e7","to":"e8"})),
        ("scroll", json!({"deltaY":-600})),
        ("scroll", json!({"ref":"e9","to":"bottom"})),
    ] {
        assert_eq!(
            decode_call(name, arguments.clone()).unwrap(),
            Call::Act {
                tool: ActTool::parse(name).unwrap(),
                args: arguments
            },
            "{name}"
        );
    }
    for (name, arguments) in [
        (
            "propose_task_plan",
            json!({"goal":"", "intended_effects":[], "scope":"Current page"}),
        ),
        ("navigate_page", json!({})),
        ("navigate_page", json!({"url":"https://example.com/\n"})),
        (
            "navigate_page",
            json!({"url":"https://example.com","approve":true}),
        ),
        ("snapshot_page", json!({"within":"#main"})),
        ("snapshot_page", json!({"diff":true})),
        ("snapshot_page", json!([])),
        ("enter_secret", json!({"ref":"e1","kind":"card_number"})),
        (
            "enter_secret",
            json!({"ref":"e1","kind":"password","value":"hunter2"}),
        ),
        ("click", json!({})),
        ("click", json!({"ref":"E12"})),
        ("click", json!({"ref":"#submit"})),
        ("click", json!({"ref":"e".repeat(17)})),
        ("click", json!({"selector":"button"})),
        ("click", json!({"ref":"e1","x":10,"y":10})),
        ("click", json!({"ref":"e1","force":true})),
        ("click", json!({"ref":"e1","clickCount":4})),
        ("click", json!({"ref":"e1","approve":true})),
        ("fill", json!({"ref":"e1"})),
        ("fill", json!({"ref":"e1","value":"a","checked":true})),
        ("fill", json!({"ref":"e1","value":"\u{7}"})),
        ("fill", json!({"ref":"e1","value":"a".repeat(8193)})),
        (
            "fill",
            json!({"ref":"e1","value":"a","fields":[{"ref":"e2","value":"b"}]}),
        ),
        ("fill", json!({"fields":[]})),
        ("fill", json!({"fields":[{"ref":"e2"}]})),
        ("fill", json!({"value":"a"})),
        ("type", json!({"ref":"e1"})),
        ("type", json!({"text":""})),
        ("press", json!({"key":"Ctrl A"})),
        ("press", json!({"key":"Enter","count":51})),
        ("press", json!({"key":[]})),
        ("select", json!({"ref":"e1"})),
        ("select", json!({"values":"Red"})),
        ("hover", json!({"ref":"e1","x":1})),
        ("drag", json!({"from":"e1"})),
        ("drag", json!({"ref":"e1","to":"e2"})),
        ("scroll", json!({})),
        ("scroll", json!({"deltaY":20001})),
        ("scroll", json!({"deltaY":1.5})),
        ("scroll", json!({"to":"middle"})),
        ("settings", json!({"name":"pi.offline","value":false})),
    ] {
        assert!(
            decode_call(name, arguments.clone()).is_err(),
            "accepted {name}: {arguments}"
        );
    }
    let tools = definitions();
    assert_eq!(
        tools.each_ref().map(|tool| tool.name),
        [
            "snapshot_page",
            "navigate_page",
            "click",
            "fill",
            "type",
            "press",
            "select",
            "hover",
            "drag",
            "scroll",
            "page_logs",
            "status",
            "settings",
            "propose_task_plan",
            "network_requests",
            "enter_secret",
        ]
    );
    assert!(
        metldr_browser_tools::tools::TOOLS
            .windows(2)
            .all(|pair| pair[0].order < pair[1].order),
        "the registry order must ascend"
    );
    for tool in definitions() {
        assert_eq!(tool.input_schema["additionalProperties"], false);
    }
}
