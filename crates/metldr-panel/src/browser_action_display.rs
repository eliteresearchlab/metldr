use crate::model::{
    BrowserActionIntent, BrowserActionOutcome, BrowserActionPhase, BrowserActionView,
    BrowserElement,
};
use serde_json::Value;

pub(crate) struct Display {
    pub(crate) title: &'static str,
    pub(crate) status: String,
    pub(crate) receipt: bool,
    pub(crate) details: Vec<Detail>,
}

pub(crate) struct Detail {
    pub(crate) part: &'static str,
    pub(crate) label: &'static str,
    pub(crate) value: String,
}

/// The card title of a page action: to approve, and once it ran.
fn act_title(tool: &str) -> (&'static str, &'static str) {
    match tool {
        "click" => ("Approve click", "Click"),
        "fill" => ("Approve fill", "Fill"),
        "type" => ("Approve typing", "Typing"),
        "press" => ("Approve key press", "Key press"),
        "select" => ("Approve option choice", "Option choice"),
        "hover" => ("Approve hover", "Hover"),
        "drag" => ("Approve drag", "Drag"),
        _ => ("Approve scroll", "Scroll"),
    }
}

/// How an element reads on the card: `button "Save"`.
fn named(element: &BrowserElement) -> String {
    if element.name.is_empty() {
        element.role.clone()
    } else {
        format!("{} \"{}\"", element.role, element.name)
    }
}

/// One text, or a list joined with commas.
fn listed(value: &Value) -> String {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn checked(value: &Value) -> String {
    if value == true {
        "Checked"
    } else {
        "Not checked"
    }
    .to_owned()
}

pub(crate) fn display(action: &BrowserActionView) -> Display {
    let pending = action.phase == BrowserActionPhase::Pending;
    let title = match &action.intent {
        BrowserActionIntent::Navigate { .. } if pending => "Approve navigation",
        BrowserActionIntent::Navigate { .. } => "Navigation",
        BrowserActionIntent::Act { tool, .. } => {
            let (ask, done) = act_title(tool);
            if pending { ask } else { done }
        }
        BrowserActionIntent::EnterSecret { .. } if pending => "Enter a secret",
        BrowserActionIntent::EnterSecret { .. } => "Secret entry",
    };
    let mut details = Vec::new();
    let mut add = |part, label, value: String| details.push(Detail { part, label, value });
    match &action.intent {
        BrowserActionIntent::EnterSecret {
            element, secret, ..
        } => {
            add("destination", "Site", origin(&action.url));
            add("field", "Field", named(element));
            add("secret", "Kind", secret.label().to_owned());
        }
        BrowserActionIntent::Navigate { destination } => {
            add(
                "destination",
                "Destination",
                format!("{} → {destination}", action.url),
            );
        }
        BrowserActionIntent::Act {
            tool,
            args,
            elements,
        } => {
            add("destination", "Page", action.url.clone());
            let target = |index: usize| elements.get(index).map(named);
            match tool.as_str() {
                "fill" if args["fields"].is_array() => {
                    for (field, element) in args["fields"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .zip(elements)
                    {
                        add("field", "Field", named(element));
                        if let Some(value) = field["value"].as_str() {
                            add("value", "Value", value.to_owned());
                        } else {
                            add("value", "State", checked(&field["checked"]));
                        }
                    }
                }
                "fill" => {
                    add("field", "Field", target(0).unwrap_or_default());
                    if let Some(value) = args["value"].as_str() {
                        add("value", "Value", value.to_owned());
                    } else {
                        add("value", "State", checked(&args["checked"]));
                    }
                }
                "type" => {
                    add(
                        "field",
                        "Field",
                        target(0).unwrap_or_else(|| "The focused element".into()),
                    );
                    add("value", "Text", listed(&args["text"]));
                }
                "press" => {
                    add(
                        "field",
                        "Element",
                        target(0).unwrap_or_else(|| "The focused element".into()),
                    );
                    let mut keys = listed(&args["key"]);
                    if let Some(count) = args["count"].as_u64().filter(|count| *count > 1) {
                        keys = format!("{keys} × {count}");
                    }
                    add("key", "Keys", keys);
                }
                "select" => {
                    add("field", "Control", target(0).unwrap_or_default());
                    add("value", "Choice", listed(&args["values"]));
                }
                "drag" => {
                    add("button", "From", target(0).unwrap_or_default());
                    add("button", "To", target(1).unwrap_or_default());
                }
                "scroll" => {
                    add(
                        "scroll-target",
                        "Target",
                        target(0).unwrap_or_else(|| "The page".into()),
                    );
                    let motion = match (args["to"].as_str(), &args["deltaX"], &args["deltaY"]) {
                        (Some(edge), _, _) => format!("To the {edge}"),
                        (None, Value::Null, Value::Null) => "Into view".to_owned(),
                        (None, x, y) => format!(
                            "{} px across, {} px down",
                            x.as_i64().unwrap_or(0),
                            y.as_i64().unwrap_or(0)
                        ),
                    };
                    add("scroll-motion", "Scroll", motion);
                }
                _ => add("button", "Element", target(0).unwrap_or_default()),
            }
            if args["submit"] == true {
                add("key", "Then", "Press Enter".to_owned());
            }
            if let Some(result) = &action.result {
                add("result", "What changed", result.clone());
            }
        }
    }
    let status = match action.outcome {
        Some(BrowserActionOutcome::Acted) => "Done on this page".to_owned(),
        Some(BrowserActionOutcome::SecretEntered) => "Secret entered on this page".to_owned(),
        _ => format!(
            "{}{}{}",
            action.phase,
            action
                .outcome
                .map(|outcome| format!(" ({outcome})"))
                .unwrap_or_default(),
            action
                .reason
                .as_ref()
                .map(|reason| format!(": {reason}"))
                .unwrap_or_default()
        ),
    };
    Display {
        title,
        status,
        receipt: action.phase == BrowserActionPhase::Settled
            && matches!(
                action.outcome,
                Some(BrowserActionOutcome::Verified | BrowserActionOutcome::Acted)
            ),
        details,
    }
}

/// The scheme, host and port of `url`, which is all a secret entry card shows of the page.
pub(crate) fn origin(url: &str) -> String {
    let rest = url.find("://").map_or(0, |at| at + 3);
    let end = url[rest..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |at| rest + at);
    url[..end].to_owned()
}
