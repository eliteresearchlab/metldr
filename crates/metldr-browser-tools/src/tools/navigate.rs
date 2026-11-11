use crate::{Call, Definition, NAVIGATE_TOOL, Result};
use serde_json::{Map, Value, json};

pub const NAME: &str = NAVIGATE_TOOL;
pub const ORDER: u8 = 2;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Propose navigation to an exact URL. The user must approve. Success returns a fresh observation; use only its new handles.",
        input_schema: json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"],"additionalProperties":false}),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 1 {
        return Err("invalid navigate_page input".into());
    }
    let destination = object
        .get("url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 2048 && !s.chars().any(char::is_control))
        .ok_or("invalid browser navigation URL")?
        .to_owned();
    Ok(Call::Navigate { destination })
}
