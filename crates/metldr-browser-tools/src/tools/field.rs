use super::schema;
use crate::{Call, Definition, FIELD_TOOL, Result};
use serde_json::{Map, Value, json};

pub const NAME: &str = FIELD_TOOL;
pub const ORDER: u8 = 1;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Propose a text field change on the current page. The user must approve before dispatch.",
        input_schema: json!({
            "type":"object",
            "properties":{"handle":schema::handle(),"value":{"type":"string"}},
            "required":["handle","value"],
            "additionalProperties":false
        }),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 2 || !object.contains_key("handle") || !object.contains_key("value") {
        return Err("invalid set_field_value input".into());
    }
    let handle = schema::handle_argument(object.get("handle"), "invalid browser field handle")?;
    let value = object["value"]
        .as_str()
        .filter(|s| s.len() <= 1024)
        .ok_or("invalid browser field value")?
        .to_owned();
    Ok(Call::Change { handle, value })
}
