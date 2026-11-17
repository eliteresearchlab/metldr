//! The page actions: click, fill, type, press, select, hover, drag and scroll on refs from the
//! latest snapshot. Each runs the browser's own trusted input once the user (or the chat's
//! approval mode) allows it. Arguments are checked here and passed on unchanged.

use crate::{ActTool, Call, Definition, Result, valid_ref};
use serde_json::{Map, Value, json};

const TEXT_MAX: usize = 8192;
const KEY_MAX: usize = 64;

fn reference() -> Value {
    json!({"type":"string","minLength":1,"maxLength":16,"pattern":"^[a-z0-9]+$","description":"A ref from the latest snapshot_page, e.g. \"e12\"."})
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

pub(super) fn definition(tool: ActTool) -> Definition {
    let (description, input_schema) = match tool {
        ActTool::Click => (
            "Click an element. It waits until the element is visible, enabled and still, scrolls it into view and fails naming whatever covers it. Returns what changed: navigation, a dialog, new tabs.",
            schema(
                json!({"ref":reference(),"button":{"type":"string","enum":["left","right","middle"]},"clickCount":{"type":"integer","minimum":1,"maximum":3},"modifiers":{"type":"array","maxItems":4,"items":{"type":"string","enum":["Shift","Control","Alt","Meta","ControlOrMeta"]}}}),
                &["ref"],
            ),
        ),
        ActTool::Fill => (
            "Replace what a field holds with trusted input: text fields, textareas and editors get value, selects pick an option by value or label, checkboxes, radios and switches take checked. Fill several fields with fields. submit presses Enter after the last one.",
            schema(
                json!({"ref":reference(),"value":{"type":"string","maxLength":TEXT_MAX},"checked":{"type":"boolean"},"fields":{"type":"array","minItems":1,"maxItems":32,"items":schema(json!({"ref":reference(),"value":{"type":"string","maxLength":TEXT_MAX},"checked":{"type":"boolean"}}),&["ref"])},"submit":{"type":"boolean"}}),
                &[],
            ),
        ),
        ActTool::Type => (
            "Type text key by key into ref, or into the focused element. It keeps what is there and fires real key events: use it for search boxes with suggestions and editors. submit presses Enter afterwards.",
            schema(
                json!({"ref":reference(),"text":{"type":"string","minLength":1,"maxLength":TEXT_MAX},"submit":{"type":"boolean"}}),
                &["text"],
            ),
        ),
        ActTool::Press => (
            "Press a key or shortcut on ref, or on the focused element: \"Enter\", \"Escape\", \"Tab\", \"Backspace\", \"ArrowDown\", \"Control+A\", \"Shift+Tab\". key may be a list pressed in order; count repeats it.",
            schema(
                json!({"ref":reference(),"key":{"anyOf":[{"type":"string","minLength":1,"maxLength":KEY_MAX},{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string","minLength":1,"maxLength":KEY_MAX}}]},"count":{"type":"integer","minimum":1,"maximum":50}}),
                &["key"],
            ),
        ),
        ActTool::Select => (
            "Choose option(s) of a select by value or label. For custom dropdowns click the control, take a snapshot, then click the option.",
            schema(
                json!({"ref":reference(),"values":{"anyOf":[{"type":"string","maxLength":512},{"type":"array","minItems":1,"maxItems":32,"items":{"type":"string","maxLength":512}}]}}),
                &["ref", "values"],
            ),
        ),
        ActTool::Hover => (
            "Move the pointer over an element to open a menu or tooltip, then take a snapshot to see what appeared.",
            schema(json!({"ref":reference()}), &["ref"]),
        ),
        ActTool::Drag => (
            "Drag one element onto another with the mouse, HTML drag and drop included.",
            schema(
                json!({"from":reference(),"to":reference()}),
                &["from", "to"],
            ),
        ),
        ActTool::Scroll => (
            "Scroll the page, or ref, with the mouse wheel by deltaX and deltaY pixels, jump to an edge with to, or bring ref into view. Returns whether the end was reached.",
            schema(
                json!({"ref":reference(),"deltaX":{"type":"integer","minimum":-20000,"maximum":20000},"deltaY":{"type":"integer","minimum":-20000,"maximum":20000},"to":{"type":"string","enum":["top","bottom","left","right"]}}),
                &[],
            ),
        ),
    };
    Definition {
        name: tool.as_str(),
        description,
        input_schema,
    }
}

fn text(value: &Value, min: usize, max: usize) -> Option<&str> {
    value.as_str().filter(|text| {
        (min..=max).contains(&text.len())
            && !text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    })
}

fn key(value: &Value) -> bool {
    value.as_str().is_some_and(|key| {
        (1..=KEY_MAX).contains(&key.len())
            && (key.chars().count() == 1 && !key.chars().any(char::is_control)
                || key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+'))
    })
}

fn integer(value: &Value, range: std::ops::RangeInclusive<i64>) -> bool {
    value.as_i64().is_some_and(|n| range.contains(&n))
}

fn reference_ok(value: &Value) -> bool {
    value.as_str().is_some_and(valid_ref)
}

/// One field of fill: a ref with a value or checked, never both.
fn field(value: &Value) -> bool {
    let Some(field) = value.as_object() else {
        return false;
    };
    field.get("ref").is_some_and(reference_ok)
        && field.len() == 2
        && (field
            .get("value")
            .is_some_and(|v| text(v, 0, TEXT_MAX).is_some())
            || field.get("checked").is_some_and(Value::is_boolean))
}

pub(super) fn decode(tool: ActTool, object: &Map<String, Value>) -> Result<Call> {
    let valid = object
        .iter()
        .all(|(name, value)| match (tool, name.as_str()) {
            (_, "ref") if tool != ActTool::Drag => reference_ok(value),
            (ActTool::Click, "button") => {
                matches!(value.as_str(), Some("left" | "right" | "middle"))
            }
            (ActTool::Click, "clickCount") => integer(value, 1..=3),
            (ActTool::Click, "modifiers") => value.as_array().is_some_and(|keys| {
                keys.len() <= 4
                    && keys.iter().all(|key| {
                        matches!(
                            key.as_str(),
                            Some("Shift" | "Control" | "Alt" | "Meta" | "ControlOrMeta")
                        )
                    })
            }),
            (ActTool::Fill, "value") => text(value, 0, TEXT_MAX).is_some(),
            (ActTool::Fill, "checked") | (ActTool::Fill | ActTool::Type, "submit") => {
                value.is_boolean()
            }
            (ActTool::Fill, "fields") => value
                .as_array()
                .is_some_and(|fields| (1..=32).contains(&fields.len()) && fields.iter().all(field)),
            (ActTool::Type, "text") => text(value, 1, TEXT_MAX).is_some(),
            (ActTool::Press, "key") => {
                key(value)
                    || value
                        .as_array()
                        .is_some_and(|keys| (1..=16).contains(&keys.len()) && keys.iter().all(key))
            }
            (ActTool::Press, "count") => integer(value, 1..=50),
            (ActTool::Select, "values") => {
                text(value, 0, 512).is_some()
                    || value.as_array().is_some_and(|values| {
                        (1..=32).contains(&values.len())
                            && values.iter().all(|v| text(v, 0, 512).is_some())
                    })
            }
            (ActTool::Drag, "from" | "to") => reference_ok(value),
            (ActTool::Scroll, "deltaX" | "deltaY") => integer(value, -20000..=20000),
            (ActTool::Scroll, "to") => {
                matches!(value.as_str(), Some("top" | "bottom" | "left" | "right"))
            }
            _ => false,
        });
    let has = |name| object.contains_key(name);
    let complete = match tool {
        ActTool::Click | ActTool::Hover => has("ref"),
        ActTool::Fill => {
            has("fields") != has("ref")
                && (!has("ref") || has("value") != has("checked"))
                && (has("ref") || !has("value") && !has("checked"))
        }
        ActTool::Type => has("text"),
        ActTool::Press => has("key"),
        ActTool::Select => has("ref") && has("values"),
        ActTool::Drag => has("from") && has("to"),
        ActTool::Scroll => !object.is_empty(),
    };
    if !valid || !complete {
        return Err(format!("invalid {} input", tool.as_str()).into());
    }
    Ok(Call::Act {
        tool,
        args: Value::Object(object.clone()),
    })
}

macro_rules! act_tools {
    ($($tool:ident: $definition:ident, $decode:ident;)*) => {$(
        pub fn $definition() -> Definition {
            definition(ActTool::$tool)
        }
        pub fn $decode(object: &Map<String, Value>) -> Result<Call> {
            decode(ActTool::$tool, object)
        }
    )*};
}

act_tools! {
    Click: click_definition, click_decode;
    Fill: fill_definition, fill_decode;
    Type: type_definition, type_decode;
    Press: press_definition, press_decode;
    Select: select_definition, select_decode;
    Hover: hover_definition, hover_decode;
    Drag: drag_definition, drag_decode;
    Scroll: scroll_definition, scroll_decode;
}
