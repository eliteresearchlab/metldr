use super::schema;
use crate::{ACTIVATE_TOOL, Call, Definition, Result};
use serde_json::{Map, Value, json};

pub const NAME: &str = ACTIVATE_TOOL;
pub const ORDER: u8 = 3;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Propose a click on an observed button. The user must approve before dispatch.",
        input_schema: json!({"type":"object","properties":{"handle":schema::handle()},"required":["handle"],"additionalProperties":false}),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 1 || !object.contains_key("handle") {
        return Err("invalid activate_button input".into());
    }
    let handle = schema::handle_argument(object.get("handle"), "invalid browser button handle")?;
    Ok(Call::ActivateButton { handle })
}
