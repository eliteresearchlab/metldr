use crate::{Call, Definition, OBSERVE_TOOL, Result};
use serde_json::{Map, Value, json};

pub const NAME: &str = OBSERVE_TOOL;
pub const ORDER: u8 = 0;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Observe the current page and discover writable text fields, buttons, native checkbox and radio controls, single-select options, and scroll targets.",
        input_schema: json!({"type":"object","properties":{"within":{"type":"string","description":"Scope the observation to a ref from the latest observation."}},"additionalProperties":false}),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.is_empty() {
        Ok(Call::Observe)
    } else if object.len() == 1
        && let Some(within) = object.get("within").and_then(Value::as_str)
        && crate::valid_handle(within)
    {
        Ok(Call::ObserveWithin {
            within: within.into(),
        })
    } else {
        Err("invalid observe_page input".into())
    }
}
