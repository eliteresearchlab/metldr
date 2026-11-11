use crate::{Call, Definition, OBSERVE_TOOL, Result};
use serde_json::{Map, Value, json};

pub const NAME: &str = OBSERVE_TOOL;
pub const ORDER: u8 = 0;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Observe the current page and discover writable text fields, buttons, native checkbox and radio controls, single-select options, and scroll targets.",
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.is_empty() {
        Ok(Call::Observe)
    } else {
        Err("invalid observe_page input".into())
    }
}
