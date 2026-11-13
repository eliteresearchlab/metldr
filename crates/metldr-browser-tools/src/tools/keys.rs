use super::schema;
use crate::{Call, Definition, KEY_TOOL, Key, Result};
use serde_json::{Map, Value, json};

pub const NAME: &str = KEY_TOOL;
pub const ORDER: u8 = 13;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Propose one key press in an observed text field: Enter, Tab, Escape, Backspace, an arrow key, Home, End, PageUp or PageDown. The user must approve before dispatch. Enter may submit a form.",
        input_schema: json!({
            "type":"object",
            "properties":{
                "handle":schema::handle(),
                "key":{"type":"string","enum":Key::ALL.map(Key::as_str)}
            },
            "required":["handle","key"],
            "additionalProperties":false
        }),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 2 || !object.contains_key("handle") || !object.contains_key("key") {
        return Err("invalid press_key input".into());
    }
    let handle = schema::handle_argument(object.get("handle"), "invalid key target handle")?;
    let key = object["key"]
        .as_str()
        .and_then(Key::parse)
        .ok_or("unsupported key")?;
    Ok(Call::PressKey { handle, key })
}
